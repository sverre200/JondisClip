//! BGRA → NV12 (BT.709, limited range) conversion and scaling on the GPU with
//! the D3D11 video processor. It's vendor-neutral and does no CPU copies.

use std::mem::ManuallyDrop;

use windows::Win32::Foundation::{RECT, TRUE};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_TEX2D_ARRAY_VPOV, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2DARRAY, ID3D11Texture2D,
    ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
    ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
    DXGI_RATIONAL,
};
use windows::core::{Interface, Result};

use crate::gpu::Gpu;

/// Upper bound on cached output views (one per encoder pool surface).
const MAX_OUTPUT_VIEWS: usize = 48;

pub struct Converter {
    vdev: ID3D11VideoDevice,
    vctx: ID3D11VideoContext1,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    /// Built once and reused every frame, so there's no per-frame AddRef/Release.
    stream: [D3D11_VIDEO_PROCESSOR_STREAM; 1],
    /// Output views keyed by (texture pointer, array index).
    views: Vec<(usize, u32, ID3D11VideoProcessorOutputView)>,
}

// SAFETY: see `Gpu`. The converter is used only from the encoder thread.
unsafe impl Send for Converter {}

impl Converter {
    pub fn new(gpu: &Gpu, src: &ID3D11Texture2D, out_w: u32, out_h: u32, fps: u32) -> Result<Self> {
        let mut sd = D3D11_TEXTURE2D_DESC::default();
        unsafe { src.GetDesc(&mut sd) };
        let vdev: ID3D11VideoDevice = gpu.device.cast()?;
        let vctx: ID3D11VideoContext1 = gpu.context.cast()?;
        let rate = DXGI_RATIONAL {
            Numerator: fps,
            Denominator: 1,
        };
        let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: sd.Width,
            InputHeight: sd.Height,
            OutputFrameRate: rate,
            OutputWidth: out_w,
            OutputHeight: out_h,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        let enumerator = unsafe { vdev.CreateVideoProcessorEnumerator(&content)? };
        let processor = unsafe { vdev.CreateVideoProcessor(&enumerator, 0)? };

        let in_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let mut input_view = None;
        unsafe {
            vdev.CreateVideoProcessorInputView(src, &enumerator, &in_desc, Some(&mut input_view))?
        };

        let src_rect = RECT {
            left: 0,
            top: 0,
            right: sd.Width as i32,
            bottom: sd.Height as i32,
        };
        let dst_rect = RECT {
            left: 0,
            top: 0,
            right: out_w as i32,
            bottom: out_h as i32,
        };
        unsafe {
            vctx.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            vctx.VideoProcessorSetStreamColorSpace1(
                &processor,
                0,
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            );
            vctx.VideoProcessorSetOutputColorSpace1(
                &processor,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
            );
            vctx.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&src_rect));
            vctx.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dst_rect));
            vctx.VideoProcessorSetOutputTargetRect(&processor, true, Some(&dst_rect));
            vctx.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
        }

        let stream = [D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: TRUE,
            pInputSurface: ManuallyDrop::new(input_view),
            ..Default::default()
        }];
        Ok(Self {
            vdev,
            vctx,
            enumerator,
            processor,
            stream,
            views: Vec::with_capacity(MAX_OUTPUT_VIEWS),
        })
    }

    /// Converts the source texture into `target[index]` (an NV12 surface from
    /// the encoder's frame pool).
    pub fn blit(&mut self, target: &ID3D11Texture2D, index: u32) -> Result<()> {
        let key = target.as_raw() as usize;
        let pos = match self
            .views
            .iter()
            .position(|(k, i, _)| *k == key && *i == index)
        {
            Some(p) => p,
            None => {
                if self.views.len() >= MAX_OUTPUT_VIEWS {
                    self.views.clear();
                }
                let view = self.create_output_view(target, index)?;
                self.views.push((key, index, view));
                self.views.len() - 1
            }
        };
        let view = &self.views[pos].2;
        unsafe {
            self.vctx
                .VideoProcessorBlt(&self.processor, view, 0, &self.stream)
        }
    }

    fn create_output_view(
        &self,
        target: &ID3D11Texture2D,
        index: u32,
    ) -> Result<ID3D11VideoProcessorOutputView> {
        let mut td = D3D11_TEXTURE2D_DESC::default();
        unsafe { target.GetDesc(&mut td) };
        let desc = if td.ArraySize > 1 {
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2DARRAY,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2DArray: D3D11_TEX2D_ARRAY_VPOV {
                        MipSlice: 0,
                        FirstArraySlice: index,
                        ArraySize: 1,
                    },
                },
            }
        } else {
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            }
        };
        let mut view = None;
        unsafe {
            self.vdev.CreateVideoProcessorOutputView(
                target,
                &self.enumerator,
                &desc,
                Some(&mut view),
            )?
        };
        view.ok_or_else(windows::core::Error::empty)
    }
}

impl Drop for Converter {
    fn drop(&mut self) {
        // SAFETY: `stream` is dropped exactly once, here.
        unsafe { ManuallyDrop::drop(&mut self.stream[0].pInputSurface) };
    }
}

/// Output size for a requested height, keeping the aspect ratio. Both
/// dimensions are even (NV12 requirement), and it never upscales.
pub fn output_size(src_w: u32, src_h: u32, height: Option<u32>) -> (u32, u32) {
    let h = height.map_or(src_h, |h| h.min(src_h)).max(2);
    let w = (u64::from(src_w) * u64::from(h) / u64::from(src_h.max(1))) as u32;
    (w & !1, h & !1)
}

#[cfg(test)]
mod tests {
    use super::output_size;

    #[test]
    fn output_size_keeps_aspect_and_even() {
        assert_eq!(output_size(2560, 1440, None), (2560, 1440));
        assert_eq!(output_size(2560, 1440, Some(1080)), (1920, 1080));
        assert_eq!(output_size(3440, 1440, Some(1080)), (2580, 1080));
        assert_eq!(output_size(1920, 1080, Some(1440)), (1920, 1080)); // no upscale
        assert_eq!(output_size(1366, 767, None), (1366, 766));
    }
}
