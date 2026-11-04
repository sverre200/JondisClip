//! Windows Graphics Capture of a monitor (ADR-0003).
//!
//! WGC delivers frames only when the screen changes, on a thread-pool thread.
//! The callback does one GPU→GPU `CopyResource` into `latest` and returns the
//! pool buffer straight away. The encoder thread samples `latest` at a fixed
//! rate, which gives constant-frame-rate output even when the screen is static.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{IInspectable, Interface, Result, factory};

use crate::gpu::Gpu;

/// Counters and flags shared between the WGC callback and the encoder.
#[derive(Default)]
pub struct CaptureShared {
    pub frames_arrived: AtomicU64,
    pub has_frame: AtomicBool,
    /// The captured content no longer matches `latest` (display mode change).
    pub size_changed: AtomicBool,
    /// The capture item was closed (monitor unplugged or disabled).
    pub source_closed: AtomicBool,
}

pub struct Capture {
    item: GraphicsCaptureItem,
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    token: i64,
    closed_token: i64,
    pub shared: Arc<CaptureShared>,
    /// Most recent captured frame (BGRA, source resolution).
    pub latest: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
}

// SAFETY: same reasoning as for `Gpu`. The WinRT capture objects are agile.
unsafe impl Send for Capture {}

struct CopyTarget {
    context: ID3D11DeviceContext,
    latest: ID3D11Texture2D,
    width: u32,
    height: u32,
    shared: Arc<CaptureShared>,
}
// SAFETY: the context is multithread-protected (see `Gpu`), and `latest` is only
// written here and read by the GPU through the same context.
unsafe impl Send for CopyTarget {}

impl CopyTarget {
    fn on_frame(&self, pool: &Direct3D11CaptureFramePool) -> Result<()> {
        let frame = pool.TryGetNextFrame()?;
        // Surfaces always have the pool's size. A mode change shows up only in
        // ContentSize, so that's what has to be compared.
        let content = frame.ContentSize()?;
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let tex: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { tex.GetDesc(&mut desc) };
        let same = content.Width as u32 == self.width
            && content.Height as u32 == self.height
            && desc.Width == self.width
            && desc.Height == self.height;
        if same {
            unsafe { self.context.CopyResource(&self.latest, &tex) };
            self.shared.has_frame.store(true, Ordering::Release);
        } else {
            self.shared.size_changed.store(true, Ordering::Release);
        }
        self.shared.frames_arrived.fetch_add(1, Ordering::Relaxed);
        frame.Close()
    }
}

impl Capture {
    /// Starts capturing the primary monitor.
    pub fn primary_monitor(gpu: &Gpu, fps: u32) -> Result<Self> {
        let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(monitor)? };
        let size = item.Size()?;
        let (width, height) = (size.Width as u32, size.Height as u32);

        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut latest = None;
        unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut latest))? };
        let latest = latest.ok_or_else(windows::core::Error::empty)?;

        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &gpu.winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        // These setters only exist on newer Windows 11 builds. Failure is harmless.
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(true);
        let _ = session.SetMinUpdateInterval(TimeSpan {
            Duration: 10_000_000 / i64::from(fps.max(1)),
        });

        let shared = Arc::new(CaptureShared::default());
        let target = CopyTarget {
            context: gpu.context.clone(),
            latest: latest.clone(),
            width,
            height,
            shared: shared.clone(),
        };
        let handler =
            TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _| {
                if let Some(pool) = sender.as_ref() {
                    // Errors here (e.g. a frame closed during shutdown) must not
                    // propagate into WGC. Skip the frame.
                    let _ = target.on_frame(pool);
                }
                Ok(())
            });
        let token = pool.FrameArrived(&handler)?;
        let closed_flag = shared.clone();
        let closed_token = item.Closed(
            &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                closed_flag.source_closed.store(true, Ordering::Release);
                Ok(())
            }),
        )?;
        session.StartCapture()?;
        Ok(Self {
            item,
            session,
            pool,
            token,
            closed_token,
            shared,
            latest,
            width,
            height,
        })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.token);
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
