//! Segment writer thread (ADR-0002): muxes encoded packets into ~2 s MPEG-TS
//! files, cut at keyframes, feeds the [`Ring`] and deletes expired files.
//!
//! It runs off the capture/encode path at below-normal priority. File IO here
//! never blocks the encoder, which hands packets over with `try_send`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use ffmpeg::{Packet, Rational, codec, format};
use ffmpeg_next as ffmpeg;

use crate::ring::{Pin, Ring, RingError, Segment};

pub enum WriterMsg {
    Packet(Packet),
    /// Pin the last `len_us` at the next keyframe. The writer cuts the current
    /// segment there so the clip runs right up to the moment of the request.
    Save {
        len_us: i64,
        reply: Sender<Result<Pin, RingError>>,
    },
    /// The timeline was interrupted (stall, resume). Close the current segment
    /// and start a new capture run that clips won't concatenate across.
    NewRun,
    Stop,
}

#[derive(Debug, thiserror::Error)]
pub enum WriterError {
    #[error("muxer: {0}")]
    Ffmpeg(#[from] ffmpeg::Error),
    #[error("segment file: {0}")]
    Io(#[from] std::io::Error),
    #[error("ring: {0}")]
    Ring(#[from] RingError),
    #[error("writer thread panicked")]
    Panicked,
}

pub fn segment_path(dir: &Path, seq: u64) -> PathBuf {
    dir.join(format!("{seq:08}.ts"))
}

/// Deletes segment files. Failures are left for the crash-recovery sweep.
pub fn delete_segments(dir: &Path, segments: &[Segment]) {
    for s in segments {
        let _ = fs::remove_file(segment_path(dir, s.seq));
    }
}

struct OpenSegment {
    out: format::context::Output,
    out_tb: Rational,
    seq: u64,
    first_pts: i64,
}

pub struct SegmentWriter {
    dir: PathBuf,
    params: codec::Parameters,
    /// Encoder time base (1/fps): packet pts are frame indices.
    time_base: Rational,
    segment_frames: i64,
    run: u32,
    next_seq: u64,
    current: Option<OpenSegment>,
    last_pts: i64,
    pending: Vec<(i64, Sender<Result<Pin, RingError>>)>,
    ring: Arc<Mutex<Ring>>,
}

// SAFETY: the FFmpeg objects are owned by this writer and used only on its thread.
unsafe impl Send for SegmentWriter {}

impl SegmentWriter {
    pub fn new(
        dir: PathBuf,
        params: codec::Parameters,
        time_base: Rational,
        segment_frames: i64,
        ring: Arc<Mutex<Ring>>,
    ) -> Self {
        Self {
            dir,
            params,
            time_base,
            segment_frames: segment_frames.max(1),
            run: 0,
            next_seq: 0,
            current: None,
            last_pts: -1,
            pending: Vec::new(),
            ring,
        }
    }

    pub fn run(mut self, rx: Receiver<WriterMsg>) -> Result<(), WriterError> {
        let result = self.run_inner(&rx);
        // Close the last segment and answer outstanding saves, even after an error.
        let end = self.last_pts + 1;
        let closed = self.close_current(end);
        self.serve_pending();
        result.and(closed)
    }

    fn run_inner(&mut self, rx: &Receiver<WriterMsg>) -> Result<(), WriterError> {
        while let Ok(msg) = rx.recv() {
            match msg {
                WriterMsg::Packet(p) => self.on_packet(p)?,
                WriterMsg::Save { len_us, reply } => self.pending.push((len_us, reply)),
                WriterMsg::NewRun => {
                    self.close_current(self.last_pts + 1)?;
                    self.serve_pending();
                    self.run += 1;
                }
                WriterMsg::Stop => break,
            }
        }
        Ok(())
    }

    fn on_packet(&mut self, mut p: Packet) -> Result<(), WriterError> {
        let pts = p.pts().unwrap_or(self.last_pts + 1);
        if p.is_key() {
            let cut = match &self.current {
                None => true,
                Some(c) => !self.pending.is_empty() || pts - c.first_pts >= self.segment_frames,
            };
            if cut {
                self.close_current(pts)?;
                self.serve_pending();
                self.open(pts)?;
            }
        }
        if let Some(c) = self.current.as_mut() {
            p.rescale_ts(self.time_base, c.out_tb);
            p.set_stream(0);
            p.set_position(-1);
            p.write(&mut c.out)?;
        }
        // Packets before the first keyframe can't be decoded, so they're dropped.
        self.last_pts = pts;
        Ok(())
    }

    fn open(&mut self, first_pts: i64) -> Result<(), WriterError> {
        let seq = self.next_seq;
        self.next_seq += 1;
        let path = segment_path(&self.dir, seq);
        let mut out = format::output_as(&path, "mpegts")?;
        {
            let mut st = out.add_stream(ffmpeg::encoder::find(codec::Id::None))?;
            st.set_parameters(self.params.clone());
            st.set_time_base(self.time_base);
            unsafe { (*st.parameters().as_mut_ptr()).codec_tag = 0 };
        }
        out.write_header()?;
        let out_tb = out
            .stream(0)
            .map(|s| s.time_base())
            .unwrap_or(Rational::new(1, 90_000));
        self.current = Some(OpenSegment {
            out,
            out_tb,
            seq,
            first_pts,
        });
        Ok(())
    }

    fn close_current(&mut self, end_pts: i64) -> Result<(), WriterError> {
        let Some(mut c) = self.current.take() else {
            return Ok(());
        };
        c.out.write_trailer()?;
        drop(c.out); // closes the file
        let path = segment_path(&self.dir, c.seq);
        let frames = end_pts - c.first_pts;
        if frames <= 0 {
            let _ = fs::remove_file(&path);
            return Ok(());
        }
        let duration_us = frames * 1_000_000 * i64::from(self.time_base.numerator())
            / i64::from(self.time_base.denominator().max(1));
        let bytes = fs::metadata(&path)?.len();
        let seg = Segment {
            seq: c.seq,
            run: self.run,
            duration_us,
            bytes,
        };
        let expired = self.ring.lock().map_err(|_| RingError::Empty)?.push(seg)?;
        delete_segments(&self.dir, &expired);
        Ok(())
    }

    fn serve_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let mut orphaned = Vec::new();
        if let Ok(mut ring) = self.ring.lock() {
            for (len_us, reply) in self.pending.drain(..) {
                // If the save job gave up waiting, release its pin straight
                // away, or those segments would stay on disk forever.
                if let Err(std::sync::mpsc::SendError(Ok(pin))) = reply.send(ring.pin_last(len_us))
                {
                    orphaned.extend(ring.unpin(pin.id).unwrap_or_default());
                }
            }
        }
        delete_segments(&self.dir, &orphaned);
    }
}
