//! JondisClip window: clip library and settings (ADR-0008, ADR-0010). It's a separate process the tray
//! starts on demand and that exits on close, so it costs nothing while gaming.
//!
//! Talks to the tray through files only: it writes `settings.toml` (the tray
//! applies it within 2 s) and reads `status.json` (published by the tray).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipboard;
mod export;
mod library;
mod meter;
mod organize;

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
    let save = validated.save_dir_or(&paths::default_save_dir());
    if jondisclip_core::drive::overlaps(&save, &validated.drive_dir) {
        return Err(
            "The clips folder and the Google Drive folder can't overlap. Choose separate folders."
                .into(),
        );
    }
    if jondisclip_core::drive::overlaps(&validated.drive_dir, &paths::local_dir()) {
        return Err("The Google Drive folder can't be inside JondisClip's own folders.".into());
    }
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
        export::start(
            app,
            path,
            duration,
            size,
            start,
            end,
            export::Kind::Trim { exact },
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What "Share to Discord" will produce for `[start, end)` of a clip with the
/// current limit (ADR-0012), so the button can say so before anything runs.
#[derive(Serialize)]
struct SharePlan {
    limit_mb: u32,
    plan: Option<jondisclip_core::discord::Plan>,
    /// Longest selection that can fit, when this one can't.
    max_seconds: Option<f64>,
}

#[tauri::command]
async fn share_plan(id: String, start: f64, end: f64) -> Result<SharePlan, String> {
    // File IO (and a possibly sleeping disk) off the main thread.
    tauri::async_runtime::spawn_blocking(move || share_plan_blocking(id, start, end))
        .await
        .map_err(|e| e.to_string())?
}

fn share_plan_blocking(id: String, start: f64, end: f64) -> Result<SharePlan, String> {
    use jondisclip_core::discord::{PlanError, plan};
    let path = library::path_of(&id)?;
    let probe = jondisclip_core::library::probe_file(&path).map_err(|e| e.to_string())?;
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    let (settings, _) = read_settings();
    let limit = u64::from(settings.discord_limit_mb) * 1_000_000;
    // Video bitrate ≈ the file's average minus its audio tracks.
    let source_bps =
        (size as f64 * 8.0 / probe.duration.max(0.1)) - 160_000.0 * f64::from(probe.audio_tracks);
    // The window doesn't know the exact frame rate; recordings are 60 fps class.
    let r = plan(
        (end - start).max(0.0),
        probe.width,
        probe.height,
        60.0,
        Some(source_bps),
        limit,
    );
    Ok(SharePlan {
        limit_mb: settings.discord_limit_mb,
        max_seconds: match &r {
            Err(PlanError::TooLong { max_seconds }) => Some(*max_seconds),
            _ => None,
        },
        plan: r.ok(),
    })
}

/// Makes a Discord-sized copy of `[start, end)` and puts it on the clipboard.
#[tauri::command]
async fn share_start(
    app: tauri::AppHandle,
    id: String,
    start: f64,
    end: f64,
    title: Option<String>,
) -> Result<export::Started, String> {
    let path = library::path_of(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        let duration = jondisclip_core::library::probe_file(&path)
            .map_err(|e| e.to_string())?
            .duration;
        let (settings, _) = read_settings();
        let limit = u64::from(settings.discord_limit_mb) * 1_000_000;
        export::start(
            app,
            path,
            duration,
            size,
            start,
            end,
            export::Kind::Share { limit, title },
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Copies a finished share file to the clipboard again (only files in our
/// share folder).
#[tauri::command]
async fn share_copy_again(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let owner = export::main_hwnd(&app);
    tauri::async_runtime::spawn_blocking(move || share_copy_again_blocking(path, owner))
        .await
        .map_err(|e| e.to_string())?
}

fn share_copy_again_blocking(path: String, owner: Option<isize>) -> Result<(), String> {
    let p = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
    let dir = std::fs::canonicalize(export::share_dir()).map_err(|e| e.to_string())?;
    if !p.starts_with(&dir) {
        return Err("not a share file".into());
    }
    clipboard::copy_file(&p, owner)
}

/// Shows a share file in Explorer (only files in our share folder).
#[tauri::command]
fn share_reveal(path: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let p = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
    let dir = std::fs::canonicalize(export::share_dir()).map_err(|e| e.to_string())?;
    if !p.starts_with(&dir) {
        return Err("not a share file".into());
    }
    let plain = p.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{plain}\""))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct DriveStatus {
    /// The configured folder ("" = off).
    dir: String,
    /// It exists right now (Drive for desktop running and signed in).
    available: bool,
    /// Drive for desktop folders found on this PC.
    suggestions: Vec<String>,
}

#[tauri::command]
async fn drive_status() -> DriveStatus {
    tauri::async_runtime::spawn_blocking(|| {
        let (settings, _) = read_settings();
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
        // Leftovers from a copy cut short by closing the window (Drive would
        // upload them). Only one window runs, so none is in use now.
        if !export::busy() && !settings.drive_dir.as_os_str().is_empty() {
            for p in jondisclip_core::drive::stale_partials(&settings.drive_dir) {
                let _ = std::fs::remove_file(p);
            }
        }
        // Skip network drive letters: an offline one can stall for seconds.
        let local_dir_only = |p: &std::path::Path| !is_remote_drive(p) && p.is_dir();
        let suggestions = jondisclip_core::drive::suggestions(&local_dir_only, home.as_deref())
            .into_iter()
            .map(|p| p.display().to_string())
            .collect();
        DriveStatus {
            dir: settings.drive_dir.display().to_string(),
            available: !settings.drive_dir.as_os_str().is_empty() && settings.drive_dir.is_dir(),
            suggestions,
        }
    })
    .await
    .unwrap_or(DriveStatus {
        dir: String::new(),
        available: false,
        suggestions: Vec::new(),
    })
}

#[derive(Serialize)]
struct DriveCheck {
    dir: String,
    /// "off" (no folder set), "ok", or "problem" (see `message`).
    state: &'static str,
    message: Option<String>,
}

/// Checks the Drive folder (`dir`, or the configured one): it exists, isn't
/// inside the clips or JondisClip's folders, and a small test file can be
/// written, read back and removed. Says nothing about the cloud upload.
#[tauri::command]
async fn drive_check(dir: Option<String>) -> DriveCheck {
    tauri::async_runtime::spawn_blocking(move || {
        let (settings, _) = read_settings();
        let dir = dir.map(PathBuf::from).unwrap_or(settings.drive_dir.clone());
        let shown = dir.display().to_string();
        if dir.as_os_str().is_empty() {
            return DriveCheck {
                dir: shown,
                state: "off",
                message: None,
            };
        }
        let save = settings.save_dir_or(&paths::default_save_dir());
        let res =
            jondisclip_core::drive::check_folder(&dir, &save, &paths::local_dir(), &|p| p.is_dir())
                .and_then(|()| jondisclip_core::drive::write_test(&dir));
        match res {
            Ok(()) => DriveCheck {
                dir: shown,
                state: "ok",
                message: None,
            },
            Err(m) => DriveCheck {
                dir: shown,
                state: "problem",
                message: Some(m),
            },
        }
    })
    .await
    .unwrap_or(DriveCheck {
        dir: String::new(),
        state: "problem",
        message: Some("The check didn't finish.".into()),
    })
}

/// Opens the Drive folder (its `JondisClip` subfolder once it exists) in Explorer.
#[tauri::command]
async fn drive_open() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let (settings, _) = read_settings();
        let dir = settings.drive_dir;
        if dir.as_os_str().is_empty() || !dir.is_dir() {
            return Err("The Google Drive folder isn't available right now.".to_string());
        }
        let ours = dir.join("JondisClip");
        let open = if ours.is_dir() { ours } else { dir };
        std::process::Command::new("explorer")
            .arg(&open)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn is_remote_drive(p: &std::path::Path) -> bool {
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    let s = p.to_string_lossy();
    if s.len() < 2 || s.as_bytes()[1] != b':' {
        return false;
    }
    const DRIVE_REMOTE: u32 = 4;
    unsafe { GetDriveTypeW(&HSTRING::from(format!("{}\\", &s[..2]))) == DRIVE_REMOTE }
}

/// Size and modified time, for "already there".
fn stamp(p: &std::path::Path) -> Option<jondisclip_core::drive::Stamp> {
    std::fs::metadata(p)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

#[derive(Serialize)]
struct DriveStarted {
    /// The running copy/export, or None when nothing had to be copied.
    job: Option<u32>,
    output: String,
    /// The same file was already in the Drive folder.
    already: bool,
}

/// Save to Drive (ADR-0013): copies the whole clip, or writes the selection
/// as a trim, into `<drive>\JondisClip\<Game>\`. Drive does the upload.
#[tauri::command]
async fn drive_save(
    app: tauri::AppHandle,
    id: String,
    start: f64,
    end: f64,
    whole: bool,
    exact: bool,
) -> Result<DriveStarted, String> {
    use jondisclip_core::drive::{Target, check_folder, target};
    let path = library::path_of(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (settings, _) = read_settings();
        let save = settings.save_dir_or(&paths::default_save_dir());
        check_folder(&settings.drive_dir, &save, &paths::local_dir(), &|p| {
            p.is_dir()
        })?;
        if !export::busy() {
            for p in jondisclip_core::drive::stale_partials(&settings.drive_dir) {
                let _ = std::fs::remove_file(p);
            }
        }
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Drive keeps the clip's folder: its user folder if it's in one
        // (the directory it's in), else its game.
        let parent = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        let org = jondisclip_core::organize::Store::new(
            paths::local_dir().join("library").join("organize.json"),
        )
        .load()
        .unwrap_or_default();
        let keep = settings.drive_keep_folders;
        let game = parent
            .clone()
            .filter(|d| keep && org.folder(d).is_some() && !org.is_game_dir(d))
            .or_else(|| jondisclip_core::library::parse_name(&file_name).map(|p| p.0))
            .or(parent)
            .unwrap_or_else(|| "Clips".into());
        if whole {
            return match target(&settings.drive_dir, &game, &file_name, stamp(&path), &stamp) {
                Target::AlreadyThere(p) => Ok(DriveStarted {
                    job: None,
                    output: p.display().to_string(),
                    already: true,
                }),
                Target::Write(p) => export::start_copy(app, path, p).map(|s| DriveStarted {
                    job: Some(s.job),
                    output: s.output,
                    already: false,
                }),
            };
        }
        let stem = std::path::Path::new(&file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = format!("{} edit.mp4", jondisclip_core::library::split_edit(&stem).0);
        let Target::Write(out) = target(&settings.drive_dir, &game, &name, None, &stamp) else {
            return Err("couldn't choose a file name in the Drive folder".into());
        };
        std::fs::create_dir_all(out.parent().unwrap_or(&out)).map_err(|e| e.to_string())?;
        let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        let duration = jondisclip_core::library::probe_file(&path)
            .map_err(|e| e.to_string())?
            .duration;
        export::start(
            app,
            path,
            duration,
            size,
            start,
            end,
            export::Kind::TrimTo { exact, output: out },
        )
        .map(|s| DriveStarted {
            job: Some(s.job),
            output: s.output,
            already: false,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Shows a file in Explorer, only inside the configured Drive folder.
#[tauri::command]
async fn drive_reveal(path: String) -> Result<(), String> {
    // canonicalize on a hung network/virtual drive can block: off the main thread.
    tauri::async_runtime::spawn_blocking(move || drive_reveal_blocking(path))
        .await
        .map_err(|e| e.to_string())?
}

/// `\\?\C:\x` → `C:\x`, `\\?\UNC\srv\share\x` → `\\srv\share\x`.
fn plain_path(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        s.trim_start_matches(r"\\?\").to_string()
    }
}

fn drive_reveal_blocking(path: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let (settings, _) = read_settings();
    let p = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
    let dir = std::fs::canonicalize(&settings.drive_dir).map_err(|e| e.to_string())?;
    if !p.starts_with(&dir) {
        return Err("not in the Drive folder".into());
    }
    let plain = plain_path(&p);
    std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{plain}\""))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
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
            organize::library_load,
            thumb_save,
            show_in_folder,
            open_external,
            clip_keyframes,
            export_start,
            export_cancel,
            share_plan,
            share_start,
            share_copy_again,
            share_reveal,
            drive_status,
            drive_save,
            drive_reveal,
            drive_check,
            drive_open,
            organize::folder_create,
            organize::folder_rename,
            organize::folder_color,
            organize::folder_delete,
            organize::folders_reorder,
            organize::clips_move,
            organize::clips_arrange,
            organize::pending_resolve,
            organize::clip_title,
            organize::clips_delete,
            organize::clip_copy_file
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the JondisClip window");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_for_explorer() {
        assert_eq!(
            plain_path(std::path::Path::new(r"\\?\C:\a\b.mp4")),
            r"C:\a\b.mp4"
        );
        assert_eq!(
            plain_path(std::path::Path::new(r"\\?\UNC\srv\share\b.mp4")),
            r"\\srv\share\b.mp4"
        );
        assert_eq!(
            plain_path(std::path::Path::new(r"G:\My Drive\x.mp4")),
            r"G:\My Drive\x.mp4"
        );
    }
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
