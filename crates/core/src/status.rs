//! Tray status published to `status.json` so the settings window can show
//! live state (recording, hotkey conflicts) without an IPC channel.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrayStatus {
    /// "recording", "paused", "sleeping", "low_disk" or "restarting".
    pub state: String,
    /// Human summary, e.g. "Buffering last 5 min · H.264 1920×1080 60 fps".
    pub summary: String,
    pub hotkey: String,
    pub hotkey_ok: bool,
    pub encoder: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<u32>,
    pub bitrate: Option<u64>,
    pub audio_tracks: Vec<String>,
    /// Set when audio failed to start.
    pub audio_error: Option<String>,
    /// Some(false) while the chosen microphone isn't connected (None = mic off).
    pub mic_connected: Option<bool>,
    /// The running tray's version, e.g. "0.1.1".
    pub version: String,
    /// A newer version the tray found on GitHub, if any.
    pub update_version: Option<String>,
    /// "downloading" or "installing" while an update is under way.
    pub update_state: Option<String>,
}

impl TrayStatus {
    /// Writes atomically (temp file + rename) so readers never see half a file.
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
        std::fs::rename(tmp, path)
    }

    pub fn read(path: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_file() {
        let p = std::env::temp_dir().join(format!("jc-status-test-{}.json", std::process::id()));
        let s = TrayStatus {
            state: "recording".into(),
            hotkey: "Alt+F10".into(),
            hotkey_ok: true,
            width: Some(1920),
            ..Default::default()
        };
        s.write(&p).unwrap();
        assert_eq!(TrayStatus::read(&p), Some(s));
        std::fs::remove_file(p).unwrap();
    }
}
