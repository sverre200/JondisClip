//! Audio diagnostics (no audio is stored):
//! - `audio_probe formats`: every active capture/render endpoint with its
//!   native mix format and how it's connected (USB, Bluetooth hands-free, ...).
//! - `audio_probe jitter <loopback|name substring> [seconds]`: opens the device
//!   like the recorder does and reports how its WASAPI QPC timestamps drift
//!   and jitter against sample counting, and how often the drift corrector
//!   would intervene.
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
