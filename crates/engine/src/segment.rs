//! Segment writer thread (ADR-0002): muxes encoded packets into ~2 s MPEG-TS
//! files cut at video keyframes, feeds the [`Ring`] and deletes expired files.
//!
//! Stream 0 is video and streams 1.. are audio. Audio runs about 100 ms behind
//! video (mixer latency), so each audio packet goes to a segment by
//! *timestamp*, not arrival time. After a cut, the old segment stays open
//! ("closing") until every audio stream has passed the cut point, or a short
//! deadline expires. Only then is it added to the ring and any saves waiting
//! on it are served. That way clips have no audio holes at either end.
//!
//! It runs below normal priority. The encoder threads never block on it.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SendError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ffmpeg::{Packet, Rational, Rescale, codec, format};
use ffmpeg_next as ffmpeg;

use crate::ring::{Pin, Ring, RingError, Segment};

/// How long a closing segment waits for late audio before it's finalised anyway.
const AUDIO_GRACE: Duration = Duration::from_millis(500);
const MICROS: Rational = Rational(1, 1_000_000);

pub type SaveReply = Sender<Result<Pin, RingError>>;

pub enum WriterMsg {
    Packet(Packet),
    /// Pin the last `len_us` at the next video keyframe. The writer cuts there,
    /// so the clip runs right up to the moment of the request.
    Save {
        len_us: i64,
        reply: SaveReply,
    },
    /// The timeline was interrupted (stall, resume). Close everything and
    /// start a new capture run that clips won't concatenate across.
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

/// One elementary stream in every segment.
#[derive(Clone)]
pub struct StreamSpec {
    pub params: codec::Parameters,
    /// Time base of the packets the encoder produces.
    pub time_base: Rational,
}

pub fn segment_path(dir: &Path, seq: u64) -> PathBuf {
    dir.join(format!("{seq:08}.ts"))
}

/// Deletes segment files. Failures are left for the stale-session sweep.
pub fn delete_segments(dir: &Path, segments: &[Segment]) {
    for s in segments {
        let _ = fs::remove_file(segment_path(dir, s.seq));
    }
}

struct OpenSegment {
    out: format::context::Output,
    out_tbs: Vec<Rational>,
    seq: u64,
    first_pts: i64,
}

struct Closing {
    seg: OpenSegment,
    end_pts: i64,
    cut_us: i64,
    audio_done: Vec<bool>,
    deadline: Instant,
    saves: Vec<(i64, SaveReply)>,
}

pub struct SegmentWriter {
    dir: PathBuf,
    streams: Vec<StreamSpec>,
    segment_frames: i64,
    run: u32,
    next_seq: u64,
    current: Option<OpenSegment>,
    /// Segments past their cut that are still collecting late audio, oldest first.
    closing: VecDeque<Closing>,
    last_video_pts: i64,
    pending: Vec<(i64, SaveReply)>,
    ring: Arc<Mutex<Ring>>,
}

// SAFETY: the FFmpeg objects are owned by this writer and used only on its thread.
unsafe impl Send for SegmentWriter {}

impl SegmentWriter {
    /// `streams[0]` must be the video stream.
    pub fn new(
        dir: PathBuf,
        streams: Vec<StreamSpec>,
        segment_frames: i64,
        ring: Arc<Mutex<Ring>>,
    ) -> Self {
        Self {
            dir,
            streams,
            segment_frames: segment_frames.max(1),
            run: 0,
            next_seq: 0,
            current: None,
            closing: VecDeque::new(),
            last_video_pts: -1,
            pending: Vec::new(),
            ring,
        }
    }

    fn audio_count(&self) -> usize {
        self.streams.len().saturating_sub(1)
    }

    pub fn run(mut self, rx: Receiver<WriterMsg>) -> Result<(), WriterError> {
        let result = self.run_inner(&rx);
        // Finalise everything and answer outstanding saves, even after an error.
        let closed = self.close_all();
        let pending = std::mem::take(&mut self.pending);
        self.serve(pending);
        result.and(closed)
    }

    fn run_inner(&mut self, rx: &Receiver<WriterMsg>) -> Result<(), WriterError> {
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(WriterMsg::Packet(p)) => self.on_packet(p)?,
                Ok(WriterMsg::Save { len_us, reply }) => self.pending.push((len_us, reply)),
                Ok(WriterMsg::NewRun) => {
                    self.close_all()?;
                    self.run += 1;
                }
                Ok(WriterMsg::Stop) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.finalize_ready(false)?;
        }
    }

    fn on_packet(&mut self, p: Packet) -> Result<(), WriterError> {
        if p.stream() == 0 {
            self.on_video(p)
        } else {
            self.on_audio(p)
        }
    }

    fn on_video(&mut self, p: Packet) -> Result<(), WriterError> {
        let pts = p.pts().unwrap_or(self.last_video_pts + 1);
        if p.is_key() {
            let cut = match &self.current {
                None => true,
                Some(c) => !self.pending.is_empty() || pts - c.first_pts >= self.segment_frames,
            };
            if cut {
                self.cut(pts)?;
            }
        }
        // Video before the first keyframe can't be decoded, so it's dropped.
        if let Some(c) = self.current.as_mut() {
            write_packet(c, p, self.streams[0].time_base)?;
        }
        self.last_video_pts = pts;
        Ok(())
    }

    fn on_audio(&mut self, p: Packet) -> Result<(), WriterError> {
        let idx = p.stream();
        let Some(spec) = self.streams.get(idx) else {
            return Ok(());
        };
        let tb = spec.time_base;
        let pts_us = p.pts().unwrap_or(0).rescale(tb, MICROS);
        // Every closing segment this packet is past has received all its audio
        // on this stream.
        for cl in self.closing.iter_mut() {
            if pts_us >= cl.cut_us
                && let Some(d) = cl.audio_done.get_mut(idx - 1)
            {
                *d = true;
            }
        }
        // The packet belongs to the oldest segment whose cut it hasn't reached.
        if let Some(cl) = self.closing.iter_mut().find(|cl| pts_us < cl.cut_us) {
            write_packet(&mut cl.seg, p, tb)?;
        } else if let Some(c) = self.current.as_mut() {
            write_packet(c, p, tb)?;
        }
        self.finalize_ready(false)
    }

    /// Finalises closing segments in order while the oldest is complete (all
    /// audio passed its cut, or its deadline expired). With `all`, it finalises
    /// everything.
    fn finalize_ready(&mut self, all: bool) -> Result<(), WriterError> {
        let now = Instant::now();
        while let Some(front) = self.closing.front() {
            let done = all || now >= front.deadline || front.audio_done.iter().all(|&d| d);
            if !done {
                break;
            }
            if let Some(cl) = self.closing.pop_front() {
                self.finalize(cl.seg, cl.end_pts, cl.saves)?;
            }
        }
        Ok(())
    }

    /// Starts a new segment at video keyframe `pts`. The old one moves to
    /// `closing` and takes the saves that triggered this cut with it.
    fn cut(&mut self, pts: i64) -> Result<(), WriterError> {
        if let Some(seg) = self.current.take() {
            self.closing.push_back(Closing {
                seg,
                end_pts: pts,
                cut_us: pts.rescale(self.streams[0].time_base, MICROS),
                audio_done: vec![false; self.audio_count()],
                deadline: Instant::now() + AUDIO_GRACE,
                saves: std::mem::take(&mut self.pending),
            });
            // Bound the queue: more than a few pending cuts means audio has stalled.
            while self.closing.len() > 4 {
                if let Some(cl) = self.closing.pop_front() {
                    self.finalize(cl.seg, cl.end_pts, cl.saves)?;
                }
            }
            self.finalize_ready(self.audio_count() == 0)?;
        }
        self.open(pts)
    }

    fn open(&mut self, first_pts: i64) -> Result<(), WriterError> {
        let seq = self.next_seq;
        self.next_seq += 1;
        let path = segment_path(&self.dir, seq);
        let mut out = format::output_as(&path, "mpegts")?;
        // Bound the interleaving queue: if an audio stream stalls, don't buffer
        // more than 1 s of video in memory waiting for it.
        unsafe { (*out.as_mut_ptr()).max_interleave_delta = 1_000_000 };
        for spec in &self.streams {
            let mut st = out.add_stream(ffmpeg::encoder::find(codec::Id::None))?;
            st.set_parameters(spec.params.clone());
            st.set_time_base(spec.time_base);
            unsafe { (*st.parameters().as_mut_ptr()).codec_tag = 0 };
        }
        out.write_header()?;
        let out_tbs = out.streams().map(|s| s.time_base()).collect();
        self.current = Some(OpenSegment {
            out,
            out_tbs,
            seq,
            first_pts,
        });
        Ok(())
    }

    /// Finalises the closing segment (if any), then the current one, and serves pending saves.
    fn close_all(&mut self) -> Result<(), WriterError> {
        let a = self.finalize_ready(true);
        let b = match self.current.take() {
            Some(seg) => {
                let saves = std::mem::take(&mut self.pending);
                self.finalize(seg, self.last_video_pts + 1, saves)
            }
            None => Ok(()),
        };
        a.and(b)
    }

    fn finalize(
        &mut self,
        mut seg: OpenSegment,
        end_pts: i64,
        saves: Vec<(i64, SaveReply)>,
    ) -> Result<(), WriterError> {
        let result = (|| {
            seg.out.write_trailer()?;
            let path = segment_path(&self.dir, seg.seq);
            drop(seg.out); // closes the file
            let frames = end_pts - seg.first_pts;
            if frames <= 0 {
                let _ = fs::remove_file(&path);
                return Ok(());
            }
            let duration_us = frames.rescale(self.streams[0].time_base, MICROS);
            let bytes = fs::metadata(&path)?.len();
            let s = Segment {
                seq: seg.seq,
                run: self.run,
                duration_us,
                bytes,
            };
            let expired = self.ring.lock().map_err(|_| RingError::Empty)?.push(s)?;
            delete_segments(&self.dir, &expired);
            Ok(())
        })();
        // Answer the saves even if this segment failed, so no caller waits forever.
        self.serve(saves);
        result
    }

    fn serve(&mut self, saves: Vec<(i64, SaveReply)>) {
        if saves.is_empty() {
            return;
        }
        let mut orphaned = Vec::new();
        if let Ok(mut ring) = self.ring.lock() {
            for (len_us, reply) in saves {
                // If the save job gave up waiting, release its pin straight
                // away, or those segments would stay on disk forever.
                if let Err(SendError(Ok(pin))) = reply.send(ring.pin_last(len_us)) {
                    orphaned.extend(ring.unpin(pin.id).unwrap_or_default());
                }
            }
        }
        delete_segments(&self.dir, &orphaned);
    }
}

fn write_packet(seg: &mut OpenSegment, mut p: Packet, in_tb: Rational) -> Result<(), WriterError> {
    let idx = p.stream();
    let Some(&out_tb) = seg.out_tbs.get(idx) else {
        return Ok(());
    };
    p.rescale_ts(in_tb, out_tb);
    p.set_position(-1);
    p.write_interleaved(&mut seg.out)?;
    Ok(())
}
