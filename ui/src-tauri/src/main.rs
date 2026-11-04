//! JondisClip window: clip library and settings (ADR-0008, ADR-0010). It's a separate process the tray
//! starts on demand and that exits on close, so it costs nothing while gaming.
//!
//! Talks to the tray through files only: it writes `settings.toml` (the tray
//! applies it within 2 s) and reads `status.json` (published by the tray).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod export;
mod library;
mod meter;

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

/// Microphone level since the last call (starts the meter on first use; it
/// stops by itself shortly after the window stops asking).
#[tauri::command]
fn mic_level(device: String, gain_db: f32) -> meter::Level {
    meter::read(&device, gain_db)
}

#[tauri::command]
fn mic_meter_stop() {
    meter::stop();
}

#[derive(Serialize)]
struct MicCheck {
    report: jondisclip_core::mic_check::MicReport,
    /// Processed mic audio (what the Microphone track would contain) as a WAV
    /// data URL for listening back. Kept in memory only, never written to disk.
    audio: String,
}

/// Records `seconds` of the mic, analyses it with the recorder's processing,
/// saves the statistics (no audio) to `mic-check.json`, and returns them plus
/// the processed audio for listening back.
#[tauri::command]
async fn mic_check(
    device: String,
    gain_db: f32,
    low_cut: bool,
    seconds: u32,
) -> Result<MicCheck, String> {
    let seconds = seconds.clamp(3, 20);
    tauri::async_runtime::spawn_blocking(move || {
        let raw = meter::record(&device, seconds)?;
        let (report, processed) = jondisclip_core::mic_check::analyze(&raw, gain_db, low_cut);
        let path = paths::local_dir().join("mic-check.json");
        let _ = std::fs::write(
            &path,
            serde_json::to_vec_pretty(&report).unwrap_or_default(),
        );
        let wav = jondisclip_core::mic_check::wav_bytes(&processed);
        Ok(MicCheck {
            report,
            audio: format!(
                "data:audio/wav;base64,{}",
                jondisclip_core::mic_check::base64(&wav)
            ),
        })
    })
    .await
    .map_err(|e| e.to_string())?
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

/// Page to open first: `--settings` from the tray's "Settings…", else the library.
#[tauri::command]
fn initial_page() -> &'static str {
    if std::env::args().any(|a| a == "--settings") {
        "settings"
    } else {
        "library"
    }
}

/// Lists the clips in the save folder (reads headers only for new or changed files).
#[tauri::command]
async fn library_scan() -> Result<library::LibraryView, String> {
    let (settings, _) = read_settings();
    let root = settings.save_dir_or(&paths::default_save_dir());
    tauri::async_runtime::spawn_blocking(move || {
        library::in_background(|| {
            // Left over if the window closed mid-export (the exporter dies
            // with it). Only one window runs, so none of these is in use.
            if !export::busy() {
                for p in jondisclip_core::library::stale_edit_partials(&root) {
                    let _ = std::fs::remove_file(p);
                }
            }
            library::scan(root)
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Stores a thumbnail the page drew. Body: JPEG bytes; header `x-clip-id`.
#[tauri::command]
fn thumb_save(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let id = request
        .headers()
        .get("x-clip-id")
        .and_then(|v| v.to_str().ok())
        .ok_or("missing clip id")?;
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => {
            library::in_background(|| library::save_thumb(id, bytes))
        }
        _ => Err("expected raw JPEG bytes".into()),
    }
}

/// Keyframe times (s) of a clip, so the editor knows when a cut is lossless.
#[tauri::command]
async fn clip_keyframes(id: String) -> Result<Vec<f64>, String> {
    let path = library::path_of(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        jondisclip_core::library::keyframes_file(&path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Saves `[start, end)` of a clip as a new file next to it (ADR-0011).
#[tauri::command]
async fn export_start(
    app: tauri::AppHandle,
    id: String,
    start: f64,
    end: f64,
    exact: bool,
) -> Result<export::Started, String> {
    let path = library::path_of(&id)?;
    // File IO off the main thread.
    tauri::async_runtime::spawn_blocking(move || {
        let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        let duration = jondisclip_core::library::probe_file(&path)
            .map_err(|e| e.to_string())?
            .duration;
        export::start(app, path, duration, size, start, end, exact)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn export_cancel() {
    export::cancel();
}

/// Opens Explorer with the clip selected.
#[tauri::command]
fn show_in_folder(id: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let path = library::path_of(&id)?;
    std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Opens the clip in the default video player.
#[tauri::command]
fn open_external(id: String) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let path = library::path_of(&id)?;
    let r = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(path.as_os_str()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns a value > 32 on success.
    if r.0 as usize > 32 {
        Ok(())
    } else {
        Err("Windows couldn't open the clip (no video player set?)".into())
    }
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
        .register_asynchronous_uri_scheme_protocol("clip", |_ctx, request, responder| {
            // File reads happen off the webview thread, on a small fixed pool.
            library::enqueue(request, responder);
        })
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
            mic_level,
            mic_meter_stop,
            mic_check,
            load,
            tray_status,
            update,
            check_hotkey,
            open_folder,
            start_tray,
            initial_page,
            library_scan,
            thumb_save,
            show_in_folder,
            open_external,
            clip_keyframes,
            export_start,
            export_cancel
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
