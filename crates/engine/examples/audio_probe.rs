//! Audio diagnostics (no audio is stored):
//! - `audio_probe formats`: every active capture/render endpoint with its
//!   native mix format and how it's connected (USB, Bluetooth hands-free, ...).
//! - `audio_probe jitter <loopback|name substring> [seconds]`: opens the device
//!   like the recorder does and reports how its WASAPI QPC timestamps drift
//!   and jitter against sample counting, and how often the drift corrector
//!   would intervene.
//! - `audio_probe device <name substring> [seconds]`: one microphone's real
//!   configuration (device format, Windows input level, enhancements) plus
//!   per-channel statistics of a short capture: level, DC, channel identity,
//!   low-frequency energy, effective bit depth. Statistics only, no audio kept.
use std::time::{Duration, Instant};

use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_EnumeratorName, PKEY_Device_FriendlyName,
};
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE, EDataFlow, IAudioCaptureClient,
    IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE, eCapture, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

fn prop(d: &IMMDevice, key: &windows::Win32::Foundation::PROPERTYKEY) -> String {
    unsafe {
        d.OpenPropertyStore(STGM_READ)
            .and_then(|s| s.GetValue(key))
            .map(|v| v.to_string())
            .unwrap_or_default()
    }
}

fn mix_format(d: &IMMDevice) -> String {
    unsafe {
        let Ok(c) = d.Activate::<IAudioClient>(CLSCTX_ALL, None) else {
            return "?".into();
        };
        let Ok(p) = c.GetMixFormat() else {
            return "?".into();
        };
        let f = std::ptr::read_unaligned(p);
        let (rate, ch, bits, tag) = (
            f.nSamplesPerSec,
            f.nChannels,
            f.wBitsPerSample,
            f.wFormatTag,
        );
        let mut s = format!("{rate} Hz, {ch} ch, {bits} bit");
        if tag == 0xFFFE {
            let x = std::ptr::read_unaligned(p as *const WAVEFORMATEXTENSIBLE);
            let mask = x.dwChannelMask;
            s.push_str(&format!(", mask {mask:#x}"));
        }
        CoTaskMemFree(Some(p as *const _));
        s
    }
}

fn list(e: &IMMDeviceEnumerator, flow: EDataFlow, label: &str) -> windows::core::Result<()> {
    unsafe {
        let def = e
            .GetDefaultAudioEndpoint(flow, eConsole)
            .ok()
            .map(|d| prop(&d, &PKEY_Device_FriendlyName));
        let coll = e.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        println!("{label}:");
        for i in 0..coll.GetCount()? {
            let d = coll.Item(i)?;
            let name = prop(&d, &PKEY_Device_FriendlyName);
            let star = if def.as_deref() == Some(name.as_str()) {
                "*"
            } else {
                " "
            };
            println!(
                "  {star} {name}\n      bus: {:<10} format: {}",
                prop(&d, &PKEY_Device_EnumeratorName),
                mix_format(&d)
            );
        }
    }
    Ok(())
}

fn find(e: &IMMDeviceEnumerator, needle: &str) -> windows::core::Result<Option<IMMDevice>> {
    unsafe {
        let coll = e.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        for i in 0..coll.GetCount()? {
            let d = coll.Item(i)?;
            if prop(&d, &PKEY_Device_FriendlyName)
                .to_lowercase()
                .contains(&needle.to_lowercase())
            {
                return Ok(Some(d));
            }
        }
    }
    Ok(None)
}

fn jitter(e: &IMMDeviceEnumerator, which: &str, secs: u64) -> windows::core::Result<()> {
    let (device, loopback) = if which == "loopback" {
        (
            unsafe { e.GetDefaultAudioEndpoint(eRender, eConsole)? },
            true,
        )
    } else {
        match find(e, which)? {
            Some(d) => (d, false),
            None => {
                println!("no capture device matching \"{which}\"");
                return Ok(());
            }
        }
    };
    println!(
        "device: {} ({})",
        prop(&device, &PKEY_Device_FriendlyName),
        if loopback { "loopback" } else { "capture" }
    );
    let mut freq = 0;
    unsafe { QueryPerformanceFrequency(&mut freq)? };
    let fmt = WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: 2,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 48_000 * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let mut flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    if loopback {
        flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    }
    unsafe {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &fmt, None)?;
        let cap: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        let t_end = Instant::now() + Duration::from_secs(secs);
        let (mut first_hns, mut frames_total, mut packets) = (None::<u64>, 0i64, 0u64);
        let mut errs: Vec<f64> = Vec::new(); // stamp - counted, in ms
        while Instant::now() < t_end {
            std::thread::sleep(Duration::from_millis(5));
            while cap.GetNextPacketSize()? > 0 {
                let (mut data, mut n, mut fl, mut qpc) = (std::ptr::null_mut(), 0u32, 0u32, 0u64);
                cap.GetBuffer(&mut data, &mut n, &mut fl, None, Some(&mut qpc))?;
                cap.ReleaseBuffer(n)?;
                let base = *first_hns.get_or_insert(qpc);
                let stamp_ms = (qpc - base) as f64 / 10_000.0;
                let counted_ms = frames_total as f64 / 48.0;
                errs.push(stamp_ms - counted_ms);
                frames_total += i64::from(n);
                packets += 1;
            }
        }
        client.Stop()?;
        if errs.len() < 10 {
            println!(
                "only {} packets (nothing playing? loopback is silent-idle)",
                errs.len()
            );
            return Ok(());
        }
        // Linear fit = clock drift; residual = jitter.
        let n = errs.len() as f64;
        let xs: Vec<f64> = (0..errs.len()).map(|i| i as f64).collect();
        let (mx, my) = (xs.iter().sum::<f64>() / n, errs.iter().sum::<f64>() / n);
        let slope = xs
            .iter()
            .zip(&errs)
            .map(|(x, y)| (x - mx) * (y - my))
            .sum::<f64>()
            / xs.iter().map(|x| (x - mx).powi(2)).sum::<f64>();
        let resid: Vec<f64> = xs
            .iter()
            .zip(&errs)
            .map(|(x, y)| y - (my + slope * (x - mx)))
            .collect();
        let rms = (resid.iter().map(|r| r * r).sum::<f64>() / n).sqrt();
        let maxr = resid.iter().fold(0f64, |m, r| m.max(r.abs()));
        let dur_s = frames_total as f64 / 48_000.0;
        let drift_ms = errs.last().unwrap_or(&0.0) - errs.first().unwrap_or(&0.0);
        let ppm = drift_ms / 1000.0 / dur_s * 1e6;
        // How often the current corrector (1 ms deadband, ±1 frame/packet) acts.
        let (mut expected, mut nudges, mut resyncs, mut count) = (None::<i64>, 0u32, 0u32, 0i64);
        let pkt_frames = frames_total / packets as i64;
        for e in &errs {
            let stamped = count + (e * 48.0) as i64;
            let at = match expected {
                None => stamped,
                Some(x) if (stamped - x).abs() > 960 => {
                    resyncs += 1;
                    stamped
                }
                Some(x) if stamped - x > 48 => {
                    nudges += 1;
                    x + 1
                }
                Some(x) if stamped - x < -48 => {
                    nudges += 1;
                    x - 1
                }
                Some(x) => x,
            };
            expected = Some(at + pkt_frames);
            count += pkt_frames;
        }
        println!(
            "{:.1} s, {} packets of ~{} frames | clock drift {:+.1} ms ({:+.0} ppm) | timestamp jitter rms {:.3} ms, max {:.3} ms\n\
             current corrector would act: {} one-sample nudges ({:.2}/s), {} hard resyncs",
            dur_s,
            packets,
            pkt_frames,
            drift_ms,
            ppm,
            rms,
            maxr,
            nudges,
            nudges as f64 / dur_s,
            resyncs
        );
    }
    Ok(())
}

fn device_report(e: &IMMDeviceEnumerator, needle: &str, secs: u64) -> windows::core::Result<()> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        PKEY_AudioEndpoint_Disable_SysFx, PKEY_AudioEngine_DeviceFormat,
    };
    let Some(d) = find(e, needle)? else {
        println!("no capture device matching \"{needle}\"");
        return Ok(());
    };
    println!("device: {}", prop(&d, &PKEY_Device_FriendlyName));
    unsafe {
        if let Ok(id) = d.GetId() {
            println!("  endpoint id: {}", id.to_string().unwrap_or_default());
            CoTaskMemFree(Some(id.0 as *const _));
        }
        let store = d.OpenPropertyStore(STGM_READ)?;
        // Device (hardware-side) format: a WAVEFORMATEX in a VT_BLOB.
        if let Ok(v) = store.GetValue(&PKEY_AudioEngine_DeviceFormat) {
            let blob = v.Anonymous.Anonymous.Anonymous.blob;
            if blob.cbSize as usize >= std::mem::size_of::<WAVEFORMATEX>()
                && !blob.pBlobData.is_null()
            {
                let f = std::ptr::read_unaligned(blob.pBlobData as *const WAVEFORMATEX);
                let (rate, ch, bits, tag) = (
                    f.nSamplesPerSec,
                    f.nChannels,
                    f.wBitsPerSample,
                    f.wFormatTag,
                );
                let mut extra = String::new();
                if tag == 0xFFFE
                    && blob.cbSize as usize >= std::mem::size_of::<WAVEFORMATEXTENSIBLE>()
                {
                    let x = std::ptr::read_unaligned(blob.pBlobData as *const WAVEFORMATEXTENSIBLE);
                    let (valid, mask) = (x.Samples.wValidBitsPerSample, x.dwChannelMask);
                    extra = format!(", valid bits {valid}, channel mask {mask:#x}");
                }
                println!(
                    "  device format (Windows 'Default Format'): {rate} Hz, {ch} ch, {bits}-bit container{extra}"
                );
            }
        }
        let sysfx = store
            .GetValue(&PKEY_AudioEndpoint_Disable_SysFx)
            .map(|v| v.to_string())
            .unwrap_or_default();
        println!(
            "  audio enhancements disabled flag: {}",
            if sysfx.is_empty() {
                "(not set)".into()
            } else {
                sysfx
            }
        );
        println!("  shared mix format: {}", mix_format(&d));
        if let Ok(vol) = d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
            let (mut mn, mut mx, mut step) = (0f32, 0f32, 0f32);
            let _ = vol.GetVolumeRange(&mut mn, &mut mx, &mut step);
            let db = vol.GetMasterVolumeLevel().unwrap_or(f32::NAN);
            let sc = vol.GetMasterVolumeLevelScalar().unwrap_or(f32::NAN);
            let mute = vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            let hw = vol.QueryHardwareSupport().unwrap_or(0);
            println!(
                "  Windows input level: {:.0}% = {db:+.1} dB (range {mn:+.1}..{mx:+.1} dB){}{}",
                sc * 100.0,
                if mute { ", MUTED" } else { "" },
                if hw & 1 != 0 {
                    ", applied in the device (hardware)"
                } else {
                    ", applied in software"
                }
            );
        }
    }
    // Capture statistics.
    let fmt = WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: 2,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 48_000 * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let mut lr: Vec<[f32; 2]> = Vec::with_capacity(48_000 * secs as usize + 4800);
    unsafe {
        let client: IAudioClient = d.Activate(CLSCTX_ALL, None)?;
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            2_000_000,
            0,
            &fmt,
            None,
        )?;
        let cap: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        let t_end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < t_end {
            std::thread::sleep(Duration::from_millis(10));
            while cap.GetNextPacketSize()? > 0 {
                let (mut data, mut n, mut fl) = (std::ptr::null_mut(), 0u32, 0u32);
                cap.GetBuffer(&mut data, &mut n, &mut fl, None, None)?;
                if !data.is_null() {
                    let s = std::slice::from_raw_parts(data as *const f32, n as usize * 2);
                    lr.extend(s.as_chunks::<2>().0.iter().map(|f| [f[0], f[1]]));
                }
                cap.ReleaseBuffer(n)?;
            }
        }
        client.Stop()?;
    }
    let n = lr.len().max(1) as f64;
    let db = |v: f64| if v > 1e-12 { 20.0 * v.log10() } else { -240.0 };
    println!("  captured {:.1} s", lr.len() as f64 / 48_000.0);
    for c in 0..2 {
        let x: Vec<f64> = lr.iter().map(|f| f64::from(f[c])).collect();
        let mean = x.iter().sum::<f64>() / n;
        let rms = (x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
        let peak = x.iter().fold(0f64, |m, v| m.max(v.abs()));
        // One-pole low-pass at 80 Hz to estimate rumble energy share.
        let a = 1.0 - (-2.0 * std::f64::consts::PI * 80.0 / 48_000.0).exp();
        let (mut y, mut lf) = (0f64, 0f64);
        for v in &x {
            y += a * ((v - mean) - y);
            lf += y * y;
        }
        let lf_rms = (lf / n).sqrt();
        // Effective resolution: are all samples multiples of 2^-15 (16-bit) or 2^-23 (24-bit)?
        let on = |bits: i32| {
            x.iter()
                .filter(|v| {
                    let s = **v * f64::from(1u32 << bits);
                    (s - s.round()).abs() < 1e-6
                })
                .count() as f64
                / n
        };
        println!(
            "  ch{} {}: rms {:.1} dBFS, peak {:.1} dBFS, DC {:+.6}, energy below 80 Hz {:.1} dB rel., 16-bit grid {:.0}%, 24-bit grid {:.0}%",
            c,
            if c == 0 { "L" } else { "R" },
            db(rms),
            db(peak),
            mean,
            db(lf_rms) - db(rms),
            on(15) * 100.0,
            on(23) * 100.0
        );
    }
    let identical = lr.iter().filter(|f| f[0] == f[1]).count() as f64 / n;
    let (sl, sr, slr) = lr.iter().fold((0f64, 0f64, 0f64), |(a, b, c), f| {
        (
            a + f64::from(f[0]).powi(2),
            b + f64::from(f[1]).powi(2),
            c + f64::from(f[0]) * f64::from(f[1]),
        )
    });
    println!(
        "  L==R identical samples {:.1}%, L/R correlation {:+.3}",
        identical * 100.0,
        slr / (sl * sr).sqrt().max(1e-30)
    );
    Ok(())
}

/// Runs the settings window's mic check analysis on a real capture.
fn mic_check(e: &IMMDeviceEnumerator, needle: &str, secs: u64) -> windows::core::Result<()> {
    let Some(d) = find(e, needle)? else {
        println!("no capture device matching \"{needle}\"");
        return Ok(());
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
    let mut raw: Vec<f32> = Vec::with_capacity(96_000 * secs as usize + 9600);
    unsafe {
        let client: IAudioClient = d.Activate(CLSCTX_ALL, None)?;
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            2_000_000,
            0,
            &fmt,
            None,
        )?;
        let cap: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        let t_end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < t_end {
            std::thread::sleep(Duration::from_millis(10));
            while cap.GetNextPacketSize()? > 0 {
                let (mut data, mut n, mut fl) = (std::ptr::null_mut(), 0u32, 0u32);
                cap.GetBuffer(&mut data, &mut n, &mut fl, None, None)?;
                if !data.is_null() {
                    raw.extend_from_slice(std::slice::from_raw_parts(
                        data as *const f32,
                        n as usize * 2,
                    ));
                }
                cap.ReleaseBuffer(n)?;
            }
        }
        client.Stop()?;
    }
    let gain: f32 = std::env::var("JC_GAIN")
        .ok()
        .and_then(|g| g.parse().ok())
        .unwrap_or(0.0);
    let (r, processed) = jondisclip_core::mic_check::analyze(&raw, gain, false);
    println!(
        "{} s captured, {} processed frames, gain {gain:+} dB",
        r.seconds,
        processed.len()
    );
    println!("{r:#?}");
    Ok(())
}

fn main() -> windows::core::Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let mut _t = 0;
        let _ = QueryPerformanceCounter(&mut _t);
    }
    let e: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("miccheck") => mic_check(
            &e,
            args.get(1).map_or("Samson", String::as_str),
            args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8),
        ),
        Some("device") => device_report(
            &e,
            args.get(1).map_or("Samson", String::as_str),
            args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5),
        ),
        Some("jitter") => jitter(
            &e,
            args.get(1).map_or("loopback", String::as_str),
            args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20),
        ),
        _ => {
            list(&e, eCapture, "Capture (microphones)")?;
            list(&e, eRender, "Render (outputs, * = default)")
        }
    }
}
