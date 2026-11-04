//! Well-known locations. All per-user, no admin rights needed.

use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// `%APPDATA%\JondisClip\settings.toml`
pub fn settings_file() -> PathBuf {
    env_dir("APPDATA").join("JondisClip").join("settings.toml")
}

/// `%LOCALAPPDATA%\JondisClip` (log, status, buffer).
pub fn local_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join("JondisClip")
}

/// `%LOCALAPPDATA%\JondisClip\status.json`, written by the tray app.
pub fn status_file() -> PathBuf {
    local_dir().join("status.json")
}

/// The user's Videos folder (respects relocation via Known Folders).
pub fn videos_dir() -> PathBuf {
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Videos, KF_FLAG_DEFAULT, None) {
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            if let Some(s) = s {
                return PathBuf::from(s);
            }
        }
    }
    env_dir("USERPROFILE").join("Videos")
}

/// Default clip folder: `Videos\JondisClip`.
pub fn default_save_dir() -> PathBuf {
    videos_dir().join("JondisClip")
}

/// Name of the mutex the running tray app holds.
pub const TRAY_MUTEX: &str = "Local\\JondisClip.SingleInstance";
