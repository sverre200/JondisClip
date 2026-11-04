//! Audio capture and encoding.
//!
//! - One WASAPI capture thread per source: system loopback (game audio) and,
//!   if enabled, the default microphone. Both ask WASAPI for 48 kHz stereo
//!   float (AUTOCONVERTPCM), so there's no resampler. Packets are placed on the
//!   shared timeline with their QPC capture timestamps.
//! - One mixer thread that reads fixed 1024-frame blocks a constant latency
//!   behind real time, mixes, and encodes AAC. Track 1 is always what players
//!   and Discord use (the mix, or the only source).
//! - The mic is folded to mono and gets the user's gain (live, ramped). The
//!   Mix and Mic tracks pass a transparent -1 dBFS lookahead limiter; the Game
//!   track is untouched. Mic is a mono AAC track (ADR-0009).
//!
//! Devices that fail (headset unplugged) are reopened automatically, and so is
//! a source whose Windows default device changes (checked every second). The
//! timeline continues as silence in the meantime.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ffmpeg::{ChannelLayout, Packet, Rational, codec, encoder, ffi, format::Sample, frame};
use ffmpeg_next as ffmpeg;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient,
    IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eCapture, eConsole,
    eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Performance::QueryPerformanceCounter;
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
    WaitForSingleObject,
};
use windows::core::PCWSTR;

use crate::audio_buf::{CHANNELS, SourceBuf};
use crate::audio_dsp::{Asrc, GainRamp, LIMIT_DELAY, Limiter, MonoFold, Placed, db_to_gain};
use crate::segment::WriterMsg;

pub const SAMPLE_RATE: u32 = 48_000;
/// AAC frame size.
const BLOCK: usize = 1024;
/// How far behind real time the mixer runs. It must exceed WASAPI's packet
/// latency so both sources have delivered before a block is encoded.
const MIX_LATENCY_FRAMES: i64 = SAMPLE_RATE as i64 / 10; // 100 ms
/// Re-anchor a source to its QPC timestamps when it drifts more than this.
const RESYNC_FRAMES: i64 = SAMPLE_RATE as i64 / 50; // 20 ms
/// The pre-ADR-0009 corrector's deadband (kept for the before/after test).
#[cfg(test)]
const DRIFT_DEADBAND_FRAMES: i64 = SAMPLE_RATE as i64 / 1000; // 1 ms
/// If the mixer falls further behind than this (sleep/resume, long hang), it
/// skips ahead instead of encoding the backlog.
const MAX_MIX_BACKLOG_FRAMES: i64 = SAMPLE_RATE as i64; // 1 s
const DEFAULT_DEVICE_POLL: Duration = Duration::from_secs(1);
/// Mono voice track bitrate (stereo tracks use `AudioConfig::bitrate`).
const MIC_BITRATE: u64 = 128_000;
pub const MIC_GAIN_MIN_DB: f32 = -12.0;
pub const MIC_GAIN_MAX_DB: f32 = 24.0;

pub fn clamp_gain_db(db: f32) -> f32 {
    if db.is_finite() {
        db.clamp(MIC_GAIN_MIN_DB, MIC_GAIN_MAX_DB)
    } else {
        0.0
    }
}

/// Per-block signal processing for the mixer, separated from IO so routing,
/// levels and limiting are unit-tested.
struct MixDsp {
    fold: MonoFold,
    gain: GainRamp,
    gain_db: Arc<AtomicU32>,
    mix_limit: Limiter<2>,
    mic_limit: Limiter<1>,
}

impl MixDsp {
    fn new(gain_db: Arc<AtomicU32>) -> Self {
        let g = db_to_gain(f32::from_bits(gain_db.load(Ordering::Relaxed)));
        Self {
            fold: MonoFold::default(),
            gain: GainRamp::new(g),
            gain_db,
            mix_limit: Limiter::new(-1.0, 150.0),
            mic_limit: Limiter::new(-1.0, 150.0),
        }
    }

    fn reset_limiters(&mut self) {
        self.mix_limit.reset();
        self.mic_limit.reset();
    }

    /// From one block of game (stereo) and mic (stereo) input, fills
    /// `mic_mono` (gained, unlimited), `mix` (stereo, limited) and
    /// `mic_track` (mono, limited). The limited outputs are delayed
    /// `LIMIT_DELAY` frames (compensated in pts).
    fn process(
        &mut self,
        game: &[f32],
        mic: &[f32],
        mic_mono: &mut [f32],
        mix: &mut [f32],
        mic_track: &mut [f32],
    ) {
        self.fold.process(mic, mic_mono);
        let target = db_to_gain(f32::from_bits(self.gain_db.load(Ordering::Relaxed)));
        self.gain.apply(target, mic_mono);
        for (i, f) in mix.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            // Voice centred: the same mono signal on both sides.
            f[0] = game[i * 2] + mic_mono[i];
            f[1] = game[i * 2 + 1] + mic_mono[i];
        }
        self.mix_limit.process(mix);
        mic_track.copy_from_slice(mic_mono);
        self.mic_limit.process(mic_track);
    }
}
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct AudioConfig {
    /// Game/system audio (loopback of the default output device).
    pub system: bool,
    /// Record a microphone. Off by default: never record a voice by surprise.
    pub mic: bool,
    /// Endpoint ID of the microphone to record, or None for the Windows
    /// default (which is followed if it changes). If a chosen device is missing,
    /// the mic track is silent until it returns. It never falls back to a
    /// different microphone.
    pub mic_device: Option<String>,
    /// Mic gain in dB (-12 to +24), changeable live via `AudioPipeline::set_mic_gain_db`.
    pub mic_gain_db: f32,
    pub bitrate: u64,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            system: true,
            mic: false,
            mic_device: None,
            mic_gain_db: 0.0,
            bitrate: 160_000,
        }
    }
}

impl AudioConfig {
    /// Track names in stream order (stream 1, 2, ...).
    pub fn track_names(&self) -> Vec<&'static str> {
        match (self.system, self.mic) {
            (true, true) => vec!["Mix", "Game", "Microphone"],
            (true, false) => vec!["Game"],
            (false, true) => vec!["Microphone"],
            (false, false) => vec![],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("FFmpeg {what}: {source}")]
    Ffmpeg {
        what: &'static str,
        source: ffmpeg::Error,
    },
    #[error("thread: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Default)]
pub struct AudioCounters {
    pub blocks_encoded: AtomicU64,
    /// Frames skipped because the mixer fell behind (e.g. after sleep).
    pub frames_skipped: AtomicU64,
    pub packets_dropped: AtomicU64,
    pub device_reopens: AtomicU64,
    /// The microphone capture stream is open and running.
    pub mic_connected: AtomicBool,
}

/// Shared clock: QPC ticks at the recorder's epoch.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    pub epoch: i64,
    pub freq: i64,
}

impl Clock {
    pub fn now_qpc() -> i64 {
        let mut t = 0;
        unsafe {
            let _ = QueryPerformanceCounter(&mut t);
        }
        t
    }

    /// Sample index (48 kHz) for a raw QPC value.
    pub fn qpc_to_frames(&self, qpc: i64) -> i64 {
        ((i128::from(qpc - self.epoch) * i128::from(SAMPLE_RATE)) / i128::from(self.freq)) as i64
    }

    /// Sample index for a WASAPI QPC position (100 ns units).
    fn hns_to_frames(&self, hns: u64) -> i64 {
        let epoch_hns = (i128::from(self.epoch) * 10_000_000) / i128::from(self.freq);
        (((hns as i128) - epoch_hns) * i128::from(SAMPLE_RATE) / 10_000_000) as i64
    }
}

struct AacEncoder {
    enc: encoder::Audio,
    frame: frame::Audio,
    pkt: Packet,
    stream: usize,
    channels: usize,
}

// SAFETY: owned and used by one thread at a time (built on the caller, then
// moved into the mixer thread).
unsafe impl Send for AacEncoder {}

impl AacEncoder {
    fn open(bitrate: u64, stream: usize, channels: usize) -> Result<Self, AudioError> {
        let layout = if channels == 1 {
            ChannelLayout::MONO
        } else {
            ChannelLayout::STEREO
        };
        let err = |what| move |source| AudioError::Ffmpeg { what, source };
        let codec = encoder::find(codec::Id::AAC).ok_or(AudioError::Ffmpeg {
            what: "find aac",
            source: ffmpeg::Error::EncoderNotFound,
        })?;
        let mut enc = codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .map_err(err("aac context"))?;
        enc.set_rate(SAMPLE_RATE as i32);
        enc.set_channel_layout(layout);
        enc.set_format(Sample::F32(ffmpeg::format::sample::Type::Planar));
        enc.set_bit_rate(bitrate as usize);
        enc.set_time_base(Rational::new(1, SAMPLE_RATE as i32));
        let enc = enc.open_as(codec).map_err(err("open aac"))?;
        let mut frame = frame::Audio::new(
            Sample::F32(ffmpeg::format::sample::Type::Planar),
            BLOCK,
            layout,
        );
        frame.set_rate(SAMPLE_RATE);
        Ok(Self {
            enc,
            frame,
            pkt: Packet::empty(),
            stream,
            channels,
        })
    }

    fn parameters(&self) -> codec::Parameters {
        codec::Parameters::from(&*self.enc)
    }

    /// Encodes one interleaved block (`channels` wide) at sample index `pts`.
    fn encode(
        &mut self,
        pts: i64,
        interleaved: &[f32],
        sink: &mut impl FnMut(Packet),
    ) -> Result<(), AudioError> {
        unsafe {
            // The encoder copies samples internally, so this is normally a no-op.
            ffi::av_frame_make_writable(self.frame.as_mut_ptr());
        }
        if self.channels == 1 {
            // SAFETY: planar float mono frame with BLOCK samples.
            let m = unsafe {
                std::slice::from_raw_parts_mut(
                    (*self.frame.as_mut_ptr()).data[0] as *mut f32,
                    BLOCK,
                )
            };
            m.copy_from_slice(&interleaved[..BLOCK]);
        } else {
            let (l, r) = self.frame_planes();
            for (i, [a, b]) in interleaved.as_chunks::<CHANNELS>().0.iter().enumerate() {
                l[i] = *a;
                r[i] = *b;
            }
        }
        self.frame.set_pts(Some(pts));
        self.enc
            .send_frame(&self.frame)
            .map_err(|source| AudioError::Ffmpeg {
                what: "aac send",
                source,
            })?;
        self.drain(sink);
        Ok(())
    }

    fn frame_planes(&mut self) -> (&mut [f32], &mut [f32]) {
        // SAFETY: planar float stereo frame with BLOCK samples per plane,
        // allocated by `frame::Audio::new`.
        unsafe {
            let f = self.frame.as_mut_ptr();
            let l = std::slice::from_raw_parts_mut((*f).data[0] as *mut f32, BLOCK);
            let r = std::slice::from_raw_parts_mut((*f).data[1] as *mut f32, BLOCK);
            (l, r)
        }
    }

    fn finish(&mut self, sink: &mut impl FnMut(Packet)) {
        let _ = self.enc.send_eof();
        self.drain(sink);
    }

    fn drain(&mut self, sink: &mut impl FnMut(Packet)) {
        while self.enc.receive_packet(&mut self.pkt).is_ok() {
            let mut p = std::mem::replace(&mut self.pkt, Packet::empty());
            p.set_stream(self.stream);
            sink(p);
        }
    }
}

#[derive(Clone, Copy)]
enum Source {
    Loopback,
    Mic,
}

pub struct AudioPipeline {
    mic_gain: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    mixer: Option<JoinHandle<()>>,
    pub counters: Arc<AudioCounters>,
}

pub struct AudioSetup {
    pub pipeline: Option<AudioPipeline>,
    /// Codec parameters for audio streams 1..=n, in order.
    pub streams: Vec<codec::Parameters>,
}

/// Opens the encoders (so their parameters can be given to the muxer) and
/// starts capture. With no sources enabled, it returns an empty setup.
pub fn start(
    cfg: AudioConfig,
    clock: Clock,
    tx: SyncSender<WriterMsg>,
) -> Result<AudioSetup, AudioError> {
    let names = cfg.track_names();
    if names.is_empty() {
        return Ok(AudioSetup {
            pipeline: None,
            streams: Vec::new(),
        });
    }
    // Stream i+1 in track order; the Microphone track is mono.
    let encoders: Vec<AacEncoder> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if *name == "Microphone" {
                AacEncoder::open(MIC_BITRATE, i + 1, 1)
            } else {
                AacEncoder::open(cfg.bitrate, i + 1, 2)
            }
        })
        .collect::<Result<_, _>>()?;
    let streams = encoders.iter().map(AacEncoder::parameters).collect();

    let stop = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(AudioCounters::default());
    let mic_gain = Arc::new(AtomicU32::new(clamp_gain_db(cfg.mic_gain_db).to_bits()));
    let max_frames = SAMPLE_RATE as usize * 2;
    let game = cfg
        .system
        .then(|| Arc::new(Mutex::new(SourceBuf::new(max_frames))));
    let mic = cfg
        .mic
        .then(|| Arc::new(Mutex::new(SourceBuf::new(max_frames))));

    let mic_device = cfg.mic_device.clone();
    let mut threads = Vec::new();
    for (src, buf, name) in [
        (Source::Loopback, &game, "jc-audio-loopback"),
        (Source::Mic, &mic, "jc-audio-mic"),
    ] {
        if let Some(buf) = buf {
            let (buf, stop, counters) = (buf.clone(), stop.clone(), counters.clone());
            let device = match src {
                Source::Mic => mic_device.clone(),
                Source::Loopback => None,
            };
            threads.push(
                thread::Builder::new()
                    .name(name.into())
                    .spawn(move || capture_thread(src, device, buf, clock, stop, counters))?,
            );
        }
    }
    let mixer = {
        let (stop, counters) = (stop.clone(), counters.clone());
        let mut m = Mixer {
            encoders,
            game,
            mic,
            clock,
            tx,
            counters,
            dsp: MixDsp::new(mic_gain.clone()),
        };
        thread::Builder::new()
            .name("jc-audio-mix".into())
            .spawn(move || {
                unsafe {
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
                }
                m.run(&stop);
            })?
    };
    Ok(AudioSetup {
        pipeline: Some(AudioPipeline {
            stop,
            threads,
            mixer: Some(mixer),
            counters,
            mic_gain,
        }),
        streams,
    })
}

impl AudioPipeline {
    /// Changes the mic gain live (ramped over one block, no restart).
    pub fn set_mic_gain_db(&self, db: f32) {
        self.mic_gain
            .store(clamp_gain_db(db).to_bits(), Ordering::Relaxed);
    }

    /// Stops capture, flushes the encoders and joins every audio thread.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        if let Some(m) = self.mixer.take() {
            let _ = m.join();
        }
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Mixer {
    dsp: MixDsp,
    encoders: Vec<AacEncoder>,
    game: Option<Arc<Mutex<SourceBuf>>>,
    mic: Option<Arc<Mutex<SourceBuf>>>,
    clock: Clock,
    tx: SyncSender<WriterMsg>,
    counters: Arc<AudioCounters>,
}

impl Mixer {
    fn run(&mut self, stop: &AtomicBool) {
        let mut g = vec![0.0f32; BLOCK * CHANNELS];
        let mut m = vec![0.0f32; BLOCK * CHANNELS];
        let mut mix = vec![0.0f32; BLOCK * CHANNELS];
        let mut mic_mono = vec![0.0f32; BLOCK];
        let mut mic_track = vec![0.0f32; BLOCK];
        let mut next: Option<i64> = None;
        let tx = self.tx.clone();
        let counters = self.counters.clone();
        let mut sink = |p: Packet| {
            // Audio packets are all independently decodable, so a drop is just a gap.
            if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
                tx.try_send(WriterMsg::Packet(p))
            {
                counters.packets_dropped.fetch_add(1, Ordering::Relaxed);
            }
        };
        while !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(10));
            let ready = self.clock.qpc_to_frames(Clock::now_qpc()) - MIX_LATENCY_FRAMES;
            // Start at least one block in: the AAC encoder's first packet is
            // stamped one block early (priming), and negative timestamps make the
            // TS muxer shift the whole first segment.
            // The limited tracks are encoded at k - LIMIT_DELAY, so add that too.
            let k = next.get_or_insert(ready.max(BLOCK as i64 + LIMIT_DELAY));
            if ready - *k > MAX_MIX_BACKLOG_FRAMES {
                let skip = ready - BLOCK as i64 - *k;
                self.counters
                    .frames_skipped
                    .fetch_add(skip as u64, Ordering::Relaxed);
                *k += skip;
                // Don't emit pre-skip audio still inside the limiters' delay.
                self.dsp.reset_limiters();
            }
            while *k + BLOCK as i64 <= ready {
                if let Some(b) = &self.game
                    && let Ok(mut b) = b.lock()
                {
                    b.read(*k, &mut g);
                }
                if let Some(b) = &self.mic
                    && let Ok(mut b) = b.lock()
                {
                    b.read(*k, &mut m);
                }
                // Game is silent when it isn't recorded, so one DSP path serves
                // every layout. Limited outputs carry LIMIT_DELAY in their pts.
                if self.game.is_none() {
                    g.fill(0.0);
                }
                if self.mic.is_some() {
                    self.dsp
                        .process(&g, &m, &mut mic_mono, &mut mix, &mut mic_track);
                }
                let lk = *k - LIMIT_DELAY;
                let result = match (self.game.is_some(), self.mic.is_some()) {
                    (true, true) => self.encoders[0]
                        .encode(lk, &mix, &mut sink)
                        .and_then(|()| self.encoders[1].encode(*k, &g, &mut sink))
                        .and_then(|()| self.encoders[2].encode(lk, &mic_track, &mut sink)),
                    (true, false) => self.encoders[0].encode(*k, &g, &mut sink),
                    (false, true) => self.encoders[0].encode(lk, &mic_track, &mut sink),
                    (false, false) => Ok(()),
                };
                if result.is_err() {
                    self.counters
                        .packets_dropped
                        .fetch_add(1, Ordering::Relaxed);
                }
                self.counters.blocks_encoded.fetch_add(1, Ordering::Relaxed);
                *k += BLOCK as i64;
            }
        }
        for e in &mut self.encoders {
            e.finish(&mut sink);
        }
    }
}

fn capture_thread(
    src: Source,
    device: Option<String>,
    buf: Arc<Mutex<SourceBuf>>,
    clock: Clock,
    stop: Arc<AtomicBool>,
    counters: Arc<AudioCounters>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
    }
    let mut scratch: Vec<f32> = Vec::with_capacity(SAMPLE_RATE as usize * CHANNELS / 2);
    let mut first = true;
    while !stop.load(Ordering::Acquire) {
        if !first {
            counters.device_reopens.fetch_add(1, Ordering::Relaxed);
        }
        first = false;
        // Ends on stop (Ok) or a device error. On error, retry after a pause.
        let ok = capture_session(
            src,
            device.as_deref(),
            &buf,
            clock,
            &stop,
            &mut scratch,
            &counters,
        )
        .is_ok();
        if matches!(src, Source::Mic) {
            counters.mic_connected.store(false, Ordering::Relaxed);
        }
        if ok {
            break;
        }
        for _ in 0..10 {
            if stop.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    unsafe { CoUninitialize() };
}

struct EventHandle(HANDLE);
impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn capture_session(
    src: Source,
    chosen: Option<&str>,
    buf: &Mutex<SourceBuf>,
    clock: Clock,
    stop: &AtomicBool,
    scratch: &mut Vec<f32>,
    counters: &AudioCounters,
) -> windows::core::Result<()> {
    let flow = flow_of(src);
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device = match chosen {
        // A specific device. If it's unplugged, Activate fails and the caller
        // retries; the timeline stays silent meanwhile.
        Some(id) => unsafe { enumerator.GetDevice(&windows::core::HSTRING::from(id))? },
        None => unsafe { enumerator.GetDefaultAudioEndpoint(flow, eConsole)? },
    };
    let device_id = endpoint_id(&device);
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let fmt = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
        nChannels: CHANNELS as u16,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * (CHANNELS as u32) * 4,
        nBlockAlign: (CHANNELS as u16) * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    if matches!(src, Source::Loopback) {
        flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    }
    let event = EventHandle(unsafe { CreateEventW(None, false, false, PCWSTR::null())? });
    let capture: IAudioCaptureClient = unsafe {
        // 200 ms device buffer: generous, so a busy system never overflows it.
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &fmt, None)?;
        client.SetEventHandle(event.0)?;
        let c = client.GetService()?;
        client.Start()?;
        c
    };
    if matches!(src, Source::Mic) {
        counters.mic_connected.store(true, Ordering::Relaxed);
    }

    let mut timeline = Timeline::default();
    let mut last_poll = Instant::now();
    let result = (|| {
        while !stop.load(Ordering::Acquire) {
            // Loopback may not signal at all while nothing plays. The timeout
            // keeps the stop flag responsive, and the mixer fills silence.
            if chosen.is_none() && last_poll.elapsed() >= DEFAULT_DEVICE_POLL {
                last_poll = Instant::now();
                // The user switched output or input: reopen on the new default.
                let current = unsafe { enumerator.GetDefaultAudioEndpoint(flow, eConsole) }
                    .ok()
                    .and_then(|d| endpoint_id(&d));
                if current != device_id {
                    return Err(windows::core::Error::from(
                        windows::Win32::Foundation::E_CHANGED_STATE,
                    ));
                }
            }
            let w = unsafe { WaitForSingleObject(event.0, 20) };
            if w != WAIT_OBJECT_0 {
                continue;
            }
            loop {
                let pending = unsafe { capture.GetNextPacketSize()? };
                if pending == 0 {
                    break;
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut bflags = 0u32;
                let mut qpc_pos = 0u64;
                unsafe {
                    capture.GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut bflags,
                        None,
                        Some(&mut qpc_pos),
                    )?;
                }
                let n = frames as usize * CHANNELS;
                let silent = bflags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null();
                scratch.clear();
                if silent {
                    scratch.resize(n, 0.0);
                } else {
                    // SAFETY: WASAPI hands us `frames` frames in the requested
                    // 32-bit float stereo format.
                    let s = unsafe { std::slice::from_raw_parts(data as *const f32, n) };
                    scratch.extend_from_slice(s);
                }
                unsafe { capture.ReleaseBuffer(frames)? };

                let ts_ok =
                    bflags & (AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32) == 0 && qpc_pos != 0;
                let discontinuity = bflags & (AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32) != 0;
                let stamped = clock.hns_to_frames(qpc_pos);
                // Resample outside the lock; hold it only to append.
                if let Some(p) = timeline.prepare(stamped, scratch, ts_ok, discontinuity)
                    && let Ok(mut b) = buf.lock()
                {
                    timeline.publish(p, &mut b);
                }
            }
        }
        Ok(())
    })();
    unsafe {
        let _ = client.Stop();
    }
    result
}

fn flow_of(src: Source) -> windows::Win32::Media::Audio::EDataFlow {
    match src {
        Source::Loopback => eRender,
        Source::Mic => eCapture,
    }
}

/// The endpoint's id string, used to notice default-device changes.
fn endpoint_id(device: &windows::Win32::Media::Audio::IMMDevice) -> Option<String> {
    unsafe {
        let p = device.GetId().ok()?;
        let id = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        id
    }
}

/// Places capture packets on the shared timeline. The device clock is
/// followed with a drift-correcting resampler ([`Asrc`]), so the stream stays
/// continuous: no inserted zeros or slipped samples, only fades at real
/// discontinuities (ADR-0009).
struct Timeline {
    asrc: Asrc,
    out: Vec<f32>,
}

impl Default for Timeline {
    fn default() -> Self {
        // Preallocated once: a packet is at most ~100 ms (4800 frames).
        Self {
            asrc: Asrc::default(),
            out: Vec::with_capacity(SAMPLE_RATE as usize * CHANNELS / 4),
        }
    }
}

impl Timeline {
    /// Resamples one packet stamped at QPC frame `stamped`. No lock is
    /// needed; `publish` then hands the result to the buffer.
    fn prepare(
        &mut self,
        stamped: i64,
        samples: &[f32],
        ts_ok: bool,
        discontinuity: bool,
    ) -> Option<Placed> {
        self.out.clear();
        let st = ts_ok.then_some(stamped);
        self.asrc
            .process(st, samples, discontinuity, RESYNC_FRAMES, &mut self.out)
    }

    fn publish(&self, p: Placed, buf: &mut SourceBuf) {
        buf.push(p.at, &self.out, p.restarted);
    }

    /// Prepare and publish in one step (tests).
    #[cfg(test)]
    fn push(
        &mut self,
        stamped: i64,
        samples: &[f32],
        ts_ok: bool,
        discontinuity: bool,
        buf: &mut SourceBuf,
    ) {
        if let Some(p) = self.prepare(stamped, samples, ts_ok, discontinuity) {
            self.publish(p, buf);
        }
    }

    /// Timeline position just after the last pushed frame.
    #[cfg(test)]
    fn end(&self) -> Option<i64> {
        self.asrc.end()
    }
}

/// BEFORE baseline (pre-ADR-0009), kept only for the comparison test: the old
/// per-packet corrector. Where a packet goes on the timeline. It counts samples from `expected`,
/// nudging one frame per packet towards the QPC stamp when they drift apart by
/// more than 1 ms, and jumps on a discontinuity or a drift over 20 ms. It
/// returns None if the packet can't be placed (no reliable timestamp yet).
#[cfg(test)]
fn place(expected: Option<i64>, stamped: i64, ts_ok: bool, discontinuity: bool) -> Option<i64> {
    match expected {
        None => ts_ok.then_some(stamped),
        Some(e) if !ts_ok => Some(e),
        Some(e) => {
            let err = stamped - e;
            if discontinuity || err.abs() > RESYNC_FRAMES {
                Some(stamped)
            } else if err > DRIFT_DEADBAND_FRAMES {
                Some(e + 1)
            } else if err < -DRIFT_DEADBAND_FRAMES {
                Some(e - 1)
            } else {
                Some(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_dsp::db_to_gain;

    /// Deterministic capture simulation used for before/after comparisons
    /// (docs/perf/2026-11-04-mic-audio.md). A device produces a pure tone on
    /// its own clock; packets arrive with QPC stamps that include clock drift
    /// and jitter (profiles measured with `audio_probe jitter`). The output
    /// timeline is checked for clicks and slips: any sample deviating from a
    /// smooth waveform.
    pub(super) mod sim {
        pub struct Profile {
            pub name: &'static str,
            pub drift_ppm: f64,
            pub jitter_ms: f64,
            pub spike_ms: f64,
            pub spike_every: u64,
        }

        pub const PROFILES: [Profile; 3] = [
            Profile {
                name: "virtual cable",
                drift_ppm: 1.0,
                jitter_ms: 0.20,
                spike_ms: 2.0,
                spike_every: 100,
            },
            Profile {
                name: "Samson USB",
                drift_ppm: 28.0,
                jitter_ms: 0.17,
                spike_ms: 0.6,
                spike_every: 400,
            },
            Profile {
                name: "Arctis USB",
                drift_ppm: 5.0,
                jitter_ms: 0.17,
                spike_ms: 0.55,
                spike_every: 400,
            },
        ];

        struct Rng(u64);
        impl Rng {
            fn uniform(&mut self) -> f64 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 >> 11) as f64 / (1u64 << 53) as f64
            }
            fn gauss(&mut self) -> f64 {
                let (u1, u2) = (self.uniform().max(1e-12), self.uniform());
                (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
            }
        }

        /// Packets of (QPC-stamped frame position, interleaved stereo tone).
        pub fn packets(p: &Profile, secs: u64, freq: f64, amp: f32) -> Vec<(i64, Vec<f32>)> {
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
            let pkt = 480usize;
            let n_pkts = secs * 48_000 / pkt as u64;
            let mut out = Vec::with_capacity(n_pkts as usize);
            for k in 0..n_pkts {
                let j0 = k * pkt as u64;
                let mut jitter = rng.gauss() * p.jitter_ms;
                if k % p.spike_every == p.spike_every / 2 {
                    jitter += p.spike_ms;
                }
                let stamped = (j0 as f64 * (1.0 + p.drift_ppm * 1e-6) + jitter * 48.0).round()
                    as i64
                    + 48_000;
                let mut s = Vec::with_capacity(pkt * 2);
                for j in j0..j0 + pkt as u64 {
                    let v = amp
                        * (2.0 * std::f64::consts::PI * freq * j as f64 / 48_000.0).sin() as f32;
                    s.push(v);
                    s.push(v);
                }
                out.push((stamped, s));
            }
            out
        }

        /// Counts samples that break a smooth sinusoid (clicks, slips), ignoring
        /// the first and last 0.5 s. Returns (glitches, worst deviation / amplitude).
        pub fn glitches(y: &[f32], freq: f64, amp: f32) -> (usize, f64) {
            let w2 = (2.0 * std::f64::consts::PI * freq / 48_000.0).powi(2);
            let (mut count, mut worst) = (0usize, 0f64);
            for n in 24_000..y.len().saturating_sub(24_000) {
                let d2 = f64::from(y[n + 1]) - 2.0 * f64::from(y[n]) + f64::from(y[n - 1]);
                let dev = (d2 + w2 * f64::from(y[n])).abs() / f64::from(amp);
                worst = worst.max(dev);
                if dev > 0.002 {
                    count += 1;
                }
            }
            (count, worst)
        }
    }

    fn stereo_tone(freq: f64, amp: f32, offset: usize) -> Vec<f32> {
        (0..BLOCK)
            .flat_map(|i| {
                let v = amp
                    * (2.0 * std::f64::consts::PI * freq * (i + offset) as f64 / 48_000.0).sin()
                        as f32;
                [v, v]
            })
            .collect()
    }

    fn run_dsp(
        dsp: &mut MixDsp,
        blocks: usize,
        game_amp: f32,
        mic_amp: f32,
    ) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let (mut mixes, mut mics, mut monos) = (Vec::new(), Vec::new(), Vec::new());
        let (mut mono, mut mix, mut mt) =
            (vec![0f32; BLOCK], vec![0f32; BLOCK * 2], vec![0f32; BLOCK]);
        for b in 0..blocks {
            let g = stereo_tone(110.0, game_amp, b * BLOCK);
            let m = stereo_tone(440.0, mic_amp, b * BLOCK);
            dsp.process(&g, &m, &mut mono, &mut mix, &mut mt);
            mixes.extend_from_slice(&mix);
            mics.extend_from_slice(&mt);
            monos.extend_from_slice(&mono);
        }
        (mixes, mics, monos)
    }

    #[test]
    fn mixer_start_keeps_every_track_pts_non_negative() {
        // The mixer starts at ready.max(BLOCK + LIMIT_DELAY); AAC stamps its
        // first packet one block early; limited tracks use k - LIMIT_DELAY.
        let k0 = 0i64.max(BLOCK as i64 + LIMIT_DELAY);
        assert!((k0 - LIMIT_DELAY) - BLOCK as i64 >= 0, "limited tracks");
        assert!(k0 - BLOCK as i64 >= 0, "game track");
    }

    #[test]
    fn mic_keeps_its_exact_level_and_mix_is_game_plus_voice() {
        let mut dsp = MixDsp::new(Arc::new(AtomicU32::new(0f32.to_bits())));
        let (mix, mic, mono) = run_dsp(&mut dsp, 8, 0.3, 0.2);
        let d = LIMIT_DELAY as usize;
        for i in 2000..BLOCK * 8 {
            // Duplicated-mono mic folds to the same level (no -3/-6 dB loss).
            let expect_mic =
                0.2 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 48_000.0).sin() as f32;
            assert!((mono[i] - expect_mic).abs() < 1e-5, "mono level at {i}");
            // Mic track = mono, delayed by the (inactive) limiter.
            assert!((mic[i] - mono[i - d]).abs() < 1e-6);
            // Mix = game + voice on both sides, untouched below the ceiling.
            let g =
                0.3 * (2.0 * std::f64::consts::PI * 110.0 * (i - d) as f64 / 48_000.0).sin() as f32;
            assert!(
                (mix[i * 2] - (g + mono[i - d])).abs() < 1e-5,
                "mix L at {i}"
            );
            assert_eq!(
                mix[i * 2],
                mix[i * 2 + 1],
                "voice centred, channels balanced"
            );
        }
    }

    #[test]
    fn loud_game_plus_voice_never_clips_the_mix() {
        let mut dsp = MixDsp::new(Arc::new(AtomicU32::new(0f32.to_bits())));
        // Game near full scale (as measured: -0.2 dBFS) plus voice peaks.
        let (mix, mic, _) = run_dsp(&mut dsp, 20, 0.98, 0.5);
        let ceiling = db_to_gain(-1.0);
        assert!(
            mix.iter().all(|v| v.abs() <= ceiling + 1e-6),
            "mix stays under -1 dBFS"
        );
        assert!(mic.iter().all(|v| v.abs() <= ceiling + 1e-6));
        // The old clamp would have flattened about 1/3 of the waveform; the
        // limiter keeps the waveform shape (no flat-topped runs).
        let flat = mix
            .windows(3)
            .filter(|w| w[0] == w[2] && w[0].abs() > 0.8)
            .count();
        assert!(flat < 10, "no hard-clipped plateaus ({flat})");
    }

    #[test]
    fn mic_gain_applies_live_and_mic_only_layout_works() {
        let gain = Arc::new(AtomicU32::new(0f32.to_bits()));
        let mut dsp = MixDsp::new(gain.clone());
        let _ = run_dsp(&mut dsp, 2, 0.0, 0.1);
        gain.store(6.0206f32.to_bits(), Ordering::Relaxed); // x2
        let (_, mic, mono) = run_dsp(&mut dsp, 3, 0.0, 0.1);
        let peak = |v: &[f32]| v.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(
            (peak(&mono[BLOCK * 2..]) - 0.2).abs() < 1e-3,
            "+6 dB doubles the voice"
        );
        assert!(peak(&mic[BLOCK * 2..]) < db_to_gain(-1.0) + 1e-6);
        assert_eq!(clamp_gain_db(99.0), MIC_GAIN_MAX_DB);
        assert_eq!(clamp_gain_db(f32::NAN), 0.0);
    }

    /// Reads the SourceBuf in mixer-sized blocks into one left-channel stream.
    fn drain_left(buf: &mut SourceBuf, from: i64, to: i64) -> Vec<f32> {
        let mut out = Vec::new();
        let mut block = vec![0.0f32; BLOCK * CHANNELS];
        let mut k = from;
        while k + BLOCK as i64 <= to {
            buf.read(k, &mut block);
            out.extend(block.iter().step_by(2));
            k += BLOCK as i64;
        }
        out
    }

    #[test]
    fn capture_timeline_has_no_clicks_or_slips() {
        let mut report = Vec::new();
        for p in &sim::PROFILES {
            let pkts = sim::packets(p, 60, 440.0, 0.5);
            let mut buf = SourceBuf::new(48_000 * 4);
            let mut out = Vec::new();
            let mut tl = Timeline::default();
            let first = pkts[0].0;
            let mut read_k = first + 4800;
            for (stamped, samples) in &pkts {
                tl.push(*stamped, samples, true, false, &mut buf);
                let latest = tl.end().unwrap_or(0);
                if latest - 4800 > read_k + BLOCK as i64 {
                    let to = latest - 4800;
                    let n = (to - read_k) / BLOCK as i64 * BLOCK as i64;
                    out.extend(drain_left(&mut buf, read_k, read_k + n));
                    read_k += n;
                }
            }
            let (g, worst) = sim::glitches(&out, 440.0, 0.5);
            report.push(format!(
                "{}: {} glitches ({:.2}/s), worst {:.4}",
                p.name,
                g,
                g as f64 / 59.0,
                worst
            ));
        }
        println!(
            "{}",
            report.join(
                "
"
            )
        );
        assert!(
            report.iter().all(|r| r.contains(": 0 glitches")),
            "clicks/slips found:
{}",
            report.join(
                "
"
            )
        );
    }

    #[test]
    fn place_follows_sample_count_within_deadband() {
        assert_eq!(place(Some(1000), 1030, true, false), Some(1000));
        assert_eq!(place(Some(1000), 970, true, false), Some(1000));
    }

    #[test]
    fn place_nudges_one_frame_for_small_drift() {
        assert_eq!(place(Some(1000), 1000 + 100, true, false), Some(1001));
        assert_eq!(place(Some(1000), 1000 - 100, true, false), Some(999));
    }

    #[test]
    fn place_jumps_on_large_drift_or_discontinuity() {
        assert_eq!(place(Some(1000), 1000 + 2000, true, false), Some(3000));
        assert_eq!(place(Some(1000), 1010, true, true), Some(1010));
    }

    #[test]
    fn place_ignores_bad_timestamps() {
        assert_eq!(place(Some(1000), -5_000_000, false, false), Some(1000));
        assert_eq!(place(None, 0, false, false), None);
        assert_eq!(place(None, 500, true, false), Some(500));
    }
}
