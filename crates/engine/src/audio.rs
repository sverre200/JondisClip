//! Audio capture and encoding.
//!
//! - One WASAPI capture thread per source: system loopback (game audio) and,
//!   if enabled, the default microphone. Both ask WASAPI for 48 kHz stereo
//!   float (AUTOCONVERTPCM), so there's no resampler. Packets are placed on the
//!   shared timeline with their QPC capture timestamps.
//! - One mixer thread that reads fixed 1024-frame blocks a constant latency
//!   behind real time, mixes, and encodes AAC. Track 1 is always what players
//!   and Discord use (the mix, or the only source).
//!
//! Devices that fail (headset unplugged) are reopened automatically, and so is
//! a source whose Windows default device changes (checked every second). The
//! timeline continues as silence in the meantime.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
use crate::segment::WriterMsg;

pub const SAMPLE_RATE: u32 = 48_000;
/// AAC frame size.
const BLOCK: usize = 1024;
/// How far behind real time the mixer runs. It must exceed WASAPI's packet
/// latency so both sources have delivered before a block is encoded.
const MIX_LATENCY_FRAMES: i64 = SAMPLE_RATE as i64 / 10; // 100 ms
/// Re-anchor a source to its QPC timestamps when it drifts more than this.
const RESYNC_FRAMES: i64 = SAMPLE_RATE as i64 / 50; // 20 ms
/// Below the resync threshold, drift is corrected one frame per packet once it
/// exceeds this. That's inaudible, and it stops device-vs-QPC clock drift from
/// building up into periodic 20 ms jumps.
const DRIFT_DEADBAND_FRAMES: i64 = SAMPLE_RATE as i64 / 1000; // 1 ms
/// If the mixer falls further behind than this (sleep/resume, long hang), it
/// skips ahead instead of encoding the backlog.
const MAX_MIX_BACKLOG_FRAMES: i64 = SAMPLE_RATE as i64; // 1 s
const DEFAULT_DEVICE_POLL: Duration = Duration::from_secs(1);
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub bitrate: u64,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            system: true,
            mic: false,
            mic_device: None,
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
}

// SAFETY: owned and used by one thread at a time (built on the caller, then
// moved into the mixer thread).
unsafe impl Send for AacEncoder {}

impl AacEncoder {
    fn open(bitrate: u64, stream: usize) -> Result<Self, AudioError> {
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
        enc.set_channel_layout(ChannelLayout::STEREO);
        enc.set_format(Sample::F32(ffmpeg::format::sample::Type::Planar));
        enc.set_bit_rate(bitrate as usize);
        enc.set_time_base(Rational::new(1, SAMPLE_RATE as i32));
        let enc = enc.open_as(codec).map_err(err("open aac"))?;
        let mut frame = frame::Audio::new(
            Sample::F32(ffmpeg::format::sample::Type::Planar),
            BLOCK,
            ChannelLayout::STEREO,
        );
        frame.set_rate(SAMPLE_RATE);
        Ok(Self {
            enc,
            frame,
            pkt: Packet::empty(),
            stream,
        })
    }

    fn parameters(&self) -> codec::Parameters {
        codec::Parameters::from(&*self.enc)
    }

    /// Encodes one interleaved stereo block at sample index `pts`.
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
        {
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
    let encoders: Vec<AacEncoder> = (0..names.len())
        .map(|i| AacEncoder::open(cfg.bitrate, i + 1))
        .collect::<Result<_, _>>()?;
    let streams = encoders.iter().map(AacEncoder::parameters).collect();

    let stop = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(AudioCounters::default());
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
        }),
        streams,
    })
}

impl AudioPipeline {
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
            let k = next.get_or_insert(ready.max(BLOCK as i64));
            if ready - *k > MAX_MIX_BACKLOG_FRAMES {
                let skip = ready - BLOCK as i64 - *k;
                self.counters
                    .frames_skipped
                    .fetch_add(skip as u64, Ordering::Relaxed);
                *k += skip;
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
                let result = match (self.game.is_some(), self.mic.is_some()) {
                    (true, true) => {
                        for ((o, a), b) in mix.iter_mut().zip(&g).zip(&m) {
                            *o = (a + b).clamp(-1.0, 1.0);
                        }
                        self.encoders[0]
                            .encode(*k, &mix, &mut sink)
                            .and_then(|()| self.encoders[1].encode(*k, &g, &mut sink))
                            .and_then(|()| self.encoders[2].encode(*k, &m, &mut sink))
                    }
                    (true, false) => self.encoders[0].encode(*k, &g, &mut sink),
                    (false, true) => self.encoders[0].encode(*k, &m, &mut sink),
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
        if capture_session(src, device.as_deref(), &buf, clock, &stop, &mut scratch).is_ok() {
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

    let mut expected: Option<i64> = None;
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
                let Some(at) = place(expected, stamped, ts_ok, discontinuity) else {
                    continue;
                };
                expected = Some(at + i64::from(frames));
                if let Ok(mut b) = buf.lock() {
                    b.push(at, scratch, false);
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

/// Where a packet goes on the timeline. It counts samples from `expected`,
/// nudging one frame per packet towards the QPC stamp when they drift apart by
/// more than 1 ms, and jumps on a discontinuity or a drift over 20 ms. It
/// returns None if the packet can't be placed (no reliable timestamp yet).
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
