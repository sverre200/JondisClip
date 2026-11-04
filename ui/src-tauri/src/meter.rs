//! Microphone level meter for the settings window.
//!
//! It runs only while the window asks for levels: a small shared-mode capture
//! of the selected mic in *this* (UI) process, so the recorder is never
//! touched, and it stops by itself 2 s after the last read (window closed or
//! hidden). It reports the peak and RMS since the previous read, folded to mono
//! like the recorder, with the user's gain applied, i.e. what will be
//! recorded.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient, IAudioClient,
    IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eCapture, eConsole,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::HSTRING;

#[derive(Default)]
struct Acc {
    /// Per-channel peak and energy since the last read; folded to mono at read
    /// time with the recorder's rule (average, or the live side of a one-sided mic).
    peak: [f32; 2],
    sum_sq: [f64; 2],
    n: u64,
    error: Option<String>,
}

struct Running {
    device: String,
    /// When the capture thread ended (error or idle), for retry backoff.
    ended_at: Arc<Mutex<Option<Instant>>>,
    stop: Arc<AtomicBool>,
    acc: Arc<Mutex<Acc>>,
    last_read: Arc<Mutex<Instant>>,
}

static METER: Mutex<Option<Running>> = Mutex::new(None);

#[derive(Serialize)]
pub struct Level {
    /// Peak since the last read, dBFS after gain (-100 = silence).
    pub peak_db: f32,
    pub rms_db: f32,
    /// The microphone couldn't be opened (unplugged, in exclusive use...).
    pub error: Option<String>,
}

fn db(v: f64) -> f32 {
    if v > 1e-5 {
        (20.0 * v.log10()) as f32
    } else {
        -100.0
    }
}

/// Wait this long before retrying a mic that failed to open.
const RETRY_AFTER: Duration = Duration::from_millis(1500);

/// Starts (or retargets) the meter and returns the level since the last call.
pub fn read(device: &str, gain_db: f32) -> Level {
    let mut m = METER.lock().unwrap_or_else(|e| e.into_inner());
    let restart = m.as_ref().is_some_and(|r| {
        let ended = r.ended_at.lock().map(|t| *t).unwrap_or(None);
        // Retarget at once; after a failure, keep reporting the error and
        // only retry after a pause (no thread storm while a mic is missing).
        r.device != device || ended.is_some_and(|t| t.elapsed() >= RETRY_AFTER)
    });
    if restart && let Some(r) = m.take() {
        r.stop.store(true, Ordering::Relaxed);
    }
    let r = m.get_or_insert_with(|| spawn(device.to_string()));
    *r.last_read.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    let mut acc = r.acc.lock().unwrap_or_else(|e| e.into_inner());
    let g = 10f64.powf(f64::from(gain_db) / 20.0);
    // Same fold as the recorder (MonoFold): one side >30 dB quieter means a
    // one-sided mic, recorded from its live side at full level.
    let (el, er) = (acc.sum_sq[0], acc.sum_sq[1]);
    let (peak, energy) = if el > 0.0 && er < el * 1e-3 {
        (f64::from(acc.peak[0]), el)
    } else if er > 0.0 && el < er * 1e-3 {
        (f64::from(acc.peak[1]), er)
    } else {
        // Average of L and R: for duplicated mono equal to either side.
        (f64::from(acc.peak[0].max(acc.peak[1])), (el + er) / 2.0)
    };
    let level = Level {
        peak_db: db(peak * g),
        rms_db: if acc.n > 0 {
            db((energy / acc.n as f64).sqrt() * g)
        } else {
            -100.0
        },
        error: acc.error.clone(),
    };
    acc.peak = [0.0; 2];
    acc.sum_sq = [0.0; 2];
    acc.n = 0;
    level
}

pub fn stop() {
    if let Some(r) = METER.lock().unwrap_or_else(|e| e.into_inner()).take() {
        r.stop.store(true, Ordering::Relaxed);
    }
}

fn spawn(device: String) -> Running {
    let stop = Arc::new(AtomicBool::new(false));
    let acc = Arc::new(Mutex::new(Acc::default()));
    let last_read = Arc::new(Mutex::new(Instant::now()));
    let ended_at = Arc::new(Mutex::new(None));
    let (s, a, l, d, e_at) = (
        stop.clone(),
        acc.clone(),
        last_read.clone(),
        device.clone(),
        ended_at.clone(),
    );
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        if let Err(e) = capture(&d, &s, &a, &l) {
            a.lock().unwrap_or_else(|e| e.into_inner()).error = Some(e.message().to_string());
        }
        s.store(true, Ordering::Relaxed);
        if let Ok(mut t) = e_at.lock() {
            *t = Some(Instant::now());
        }
        unsafe { CoUninitialize() };
    });
    Running {
        device,
        ended_at,
        stop,
        acc,
        last_read,
    }
}

fn capture(
    device: &str,
    stop: &AtomicBool,
    acc: &Mutex<Acc>,
    last_read: &Mutex<Instant>,
) -> windows::core::Result<()> {
    let client: IAudioClient = unsafe {
        let e: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let d = if device.is_empty() {
            e.GetDefaultAudioEndpoint(eCapture, eConsole)?
        } else {
            e.GetDevice(&HSTRING::from(device))?
        };
        d.Activate(CLSCTX_ALL, None)?
    };
    let fmt = WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: 2,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 48_000 * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let cap: IAudioCaptureClient = unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            2_000_000,
            0,
            &fmt,
            None,
        )?;
        let c = client.GetService()?;
        client.Start()?;
        c
    };
    let result = (|| {
        while !stop.load(Ordering::Relaxed) {
            if last_read
                .lock()
                .map(|t| t.elapsed() > Duration::from_secs(2))
                .unwrap_or(true)
            {
                break; // nobody is looking: stop measuring
            }
            std::thread::sleep(Duration::from_millis(20));
            while unsafe { cap.GetNextPacketSize()? } > 0 {
                let (mut data, mut n, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                unsafe { cap.GetBuffer(&mut data, &mut n, &mut flags, None, None)? };
                let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null();
                if !silent {
                    // SAFETY: n frames of 32-bit float stereo, as requested.
                    let s =
                        unsafe { std::slice::from_raw_parts(data as *const f32, n as usize * 2) };
                    let (mut peak, mut sum) = ([0f32; 2], [0f64; 2]);
                    for f in s.as_chunks::<2>().0 {
                        for c in 0..2 {
                            peak[c] = peak[c].max(f[c].abs());
                            sum[c] += f64::from(f[c]) * f64::from(f[c]);
                        }
                    }
                    if let Ok(mut a) = acc.lock() {
                        for c in 0..2 {
                            a.peak[c] = a.peak[c].max(peak[c]);
                            a.sum_sq[c] += sum[c];
                        }
                        a.n += u64::from(n);
                        a.error = None;
                    }
                } else if let Ok(mut a) = acc.lock() {
                    a.n += u64::from(n);
                }
                unsafe { cap.ReleaseBuffer(n)? };
            }
        }
        Ok(())
    })();
    unsafe {
        let _ = client.Stop();
    }
    result
}
