//! Clip finalisation: concatenates pinned TS segments into one MP4 with stream
//! copy, so there's no re-encode (reference: `/clip-ops`, "Finalise a clip").
//! Every stream is kept: video, plus audio tracks with titles. Track 1 is the
//! default track, which players and Discord use.

use std::fs;
use std::path::{Path, PathBuf};

use ffmpeg::{Dictionary, Rational, Rescale, codec, format, media};
use ffmpeg_next as ffmpeg;

const MICROS: Rational = Rational(1, 1_000_000);

#[derive(Debug, thiserror::Error)]
pub enum ClipError {
    #[error("no segments to save")]
    Empty,
    #[error("segment has no video stream: {0}")]
    NoVideo(PathBuf),
    #[error("FFmpeg: {0}")]
    Ffmpeg(#[from] ffmpeg::Error),
    #[error("file: {0}")]
    Io(#[from] std::io::Error),
}

const fn mktag(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

/// Writes `inputs` (same capture run, in order) into `out` as an MP4 with
/// faststart. It's written to a temporary name and renamed when complete, so the
/// library never sees a half-written clip. `audio_names[i]` titles audio stream `i + 1`.
pub fn concat_to_mp4(
    inputs: &[PathBuf],
    out: &Path,
    audio_names: &[&str],
) -> Result<(), ClipError> {
    let first_path = inputs.first().ok_or(ClipError::Empty)?;
    let tmp = out.with_extension("mp4.partial");
    match write(inputs, first_path, &tmp, audio_names) {
        Ok(()) => {
            fs::rename(&tmp, out)?;
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Timestamp (µs) of the first video packet, which becomes the clip's time 0.
fn first_video_us(path: &Path) -> Result<i64, ClipError> {
    let mut ictx = format::input(path)?;
    let vidx = ictx
        .streams()
        .best(media::Type::Video)
        .map(|s| s.index())
        .ok_or_else(|| ClipError::NoVideo(path.into()))?;
    for (stream, pkt) in ictx.packets() {
        if stream.index() == vidx
            && let Some(t) = pkt.dts().or(pkt.pts())
        {
            return Ok(t.rescale(stream.time_base(), MICROS));
        }
    }
    Err(ClipError::NoVideo(path.into()))
}

fn write(
    inputs: &[PathBuf],
    first_path: &Path,
    tmp: &Path,
    audio_names: &[&str],
) -> Result<(), ClipError> {
    let base_us = first_video_us(first_path)?;
    let first = format::input(first_path)?;
    let mut octx = format::output_as(tmp, "mp4")?;
    let mut audio_seen = 0usize;
    let mut kinds = Vec::new();
    for ist in first.streams() {
        let params = ist.parameters();
        let kind = params.medium();
        let is_hevc = params.id() == codec::Id::HEVC;
        let mut ost = octx.add_stream(ffmpeg::encoder::find(codec::Id::None))?;
        ost.set_parameters(params);
        // hvc1 is the tag browsers/WebView2 and Apple players expect for HEVC.
        let tag = if is_hevc {
            mktag(b'h', b'v', b'c', b'1')
        } else {
            0
        };
        unsafe {
            (*ost.parameters().as_mut_ptr()).codec_tag = tag;
            (*ost.as_mut_ptr()).disposition = 0;
        }
        if kind == media::Type::Audio {
            let mut md = Dictionary::new();
            if let Some(name) = audio_names.get(audio_seen) {
                md.set("title", name);
                md.set("handler_name", name);
            }
            ost.set_metadata(md);
            if audio_seen == 0 {
                unsafe { (*ost.as_mut_ptr()).disposition = ffmpeg::ffi::AV_DISPOSITION_DEFAULT };
            }
            audio_seen += 1;
        } else if kind == media::Type::Video {
            unsafe { (*ost.as_mut_ptr()).disposition = ffmpeg::ffi::AV_DISPOSITION_DEFAULT };
        }
        kinds.push(kind);
    }
    drop(first);
    let mut opts = Dictionary::new();
    opts.set("movflags", "+faststart");
    octx.write_header_with(opts)?;
    let out_tbs: Vec<Rational> = octx.streams().map(|s| s.time_base()).collect();

    for path in inputs {
        let mut ictx = format::input(path)?;
        for (stream, mut pkt) in ictx.packets() {
            let i = stream.index();
            let Some(&out_tb) = out_tbs.get(i) else {
                continue;
            };
            let in_tb = stream.time_base();
            let base = base_us.rescale(MICROS, in_tb);
            let pts = pkt.pts().map(|p| p - base);
            let dts = pkt.dts().map(|d| d - base);
            // Audio from before the first video frame isn't part of the clip.
            if kinds[i] == media::Type::Audio && pts.is_some_and(|p| p < 0) {
                continue;
            }
            pkt.set_pts(pts);
            pkt.set_dts(dts);
            pkt.rescale_ts(in_tb, out_tb);
            pkt.set_stream(i);
            pkt.set_position(-1);
            pkt.write_interleaved(&mut octx)?;
        }
    }
    octx.write_trailer()?;
    Ok(())
}

/// Output path for run `i` of a clip that spans a display change.
pub fn run_output_path(out: &Path, i: usize) -> PathBuf {
    if i == 0 {
        return out.to_path_buf();
    }
    let stem = out
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    out.with_file_name(format!("{stem} (part {}).mp4", i + 1))
}
