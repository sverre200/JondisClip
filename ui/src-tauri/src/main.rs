//! JondisClip settings window (ADR-0008). It's a separate process the tray
//! starts on demand and that exits on close, so it costs nothing while gaming.
//!
//! Talks to the tray through files only: it writes `settings.toml` (the tray
//! applies it within 2 s) and reads `status.json` (published by the tray).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use jondisclip_core::hotkey::Hotkey;
use jondisclip_core::paths;
use jondisclip_core::settings::{FPS_PRESETS, REPLAY_PRESETS, Settings};
use jondisclip_core::status::TrayStatus;
use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
};
use windows::core::{HSTRING, w};

const WINDOW_TITLE: &str = "JondisClip";

#[derive(Serialize)]
struct Snapshot {
    settings: Settings,
    default_save_dir: String,
    replay_presets: Vec<u32>,
    fps_presets: Vec<u32>,
    /// Problems found in the settings file (fixed to defaults).
    warnings: Vec<String>,
    tray: TrayView,
}

#[derive(Serialize)]
struct TrayView {
    running: bool,
    status: Option<TrayStatus>,
}

fn tray_running() -> bool {
    match unsafe {
        OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE,
            false,
            &HSTRING::from(paths::TRAY_MUTEX),
        )
    } {
        Ok(h) => {
            unsafe {
                let _ = CloseHandle(h);
            }
            true
        }
        Err(_) => false,
    }
}

fn tray_view() -> TrayView {
    let running = tray_running();
    TrayView {
        running,
        status: if running {
            TrayStatus::read(&paths::status_file())
        } else {
            None
        },
    }
}

fn read_settings() -> (Settings, Vec<String>) {
    match std::fs::read_to_string(paths::settings_file()) {
        Ok(text) => Settings::from_toml(&text),
        Err(_) => (Settings::default(), Vec::new()),
    }
}

#[tauri::command]
fn load() -> Snapshot {
    let (settings, warnings) = read_settings();
    Snapshot {
        settings,
        default_save_dir: paths::default_save_dir().display().to_string(),
        replay_presets: REPLAY_PRESETS.to_vec(),
        fps_presets: FPS_PRESETS.to_vec(),
        warnings,
        tray: tray_view(),
    }
}

#[tauri::command]
fn tray_status() -> TrayView {
    tray_view()
}

/// Applies `patch` (a subset of setting fields) on top of the settings file
/// as it is now, so changes made from the tray menu meanwhile are kept.
/// It refuses values that wouldn't survive validation.
#[tauri::command]
fn update(patch: serde_json::Map<String, serde_json::Value>) -> Result<Settings, String> {
    let (current, _) = read_settings();
    let validated = apply_patch(&current, patch)?;
    let path = paths::settings_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, validated.to_toml())
        .map_err(|e| format!("couldn't save settings: {e}"))?;
    Ok(validated)
}

/// Merges `patch` into `current` and validates the result exactly like the tray
/// does when it loads the file. Unknown keys, wrong types and values the tray
/// would reset are refused.
fn apply_patch(
    current: &Settings,
    patch: serde_json::Map<String, serde_json::Value>,
) -> Result<Settings, String> {
    let mut value = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let obj = value.as_object_mut().ok_or("settings are not an object")?;
    for (k, v) in patch {
        if !obj.contains_key(&k) {
            return Err(format!("unknown setting \"{k}\""));
        }
        obj.insert(k, v);
    }
    let next: Settings =
        serde_json::from_value(value).map_err(|e| format!("invalid value: {e}"))?;
    let (validated, warnings) = Settings::from_toml(&next.to_toml());
    match warnings.into_iter().next() {
        Some(w) => Err(w),
        None => Ok(validated),
    }
}

/// Active microphones (Windows default first), for the device picker.
#[tauri::command]
fn list_microphones() -> Result<Vec<jondisclip_core::devices::InputDevice>, String> {
    jondisclip_core::devices::list_inputs().map_err(|e| e.to_string())
}

/// Canonical form of a hotkey ("shift+alt+f10" → "Alt+Shift+F10"), or why it's invalid.
#[tauri::command]
fn check_hotkey(text: String) -> Result<String, String> {
    Hotkey::parse(&text)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    let p = PathBuf::from(path);
    std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
    std::process::Command::new("explorer")
        .arg(&p)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Starts the tray app (jondisclip.exe next to this exe) if it isn't running.
#[tauri::command]
fn start_tray() -> Result<(), String> {
    if tray_running() {
        return Ok(());
    }
    let exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("jondisclip.exe")))
        .filter(|p| p.exists())
        .ok_or("jondisclip.exe wasn't found next to the settings app")?;
    std::process::Command::new(exe)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn main() {
    // One settings window: a second launch focuses the existing one.
    let mutex = unsafe { CreateMutexW(None, true, w!("Local\\JondisClip.Settings")) };
    if mutex.is_err() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if let Ok(h) = unsafe { FindWindowW(None, &HSTRING::from(WINDOW_TITLE)) } {
            unsafe {
                let _ = ShowWindow(h, SW_SHOW);
                let _ = ShowWindow(h, SW_RESTORE);
                let _ = SetForegroundWindow(h);
            }
        }
        return;
    }
    let _mutex = mutex;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Safety net: the page reveals the window when it has rendered. If
            // that never happens (script error, stalled load), show it anyway
            // so the user never ends up with an invisible running instance.
            if let Some(w) = tauri::Manager::get_webview_window(app, "main") {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    if !w.is_visible().unwrap_or(true) {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_microphones,
            load,
            tray_status,
            update,
            check_hotkey,
            open_folder,
            start_tray
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the JondisClip window");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn patch(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn applies_only_the_patched_fields() {
        let current = Settings {
            replay_seconds: 60,
            microphone: true,
            ..Settings::default()
        };
        let next = apply_patch(&current, patch(json!({ "quality": "high" }))).unwrap();
        assert_eq!(next.replay_seconds, 60, "fields not in the patch are kept");
        assert!(next.microphone);
        assert_eq!(serde_json::to_value(next.quality).unwrap(), json!("high"));
    }

    #[test]
    fn refuses_unknown_keys_bad_types_and_invalid_values() {
        let s = Settings::default();
        assert!(
            apply_patch(&s, patch(json!({ "nope": 1 })))
                .unwrap_err()
                .contains("unknown")
        );
        assert!(
            apply_patch(&s, patch(json!({ "fps": "sixty" })))
                .unwrap_err()
                .contains("invalid")
        );
        assert!(apply_patch(&s, patch(json!({ "replay_seconds": 250 }))).is_err());
        assert!(apply_patch(&s, patch(json!({ "hotkey": "Q" }))).is_err());
        assert!(apply_patch(&s, patch(json!({ "quality": "ultra" }))).is_err());
    }

    #[test]
    fn hotkeys_are_canonicalised() {
        assert_eq!(
            check_hotkey("shift+alt+f10".into()).unwrap(),
            "Alt+Shift+F10"
        );
        assert!(check_hotkey("Q".into()).is_err());
    }
}
