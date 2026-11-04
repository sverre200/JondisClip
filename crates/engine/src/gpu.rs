//! The single D3D11 device shared by capture, conversion and the encoder
//! (`/media-pipeline`: one device, multithread-protected).

use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::core::{Interface, Result};

pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    /// WinRT wrapper of `device`, required by Windows Graphics Capture.
    pub winrt_device: IDirect3DDevice,
    pub vendor_id: u32,
    pub adapter_name: String,
}

// SAFETY: D3D11 devices are free-threaded, and the immediate context is made
// thread-safe with `SetMultithreadProtected(true)` before `Gpu` is returned.
// Every call we make on the context is a single self-contained command
// (CopyResource / VideoProcessorBlt), so there's no shared pipeline state to race on.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

pub const VENDOR_AMD: u32 = 0x1002;
pub const VENDOR_NVIDIA: u32 = 0x10DE;
pub const VENDOR_INTEL: u32 = 0x8086;

impl Gpu {
    /// Creates a hardware device on the default adapter.
    // TODO(phase 2 robustness): pick the adapter that drives the captured
    // monitor (hybrid-GPU laptops).
    pub fn create() -> Result<Self> {
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        let device: ID3D11Device = device.ok_or_else(windows::core::Error::empty)?;
        let context: ID3D11DeviceContext = context.ok_or_else(windows::core::Error::empty)?;
        let mt: ID3D11Multithread = context.cast()?;
        unsafe {
            let _ = mt.SetMultithreadProtected(true);
        }
        let dxgi: IDXGIDevice = device.cast()?;
        let desc = unsafe { dxgi.GetAdapter()?.GetDesc()? };
        let len = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let adapter_name = String::from_utf16_lossy(&desc.Description[..len]);
        let winrt_device: IDirect3DDevice =
            unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
        Ok(Self {
            device,
            context,
            winrt_device,
            vendor_id: desc.VendorId,
            adapter_name,
        })
    }
}
