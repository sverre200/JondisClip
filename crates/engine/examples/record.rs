//! Headless replay-buffer test: records the primary monitor, saves the last
//! N seconds as an MP4, and prints pipeline and process stats.
//!
//! `cargo run --release -p jondisclip-engine --example record -- [record_s] [save_s] [out_dir] [height|-] [mic]`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use jondisclip_engine::audio::AudioConfig;
use jondisclip_engine::recorder::{Recorder, RecorderConfig};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

fn cpu_time() -> Duration {
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
    }
    let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32 | u64::from(f.dwLowDateTime)) * 100;
    Duration::from_nanos(t(k) + t(u))
}

fn working_set_mb() -> f64 {
    let mut pmc = PROCESS_MEMORY_COUNTERS::default();
    unsafe {
        let _ = GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut pmc,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
    }
    pmc.WorkingSetSize as f64 / 1_048_576.0
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let record_s: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let save_s: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let out_dir = args
        .get(3)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("JondisClip"));
    let height: Option<u32> = args.get(4).and_then(|s| s.parse().ok());
    let mic = args.get(5).is_some_and(|s| s == "mic");
    std::fs::create_dir_all(&out_dir)?;

    let cfg = RecorderConfig {
        replay: Duration::from_secs(save_s.max(1) + 4),
        output_height: height,
        audio: AudioConfig {
            mic,
            ..Default::default()
        },
        ..Default::default()
    };
    let t0 = Instant::now();
    let rec = Recorder::start(&cfg)?;
    let i = rec.info();
    println!(
        "recording {}x{}@{} with {} on {} at {:.1} Mb/s (start {:?})",
        i.width,
        i.height,
        i.fps,
        i.encoder.ffmpeg_name(),
        i.adapter,
        i.bitrate as f64 / 1e6,
        t0.elapsed()
    );
    println!(
        "buffer: {} | audio tracks: {:?}",
        i.session_dir.display(),
        i.audio_tracks
    );

    let cpu0 = cpu_time();
    let wall0 = Instant::now();
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    for _ in 0..record_s {
        std::thread::sleep(Duration::from_secs(1));
        if !rec.is_running() {
            break;
        }
    }
    let cpu_pct = (cpu_time() - cpu0).as_secs_f64() / wall0.elapsed().as_secs_f64() / cores * 100.0;
    let s = rec.stats();
    println!(
        "steady state: CPU {cpu_pct:.2}% of machine, working set {:.0} MB | arrived {} encoded {} late {} dropped {} | buffered {:.1}s / {:.1} MB | audio blocks {} dropped {} reopens {}",
        working_set_mb(),
        s.frames_arrived,
        s.frames_encoded,
        s.frames_late,
        s.packets_dropped,
        s.buffered_us as f64 / 1e6,
        s.buffer_bytes as f64 / 1_048_576.0,
        s.audio_blocks,
        s.audio_dropped,
        s.audio_reopens
    );

    // Save half a GOP off the 1 s keyframe grid so the forced-IDR cut is observable.
    std::thread::sleep(Duration::from_millis(500));
    let out = out_dir.join(format!("test-clip-{}.mp4", std::process::id()));
    let t_save = Instant::now();
    let job = rec.save_clip(Duration::from_secs(save_s), out);
    let files = job.join().map_err(|_| "save thread panicked")??;
    println!("saved in {:?}: {:?}", t_save.elapsed(), files);

    let s = rec.stop()?;
    println!(
        "stopped. final: encoded {} late {} dropped {}",
        s.frames_encoded, s.frames_late, s.packets_dropped
    );
    Ok(())
}
