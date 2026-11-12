//! Hardware encoder discovery.
//!
//! We probe by actually opening each encoder (ADR-0005 / `/media-pipeline`):
//! a GPU name is not proof that a codec works (e.g. RDNA2 lists `av1_amf`
//! but cannot open it).

use ffmpeg::{Rational, codec, encoder, format::Pixel};
use ffmpeg_next as ffmpeg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Hevc,
    H264,
    Av1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Amd,
    Nvidia,
    Intel,
    /// Media Foundation hardware MFT: vendor-neutral last resort.
    MediaFoundation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderId {
    pub codec: Codec,
    pub vendor: Vendor,
}

impl EncoderId {
    /// The FFmpeg encoder name.
    pub fn ffmpeg_name(self) -> &'static str {
        use {Codec::*, Vendor::*};
        match (self.codec, self.vendor) {
            (Hevc, Amd) => "hevc_amf",
            (H264, Amd) => "h264_amf",
            (Av1, Amd) => "av1_amf",
            (Hevc, Nvidia) => "hevc_nvenc",
            (H264, Nvidia) => "h264_nvenc",
            (Av1, Nvidia) => "av1_nvenc",
            (Hevc, Intel) => "hevc_qsv",
            (H264, Intel) => "h264_qsv",
            (Av1, Intel) => "av1_qsv",
            (Hevc, MediaFoundation) => "hevc_mf",
            (H264, MediaFoundation) => "h264_mf",
            (Av1, MediaFoundation) => "av1_mf",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EncoderError {
    #[error("FFmpeg initialisation failed: {0}")]
    Init(ffmpeg::Error),
    #[error("encoder {0} is not compiled into this FFmpeg build")]
    NotCompiled(&'static str),
    #[error("encoder {name} failed to open: {source}")]
    Open {
        name: &'static str,
        source: ffmpeg::Error,
    },
}

/// Opens `id` with a small NV12 configuration and closes it again.
pub fn probe(id: EncoderId) -> Result<(), EncoderError> {
    ffmpeg::init().map_err(EncoderError::Init)?;
    let name = id.ffmpeg_name();
    let codec = encoder::find_by_name(name).ok_or(EncoderError::NotCompiled(name))?;
    let mut enc = codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|source| EncoderError::Open { name, source })?;
    enc.set_width(1280);
    enc.set_height(720);
    enc.set_format(Pixel::NV12);
    enc.set_time_base(Rational::new(1, 60));
    enc.set_frame_rate(Some(Rational::new(60, 1)));
    enc.set_bit_rate(8_000_000);
    enc.open_as(codec)
        .map(drop)
        .map_err(|source| EncoderError::Open { name, source })
}

/// All candidates in preference order for `codec`.
pub fn candidates(codec: Codec) -> [EncoderId; 4] {
    [
        Vendor::Amd,
        Vendor::Nvidia,
        Vendor::Intel,
        Vendor::MediaFoundation,
    ]
    .map(|vendor| EncoderId { codec, vendor })
}

/// The encoder vendor for a PCI vendor id, if it has a native FFmpeg encoder.
pub fn vendor_for_pci(vendor_id: u32) -> Option<Vendor> {
    match vendor_id {
        0x1002 => Some(Vendor::Amd),
        0x10DE => Some(Vendor::Nvidia),
        0x8086 => Some(Vendor::Intel),
        _ => None,
    }
}

/// Candidates for the capture adapter: its own vendor first (same device, so
/// it's zero-copy), then Media Foundation. Other vendors' encoders can't take
/// textures from this device, so they're skipped.
pub fn candidates_for_adapter(codec: Codec, vendor_id: u32) -> Vec<EncoderId> {
    let mut v: Vec<EncoderId> = vendor_for_pci(vendor_id)
        .map(|vendor| EncoderId { codec, vendor })
        .into_iter()
        .collect();
    v.push(EncoderId {
        codec,
        vendor: Vendor::MediaFoundation,
    });
    v
}

/// Largest frame hardware H.264 encoders take: 4096 pixels a side, and 36 864
/// macroblocks (level 5.2, e.g. 4096×2304).
pub fn fits_h264(width: u32, height: u32) -> bool {
    width <= 4096 && height <= 4096 && width.div_ceil(16) * height.div_ceil(16) <= 36_864
}

/// Codecs the recorder tries for a preference at an output size, in order.
/// H.264 gets HEVC as a fallback only when the frame is too large for H.264
/// (a 5120×1440 or 5K screen at its own size). A failure at a size H.264 can
/// do is reported instead of quietly switching codec.
pub fn codecs_for(preferred: Codec, width: u32, height: u32) -> Vec<Codec> {
    let mut v = codec_order(preferred).to_vec();
    if preferred == Codec::H264 && !fits_h264(width, height) {
        v.push(Codec::Hevc);
    }
    v
}

fn codec_order(preferred: Codec) -> &'static [Codec] {
    match preferred {
        Codec::H264 => &[Codec::H264],
        Codec::Hevc => &[Codec::Hevc, Codec::H264],
        Codec::Av1 => &[Codec::Av1, Codec::Hevc, Codec::H264],
    }
}

/// Like [`select`], restricted to encoders usable with the capture adapter.
pub fn select_for_adapter(
    preferred: Codec,
    vendor_id: u32,
) -> Result<EncoderId, Vec<EncoderError>> {
    let mut errors = Vec::new();
    for &c in codec_order(preferred) {
        for id in candidates_for_adapter(c, vendor_id) {
            match probe(id) {
                Ok(()) => return Ok(id),
                Err(e) => errors.push(e),
            }
        }
    }
    Err(errors)
}

/// Picks the first encoder that opens for the preferred codec, falling back
/// from HEVC to H.264 (ADR-0005). Returns the chosen encoder or every error.
pub fn select(preferred: Codec) -> Result<EncoderId, Vec<EncoderError>> {
    let mut errors = Vec::new();
    for &c in codec_order(preferred) {
        for id in candidates(c) {
            match probe(id) {
                Ok(()) => return Ok(id),
                Err(e) => errors.push(e),
            }
        }
    }
    Err(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hevc_is_only_a_fallback_for_frames_too_large_for_h264() {
        for (w, h) in [
            (1920, 1080),
            (2560, 1440),
            (3440, 1440),
            (3840, 2160),
            (4096, 2304),
        ] {
            assert!(fits_h264(w, h), "{w}x{h}");
            assert_eq!(codecs_for(Codec::H264, w, h), [Codec::H264]);
        }
        for (w, h) in [(5120, 1440), (5120, 2880), (7680, 4320), (4096, 4096)] {
            assert!(!fits_h264(w, h), "{w}x{h}");
            assert_eq!(codecs_for(Codec::H264, w, h), [Codec::H264, Codec::Hevc]);
        }
        // An explicit HEVC or AV1 choice keeps its own order at any size.
        assert_eq!(
            codecs_for(Codec::Hevc, 5120, 1440),
            [Codec::Hevc, Codec::H264]
        );
    }
}
