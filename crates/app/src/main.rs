//! JondisClip: the always-running tray app. Release builds have no console
//! window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod config;
mod icon;
mod log;
mod menu_theme;
mod naming;
mod toast;
mod tray;

use jondisclip_core::paths;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
use windows::core::w;

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    // Single instance: a second launch pokes the first one and exits. After an
    // update the old version starts us and then quits, so wait for it to go.
    let mut tries = if std::env::args().any(|a| a == "--updated") {
        60
    } else {
        1
    };
    let mutex = loop {
        let mutex = unsafe { CreateMutexW(None, true, w!("Local\\JondisClip.SingleInstance")) };
        if mutex.is_ok() && unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
            break mutex;
        }
        if let Ok(h) = mutex {
            unsafe {
                let _ = CloseHandle(h);
            }
        }
        tries -= 1;
        if tries == 0 {
            if let Ok(h) = unsafe { FindWindowW(tray::WINDOW_CLASS, None) } {
                unsafe {
                    let _ =
                        PostMessageW(Some(h), tray::WM_APP_SECOND_INSTANCE, WPARAM(0), LPARAM(0));
                }
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    // The handle stays open for the life of the process.
    let _mutex = mutex;

    log::init(paths::local_dir());
    // Remove what an earlier update left behind (`*.old` files, the download).
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.to_path_buf()))
    {
        jondisclip_core::update::cleanup(&dir, &paths::update_dir());
    }
    log::info!(
        "JondisClip {} starting (args: {:?})",
        env!("CARGO_PKG_VERSION"),
        std::env::args().skip(1).collect::<Vec<_>>()
    );
    if let Err(e) = tray::run(paths::settings_file(), paths::default_save_dir()) {
        log::error!("fatal: {e}");
    }
}
