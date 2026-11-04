//! Exports real MP4s built like our recordings (H.264, 1 s closed GOP, no
//! B-frames, 90 kHz, 60 fps, three titled AAC tracks) and checks the results
//! by decoding them. Needs the `ffmpeg` CLI on PATH to make the inputs (dev
//! machines have it); without it the tests skip with a message.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use ffmpeg::{codec, format, frame, media};
use ffmpeg_next as ffmpeg;
use jondisclip_export::{ExportError, Mode, Trim, trim};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dir(tag: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("jc-export-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    Dir(p)
}

/// 12 s source: black video with a white flash at exactly 5.000 s (6 frames)
/// and a 1 kHz beep starting at exactly 5.000 s on every audio track.
fn source() -> Option<&'static PathBuf> {
    static SRC: OnceLock<Option<PathBuf>> = OnceLock::new();
    SRC.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("jc-export-src-{}", std::process::id()));
        std::fs::create_dir_all(&d).ok()?;
        let out = d.join("Game 2026-11-04 12-00-00.mp4");
        let video =
            "color=c=black:s=640x360:r=60:d=12,drawbox=c=white:t=fill:enable='between(t,5,5.099)'";
        let audio = "sine=f=1000:sample_rate=48000:d=12,volume=enable='lt(t,5)':volume=0";
        let ok = Command::new("ffmpeg")
            .args([
                "-v", "error", "-y", "-f", "lavfi", "-i", video, "-f", "lavfi", "-i", audio,
            ])
            .args(["-map", "0:v", "-map", "1:a", "-map", "1:a", "-map", "1:a"])
            .args([
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-g",
                "60",
                "-keyint_min",
                "60",
                "-sc_threshold",
                "0",
                "-bf",
                "0",
            ])
            .args(["-pix_fmt", "yuv420p", "-video_track_timescale", "90000"])
            .args(["-c:a", "aac", "-b:a", "128k"])
            .args([
                "-metadata:s:a:0",
                "handler_name=Mix",
                "-metadata:s:a:1",
                "handler_name=Game",
                "-metadata:s:a:2",
                "handler_name=Microphone",
            ])
            .args(["-movflags", "+faststart"])
            .arg(&out)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("ffmpeg CLI not available: skipping export tests");
            return None;
        }
        Some(out)
    })
    .as_ref()
}

struct Probe {
    streams: usize,
    titles: Vec<String>,
    video_frames: usize,
    video_first: f64,
    video_last: f64,
    audio_first: f64,
    /// Output time of the first bright frame and of the beep onset.
    flash: Option<f64>,
    beep: Option<f64>,
}

/// Decodes the whole file: frame count, first/last timestamps, and where
/// the flash and the beep land.
fn probe(path: &Path) -> Probe {
    ffmpeg::init().unwrap();
    let mut ictx = format::input(path).unwrap();
    let vidx = ictx.streams().best(media::Type::Video).unwrap().index();
    let aidx = ictx.streams().best(media::Type::Audio).unwrap().index();
    let titles = ictx
        .streams()
        .filter(|s| s.parameters().medium() == media::Type::Audio)
        .map(|s| s.metadata().get("handler_name").unwrap_or("").to_string())
        .collect();
    let streams = ictx.streams().count();
    let vtb = ictx.stream(vidx).unwrap().time_base();
    let atb = ictx.stream(aidx).unwrap().time_base();
    let mut vdec =
        codec::context::Context::from_parameters(ictx.stream(vidx).unwrap().parameters())
            .unwrap()
            .decoder()
            .video()
            .unwrap();
    let mut adec =
        codec::context::Context::from_parameters(ictx.stream(aidx).unwrap().parameters())
            .unwrap()
            .decoder()
            .audio()
            .unwrap();
    let mut p = Probe {
        streams,
        titles,
        video_frames: 0,
        video_first: f64::MAX,
        video_last: f64::MIN,
        audio_first: f64::MAX,
        flash: None,
        beep: None,
    };
    let mut vf = frame::Video::empty();
    let mut af = frame::Audio::empty();
    let on_video = |vf: &frame::Video, p: &mut Probe| {
        let t = vf.pts().unwrap_or(0) as f64 * f64::from(vtb);
        p.video_frames += 1;
        p.video_first = p.video_first.min(t);
        p.video_last = p.video_last.max(t);
        let y = vf.data(0);
        let mean = y.iter().step_by(97).map(|v| u32::from(*v)).sum::<u32>() as f64
            / (y.len() / 97).max(1) as f64;
        if mean > 128.0 && p.flash.is_none_or(|f| t < f) {
            p.flash = Some(t);
        }
    };
    let on_audio = |af: &frame::Audio, p: &mut Probe| {
        let t0 = af.pts().unwrap_or(0) as f64 * f64::from(atb);
        p.audio_first = p.audio_first.min(t0);
        // AAC decodes to planar float.
        let s: &[f32] = af.plane(0);
        if p.beep.is_none()
            && let Some(i) = s.iter().position(|v| v.abs() > 0.1)
        {
            p.beep = Some(t0 + i as f64 / f64::from(af.rate()));
        }
    };
    for (stream, pkt) in ictx.packets() {
        if stream.index() == vidx {
            vdec.send_packet(&pkt).unwrap();
            while vdec.receive_frame(&mut vf).is_ok() {
                on_video(&vf, &mut p);
            }
        } else if stream.index() == aidx {
            adec.send_packet(&pkt).unwrap();
            while adec.receive_frame(&mut af).is_ok() {
                on_audio(&af, &mut p);
            }
        }
    }
    vdec.send_eof().unwrap();
    while vdec.receive_frame(&mut vf).is_ok() {
        on_video(&vf, &mut p);
    }
    p
}

fn job(src: &Path, out: &Path, start: f64, end: f64, mode: Mode) -> Trim {
    Trim {
        input: src.into(),
        output: out.into(),
        start,
        end,
        mode,
    }
}

fn sha(path: &Path) -> (u64, std::time::SystemTime, Vec<u8>) {
    let m = std::fs::metadata(path).unwrap();
    (m.len(), m.modified().unwrap(), std::fs::read(path).unwrap())
}

const FRAME: f64 = 1.0 / 60.0;

#[test]
fn lossless_cut_on_a_keyframe_is_exact_and_keeps_every_track() {
    let Some(src) = source() else { return };
    let d = dir("copy-kf");
    let before = sha(src);
    let out = d.0.join("cut.mp4");
    let o = trim(&job(src, &out, 4.0, 7.0, Mode::Copy), |_| {}).unwrap();
    assert_eq!(o.video, "copy");
    assert!((o.start - 4.0).abs() < 1e-6, "start {}", o.start);
    let p = probe(&out);
    assert_eq!(p.streams, 4);
    assert_eq!(p.titles, ["Mix", "Game", "Microphone"]);
    assert_eq!(p.video_frames, 180, "3 s at 60 fps");
    assert!(p.video_first.abs() < 1e-6);
    // The flash is at 5.000 s in the source → 1.000 s in the cut; the beep too.
    let (flash, beep) = (p.flash.unwrap(), p.beep.unwrap());
    assert!((flash - 1.0).abs() < FRAME / 2.0, "flash at {flash}");
    assert!(
        (beep - flash).abs() < 0.025,
        "A/V offset {:.1} ms",
        (beep - flash) * 1000.0
    );
    assert_eq!(sha(src), before, "the original is untouched");
    assert!(!out.with_extension("mp4.partial").exists());
}

#[test]
fn lossless_cut_between_keyframes_starts_at_the_previous_keyframe() {
    let Some(src) = source() else { return };
    let d = dir("copy-snap");
    let out = d.0.join("cut.mp4");
    let o = trim(&job(src, &out, 4.6, 6.5, Mode::Copy), |_| {}).unwrap();
    assert!(
        (o.start - 4.0).abs() < 1e-6,
        "snapped to 4.0, got {}",
        o.start
    );
    let p = probe(&out);
    // 4.0 .. 6.5 exactly: the end is frame-exact (no B-frames).
    assert_eq!(p.video_frames, 150);
    assert!((p.flash.unwrap() - 1.0).abs() < FRAME / 2.0);
}

#[test]
fn exact_cut_starts_on_the_chosen_frame_with_audio_in_sync() {
    let Some(src) = source() else { return };
    let d = dir("exact");
    let before = sha(src);
    let out = d.0.join("cut.mp4");
    let mut last = 0.0;
    let o = trim(&job(src, &out, 4.3, 6.8, Mode::Exact), |p| {
        assert!(p >= last, "progress only moves forward");
        last = p;
    })
    .unwrap();
    assert_ne!(o.video, "copy");
    assert_eq!(last, 1.0);
    let p = probe(&out);
    assert_eq!(p.streams, 4);
    assert_eq!(p.titles, ["Mix", "Game", "Microphone"]);
    assert_eq!(p.video_frames, 150, "2.5 s at 60 fps");
    assert!(
        p.video_first.abs() < 1e-6,
        "video starts at 0, got {}",
        p.video_first
    );
    assert!(
        p.audio_first >= 0.0 && p.audio_first < 0.025,
        "audio starts at {}",
        p.audio_first
    );
    // Flash at 5.0 s → 0.7 s after a 4.3 s start.
    let (flash, beep) = (p.flash.unwrap(), p.beep.unwrap());
    assert!((flash - 0.7).abs() < FRAME / 2.0 + 1e-3, "flash at {flash}");
    assert!(
        (beep - flash).abs() < 0.025,
        "A/V offset {:.1} ms",
        (beep - flash) * 1000.0
    );
    assert!(
        (o.duration - 2.5).abs() < FRAME,
        "reported duration {}",
        o.duration
    );
    assert_eq!(sha(src), before);
}

#[test]
fn exact_cut_to_the_very_end_flushes_every_frame() {
    let Some(src) = source() else { return };
    let d = dir("exact-end");
    let out = d.0.join("cut.mp4");
    trim(&job(src, &out, 10.5, 12.0, Mode::Exact), |_| {}).unwrap();
    let p = probe(&out);
    assert_eq!(
        p.video_frames, 90,
        "1.5 s at 60 fps, including the decoder's delayed frames"
    );
}

#[test]
fn exact_cut_found_only_at_the_decoder_flush_keeps_its_audio() {
    let Some(src) = source() else { return };
    let d = dir("exact-tail");
    let out = d.0.join("cut.mp4");
    // The last few frames: with frame threading they only leave the decoder
    // at the flush, after all packets (and their audio) were read.
    trim(&job(src, &out, 11.93, 12.0, Mode::Exact), |_| {}).unwrap();
    let p = probe(&out);
    assert!(p.video_frames >= 3, "frames {}", p.video_frames);
    assert!(
        p.audio_first < 0.03,
        "audio is there and starts with the video: {}",
        p.audio_first
    );
}

#[test]
fn failures_leave_nothing_behind_and_never_overwrite() {
    let Some(src) = source() else { return };
    let d = dir("fail");
    let out = d.0.join("cut.mp4");
    assert!(matches!(
        trim(&job(src, &out, 5.0, 5.0, Mode::Copy), |_| {}),
        Err(ExportError::EmptyRange)
    ));
    assert!(matches!(
        trim(&job(src, &out, 6.0, 2.0, Mode::Exact), |_| {}),
        Err(ExportError::EmptyRange)
    ));
    assert!(matches!(
        trim(&job(src, &out, f64::NAN, 2.0, Mode::Copy), |_| {}),
        Err(ExportError::EmptyRange)
    ));
    // Past the end of the clip: nothing to write.
    assert!(trim(&job(src, &out, 50.0, 60.0, Mode::Copy), |_| {}).is_err());
    assert!(
        trim(
            &job(&d.0.join("missing.mp4"), &out, 0.0, 1.0, Mode::Copy),
            |_| {}
        )
        .is_err()
    );
    let garbage = d.0.join("garbage.mp4");
    std::fs::write(&garbage, b"not a video").unwrap();
    assert!(trim(&job(&garbage, &out, 0.0, 1.0, Mode::Exact), |_| {}).is_err());
    assert!(!out.exists() && !out.with_extension("mp4.partial").exists());

    std::fs::write(&out, b"earlier export").unwrap();
    assert!(matches!(
        trim(&job(src, &out, 1.0, 2.0, Mode::Copy), |_| {}),
        Err(ExportError::Exists(_))
    ));
    assert_eq!(std::fs::read(&out).unwrap(), b"earlier export");
}

#[test]
fn killing_the_process_cancels_without_a_finished_file() {
    let Some(src) = source() else { return };
    let d = dir("cancel");
    let out = d.0.join("cut.mp4");
    let mut child = Command::new(env!("CARGO_BIN_EXE_jondisclip-export"))
        .args(["trim"])
        .arg(src)
        .arg(&out)
        .args(["0.2", "12", "exact"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));
    child.kill().unwrap();
    let _ = child.wait();
    assert!(
        !out.exists(),
        "a cancelled export never appears under its final name"
    );
}
