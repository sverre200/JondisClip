//! Opens the real D3D11 hardware encoder path (as the recorder does) and
//! prints FFmpeg's own log, to diagnose "open encoder failed".
//! Run: `cargo run -p jondisclip-engine --example hwopen -- [width height fps] [hevc] [mf] [verbose]`
use ffmpeg_next as ffmpeg;
use jondisclip_engine::convert::Converter;
use jondisclip_engine::encoder::{self, Codec, Vendor};
use jondisclip_engine::gpu::Gpu;
use jondisclip_engine::hwenc::{EncodeSettings, HwEncoder};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

fn main() {
    let args: Vec<u32> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let (width, height, fps) = match args[..] {
        [w, h, f] => (w, h, f),
        _ => (3440, 1440, 60),
    };
    let word = |w: &str| std::env::args().any(|a| a == w);
    ffmpeg::init().expect("ffmpeg init");
    ffmpeg::util::log::set_level(if word("verbose") {
        ffmpeg::util::log::Level::Verbose
    } else {
        ffmpeg::util::log::Level::Error
    });
    let gpu = Gpu::create().expect("gpu");
    println!(
        "adapter: {} (vendor {:#06x})",
        gpu.adapter_name, gpu.vendor_id
    );
    // The same search the recorder does: preferred codec first, the adapter's
    // own encoder before Media Foundation. "mf" skips the vendor's encoder.
    let preferred = if word("hevc") {
        Codec::Hevc
    } else {
        Codec::H264
    };
    let mut opened = None;
    'search: for &codec in &encoder::codecs_for(preferred, width, height) {
        for id in encoder::candidates_for_adapter(codec, gpu.vendor_id) {
            if word("mf") && id.vendor != Vendor::MediaFoundation {
                continue;
            }
            let s = EncodeSettings {
                id,
                width,
                height,
                fps,
                bitrate: EncodeSettings::default_bitrate(id.codec, width, height, fps),
                gop_seconds: 1,
                qp_floor: None,
            };
            println!("trying {} at {width}x{height}@{fps}", id.ffmpeg_name());
            match HwEncoder::open(&gpu, s) {
                Ok(e) => {
                    opened = Some((e, id));
                    break 'search;
                }
                Err(e) => println!("OPEN FAILED: {} : {e}", id.ffmpeg_name()),
            }
        }
    }
    let Some((mut enc, id)) = opened else {
        println!("NO ENCODER");
        return;
    };
    println!("OPEN OK: {}", id.ffmpeg_name());

    // Encode 2 s through the real converter from a BGRA source, like capture does.
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
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        ..Default::default()
    };
    let mut src = None;
    unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut src)) }.expect("src texture");
    let src = src.expect("src texture");
    let mut conv = Converter::new(&gpu, &src, width, height, fps).expect("converter");
    let (mut packets, mut bytes, mut dropped) = (0usize, 0usize, 0usize);
    for pts in 0..i64::from(fps * 2) {
        let sent = enc.encode(
            pts,
            pts == i64::from(fps),
            |tex, index| conv.blit(tex, index),
            |p| {
                packets += 1;
                bytes += p.size();
            },
        );
        match sent {
            Ok(true) => {}
            Ok(false) => dropped += 1,
            Err(e) => {
                println!("ENCODE FAILED at frame {pts}: {e}");
                return;
            }
        }
    }
    enc.finish(|p| {
        packets += 1;
        bytes += p.size();
    })
    .expect("finish");
    println!("ENCODE OK: {packets} packets, {bytes} bytes, {dropped} dropped");
}
