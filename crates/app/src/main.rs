//! JondisClip: the always-running tray app. Release builds have no console
//! window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod hotkey;
mod icon;
mod log;
mod naming;
mod settings;
mod tray;

use std::path::PathBuf;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, LPARAM, WPARAM};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::{FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
use windows::core::w;

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn videos_dir() -> PathBuf {
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

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    // Single instance: a second launch pokes the first one and exits.
    let mutex = unsafe { CreateMutexW(None, true, w!("Local\\JondisClip.SingleInstance")) };
    if mutex.is_err() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if let Ok(h) = unsafe { FindWindowW(tray::WINDOW_CLASS, None) } {
            unsafe {
                let _ = PostMessageW(Some(h), tray::WM_APP_SECOND_INSTANCE, WPARAM(0), LPARAM(0));
            }
        }
        return;
    }
    // The handle stays open for the life of the process.
    let _mutex = mutex;

    log::init(env_dir("LOCALAPPDATA").join("JondisClip"));
    log::info!(
        "JondisClip {} starting (args: {:?})",
        env!("CARGO_PKG_VERSION"),
        std::env::args().skip(1).collect::<Vec<_>>()
    );
    let settings_path = env_dir("APPDATA").join("JondisClip").join("settings.toml");
    let save_dir = videos_dir().join("JondisClip");
    if let Err(e) = tray::run(settings_path, save_dir) {
        log::error!("fatal: {e}");
    }
}
