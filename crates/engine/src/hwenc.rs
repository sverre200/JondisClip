//! Hardware video encoder fed with D3D11 NV12 surfaces through FFmpeg's
//! D3D11VA hwframes context. Frames never leave VRAM.

use std::ptr;

use ffmpeg::ffi;
use ffmpeg::packet::Mut as _;
use ffmpeg::{Dictionary, Packet, Rational, codec, color, encoder, format::Pixel};
use ffmpeg_next as ffmpeg;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, ID3D11Texture2D,
};
use windows::core::Interface;

use crate::encoder::{Codec, EncoderId, Vendor};
use crate::gpu::Gpu;

#[derive(Debug, thiserror::Error)]
pub enum HwEncError {
    #[error("{what} failed: {source}")]
    Ffmpeg {
        what: &'static str,
        source: ffmpeg::Error,
    },
    #[error("{0} returned null")]
    Null(&'static str),
    #[error("encoder {0} is not in this FFmpeg build")]
    NotCompiled(&'static str),
    #[error("GPU conversion failed: {0}")]
    Convert(#[from] windows::core::Error),
}

fn check(what: &'static str, ret: i32) -> Result<(), HwEncError> {
    if ret < 0 {
        Err(HwEncError::Ffmpeg {
            what,
            source: ffmpeg::Error::from(ret),
        })
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EncodeSettings {
    pub id: EncoderId,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u64,
    pub gop_seconds: u32,
    /// Lowest QP (highest quality) the encoder may use. It stops calm scenes
    /// spending bits on invisible detail (docs/perf/2026-11-04-rate-control.md).
    pub qp_floor: Option<u32>,
}

impl EncodeSettings {
    /// Default bitrate for a resolution and frame rate (the "Balanced" preset).
    /// HEVC needs roughly 30 % less than H.264 for the same quality.
    pub fn default_bitrate(codec: Codec, width: u32, height: u32, fps: u32) -> u64 {
        let bits_per_pixel = match codec {
            Codec::H264 => 0.17,
            Codec::Hevc | Codec::Av1 => 0.12,
        };
        (f64::from(width) * f64::from(height) * f64::from(fps) * bits_per_pixel) as u64
    }
}

pub struct HwEncoder {
    enc: encoder::Video,
    device_ref: *mut ffi::AVBufferRef,
    frames_ref: *mut ffi::AVBufferRef,
    frame: *mut ffi::AVFrame,
    pkt: Packet,
    pub time_base: Rational,
}

// SAFETY: the FFmpeg contexts are owned exclusively by this value and used
// from one thread at a time (the encoder thread).
unsafe impl Send for HwEncoder {}

impl HwEncoder {
    pub fn open(gpu: &Gpu, s: EncodeSettings) -> Result<Self, HwEncError> {
        ffmpeg::init().map_err(|source| HwEncError::Ffmpeg {
            what: "ffmpeg init",
            source,
        })?;
        let name = s.id.ffmpeg_name();
        let codec = encoder::find_by_name(name).ok_or(HwEncError::NotCompiled(name))?;
        let time_base = Rational::new(1, s.fps as i32);

        // SAFETY: standard FFmpeg hwcontext setup. FFmpeg takes ownership of
        // the device reference we hand it (`into_raw`).
        let (device_ref, frames_ref) = unsafe {
            let device_ref =
                ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
            if device_ref.is_null() {
                return Err(HwEncError::Null("av_hwdevice_ctx_alloc"));
            }
            let dev_ctx = (*device_ref).data as *mut ffi::AVHWDeviceContext;
            let d3d = (*dev_ctx).hwctx as *mut ffi::AVD3D11VADeviceContext;
            (*d3d).device = gpu.device.clone().into_raw() as *mut ffi::ID3D11Device;
            let mut device_ref = device_ref;
            if let Err(e) = check(
                "av_hwdevice_ctx_init",
                ffi::av_hwdevice_ctx_init(device_ref),
            ) {
                ffi::av_buffer_unref(&mut device_ref);
                return Err(e);
            }
            let mut frames_ref = ffi::av_hwframe_ctx_alloc(device_ref);
            if frames_ref.is_null() {
                ffi::av_buffer_unref(&mut device_ref);
                return Err(HwEncError::Null("av_hwframe_ctx_alloc"));
            }
            let fc = (*frames_ref).data as *mut ffi::AVHWFramesContext;
            (*fc).format = ffi::AVPixelFormat::AV_PIX_FMT_D3D11;
            (*fc).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
            (*fc).width = s.width as i32;
            (*fc).height = s.height as i32;
            (*fc).initial_pool_size = 0; // grow on demand to what the encoder actually holds
            let fhw = (*fc).hwctx as *mut ffi::AVD3D11VAFramesContext;
            (*fhw).BindFlags = (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32;
            if let Err(e) = check("av_hwframe_ctx_init", ffi::av_hwframe_ctx_init(frames_ref)) {
                ffi::av_buffer_unref(&mut frames_ref);
                ffi::av_buffer_unref(&mut device_ref);
                return Err(e);
            }
            (device_ref, frames_ref)
        };

        let mut this_cleanup = Cleanup {
            device_ref,
            frames_ref,
        };
        let mut enc = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|source| HwEncError::Ffmpeg {
                what: "encoder context",
                source,
            })?;
        enc.set_width(s.width);
        enc.set_height(s.height);
        enc.set_format(Pixel::D3D11);
        enc.set_time_base(time_base);
        enc.set_frame_rate(Some(Rational::new(s.fps as i32, 1)));
        enc.set_bit_rate(s.bitrate as usize);
        enc.set_max_bit_rate((s.bitrate * 3 / 2) as usize);
        enc.set_gop(s.fps * s.gop_seconds.max(1));
        enc.set_max_b_frames(0);
        enc.set_colorspace(color::Space::BT709);
        enc.set_color_range(color::Range::MPEG);
        unsafe {
            let raw = enc.as_mut_ptr();
            (*raw).color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            (*raw).color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            (*raw).rc_buffer_size = s.bitrate as i32;
            // Measured on AMF only; other vendors keep their defaults until tested.
            if s.id.vendor == Vendor::Amd
                && let Some(q) = s.qp_floor
            {
                (*raw).qmin = q as i32;
            }
            (*raw).hw_frames_ctx = ffi::av_buffer_ref(frames_ref);
        }

        let mut opts = Dictionary::new();
        // Queue depth inside the encoder: lower means less VRAM and save latency.
        opts.set("async_depth", "4");
        match s.id.vendor {
            Vendor::Amd => {
                opts.set("usage", "transcoding");
                opts.set("quality", "balanced");
                opts.set("rc", "vbr_peak");
                // Without this, AV_PICTURE_TYPE_I becomes a non-IDR I-frame and a
                // forced cut waits for the next natural GOP (verified on FFmpeg 9 AMF).
                opts.set("forced_idr", "1");
                if s.id.codec == Codec::Hevc {
                    opts.set("header_insertion_mode", "idr");
                    opts.set("gops_per_idr", "1");
                } else {
                    opts.set(
                        "header_spacing",
                        &(s.fps * s.gop_seconds.max(1)).to_string(),
                    );
                }
            }
            Vendor::Nvidia => {
                opts.set("preset", "p4");
                opts.set("tune", "hq");
                opts.set("rc", "vbr");
                opts.set("forced-idr", "1");
            }
            Vendor::Intel => {
                opts.set("preset", "medium");
                opts.set("forced_idr", "1");
            }
            Vendor::MediaFoundation => {
                opts.set("hw_encoding", "1");
            }
        }
        let enc = enc
            .open_as_with(codec, opts)
            .map_err(|source| HwEncError::Ffmpeg {
                what: "open encoder",
                source,
            })?;

        let frame = unsafe { ffi::av_frame_alloc() };
        if frame.is_null() {
            return Err(HwEncError::Null("av_frame_alloc"));
        }
        this_cleanup.disarm();
        Ok(Self {
            enc,
            device_ref,
            frames_ref,
            frame,
            pkt: Packet::empty(),
            time_base,
        })
    }

    /// Codec parameters for muxers (call after `open`).
    pub fn parameters(&self) -> codec::Parameters {
        codec::Parameters::from(&*self.enc)
    }

    /// Encodes one frame. `fill` must render into the given NV12 surface.
    /// Every finished packet is passed to `sink`. Returns `Ok(false)` if the
    /// encoder was saturated and the frame was dropped (not fatal).
    pub fn encode(
        &mut self,
        pts: i64,
        force_keyframe: bool,
        fill: impl FnOnce(&ID3D11Texture2D, u32) -> windows::core::Result<()>,
        mut sink: impl FnMut(Packet),
    ) -> Result<bool, HwEncError> {
        // SAFETY: `frame` is ours and unreferenced between calls. data[0]/[1]
        // of a D3D11 frame are the texture and its array index (FFmpeg docs).
        let mut ret = unsafe {
            check(
                "av_hwframe_get_buffer",
                ffi::av_hwframe_get_buffer(self.frames_ref, self.frame, 0),
            )?;
            let tex_ptr = (*self.frame).data[0] as *mut std::ffi::c_void;
            let index = (*self.frame).data[1] as usize as u32;
            let filled = match ID3D11Texture2D::from_raw_borrowed(&tex_ptr) {
                Some(tex) => fill(tex, index).map_err(HwEncError::from),
                None => Err(HwEncError::Null("D3D11 frame texture")),
            };
            if let Err(e) = filled {
                ffi::av_frame_unref(self.frame);
                return Err(e);
            }
            (*self.frame).pts = pts;
            (*self.frame).pict_type = if force_keyframe {
                ffi::AVPictureType::AV_PICTURE_TYPE_I
            } else {
                ffi::AVPictureType::AV_PICTURE_TYPE_NONE
            };
            ffi::avcodec_send_frame(self.enc.as_mut_ptr(), self.frame)
        };
        if ret == AVERROR_EAGAIN {
            // The encoder's input queue is full (GPU saturated). Drain, then retry once.
            if let Err(e) = self.drain(&mut sink) {
                unsafe { ffi::av_frame_unref(self.frame) };
                return Err(e);
            }
            ret = unsafe { ffi::avcodec_send_frame(self.enc.as_mut_ptr(), self.frame) };
        }
        unsafe { ffi::av_frame_unref(self.frame) };
        if ret == AVERROR_EAGAIN {
            return Ok(false);
        }
        check("avcodec_send_frame", ret)?;
        self.drain(&mut sink)?;
        Ok(true)
    }

    /// Flushes delayed packets at the end of a run.
    pub fn finish(&mut self, mut sink: impl FnMut(Packet)) -> Result<(), HwEncError> {
        self.enc.send_eof().map_err(|source| HwEncError::Ffmpeg {
            what: "send_eof",
            source,
        })?;
        self.drain(&mut sink)
    }

    /// Passes every ready packet to `sink`. EAGAIN/EOF end the loop; anything
    /// else (e.g. device removed) is an error.
    fn drain(&mut self, sink: &mut impl FnMut(Packet)) -> Result<(), HwEncError> {
        loop {
            let ret = unsafe {
                ffi::avcodec_receive_packet(self.enc.as_mut_ptr(), self.pkt.as_mut_ptr())
            };
            if ret == AVERROR_EAGAIN || ret == ffi::AVERROR_EOF {
                return Ok(());
            }
            check("avcodec_receive_packet", ret)?;
            sink(std::mem::replace(&mut self.pkt, Packet::empty()));
        }
    }
}

const AVERROR_EAGAIN: i32 = -ffi::EAGAIN;

impl Drop for HwEncoder {
    fn drop(&mut self) {
        unsafe {
            ffi::av_frame_free(&mut self.frame);
            ffi::av_buffer_unref(&mut self.frames_ref);
            ffi::av_buffer_unref(&mut self.device_ref);
        }
    }
}

/// Frees the hw contexts if `open` fails after creating them.
struct Cleanup {
    device_ref: *mut ffi::AVBufferRef,
    frames_ref: *mut ffi::AVBufferRef,
}

impl Cleanup {
    fn disarm(&mut self) {
        self.device_ref = ptr::null_mut();
        self.frames_ref = ptr::null_mut();
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        unsafe {
            if !self.frames_ref.is_null() {
                ffi::av_buffer_unref(&mut self.frames_ref);
            }
            if !self.device_ref.is_null() {
                ffi::av_buffer_unref(&mut self.device_ref);
            }
        }
    }
}
