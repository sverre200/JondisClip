//! User settings: `%APPDATA%\JondisClip\settings.toml`.
//!
//! The file is created with defaults on first run. Unknown keys are ignored,
//! missing keys take defaults, and invalid values fall back to defaults with a
//! warning, so a typo never stops JondisClip from starting.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::hotkey::Hotkey;

pub const REPLAY_PRESETS: [u32; 7] = [30, 60, 120, 180, 300, 600, 900];
pub const FPS_PRESETS: [u32; 4] = [30, 60, 120, 144];
pub const DEFAULT_HOTKEY: &str = "Alt+F10";
/// Microphone gain range in dB (matches the engine's clamp).
pub const MIC_GAIN_RANGE: (f32, f32) = (-12.0, 24.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    Source,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "1440p")]
    P1440,
}

impl Resolution {
    pub fn height(self) -> Option<u32> {
        match self {
            Self::Source => None,
            Self::P720 => Some(720),
            Self::P1080 => Some(1080),
            Self::P1440 => Some(1440),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Performance,
    Balanced,
    High,
}

impl Quality {
    pub fn scale(self) -> f64 {
        match self {
            Self::Performance => 0.7,
            Self::Balanced => 1.0,
            Self::High => 1.5,
        }
    }

    /// Quality floor (minimum QP). Measured on test footage: calm scenes use
    /// 55-60 % fewer bits for about 1-2 VMAF, and busy scenes are unchanged.
    pub fn qp_floor(self) -> u32 {
        match self {
            Self::Performance => 21,
            Self::Balanced => 18,
            Self::High => 16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodecPref {
    /// H.264: plays everywhere with no extra codecs (ADR-0007).
    Auto,
    Hevc,
    H264,
    Av1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Seconds of history saved by the hotkey: 30, 60, 120, 180, 300, 600 or 900.
    pub replay_seconds: u32,
    pub hotkey: String,
    /// Where clips are saved. Empty means Videos\JondisClip.
    pub save_dir: PathBuf,
    pub fps: u32,
    pub resolution: Resolution,
    pub quality: Quality,
    pub codec: CodecPref,
    pub system_audio: bool,
    pub microphone: bool,
    /// Windows endpoint ID of the microphone to record. Empty = Windows default.
    pub microphone_device: String,
    /// Microphone gain in dB, -12 to +24. Applied live.
    pub microphone_gain_db: f32,
    /// 80 Hz low-cut on the mic (off by default). Applied live.
    pub microphone_low_cut: bool,
    /// Short sound when a clip is saved (works in fullscreen games).
    pub save_sound: bool,
    pub start_with_windows: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            replay_seconds: 300,
            hotkey: DEFAULT_HOTKEY.into(),
            save_dir: PathBuf::new(),
            fps: 60,
            resolution: Resolution::Source,
            quality: Quality::Balanced,
            codec: CodecPref::Auto,
            system_audio: true,
            microphone: false,
            microphone_device: String::new(),
            microphone_gain_db: 0.0,
            microphone_low_cut: false,
            save_sound: true,
            start_with_windows: false,
        }
    }
}

impl Settings {
    /// Parses TOML, fixing invalid values. Returns the settings and warnings.
    pub fn from_toml(text: &str) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut s: Settings = match toml::from_str(text) {
            Ok(s) => s,
            Err(e) => {
                warnings.push(format!(
                    "settings file is invalid, using defaults: {}",
                    e.message()
                ));
                Settings::default()
            }
        };
        if !REPLAY_PRESETS.contains(&s.replay_seconds) {
            let nearest = *REPLAY_PRESETS
                .iter()
                .min_by_key(|p| p.abs_diff(s.replay_seconds))
                .unwrap_or(&300);
            warnings.push(format!(
                "replay_seconds {} is not a preset; using {nearest}",
                s.replay_seconds
            ));
            s.replay_seconds = nearest;
        }
        if !FPS_PRESETS.contains(&s.fps) {
            warnings.push(format!("fps {} is not supported; using 60", s.fps));
            s.fps = 60;
        }
        if !s.microphone_gain_db.is_finite()
            || !(MIC_GAIN_RANGE.0..=MIC_GAIN_RANGE.1).contains(&s.microphone_gain_db)
        {
            let fixed = if s.microphone_gain_db.is_finite() {
                s.microphone_gain_db
                    .clamp(MIC_GAIN_RANGE.0, MIC_GAIN_RANGE.1)
            } else {
                0.0
            };
            warnings.push(format!(
                "microphone_gain_db {} is outside {}..{}; using {fixed}",
                s.microphone_gain_db, MIC_GAIN_RANGE.0, MIC_GAIN_RANGE.1
            ));
            s.microphone_gain_db = fixed;
        }
        if let Err(e) = Hotkey::parse(&s.hotkey) {
            warnings.push(format!(
                "hotkey \"{}\": {e}; using {DEFAULT_HOTKEY}",
                s.hotkey
            ));
            s.hotkey = DEFAULT_HOTKEY.into();
        }
        (s, warnings)
    }

    pub fn to_toml(&self) -> String {
        let body = toml::to_string_pretty(self).unwrap_or_default();
        format!(
            "# JondisClip settings. Changes apply automatically when you save this file.\n\
             # replay_seconds: 30, 60, 120, 180, 300, 600, 900\n\
             # hotkey: e.g. \"Alt+F10\", \"Ctrl+Shift+S\", \"F8\"\n\
             # save_dir: empty = Videos\\JondisClip\n\
             # fps: 30, 60, 120, 144 | resolution: source, 720p, 1080p, 1440p\n\
             # quality: performance, balanced, high | codec: auto (= h264), h264, hevc, av1\n\
             # (hevc/av1 need the paid HEVC extension or VLC to play on Windows)\n\n{body}"
        )
    }

    pub fn hotkey(&self) -> Hotkey {
        Hotkey::parse(&self.hotkey)
            .unwrap_or_else(|_| Hotkey::parse(DEFAULT_HOTKEY).expect("default hotkey parses"))
    }

    pub fn replay(&self) -> Duration {
        Duration::from_secs(u64::from(self.replay_seconds))
    }

    pub fn save_dir_or(&self, default: &Path) -> PathBuf {
        if self.save_dir.as_os_str().is_empty() {
            default.to_path_buf()
        } else {
            self.save_dir.clone()
        }
    }

    /// True when switching from `self` to `other` needs a recorder restart
    /// (anything but replay length, mic gain, hotkey, save dir, sound and autostart).
    pub fn needs_restart(&self, other: &Settings) -> bool {
        self.fps != other.fps
            || self.resolution != other.resolution
            || self.quality != other.quality
            || self.codec != other.codec
            || self.system_audio != other.system_audio
            || self.microphone != other.microphone
            || self.microphone_device != other.microphone_device
    }
}

/// Human label for a replay length: "30 s", "1 min", "5 min".
pub fn replay_label(secs: u32) -> String {
    if secs < 60 {
        format!("{secs} s")
    } else {
        format!("{} min", secs / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let s = Settings::default();
        let (back, warnings) = Settings::from_toml(&s.to_toml());
        assert_eq!(back, s);
        assert!(warnings.is_empty());
    }

    #[test]
    fn missing_keys_take_defaults() {
        let (s, w) = Settings::from_toml("replay_seconds = 60\n");
        assert_eq!(s.replay_seconds, 60);
        assert_eq!(s.hotkey, DEFAULT_HOTKEY);
        assert!(!s.microphone);
        assert!(w.is_empty());
    }

    #[test]
    fn invalid_values_are_fixed_with_warnings() {
        let (s, w) = Settings::from_toml("replay_seconds = 250\nfps = 75\nhotkey = \"Q\"\n");
        assert_eq!(s.replay_seconds, 300);
        assert_eq!(s.fps, 60);
        assert_eq!(s.hotkey, DEFAULT_HOTKEY);
        assert_eq!(w.len(), 3);
    }

    #[test]
    fn broken_file_falls_back_to_defaults() {
        let (s, w) = Settings::from_toml("this is = = not toml");
        assert_eq!(s, Settings::default());
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn enum_spellings() {
        let (s, w) =
            Settings::from_toml("resolution = \"1080p\"\nquality = \"high\"\ncodec = \"h264\"\n");
        assert!(w.is_empty());
        assert_eq!(s.resolution.height(), Some(1080));
        assert_eq!(s.quality, Quality::High);
        assert_eq!(s.codec, CodecPref::H264);
    }

    #[test]
    fn mic_gain_is_validated_and_live() {
        let (s, w) = Settings::from_toml("microphone_gain_db = 40.0\n");
        assert_eq!(s.microphone_gain_db, 24.0);
        assert_eq!(w.len(), 1);
        let a = Settings::default();
        let b = Settings {
            microphone_gain_db: 6.0,
            ..a.clone()
        };
        assert!(
            !a.needs_restart(&b),
            "gain applies without restarting recording"
        );
    }

    #[test]
    fn restart_only_for_pipeline_changes() {
        let a = Settings::default();
        let b = Settings {
            replay_seconds: 60,
            hotkey: "F8".into(),
            save_sound: false,
            ..a.clone()
        };
        assert!(!a.needs_restart(&b));
        let c = Settings {
            microphone: true,
            ..a.clone()
        };
        assert!(a.needs_restart(&c));
    }

    #[test]
    fn labels() {
        assert_eq!(replay_label(30), "30 s");
        assert_eq!(replay_label(300), "5 min");
    }
}
