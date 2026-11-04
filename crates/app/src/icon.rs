//! Tray icons drawn at runtime (anti-aliased), so there are no image assets and
//! they're crisp at any DPI. The glyph is a "record" mark (ring plus dot) in
//! the state colour, with a dark outline so it reads on both light and dark
//! taskbars.

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    /// Buffering normally.
    Recording,
    /// Paused by the user, or no recorder (idle).
    Paused,
    /// A clip was just saved.
    Saved,
    /// Something needs attention (error, low disk, restarting).
    Warning,
}

impl IconState {
    fn rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Recording => (0xE5, 0x48, 0x4D), // red
            Self::Paused => (0x9A, 0xA0, 0xA6),    // grey
            Self::Saved => (0x30, 0xA4, 0x6C),     // green
            Self::Warning => (0xF5, 0xA5, 0x24),   // amber
        }
    }
}

/// Coverage of a pixel by an annulus/disk, approximated by distance (1 px AA band).
fn coverage(d: f32, inner: f32, outer: f32) -> f32 {
    let a = (outer - d + 0.5).clamp(0.0, 1.0);
    let b = if inner > 0.0 {
        (d - inner + 0.5).clamp(0.0, 1.0)
    } else {
        1.0
    };
    a.min(b)
}

/// Premultiplied ARGB pixels (top-down rows) for the icon at `size` px.
pub fn render(state: IconState, size: i32) -> Vec<u32> {
    let size = size.clamp(16, 256);
    let n = size as usize;
    let s = size as f32;
    let c = (s - 1.0) / 2.0;
    let r_out = s / 2.0 - 0.5;
    let ring_w = (s / 8.0).max(1.5);
    let dot_r = s * 0.22;
    let outline = (s / 16.0).max(1.0);
    let (cr, cg, cb) = state.rgb();

    let mut px = vec![0u32; n * n];
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            // Layers: dark outline disk, coloured ring, dark gap, coloured dot.
            let base = coverage(d, 0.0, r_out); // outline/background disk
            let ring = coverage(d, r_out - outline - ring_w, r_out - outline);
            let dot = coverage(d, 0.0, dot_r);
            let colour = ring.max(dot);
            let alpha = base;
            if alpha <= 0.0 {
                continue;
            }
            // Mix colour over a near-black disk (premultiplied ARGB).
            let mix = |cc: u8, dark: u8| -> u32 {
                let v = f32::from(dark) * (1.0 - colour) + f32::from(cc) * colour;
                (v * alpha).round() as u32
            };
            let (r, g, b) = (mix(cr, 0x1A), mix(cg, 0x1C), mix(cb, 0x20));
            let a = (alpha * 255.0).round() as u32;
            px[y * n + x] = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }
    px
}

pub fn make(state: IconState, size: i32) -> Option<HICON> {
    let size = size.clamp(16, 256);
    let n = size as usize;
    let px = render(state, size);

    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let color: HBITMAP =
            CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        if bits.is_null() {
            let _ = DeleteObject(color.into());
            return None;
        }
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, n * n);
        let mask_bits = vec![0u8; n.div_ceil(16) * 2 * n];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr().cast()));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::{IconState, coverage, render};

    #[test]
    fn render_has_transparent_corners_and_state_coloured_centre() {
        for size in [16, 20, 24, 32] {
            let px = render(IconState::Recording, size);
            let n = size as usize;
            assert_eq!(px[0] >> 24, 0, "corner must be transparent at {size}px");
            let centre = px[(n / 2) * n + n / 2];
            assert_eq!(centre >> 24, 255);
            let (r, g) = ((centre >> 16) & 0xFF, (centre >> 8) & 0xFF);
            assert!(
                r > 200 && g < 120,
                "centre should be red at {size}px: {centre:08x}"
            );
        }
    }

    /// Writes the icons to a 32-bit BMP for visual review:
    /// `JC_ICON_DUMP=out.bmp cargo test -p jondisclip dump_icons -- --ignored`
    #[test]
    #[ignore]
    fn dump_icons() {
        let Ok(path) = std::env::var("JC_ICON_DUMP") else {
            return;
        };
        let states = [
            IconState::Recording,
            IconState::Paused,
            IconState::Saved,
            IconState::Warning,
        ];
        let sizes = [16usize, 24, 32, 64];
        let (w, h) = (
            sizes.iter().sum::<usize>() + 8 * sizes.len(),
            64 * states.len() + 8 * states.len(),
        );
        let mut canvas = vec![0xFF20_2124u32; w * h]; // dark taskbar background
        for (row, st) in states.iter().enumerate() {
            let mut x0 = 4;
            for &sz in &sizes {
                let px = render(*st, sz as i32);
                for y in 0..sz {
                    for x in 0..sz {
                        let p = px[y * sz + x];
                        let a = (p >> 24) as f32 / 255.0;
                        let bg = canvas[(row * 72 + 4 + y) * w + x0 + x];
                        let blend = |sh: u32| -> u32 {
                            let s = ((p >> sh) & 0xFF) as f32;
                            let b = ((bg >> sh) & 0xFF) as f32;
                            (s + b * (1.0 - a)).round().min(255.0) as u32
                        };
                        canvas[(row * 72 + 4 + y) * w + x0 + x] =
                            0xFF00_0000 | (blend(16) << 16) | (blend(8) << 8) | blend(0);
                    }
                }
                x0 += sz + 8;
            }
        }
        let mut bmp = Vec::new();
        let data_len = (w * h * 4) as u32;
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(54 + data_len).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(w as i32).to_le_bytes());
        bmp.extend_from_slice(&(-(h as i32)).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        for p in canvas {
            bmp.extend_from_slice(&p.to_le_bytes());
        }
        std::fs::write(path, bmp).unwrap();
    }

    #[test]
    fn coverage_is_antialiased_and_bounded() {
        assert_eq!(coverage(0.0, 0.0, 5.0), 1.0);
        assert_eq!(coverage(10.0, 0.0, 5.0), 0.0);
        let edge = coverage(5.0, 0.0, 5.0);
        assert!(edge > 0.0 && edge < 1.0);
        assert_eq!(coverage(1.0, 3.0, 5.0), 0.0); // inside the hole of a ring
    }
}
