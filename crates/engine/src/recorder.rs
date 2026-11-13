//! The replay recorder. It wires capture → encoder thread → writer thread →
//! ring, and runs clip saves as background jobs (`docs/ARCHITECTURE.md`).
//!
//! Threads:
//! - WGC callback (OS thread pool): one GPU copy per changed screen frame.
//! - `jc-encode` (above normal): fixed-rate pacing, GPU conversion, encode.
//! - `jc-segments` (below normal): muxing, ring updates, file deletion.
//! - `jc-save` (background mode, one per save): concatenates into an MP4.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ffmpeg_next::{Packet, Rational};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, GetCurrentThread, INFINITE,
    SetThreadPriority, SetWaitableTimer, THREAD_MODE_BACKGROUND_BEGIN, THREAD_PRIORITY,
    THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_BELOW_NORMAL, TIMER_ALL_ACCESS,
    WaitForSingleObject,
};

use crate::audio::{self, AudioConfig, AudioError, AudioPipeline, Clock, SAMPLE_RATE};
use crate::capture::{Capture, CaptureShared};
use crate::clip::{self, ClipError};
use crate::convert::{Converter, output_size};
use crate::encoder::{self, Codec, EncoderId};
use crate::gpu::Gpu;
use crate::hwenc::{EncodeSettings, HwEncError, HwEncoder};
use crate::ring::{Ring, RingError};
use crate::segment::{self, SegmentWriter, StreamSpec, WriterError, WriterMsg};

#[derive(Debug, Clone)]
pub struct RecorderConfig {
    pub fps: u32,
    /// Output height, or `None` for the source resolution. It never upscales.
    pub output_height: Option<u32>,
    pub codec: Codec,
    /// Bits per second, or `None` for the Balanced default.
    pub bitrate: Option<u64>,
    /// Multiplier on the default bitrate (quality preset: Performance about 0.7,
    /// Balanced 1.0, High about 1.5). Ignored when `bitrate` is set.
    pub quality_scale: f64,
    /// Minimum QP (quality floor). See `EncodeSettings::qp_floor`.
    pub qp_floor: Option<u32>,
    /// How much history the buffer keeps.
    pub replay: Duration,
    pub segment_seconds: u32,
    /// Parent of the per-session buffer directory.
    pub buffer_dir: PathBuf,
    pub audio: AudioConfig,
}

/// `%LOCALAPPDATA%\JondisClip\buffer` (ARCHITECTURE.md), falling back to %TEMP%.
pub fn default_buffer_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("JondisClip")
        .join("buffer")
}

/// Deletes `session-*` directories left by a crash or kill. Assumes a single
/// running instance (single-instance check comes with the tray app).
// TODO(phase 2 robustness): offer "Recover last buffer" before deleting.
fn sweep_stale_sessions(buffer_dir: &Path) {
    let Ok(entries) = fs::read_dir(buffer_dir) else {
        return;
    };
    for e in entries.flatten() {
        let is_session = e.file_name().to_string_lossy().starts_with("session-");
        if !is_session || !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        // A live session (this or another process, or a save still running)
        // holds its lock file open exclusively, so the delete fails and it's skipped.
        let lock = e.path().join(LOCK_FILE);
        if lock.exists() && fs::remove_file(&lock).is_err() {
            continue;
        }
        let _ = fs::remove_dir_all(e.path());
    }
}

const LOCK_FILE: &str = "session.lock";

/// Exclusive lock on a session directory, held until its last segment is gone
/// (after stop, a running save may still be reading segments).
struct SessionLock {
    file: Mutex<Option<fs::File>>,
    dir: PathBuf,
}

impl SessionLock {
    fn acquire(dir: &Path) -> std::io::Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0) // no sharing: nobody else can open or delete it
            .open(dir.join(LOCK_FILE))?;
        Ok(Self {
            file: Mutex::new(Some(file)),
            dir: dir.to_path_buf(),
        })
    }

    /// Releases the lock and removes the (now empty) session directory.
    fn release_and_remove(&self) {
        if let Ok(mut f) = self.file.lock() {
            f.take();
        }
        let _ = fs::remove_file(self.dir.join(LOCK_FILE));
        let _ = fs::remove_dir(&self.dir);
    }
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            fps: 60,
            output_height: None,
            codec: Codec::H264, // ADR-0007: plays everywhere without extra codecs
            bitrate: None,
            quality_scale: 1.0,
            qp_floor: Some(20),
            replay: Duration::from_secs(300),
            segment_seconds: 2,
            buffer_dir: default_buffer_dir(),
            audio: AudioConfig::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error("invalid setting: {0}")]
    Config(&'static str),
    #[error("Windows: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("no usable hardware encoder: {0}")]
    NoEncoder(String),
    #[error("encoder: {0}")]
    Encoder(#[from] HwEncError),
    #[error("segment writer: {0}")]
    Writer(#[from] WriterError),
    #[error("buffer: {0}")]
    Ring(#[from] RingError),
    #[error("saving clip: {0}")]
    Clip(#[from] ClipError),
    #[error("file: {0}")]
    Io(#[from] std::io::Error),
    #[error("no frames from capture within {0:?}")]
    NoFrames(Duration),
    #[error("display mode changed; the capture must restart")]
    DisplayChanged,
    #[error("the captured monitor went away")]
    SourceClosed,
    #[error("the segment writer stopped (see its error)")]
    WriterGone,
    #[error("the recorder is not running")]
    NotRunning,
    #[error("a recorder thread panicked")]
    Panicked,
    #[error("audio: {0}")]
    Audio(#[from] AudioError),
}

/// What's actually being recorded (shown on the Recording page).
#[derive(Debug, Clone)]
pub struct RecorderInfo {
    pub adapter: String,
    pub encoder: EncoderId,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u64,
    pub session_dir: PathBuf,
    /// Audio track titles, in stream order (empty = no audio).
    pub audio_tracks: Vec<&'static str>,
    /// Why audio is off, if it failed to start.
    pub audio_error: Option<String>,
}

#[derive(Default)]
struct Counters {
    frames_encoded: AtomicU64,
    frames_late: AtomicU64,
    packets_dropped: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Screen updates delivered by WGC (only changed frames).
    pub frames_arrived: u64,
    /// Frames sent to the encoder (constant rate).
    pub frames_encoded: u64,
    /// Ticks skipped because the encoder thread ran late.
    pub frames_late: u64,
    /// Encoded packets dropped because the writer queue was full.
    pub packets_dropped: u64,
    pub buffered_us: i64,
    pub buffer_bytes: u64,
    /// 1024-sample audio blocks encoded (per track).
    pub audio_blocks: u64,
    /// Audio packets dropped (writer queue full).
    pub audio_dropped: u64,
    /// Times an audio device was reopened (unplugged, default changed).
    pub audio_reopens: u64,
    /// Audio frames skipped because the mixer fell behind (sleep/resume).
    pub audio_skipped_frames: u64,
    /// Some(false) while the chosen microphone isn't connected (None = mic off).
    pub mic_connected: Option<bool>,
}

struct Control {
    stop: AtomicBool,
    force_keyframe: AtomicBool,
}

pub struct Recorder {
    info: RecorderInfo,
    capture: Option<Capture>,
    ctl: Arc<Control>,
    counters: Arc<Counters>,
    tx: SyncSender<WriterMsg>,
    ring: Arc<Mutex<Ring>>,
    lock: Arc<SessionLock>,
    encode_thread: Option<JoinHandle<Result<(), RecorderError>>>,
    writer_thread: Option<JoinHandle<Result<(), WriterError>>>,
    audio: Option<AudioPipeline>,
}

impl Recorder {
    pub fn start(cfg: &RecorderConfig) -> Result<Self, RecorderError> {
        if !(1..=240).contains(&cfg.fps) {
            return Err(RecorderError::Config("fps must be 1-240"));
        }
        if cfg.replay.is_zero() || cfg.segment_seconds == 0 {
            return Err(RecorderError::Config(
                "replay and segment length must be positive",
            ));
        }
        // Keep the multithreaded apartment alive for WGC's free-threaded pool
        // without changing the caller thread's apartment.
        let _cookie = unsafe { CoIncrementMTAUsage()? };

        let gpu = Gpu::create()?;
        let capture = Capture::primary_monitor(&gpu, cfg.fps)?;
        let (width, height) = output_size(capture.width, capture.height, cfg.output_height);
        // Open the real encoder at the real size, not just a probe: an encoder
        // can pass a small probe and still refuse this resolution or these
        // GPU surfaces. Order: the adapter's own encoder, then Media
        // Foundation; the preferred codec first, then its fallbacks.
        let mut errors = Vec::new();
        let mut opened = None;
        'search: for &codec in &encoder::codecs_for(cfg.codec, width, height) {
            for id in encoder::candidates_for_adapter(codec, gpu.vendor_id) {
                let bitrate = cfg.bitrate.unwrap_or_else(|| {
                    let base = EncodeSettings::default_bitrate(id.codec, width, height, cfg.fps);
                    (base as f64 * cfg.quality_scale.clamp(0.25, 4.0)) as u64
                });
                let settings = EncodeSettings {
                    id,
                    width,
                    height,
                    fps: cfg.fps,
                    bitrate,
                    gop_seconds: 1,
                    qp_floor: cfg.qp_floor,
                };
                match HwEncoder::open(&gpu, settings) {
                    Ok(enc) => {
                        opened = Some((enc, id, bitrate));
                        break 'search;
                    }
                    Err(e) => errors.push(format!("{}: {e}", id.ffmpeg_name())),
                }
            }
        }
        let Some((enc, id, bitrate)) = opened else {
            return Err(RecorderError::NoEncoder(format!(
                "{} at {width}×{height}: {}",
                gpu.adapter_name,
                errors.join("; ")
            )));
        };
        let conv = Converter::new(&gpu, &capture.latest, width, height, cfg.fps)?;

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        sweep_stale_sessions(&cfg.buffer_dir);
        let session_dir = cfg.buffer_dir.join(format!("session-{stamp}"));
        fs::create_dir_all(&session_dir)?;
        let lock = Arc::new(SessionLock::acquire(&session_dir)?);

        let ring = Arc::new(Mutex::new(Ring::new(cfg.replay.as_micros() as i64)?));
        let ctl = Arc::new(Control {
            stop: AtomicBool::new(false),
            force_keyframe: AtomicBool::new(false),
        });
        let counters = Arc::new(Counters::default());
        // Video (60/s) plus up to 3 audio tracks (~141/s): about 7 s of headroom
        // for disk hiccups without ever blocking an encoder.
        let (tx, rx) = mpsc::sync_channel::<WriterMsg>(1536);

        // One clock for video and audio: frame n and sample s both count from here.
        let mut freq = 0;
        unsafe { QueryPerformanceFrequency(&mut freq)? };
        let clock = Clock {
            epoch: Clock::now_qpc(),
            freq,
        };
        // Audio problems never stop video recording: fall back to video-only.
        let (audio_setup, audio_error) = match audio::start(cfg.audio.clone(), clock, tx.clone()) {
            Ok(a) => (a, None),
            Err(e) => (
                audio::AudioSetup {
                    pipeline: None,
                    streams: Vec::new(),
                },
                Some(e.to_string()),
            ),
        };
        let audio_tracks = if audio_setup.pipeline.is_some() {
            cfg.audio.track_names()
        } else {
            Vec::new()
        };
        let mut streams = vec![StreamSpec {
            params: enc.parameters(),
            time_base: enc.time_base,
        }];
        streams.extend(audio_setup.streams.iter().map(|p| StreamSpec {
            params: p.clone(),
            time_base: Rational::new(1, SAMPLE_RATE as i32),
        }));
        let writer = SegmentWriter::new(
            session_dir.clone(),
            streams,
            i64::from(cfg.fps * cfg.segment_seconds),
            ring.clone(),
        );
        let writer_thread = thread::Builder::new()
            .name("jc-segments".into())
            .spawn(move || {
                set_priority(THREAD_PRIORITY_BELOW_NORMAL);
                writer.run(rx)
            })?;

        let adapter = gpu.adapter_name.clone();
        let job = EncodeJob {
            enc,
            conv,
            gpu,
            shared: capture.shared.clone(),
            ctl: ctl.clone(),
            counters: counters.clone(),
            fps: cfg.fps,
            epoch: clock.epoch,
        };
        let enc_tx = tx.clone();
        let encode_thread = thread::Builder::new()
            .name("jc-encode".into())
            .spawn(move || {
                set_priority(THREAD_PRIORITY_ABOVE_NORMAL);
                let result = job.run(&enc_tx);
                // Close the open segment so pending saves are answered now,
                // not after a timeout. Stop is sent by shutdown() once audio
                // has flushed too.
                let _ = enc_tx.send(WriterMsg::NewRun);
                result
            })?;

        let info = RecorderInfo {
            adapter,
            encoder: id,
            width,
            height,
            fps: cfg.fps,
            bitrate,
            session_dir,
            audio_tracks,
            audio_error,
        };
        Ok(Self {
            info,
            capture: Some(capture),
            ctl,
            counters,
            tx,
            ring,
            lock,
            encode_thread: Some(encode_thread),
            writer_thread: Some(writer_thread),
            audio: audio_setup.pipeline,
        })
    }

    pub fn info(&self) -> &RecorderInfo {
        &self.info
    }

    /// Changes the microphone gain live (no restart, no buffer loss).
    pub fn set_mic_gain_db(&self, db: f32) {
        if let Some(a) = &self.audio {
            a.set_mic_gain_db(db);
        }
    }

    /// Turns the mic low-cut on or off live.
    pub fn set_mic_low_cut(&self, on: bool) {
        if let Some(a) = &self.audio {
            a.set_mic_low_cut(on);
        }
    }

    /// Changes how much history is kept, without interrupting recording.
    /// Shrinking deletes the now-expired segments, and growing fills up over time.
    pub fn set_replay(&mut self, replay: Duration) -> Result<(), RecorderError> {
        if replay.is_zero() {
            return Err(RecorderError::Config("replay must be positive"));
        }
        let expired = self
            .ring
            .lock()
            .map_err(|_| RecorderError::Panicked)?
            .set_retention(replay.as_micros() as i64)?;
        segment::delete_segments(&self.info.session_dir, &expired);
        Ok(())
    }

    pub fn stats(&self) -> Stats {
        let (buffered_us, buffer_bytes) = self
            .ring
            .lock()
            .map(|r| (r.available_us(), r.disk_bytes()))
            .unwrap_or_default();
        Stats {
            frames_arrived: self
                .capture
                .as_ref()
                .map_or(0, |c| c.shared.frames_arrived.load(Ordering::Relaxed)),
            frames_encoded: self.counters.frames_encoded.load(Ordering::Relaxed),
            frames_late: self.counters.frames_late.load(Ordering::Relaxed),
            packets_dropped: self.counters.packets_dropped.load(Ordering::Relaxed),
            buffered_us,
            buffer_bytes,
            audio_blocks: self
                .audio
                .as_ref()
                .map_or(0, |a| a.counters.blocks_encoded.load(Ordering::Relaxed)),
            audio_dropped: self
                .audio
                .as_ref()
                .map_or(0, |a| a.counters.packets_dropped.load(Ordering::Relaxed)),
            audio_skipped_frames: self
                .audio
                .as_ref()
                .map_or(0, |a| a.counters.frames_skipped.load(Ordering::Relaxed)),
            audio_reopens: self
                .audio
                .as_ref()
                .map_or(0, |a| a.counters.device_reopens.load(Ordering::Relaxed)),
            mic_connected: self.info.audio_tracks.contains(&"Microphone").then(|| {
                self.audio
                    .as_ref()
                    .is_some_and(|a| a.counters.mic_connected.load(Ordering::Relaxed))
            }),
        }
    }

    /// True while both pipeline threads are alive (false after a fatal error).
    pub fn is_running(&self) -> bool {
        let enc = self
            .encode_thread
            .as_ref()
            .is_some_and(|t| !t.is_finished());
        let wr = self
            .writer_thread
            .as_ref()
            .is_some_and(|t| !t.is_finished());
        enc && wr
    }

    /// Saves the last `len` of footage to `out` (`.mp4`) in the background.
    /// Recording continues. The handle yields the written file(s): more than
    /// one if the range spans a display change.
    pub fn save_clip(
        &self,
        len: Duration,
        out: PathBuf,
    ) -> JoinHandle<Result<Vec<PathBuf>, RecorderError>> {
        let tx = self.tx.clone();
        let ring = self.ring.clone();
        let lock = self.lock.clone();
        let ctl = self.ctl.clone();
        let dir = self.info.session_dir.clone();
        let names = self.info.audio_tracks.clone();
        let len_us = len.as_micros() as i64;
        let spawn = thread::Builder::new()
            .name("jc-save".into())
            .spawn(move || {
                set_priority(THREAD_MODE_BACKGROUND_BEGIN);
                save_job(tx, ring, &lock, &ctl, &dir, len_us, &out, &names)
            });
        match spawn {
            Ok(h) => h,
            Err(e) => thread::spawn(move || Err(RecorderError::Io(e))),
        }
    }

    /// Stops recording and deletes the buffer (segments still being saved are
    /// deleted when their save finishes). Returns final stats, or the error
    /// that ended recording early.
    pub fn stop(mut self) -> Result<Stats, RecorderError> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<Stats, RecorderError> {
        self.ctl.stop.store(true, Ordering::Release);
        // Never return early: every step below must run, or the writer can
        // block forever in recv() and the buffer is left on disk.
        let enc_result = match self.encode_thread.take() {
            Some(t) => t.join().unwrap_or(Err(RecorderError::Panicked)),
            None => Ok(()),
        };
        // Flush audio while the writer is still running, so the tail is kept.
        if let Some(mut a) = self.audio.take() {
            a.stop();
        }
        let stats = self.stats();
        self.capture = None; // stops WGC
        // The encode thread normally sends Stop. Send it again in case it
        // panicked first. Harmless if the writer has already exited.
        let _ = self.tx.send(WriterMsg::Stop);
        let writer_result = match self.writer_thread.take() {
            Some(t) => t.join().unwrap_or(Err(WriterError::Panicked)),
            None => Ok(()),
        };
        let (expired, drained) = match self.ring.lock() {
            Ok(mut ring) => (ring.close(), ring.is_drained()),
            Err(_) => (Vec::new(), false),
        };
        segment::delete_segments(&self.info.session_dir, &expired);
        if drained {
            self.lock.release_and_remove();
        }
        // The writer's own error is the root cause when the encoder only saw it go away.
        match (enc_result, writer_result) {
            (_, Err(w)) => Err(w.into()),
            (Err(e), Ok(())) => Err(e),
            (Ok(()), Ok(())) => Ok(stats),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.encode_thread.is_some() || self.writer_thread.is_some() {
            let _ = self.shutdown();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn save_job(
    tx: SyncSender<WriterMsg>,
    ring: Arc<Mutex<Ring>>,
    lock: &SessionLock,
    ctl: &Control,
    dir: &Path,
    len_us: i64,
    out: &Path,
    audio_names: &[&str],
) -> Result<Vec<PathBuf>, RecorderError> {
    let (reply_tx, reply_rx) = mpsc::channel();
    tx.send(WriterMsg::Save {
        len_us,
        reply: reply_tx,
    })
    .map_err(|_| RecorderError::NotRunning)?;
    // Force the cut only after the request is queued. The writer handles
    // messages in order, so the forced IDR always arrives after the Save
    // and the clip ends at this moment rather than at the next natural GOP.
    ctl.force_keyframe.store(true, Ordering::Release);
    let pin = reply_rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| RecorderError::NotRunning)??;

    let result = (|| {
        let mut written = Vec::new();
        for (i, run) in pin.runs().into_iter().enumerate() {
            let inputs: Vec<PathBuf> = run
                .iter()
                .map(|s| segment::segment_path(dir, s.seq))
                .collect();
            let path = clip::run_output_path(out, i);
            clip::concat_to_mp4(&inputs, &path, audio_names)?;
            written.push(path);
        }
        Ok(written)
    })();

    let (expired, drained) = match ring.lock() {
        Ok(mut r) => (r.unpin(pin.id).unwrap_or_default(), r.is_drained()),
        Err(_) => (Vec::new(), false),
    };
    // File IO outside the lock, so stats() and the writer never wait on deletes.
    segment::delete_segments(dir, &expired);
    if drained {
        lock.release_and_remove();
    }
    result
}

fn set_priority(p: THREAD_PRIORITY) {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), p);
    }
}

struct EncodeJob {
    enc: HwEncoder,
    conv: Converter,
    /// Keeps the device alive for as long as the encoder uses it.
    #[allow(dead_code)]
    gpu: Gpu,
    shared: Arc<CaptureShared>,
    ctl: Arc<Control>,
    counters: Arc<Counters>,
    fps: u32,
    /// Shared QPC epoch. Frame n is due at epoch + n/fps, matching audio.
    epoch: i64,
}

/// Hands packets to the writer without ever blocking. If the queue is full,
/// it drops packets until the next keyframe so the stream stays decodable.
struct PacketSink<'a> {
    tx: &'a SyncSender<WriterMsg>,
    counters: &'a Counters,
    ctl: &'a Control,
    skip_to_key: bool,
    writer_gone: bool,
}

impl PacketSink<'_> {
    fn push(&mut self, p: Packet) {
        if self.skip_to_key && !p.is_key() {
            self.counters
                .packets_dropped
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.skip_to_key = false;
        match self.tx.try_send(WriterMsg::Packet(p)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.counters
                    .packets_dropped
                    .fetch_add(1, Ordering::Relaxed);
                // Ask for one IDR per backlog episode. Repeated requests would
                // add large packets to the backlog.
                self.skip_to_key = true;
                self.ctl.force_keyframe.store(true, Ordering::Release);
            }
            Err(TrySendError::Disconnected(_)) => self.writer_gone = true,
        }
    }
}

impl EncodeJob {
    fn run(mut self, tx: &SyncSender<WriterMsg>) -> Result<(), RecorderError> {
        let pacer = Pacer::new(self.fps)?;
        self.wait_first_frame(Duration::from_secs(3))?;
        let mut sink = PacketSink {
            tx,
            counters: &self.counters,
            ctl: &self.ctl,
            skip_to_key: false,
            writer_gone: false,
        };
        // Frame indices are tied to the shared clock (not to when the first
        // frame arrived), so video pts and audio sample indices line up.
        let start = self.epoch;
        let mut n: i64 = pacer.frames_behind(start, 0);
        let mut result = Ok(());
        let stall_frames = i64::from(self.fps); // 1 s
        while !self.ctl.stop.load(Ordering::Acquire) {
            if self.shared.size_changed.load(Ordering::Acquire) {
                result = Err(RecorderError::DisplayChanged);
                break;
            }
            if self.shared.source_closed.load(Ordering::Acquire) {
                result = Err(RecorderError::SourceClosed);
                break;
            }
            if sink.writer_gone {
                result = Err(RecorderError::WriterGone);
                break;
            }
            pacer.wait_until(start, n);
            let late = pacer.frames_behind(start, n);
            if late > stall_frames {
                // Sleep/resume, debugger or GPU hang. Don't paper over the gap:
                // end the run so no segment claims minutes it doesn't contain.
                self.counters
                    .frames_late
                    .fetch_add(late as u64, Ordering::Relaxed);
                if sink.tx.send(WriterMsg::NewRun).is_err() {
                    result = Err(RecorderError::WriterGone);
                    break;
                }
                // Stay on the shared clock: jump ahead, don't rebase.
                n += late;
                self.ctl.force_keyframe.store(true, Ordering::Release);
            } else if late >= 2 {
                self.counters
                    .frames_late
                    .fetch_add(late as u64, Ordering::Relaxed);
                n += late;
            }
            let force = self.ctl.force_keyframe.swap(false, Ordering::AcqRel);
            let conv = &mut self.conv;
            match self
                .enc
                .encode(n, force, |tex, idx| conv.blit(tex, idx), |p| sink.push(p))
            {
                Ok(true) => {
                    self.counters.frames_encoded.fetch_add(1, Ordering::Relaxed);
                }
                Ok(false) => {
                    // Encoder saturated: this tick's frame is skipped.
                    self.counters.frames_late.fetch_add(1, Ordering::Relaxed);
                    if force {
                        self.ctl.force_keyframe.store(true, Ordering::Release);
                    }
                }
                Err(e) => {
                    result = Err(e.into());
                    break;
                }
            }
            n += 1;
        }
        let flushed = self.enc.finish(|p| sink.push(p));
        result?;
        flushed?;
        Ok(())
    }

    fn wait_first_frame(&self, timeout: Duration) -> Result<(), RecorderError> {
        let t0 = Instant::now();
        while !self.shared.has_frame.load(Ordering::Acquire) {
            if self.ctl.stop.load(Ordering::Acquire) {
                return Ok(());
            }
            if t0.elapsed() > timeout {
                return Err(RecorderError::NoFrames(timeout));
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

/// Fixed-rate frame clock on QPC with a high-resolution waitable timer.
struct Pacer {
    timer: HANDLE,
    freq: i64,
    fps: i64,
}

impl Pacer {
    fn new(fps: u32) -> Result<Self, RecorderError> {
        let timer = unsafe {
            CreateWaitableTimerExW(
                None,
                windows::core::PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )?
        };
        let mut freq = 0;
        unsafe { QueryPerformanceFrequency(&mut freq)? };
        Ok(Self {
            timer,
            freq,
            fps: i64::from(fps),
        })
    }

    fn now(&self) -> i64 {
        let mut t = 0;
        unsafe {
            let _ = QueryPerformanceCounter(&mut t);
        }
        t
    }

    fn deadline(&self, start: i64, n: i64) -> i64 {
        start + ((i128::from(n) * i128::from(self.freq)) / i128::from(self.fps)) as i64
    }

    fn wait_until(&self, start: i64, n: i64) {
        let remaining = self.deadline(start, n) - self.now();
        if remaining <= 0 {
            return;
        }
        // Relative due time in 100 ns units (negative = relative).
        let due = -((i128::from(remaining) * 10_000_000 / i128::from(self.freq)) as i64).max(1);
        unsafe {
            if SetWaitableTimer(self.timer, &due, 0, None, None, false).is_ok() {
                WaitForSingleObject(self.timer, INFINITE);
            }
        }
    }

    /// Whole frame periods elapsed since frame `n`'s deadline.
    fn frames_behind(&self, start: i64, n: i64) -> i64 {
        let over = self.now() - self.deadline(start, n);
        if over <= 0 {
            0
        } else {
            (i128::from(over) * i128::from(self.fps) / i128::from(self.freq)) as i64
        }
    }
}

impl Drop for Pacer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.timer);
        }
    }
}
