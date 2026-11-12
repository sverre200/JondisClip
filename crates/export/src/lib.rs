//! Clip export (ADR-0011): trims a recording into a new MP4. Runs in its own
//! process (`jondisclip-export.exe`), started by the library window; the
//! recorder is never involved and the original is only read.
//!
//! - [`Mode::Copy`]: stream copy from the keyframe at or before `start`
//!   (lossless; the start may move back by up to one GOP, 1 s in our clips).
//!   The end is exact because our recordings have no B-frames.
//! - [`Mode::Exact`]: video re-encoded from exactly `start` with the first
//!   working hardware H.264 encoder; audio is always stream-copied.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ffmpeg::software::scaling;
use ffmpeg::{
    Dictionary, Packet, Rational, codec, decoder, encoder, format, frame, media, picture,
};
use ffmpeg_next as ffmpeg;
use jondisclip_engine::encoder::{Codec, EncoderId, Vendor, candidates};
use serde::{Deserialize, Serialize};

mod share;
pub use share::{Share, ShareOutcome, share};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Copy,
    Exact,
}

#[derive(Debug, Clone)]
pub struct Trim {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Seconds from the start of the clip.
    pub start: f64,
    pub end: f64,
    pub mode: Mode,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub path: PathBuf,
    pub size: u64,
    /// Where the export really starts in the source (Copy: the keyframe).
    pub start: f64,
    pub duration: f64,
    /// "copy" or the encoder used ("h264_amf"…).
    pub video: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{what}: {source}")]
    Ffmpeg {
        what: &'static str,
        source: ffmpeg::Error,
    },
    #[error("couldn't write the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("the clip has no video")]
    NoVideo,
    #[error("the selection is empty (start must be before end)")]
    EmptyRange,
    #[error("{0} already exists; originals and earlier exports are never overwritten")]
    Exists(PathBuf),
    #[error("no hardware H.264 encoder could be opened ({0})")]
    NoEncoder(String),
    #[error("nothing was written for the selection")]
    NothingWritten,
    #[error("the video's timestamps go backwards at {0:.3} s; the clip can't be cut cleanly")]
    BadTimestamps(f64),
    #[error("this clip uses B-frames, so a lossless cut could break its last frames; use Exact")]
    BFrames,
    #[error("the encoder stayed busy for too long (the GPU is fully loaded); try again")]
    EncoderBusy,
    #[error("too long for {:.0} MB: trim it to under {} s", *limit as f64 / 1e6, *max_seconds as u64)]
    TooLongForLimit { max_seconds: f64, limit: u64 },
    #[error("couldn't get it under the limit ({:.1} MB, limit {:.1} MB); trim it shorter", *size as f64 / 1e6, *limit as f64 / 1e6)]
    OverLimit { size: u64, limit: u64 },
}

impl ExportError {
    /// The message the window shows, in the language setting (ADR-0018).
    /// `Display` stays English for logs. FFmpeg's own error text isn't translated.
    pub fn user_message(&self) -> String {
        use jondisclip_core::tr;
        match self {
            Self::Ffmpeg { what, source } => tr!(
                "FFmpeg couldn't {what}: {error}",
                what = what,
                error = source
            ),
            Self::Io(e) => tr!("couldn't write the file: {error}", error = e),
            Self::NoVideo => tr!("the clip has no video"),
            Self::EmptyRange => tr!("the selection is empty (start must be before end)"),
            Self::Exists(p) => tr!(
                "{path} already exists; originals and earlier exports are never overwritten",
                path = p.display()
            ),
            Self::NoEncoder(e) => tr!(
                "no hardware H.264 encoder could be opened ({error})",
                error = e
            ),
            Self::NothingWritten => tr!("nothing was written for the selection"),
            Self::BadTimestamps(t) => tr!(
                "the video's timestamps go backwards at {time} s; the clip can't be cut cleanly",
                time = format!("{t:.3}")
            ),
            Self::BFrames => tr!(
                "this clip uses B-frames, so a lossless cut could break its last frames; use Exact"
            ),
            Self::EncoderBusy => {
                tr!("the encoder stayed busy for too long (the GPU is fully loaded); try again")
            }
            Self::TooLongForLimit { max_seconds, limit } => tr!(
                "too long for {mb} MB: trim it to under {s} s",
                mb = format!("{:.0}", *limit as f64 / 1e6),
                s = *max_seconds as u64
            ),
            Self::OverLimit { size, limit } => tr!(
                "couldn't get it under the limit ({size} MB, limit {limit} MB); trim it shorter",
                size = format!("{:.1}", *size as f64 / 1e6),
                limit = format!("{:.1}", *limit as f64 / 1e6)
            ),
        }
    }
}

fn is_again(e: &ffmpeg::Error) -> bool {
    matches!(e, ffmpeg::Error::Other { errno } if *errno == ffmpeg::util::error::EAGAIN)
}

/// Encode at most this many times faster than real time, so an export never
/// monopolises the GPU's video engine that the recorder also uses. The
/// exporter's main sets 1× while the library window isn't in front (the game
/// is), 4× while it is.
static MAX_SPEED_BITS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0x4010_0000_0000_0000); // 4.0

pub fn set_max_speed(x: f64) {
    MAX_SPEED_BITS.store(
        x.clamp(0.25, 16.0).to_bits(),
        std::sync::atomic::Ordering::Relaxed,
    );
}

fn max_speed() -> f64 {
    f64::from_bits(MAX_SPEED_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

/// Paces encoding frame by frame, so a speed change applies from the next
/// frame (no catch-up burst or long stall).
#[derive(Default)]
struct Pacer {
    last: Option<(f64, Instant)>,
}

impl Pacer {
    fn wait(&mut self, media: f64) {
        if let Some((m, at)) = self.last {
            let due = at + Duration::from_secs_f64(((media - m) / max_speed()).max(0.0));
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait.min(Duration::from_millis(500)));
            }
        }
        self.last = Some((media, Instant::now()));
    }
}

fn ff(what: &'static str) -> impl FnOnce(ffmpeg::Error) -> ExportError {
    move |source| ExportError::Ffmpeg { what, source }
}

/// Runs a trim. `progress` gets 0..1. The file appears under its final name
/// only when complete (written as `.partial`, then renamed).
pub fn trim(job: &Trim, mut progress: impl FnMut(f64)) -> Result<Outcome, ExportError> {
    let valid =
        job.start.is_finite() && job.end.is_finite() && job.start >= 0.0 && job.end > job.start;
    if !valid {
        return Err(ExportError::EmptyRange);
    }
    if job.output.exists() {
        return Err(ExportError::Exists(job.output.clone()));
    }
    ffmpeg::init().map_err(ff("FFmpeg init"))?;
    unsafe { ffmpeg::ffi::av_log_set_level(ffmpeg::ffi::AV_LOG_ERROR) };
    let tmp = job.output.with_extension("mp4.partial");
    match run(job, &tmp, &mut progress, None) {
        Ok(mut out) => {
            if job.output.exists() {
                let _ = std::fs::remove_file(&tmp);
                return Err(ExportError::Exists(job.output.clone()));
            }
            finalize(&tmp, &job.output)?;
            out.path = job.output.clone();
            out.size = std::fs::metadata(&job.output)?.len();
            progress(1.0);
            Ok(out)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Moves the finished `.partial` to its name without ever replacing a file
/// (`rename` on Windows would). A hard link fails if the target exists; on
/// volumes without hard links (exFAT/FAT32 USB drives) fall back to a check
/// plus rename.
fn finalize(tmp: &Path, out: &Path) -> Result<(), ExportError> {
    match std::fs::hard_link(tmp, out) {
        Ok(()) => {
            let _ = std::fs::remove_file(tmp);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(tmp);
            Err(ExportError::Exists(out.to_path_buf()))
        }
        // No hard links here: an atomic move that never replaces a file.
        Err(_) => {
            use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
            use windows::core::HSTRING;
            let moved = unsafe {
                MoveFileExW(
                    &HSTRING::from(tmp.as_os_str()),
                    &HSTRING::from(out.as_os_str()),
                    MOVE_FILE_FLAGS(0),
                )
            };
            if moved.is_err() {
                let _ = std::fs::remove_file(tmp);
                return Err(if out.exists() {
                    ExportError::Exists(out.to_path_buf())
                } else {
                    ExportError::Io(std::io::Error::other("couldn't rename the finished file"))
                });
            }
            Ok(())
        }
    }
}

/// Output file plus per-stream bookkeeping.
struct Mux {
    octx: format::context::Output,
    out_tbs: Vec<Rational>,
    last_dts: Vec<Option<i64>>,
    written: usize,
    video: usize,
}

impl Mux {
    /// Writes `pkt` (timestamps in `tb`) to stream `i`, moved earlier by `shift` (in `tb`).
    fn write(
        &mut self,
        mut pkt: Packet,
        i: usize,
        tb: Rational,
        shift: i64,
    ) -> Result<(), ExportError> {
        pkt.set_pts(pkt.pts().map(|p| p - shift));
        pkt.set_dts(pkt.dts().map(|d| d - shift));
        pkt.rescale_ts(tb, self.out_tbs[i]);
        if let (Some(d), Some(l)) = (pkt.dts(), self.last_dts[i])
            && d <= l
        {
            // A dropped audio packet is a 21 ms gap; a dropped video packet
            // would corrupt every frame up to the next keyframe.
            if i == self.video {
                return Err(ExportError::BadTimestamps(secs(d, self.out_tbs[i])));
            }
            return Ok(());
        }
        self.last_dts[i] = pkt.dts();
        pkt.set_stream(i);
        pkt.set_position(-1);
        pkt.write_interleaved(&mut self.octx).map_err(ff("write"))?;
        self.written += 1;
        Ok(())
    }
}

struct VideoEncoder {
    dec: decoder::Video,
    enc: encoder::Video,
    name: &'static str,
    tb: Rational,
    frame: frame::Video,
    /// The encoder's input format when it differs from the decoder's (Intel
    /// QSV only takes NV12): frames are converted into `converted` first.
    convert_to: Option<format::Pixel>,
    converter: Option<scaling::Context>,
    converted: frame::Video,
    /// One frame in `tb` ticks.
    frame_ticks: i64,
    pacer: Pacer,
}

impl VideoEncoder {
    fn encode(&mut self, pts: i64, mux: &mut Mux, i: usize) -> Result<(), ExportError> {
        self.pacer.wait(secs(pts, self.tb));
        if let Some(target) = self.convert_to {
            if self.converter.is_none() {
                // Same size, so this only repacks the chroma planes.
                self.converter = Some(
                    scaling::Context::get(
                        self.frame.format(),
                        self.frame.width(),
                        self.frame.height(),
                        target,
                        self.frame.width(),
                        self.frame.height(),
                        scaling::Flags::BILINEAR,
                    )
                    .map_err(ff("convert"))?,
                );
            }
            if let Some(c) = self.converter.as_mut() {
                // An encoder may still hold a reference to the last frame's
                // buffer: never draw into it (no-op when it doesn't).
                unsafe {
                    if !self.converted.is_empty() {
                        ffmpeg::ffi::av_frame_make_writable(self.converted.as_mut_ptr());
                    }
                }
                c.run(&self.frame, &mut self.converted)
                    .map_err(ff("convert"))?;
            }
        }
        let frame = if self.convert_to.is_some() {
            &mut self.converted
        } else {
            &mut self.frame
        };
        frame.set_pts(Some(pts));
        frame.set_kind(picture::Type::None);
        // EAGAIN means "take packets out first", e.g. when the GPU encoder's
        // input queue is full because the recorder is busy on it too.
        for attempt in 0..2000 {
            let frame = if self.convert_to.is_some() {
                &self.converted
            } else {
                &self.frame
            };
            match self.enc.send_frame(frame) {
                Ok(()) => return self.drain(mux, i).map(drop),
                Err(e) if is_again(&e) => {
                    if self.drain(mux, i)? == 0 {
                        std::thread::sleep(Duration::from_millis(if attempt < 10 { 1 } else { 5 }));
                    }
                }
                Err(e) => {
                    return Err(ExportError::Ffmpeg {
                        what: "encode",
                        source: e,
                    });
                }
            }
        }
        Err(ExportError::EncoderBusy)
    }

    fn finish(&mut self, mux: &mut Mux, i: usize) -> Result<(), ExportError> {
        self.enc.send_eof().map_err(ff("finish encoding"))?;
        self.drain(mux, i)?;
        Ok(())
    }

    /// Writes every ready packet; returns how many. Only "not yet" and "end"
    /// stop the loop: any other encoder error fails the export instead of
    /// producing a silently truncated file.
    fn drain(&mut self, mux: &mut Mux, i: usize) -> Result<usize, ExportError> {
        let mut pkt = Packet::empty();
        let mut n = 0;
        loop {
            match self.enc.receive_packet(&mut pkt) {
                Ok(()) => {
                    // Encoders don't always set durations; without one the MP4's
                    // last sample gets duration 0 and decoders drop that frame.
                    if pkt.duration() <= 0 {
                        pkt.set_duration(self.frame_ticks);
                    }
                    mux.write(std::mem::replace(&mut pkt, Packet::empty()), i, self.tb, 0)?;
                    n += 1;
                }
                Err(e) if is_again(&e) || matches!(e, ffmpeg::Error::Eof) => return Ok(n),
                Err(e) => {
                    return Err(ExportError::Ffmpeg {
                        what: "encode",
                        source: e,
                    });
                }
            }
        }
    }

    /// Next decoded frame, `false` when the decoder needs more input or is done.
    fn next_frame(&mut self) -> Result<bool, ExportError> {
        match self.dec.receive_frame(&mut self.frame) {
            Ok(()) => Ok(true),
            Err(e) if is_again(&e) || matches!(e, ffmpeg::Error::Eof) => Ok(false),
            Err(e) => Err(ExportError::Ffmpeg {
                what: "decode",
                source: e,
            }),
        }
    }
}

fn secs(ts: i64, tb: Rational) -> f64 {
    ts as f64 * f64::from(tb)
}

fn to_ts(s: f64, tb: Rational) -> i64 {
    (s / f64::from(tb)).round() as i64
}

/// A trim that keeps only one audio track (`None`: no audio), for sharing.
fn run_tracks(
    job: &Trim,
    tmp: &Path,
    progress: &mut impl FnMut(f64),
    only_audio: Option<usize>,
) -> Result<Outcome, ExportError> {
    run(job, tmp, progress, Some(only_audio))
}

/// `only_audio`: `None` keeps every track; `Some(x)` keeps only audio stream `x`.
fn run(
    job: &Trim,
    tmp: &Path,
    progress: &mut impl FnMut(f64),
    only_audio: Option<Option<usize>>,
) -> Result<Outcome, ExportError> {
    let mut ictx = format::input(&job.input).map_err(ff("open the clip"))?;
    let vidx = ictx
        .streams()
        .best(media::Type::Video)
        .map(|s| s.index())
        .ok_or(ExportError::NoVideo)?;

    // A selection that starts after the clip ends would otherwise export the
    // last GOP (the seek lands on the final keyframe).
    let length = ictx.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE);
    if length > 0.0 && job.start >= length {
        return Err(ExportError::EmptyRange);
    }
    let mut octx = format::output_as(tmp, "mp4").map_err(ff("create the output"))?;
    let global_header = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);
    let mut kinds = Vec::new();
    let mut in_tbs = Vec::new();
    let mut venc: Option<VideoEncoder> = None;
    let mut frame_dur = 1.0 / 60.0;
    let mut audio_seen = 0;
    // Input stream → output stream (dropped tracks map to None).
    let mut map: Vec<Option<usize>> = Vec::new();
    for ist in ictx.streams() {
        let params = ist.parameters();
        let kind = params.medium();
        kinds.push(kind);
        in_tbs.push(ist.time_base());
        if kind == media::Type::Audio
            && let Some(keep) = only_audio
            && keep != Some(ist.index())
        {
            map.push(None);
            continue;
        }
        if kind != media::Type::Audio && kind != media::Type::Video {
            map.push(None);
            continue;
        }
        map.push(Some(map.iter().flatten().count()));
        let mut ost = octx
            .add_stream(encoder::find(codec::Id::None))
            .map_err(ff("add stream"))?;
        if ist.index() == vidx {
            let r = ist.avg_frame_rate();
            if r.numerator() > 0 && r.denominator() > 0 {
                frame_dur = f64::from(r.invert());
            }
        }
        // Copy mode cuts by presentation time in decode order, which is only
        // safe without B-frames (our recordings have none).
        if ist.index() == vidx
            && job.mode == Mode::Copy
            && unsafe { (*params.as_ptr()).video_delay } > 0
        {
            return Err(ExportError::BFrames);
        }
        if ist.index() == vidx && job.mode == Mode::Exact {
            let v = open_video_encoder(&ist, global_header)?;
            ost.set_parameters(codec::Parameters::from(&*v.enc));
            ost.set_time_base(v.tb);
            venc = Some(v);
        } else {
            let is_hevc = params.id() == codec::Id::HEVC;
            ost.set_parameters(params);
            unsafe {
                // hvc1 is the tag WebView2/Apple players expect; 0 = let the muxer pick.
                (*ost.parameters().as_mut_ptr()).codec_tag = if is_hevc {
                    u32::from_le_bytes(*b"hvc1")
                } else {
                    0
                };
            }
        }
        // Keep track titles (Mix / Game / Microphone) and the default track.
        let mut md = Dictionary::new();
        for (k, v) in ist.metadata().iter() {
            if k == "title" || k == "handler_name" || k == "language" {
                md.set(k, v);
            }
        }
        ost.set_metadata(md);
        let default = kind == media::Type::Video || (kind == media::Type::Audio && audio_seen == 0);
        if kind == media::Type::Audio {
            audio_seen += 1;
        }
        unsafe {
            (*ost.as_mut_ptr()).disposition = if default {
                ffmpeg::ffi::AV_DISPOSITION_DEFAULT
            } else {
                0
            };
        }
    }
    let mut opts = Dictionary::new();
    opts.set("movflags", "+faststart");
    octx.write_header_with(opts)
        .map_err(ff("write the header"))?;
    let out_tbs: Vec<Rational> = octx.streams().map(|s| s.time_base()).collect();
    let mut mux = Mux {
        octx,
        last_dts: vec![None; out_tbs.len()],
        out_tbs,
        written: 0,
        video: map[vidx].unwrap_or(0),
    };
    let vout = map[vidx].unwrap_or(0);

    // To the keyframe at or before the start (both modes).
    let start_us = (job.start * 1e6) as i64;
    ictx.seek(start_us, ..start_us).map_err(ff("seek"))?;

    let vtb = in_tbs[vidx];
    // Output time zero, in source seconds: the first keyframe (Copy) or the
    // first frame at or after `start` (Exact). Known once video decides it.
    let mut base: Option<f64> = None;
    let mut video_done = false;
    let mut audio_done: Vec<bool> = kinds
        .iter()
        .zip(&map)
        .map(|(k, m)| *k != media::Type::Audio || m.is_none())
        .collect();
    let mut pending_audio: Vec<(Packet, usize)> = Vec::new();
    let mut video_end = 0.0f64;
    let span = (job.end - job.start).max(1e-6);

    // A decoded frame belongs to the selection if it's shown at or after
    // `start` (half a frame of tolerance for rounding) and before `end`.
    let in_range = |t: f64| t + frame_dur * 0.5 >= job.start && t < job.end - 1e-6;

    for (stream, pkt) in ictx.packets() {
        let i = stream.index();
        let tb = in_tbs[i];
        let Some(pts) = pkt.pts() else { continue };
        let t = secs(pts, tb);
        if i == vidx {
            if video_done {
                continue;
            }
            if let Some(v) = venc.as_mut() {
                match v.dec.send_packet(&pkt) {
                    Ok(()) => {}
                    // One damaged packet: the decoder conceals it, carry on.
                    Err(ffmpeg::Error::InvalidData) => continue,
                    Err(e) => {
                        return Err(ExportError::Ffmpeg {
                            what: "decode",
                            source: e,
                        });
                    }
                }
                while v.next_frame()? {
                    let Some(fp) = v.frame.pts().or(v.frame.timestamp()) else {
                        continue;
                    };
                    let ft = secs(fp, vtb);
                    if ft >= job.end - 1e-6 {
                        video_done = true;
                        break;
                    }
                    if !in_range(ft) {
                        continue;
                    }
                    let b = *base.get_or_insert(ft);
                    v.encode(fp - to_ts(b, vtb), &mut mux, vout)?;
                    video_end = ft - b + frame_dur;
                    progress(((ft - job.start) / span).clamp(0.0, 0.99));
                }
                if video_done {
                    v.finish(&mut mux, vout)?;
                }
            } else {
                if base.is_none() {
                    if !pkt.is_key() {
                        continue; // the seek may land before the keyframe
                    }
                    base = Some(t);
                }
                if t >= job.end - 1e-6 {
                    video_done = true;
                    continue;
                }
                let b = base.unwrap_or(t);
                mux.write(pkt, vout, tb, to_ts(b, tb))?;
                video_end = t - b + frame_dur;
                progress(((t - job.start) / span).clamp(0.0, 0.99));
            }
            if let Some(b) = base {
                write_pending(&mut mux, &mut pending_audio, &in_tbs, &map, b, job.end)?;
            }
        } else if kinds[i] == media::Type::Audio && !audio_done[i] {
            if t >= job.end {
                audio_done[i] = true;
            } else {
                match base {
                    Some(b) if t >= b - 1e-6 => {
                        mux.write(pkt, map[i].unwrap_or(0), tb, to_ts(b, tb))?
                    }
                    Some(_) => {}
                    None => pending_audio.push((pkt, i)),
                }
            }
        }
        if video_done && audio_done.iter().all(|d| *d) {
            break;
        }
    }
    if let Some(v) = venc.as_mut()
        && !video_done
    {
        // The selection runs to the end of the clip: flush the decoder too.
        let _ = v.dec.send_eof();
        while v.next_frame()? {
            let Some(fp) = v.frame.pts().or(v.frame.timestamp()) else {
                continue;
            };
            let ft = secs(fp, vtb);
            if !in_range(ft) {
                continue;
            }
            let b = *base.get_or_insert(ft);
            v.encode(fp - to_ts(b, vtb), &mut mux, vout)?;
            video_end = ft - b + frame_dur;
        }
        v.finish(&mut mux, vout)?;
    }
    // Audio held back until the start was known (it may only be known now,
    // when the selection's frames came out of the decoder at the flush).
    if let Some(b) = base {
        write_pending(&mut mux, &mut pending_audio, &in_tbs, &map, b, job.end)?;
    }
    if mux.written == 0 || base.is_none() {
        return Err(ExportError::NothingWritten);
    }
    mux.octx.write_trailer().map_err(ff("finish the file"))?;
    Ok(Outcome {
        path: tmp.to_path_buf(),
        size: 0,
        start: base.unwrap_or(job.start),
        duration: video_end,
        video: venc.map_or_else(|| "copy".into(), |v| v.name.to_string()),
    })
}

fn write_pending(
    mux: &mut Mux,
    pending: &mut Vec<(Packet, usize)>,
    in_tbs: &[Rational],
    map: &[Option<usize>],
    base: f64,
    end: f64,
) -> Result<(), ExportError> {
    for (p, pi) in pending.drain(..) {
        let pt = secs(p.pts().unwrap_or(i64::MIN), in_tbs[pi]);
        if pt >= base - 1e-6
            && pt < end
            && let Some(o) = map[pi]
        {
            mux.write(p, o, in_tbs[pi], to_ts(base, in_tbs[pi]))?;
        }
    }
    Ok(())
}

fn open_video_encoder(
    ist: &format::stream::Stream,
    global_header: bool,
) -> Result<VideoEncoder, ExportError> {
    let params = ist.parameters();
    let bit_rate = unsafe { (*params.as_ptr()).bit_rate };
    let mut dctx = codec::context::Context::from_parameters(params).map_err(ff("decoder"))?;
    // A few threads only: a game may be running.
    dctx.set_threading(codec::threading::Config {
        kind: codec::threading::Type::Frame,
        count: 4,
    });
    let dec = dctx.decoder().video().map_err(ff("open decoder"))?;
    let tb = ist.time_base();
    let r = ist.avg_frame_rate();
    let fps = if r.numerator() > 0 && r.denominator() > 0 {
        f64::from(r).round().max(1.0) as i32
    } else {
        60
    };
    // 1.3× the source's average so generation loss stays invisible.
    let source = if bit_rate > 0 { bit_rate as f64 } else { 20e6 };
    let bitrate = (source * 1.3) as usize;
    let mut errors = Vec::new();
    for (id, software) in export_candidates() {
        let input = encoder_input(id, dec.format());
        match open_encoder(id, software, input, &dec, tb, fps, bitrate, global_header) {
            Ok(enc) => {
                return Ok(VideoEncoder {
                    convert_to: (input != dec.format()).then_some(input),
                    converter: None,
                    converted: frame::Video::empty(),
                    dec,
                    enc,
                    name: id.ffmpeg_name(),
                    tb,
                    frame: frame::Video::empty(),
                    frame_ticks: (1.0 / f64::from(fps) / f64::from(tb)).round() as i64,
                    pacer: Pacer::default(),
                });
            }
            Err(e) => errors.push(format!(
                "{}{}: {e}",
                id.ffmpeg_name(),
                if software { " (software)" } else { "" }
            )),
        }
    }
    Err(ExportError::NoEncoder(errors.join("; ")))
}

/// Encoders to try for an export, best first: each GPU vendor's own, Media
/// Foundation's hardware encoder, then Media Foundation's software encoder
/// (`true`). The last one is part of Windows, so an export works on any PC,
/// only slower.
pub(crate) fn export_candidates() -> Vec<(EncoderId, bool)> {
    let mut v: Vec<(EncoderId, bool)> = candidates(Codec::H264)
        .into_iter()
        .map(|id| (id, false))
        .collect();
    v.push((
        EncoderId {
            codec: Codec::H264,
            vendor: Vendor::MediaFoundation,
        },
        true,
    ));
    v
}

/// The pixel format to feed `id` for frames decoded as `decoded`. AMF and
/// NVENC take planar 4:2:0 as it comes from the decoder (the measured path);
/// QSV only takes NV12, Media Foundation's hardware encoders refuse planar
/// input on some drivers (seen on Intel Arc), and anything else (10-bit, 4:4:4)
/// is converted to NV12 for every encoder.
fn encoder_input(id: EncoderId, decoded: format::Pixel) -> format::Pixel {
    let direct = matches!(id.vendor, Vendor::Amd | Vendor::Nvidia)
        && matches!(decoded, format::Pixel::YUV420P | format::Pixel::NV12);
    if direct { decoded } else { format::Pixel::NV12 }
}

#[allow(clippy::too_many_arguments)]
fn open_encoder(
    id: EncoderId,
    software: bool,
    input: format::Pixel,
    dec: &decoder::Video,
    tb: Rational,
    fps: i32,
    bitrate: usize,
    global_header: bool,
) -> Result<encoder::Video, ffmpeg::Error> {
    let codec = encoder::find_by_name(id.ffmpeg_name()).ok_or(ffmpeg::Error::EncoderNotFound)?;
    let mut enc = codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()?;
    enc.set_width(dec.width());
    enc.set_height(dec.height());
    enc.set_format(input);
    enc.set_time_base(tb);
    enc.set_frame_rate(Some(Rational::new(fps, 1)));
    enc.set_bit_rate(bitrate);
    enc.set_max_bit_rate(bitrate * 2);
    // 1 s GOP like the recorder, so an edit can itself be trimmed losslessly.
    enc.set_gop(fps as u32);
    enc.set_max_b_frames(0);
    enc.set_colorspace(dec.color_space());
    enc.set_color_range(dec.color_range());
    if global_header {
        enc.set_flags(codec::Flags::GLOBAL_HEADER);
    }
    let mut opts = Dictionary::new();
    match id.vendor {
        Vendor::Amd => {
            opts.set("usage", "transcoding");
            // "quality" is ~2× slower (115 vs ~250 fps at 1080p60, measured) for no
            // visible gain at 1.3× the source bitrate; balanced matches the recorder.
            opts.set("quality", "balanced");
            opts.set("rc", "vbr_peak");
        }
        Vendor::Nvidia => {
            opts.set("preset", "p5");
            opts.set("tune", "hq");
            opts.set("rc", "vbr");
        }
        Vendor::Intel => opts.set("preset", "slow"),
        Vendor::MediaFoundation => opts.set("hw_encoding", if software { "0" } else { "1" }),
    }
    enc.open_as_with(codec, opts)
}
