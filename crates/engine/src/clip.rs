//! Clip finalisation: concatenates pinned TS segments into one MP4 with stream
//! copy, so there's no re-encode (reference: `/clip-ops`, "Finalise a clip").

use std::fs;
use std::path::{Path, PathBuf};

use ffmpeg::{Dictionary, Rational, codec, format, media};
use ffmpeg_next as ffmpeg;

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
/// library never sees a half-written clip.
pub fn concat_to_mp4(inputs: &[PathBuf], out: &Path) -> Result<(), ClipError> {
    let first_path = inputs.first().ok_or(ClipError::Empty)?;
    let tmp = out.with_extension("mp4.partial");
    let result = write(inputs, first_path, &tmp);
    match result {
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

fn write(inputs: &[PathBuf], first_path: &Path, tmp: &Path) -> Result<(), ClipError> {
    let first = format::input(first_path)?;
    let ist = first
        .streams()
        .best(media::Type::Video)
        .ok_or_else(|| ClipError::NoVideo(first_path.into()))?;
    let params = ist.parameters();
    let is_hevc = params.id() == codec::Id::HEVC;

    let mut octx = format::output_as(tmp, "mp4")?;
    {
        let mut ost = octx.add_stream(ffmpeg::encoder::find(codec::Id::None))?;
        ost.set_parameters(params);
        // hvc1 is the tag browsers/WebView2 and Apple players expect for HEVC.
        let tag = if is_hevc {
            mktag(b'h', b'v', b'c', b'1')
        } else {
            0
        };
        unsafe { (*ost.parameters().as_mut_ptr()).codec_tag = tag };
    }
    drop(first);
    let mut opts = Dictionary::new();
    opts.set("movflags", "+faststart");
    octx.write_header_with(opts)?;
    let out_tb = octx
        .stream(0)
        .map(|s| s.time_base())
        .unwrap_or(Rational::new(1, 90_000));

    let mut offset: Option<i64> = None;
    for path in inputs {
        let mut ictx = format::input(path)?;
        let Some(vidx) = ictx.streams().best(media::Type::Video).map(|s| s.index()) else {
            return Err(ClipError::NoVideo(path.clone()));
        };
        for (stream, mut pkt) in ictx.packets() {
            if stream.index() != vidx {
                continue;
            }
            let in_tb = stream.time_base();
            let base = *offset.get_or_insert_with(|| pkt.dts().or(pkt.pts()).unwrap_or(0));
            pkt.set_pts(pkt.pts().map(|p| p - base));
            pkt.set_dts(pkt.dts().map(|d| d - base));
            pkt.rescale_ts(in_tb, out_tb);
            pkt.set_stream(0);
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
