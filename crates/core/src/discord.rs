//! Discord share planning (ADR-0012): how to fit a selection into the user's
//! upload limit with the best picture. Pure: used by the window (to show what
//! you'll get) and by the exporter (to do it).

use serde::Serialize;

/// Share of the limit we plan for (container overhead + margin: hardware CBR
/// overshoots by up to ~5 %, measured on AMF). The result is measured anyway.
pub const BUDGET_SHARE: f64 = 0.92;
/// Never spend more than this × the source's video bitrate (more bits can't
/// add quality to an already-compressed recording).
pub const MAX_OVER_SOURCE: f64 = 1.3;
/// Hard ceiling for any share (H.264 level 4.2 territory; more is pointless
/// for a chat upload and some encoders refuse it).
pub const MAX_VIDEO_BPS: f64 = 50_000_000.0;
/// Bits per pixel per frame below which game motion falls apart on hw H.264.
pub const MIN_BPP: f64 = 0.06;
/// Below this there is no watchable video at any size.
pub const MIN_VIDEO_BPS: f64 = 150_000.0;
/// Bitrate of the recorder's Mix track (copied when the budget allows).
pub const MIX_BPS: f64 = 160_000.0;

/// Resolutions and frame rates we may output, best first. Games keep 60 fps
/// as long as possible (720p60 before 1080p30: similar pixel rate, and motion
/// matters more than detail in gameplay). A 30 fps source caps every rung at 30.
pub const LADDER: [(u32, u32); 6] = [
    (1080, 60),
    (720, 60),
    (720, 30),
    (540, 30),
    (480, 30),
    (360, 30),
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub video_bps: u64,
    /// 0 = copy the Mix track as is (160 kb/s).
    pub audio_bps: u64,
    /// Bits per pixel per frame at this rung (quality indicator).
    pub bpp: f64,
    /// Below the quality floor even at the lowest rung: it will look rough.
    pub rough: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum PlanError {
    /// Not even minimal video fits. `max_seconds` is the longest selection
    /// that would (at the lowest rung).
    TooLong {
        max_seconds: f64,
    },
    Invalid,
}

/// Even dimensions for a source of `src_w`×`src_h`, scaled to `height`.
fn fit(src_w: u32, src_h: u32, height: u32) -> (u32, u32) {
    let h = height.min(src_h) & !1;
    let w = ((f64::from(src_w) * f64::from(h) / f64::from(src_h.max(1))).round() as u32) & !1;
    (w.max(2), h.max(2))
}

fn audio_for(total_bps: f64) -> u64 {
    if total_bps >= 4_000_000.0 {
        0 // copy the 160 kb/s Mix
    } else if total_bps >= 2_000_000.0 {
        128_000
    } else if total_bps >= 800_000.0 {
        96_000
    } else {
        64_000
    }
}

/// Plans a share of `seconds` of a `src_w`×`src_h` clip at `src_fps` into
/// `limit_bytes`.
pub fn plan(
    seconds: f64,
    src_w: u32,
    src_h: u32,
    src_fps: f64,
    source_video_bps: Option<f64>,
    limit_bytes: u64,
) -> Result<Plan, PlanError> {
    let valid = seconds.is_finite() && seconds > 0.0 && src_w > 0 && src_h > 0 && limit_bytes > 0;
    if !valid {
        return Err(PlanError::Invalid);
    }
    let total_bps = limit_bytes as f64 * BUDGET_SHARE * 8.0 / seconds;
    let audio_bps = audio_for(total_bps);
    let audio_real = if audio_bps == 0 {
        MIX_BPS
    } else {
        audio_bps as f64
    };
    let video_bps = total_bps - audio_real;
    if video_bps < MIN_VIDEO_BPS {
        let per_second = MIN_VIDEO_BPS + 64_000.0;
        return Err(PlanError::TooLong {
            max_seconds: (limit_bytes as f64 * BUDGET_SHARE * 8.0 / per_second).floor(),
        });
    }
    let src_fps = if src_fps > 0.0 { src_fps } else { 60.0 };
    let mut chosen = None;
    for &(height, fps) in &LADDER {
        if height > src_h.max(360) {
            continue; // never upscale (but always allow the bottom rung)
        }
        let fps = (fps as f64).min(src_fps.round()) as u32;
        let (w, h) = fit(src_w, src_h, height);
        let bpp = video_bps / (f64::from(w) * f64::from(h) * f64::from(fps));
        chosen = Some((w, h, fps, bpp));
        if bpp >= MIN_BPP {
            break;
        }
    }
    let (width, height, fps, bpp) = chosen.ok_or(PlanError::Invalid)?;
    // More than the source had (per output pixel) can't add quality, and a
    // hard ceiling keeps encoders within their levels.
    let mut video_bps = video_bps.min(MAX_VIDEO_BPS);
    if let Some(s) = source_video_bps.filter(|s| *s > 0.0) {
        let share = (f64::from(width) * f64::from(height)) / (f64::from(src_w) * f64::from(src_h));
        video_bps = video_bps.min(s * MAX_OVER_SOURCE * share.min(1.0));
    }
    Ok(Plan {
        width,
        height,
        fps,
        video_bps: video_bps as u64,
        audio_bps,
        bpp,
        rough: bpp < MIN_BPP,
    })
}

/// Whether a stream copy of `bytes` (video + Mix only) fits the limit.
pub fn copy_fits(bytes: u64, limit_bytes: u64) -> bool {
    (bytes as f64) <= limit_bytes as f64 * BUDGET_SHARE
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB19: u64 = 19_000_000;

    #[test]
    fn a_short_highlight_gets_720p60() {
        let p = plan(20.0, 1920, 1080, 59.8, None, MB19).unwrap();
        assert_eq!((p.height, p.fps), (720, 60), "{p:?}");
        assert!(p.bpp >= MIN_BPP && !p.rough);
        assert_eq!(p.audio_bps, 0, "budget is generous: copy the Mix");
        let total = (p.video_bps + 160_000) as f64 * 20.0 / 8.0;
        assert!(total <= MB19 as f64 * BUDGET_SHARE + 1.0);
    }

    #[test]
    fn longer_clips_step_down_the_ladder() {
        let h = |s: f64| plan(s, 1920, 1080, 60.0, None, MB19).unwrap();
        assert_eq!((h(8.0).height, h(8.0).fps), (1080, 60));
        assert!(h(60.0).height <= 720 && h(60.0).fps == 30);
        let long = h(240.0);
        assert!(long.height <= 480, "{long:?}");
        assert!(long.audio_bps > 0 && long.audio_bps <= 96_000);
        // Monotonic: never better quality for a longer clip.
        let mut last = u64::MAX;
        for s in [5.0, 15.0, 30.0, 60.0, 120.0, 240.0] {
            let p = h(s);
            let px = u64::from(p.width) * u64::from(p.height) * u64::from(p.fps);
            assert!(px <= last, "{s} s");
            last = px;
        }
    }

    #[test]
    fn bigger_limits_buy_quality_and_never_upscale() {
        let p = plan(60.0, 1920, 1080, 60.0, None, 500_000_000).unwrap();
        assert_eq!((p.width, p.height, p.fps), (1920, 1080, 60));
        let p = plan(10.0, 1280, 720, 60.0, None, 500_000_000).unwrap();
        assert_eq!(p.height, 720, "a 720p source stays 720p");
        let p = plan(10.0, 2560, 1440, 30.0, None, 500_000_000).unwrap();
        assert_eq!((p.height, p.fps), (1080, 30), "30 fps source stays 30");
        let p = plan(10.0, 3440, 1440, 60.0, None, 500_000_000).unwrap();
        assert_eq!(p.width % 2, 0);
        assert_eq!(p.width, 2580, "ultrawide keeps its aspect");
    }

    #[test]
    fn far_too_long_is_refused_with_a_hint() {
        match plan(3600.0, 1920, 1080, 60.0, None, MB19) {
            Err(PlanError::TooLong { max_seconds }) => {
                assert!(max_seconds > 300.0 && max_seconds < 1000.0, "{max_seconds}")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            plan(0.0, 1920, 1080, 60.0, None, MB19),
            Err(PlanError::Invalid)
        );
        assert_eq!(
            plan(f64::NAN, 1920, 1080, 60.0, None, MB19),
            Err(PlanError::Invalid)
        );
        assert_eq!(plan(10.0, 0, 0, 60.0, None, MB19), Err(PlanError::Invalid));
    }

    #[test]
    fn short_clips_never_exceed_the_source_bitrate() {
        let p = plan(3.0, 1920, 1080, 60.0, Some(17e6), MB19).unwrap();
        assert_eq!(p.height, 1080);
        assert!(p.video_bps as f64 <= 17e6 * MAX_OVER_SOURCE + 1.0, "{p:?}");
        // Scaled down, the cap scales with the pixels.
        let q = plan(30.0, 1920, 1080, 60.0, Some(17e6), MB19).unwrap();
        assert!(q.height < 1080);
        // Huge limit, tiny selection, unknown source: still a sane bitrate.
        let r = plan(0.5, 3840, 2160, 60.0, None, 1_000_000_000).unwrap();
        assert!(r.video_bps as f64 <= MAX_VIDEO_BPS, "{r:?}");
    }

    #[test]
    fn copy_fits_leaves_the_margin() {
        assert!(copy_fits(17_000_000, MB19));
        assert!(!copy_fits(18_000_000, MB19), "the margin is kept");
        assert!(!copy_fits(MB19, MB19));
    }
}
