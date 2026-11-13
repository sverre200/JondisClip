//! Lets the tray's right-click menu follow Windows' dark mode (ADR-0023).
//!
//! A Win32 popup menu is light unless the process opts in. The opt-in is
//! two `uxtheme.dll` exports without names (ordinals 135 and 136), the same
//! ones File Explorer uses. Microsoft hasn't documented them, but they've been
//! stable since Windows 10 1809 and are what other tray apps use. On any
//! other build, or if a lookup fails, nothing is called and the menu stays light.

use windows::Win32::System::LibraryLoader::{
    GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::core::{PCSTR, s, w};

/// First build with dark menus (Windows 10 1809).
const MIN_BUILD: u32 = 17763;
/// `SetPreferredAppMode` (1903+; `AllowDarkModeForApp(bool)` on 1809, where 1 means the same).
const ORD_APP_MODE: usize = 135;
/// `FlushMenuThemes`.
const ORD_FLUSH_MENUS: usize = 136;
/// Follow the system setting: dark menus when Windows is in dark mode.
const ALLOW_DARK: i32 = 1;

fn windows_build() -> Option<u32> {
    // `RtlGetVersion` reports the real version without an app manifest.
    type RtlGetVersion = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;
    unsafe {
        let ntdll = GetModuleHandleW(w!("ntdll.dll")).ok()?;
        let f = GetProcAddress(ntdll, s!("RtlGetVersion"))?;
        // SAFETY: `RtlGetVersion` has this signature on every Windows version.
        let f: RtlGetVersion = std::mem::transmute(f);
        let mut v = OSVERSIONINFOW {
            dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        (f(&mut v) == 0).then_some(v.dwBuildNumber)
    }
}

/// Call once at start, before the first menu is shown. Returns whether the
/// opt-in was made.
pub fn allow_dark_menus() -> bool {
    if windows_build().is_none_or(|b| b < MIN_BUILD) {
        return false;
    }
    unsafe {
        let Ok(lib) = LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) else {
            return false;
        };
        let (Some(set), Some(flush)) = (
            GetProcAddress(lib, PCSTR(ORD_APP_MODE as *const u8)),
            GetProcAddress(lib, PCSTR(ORD_FLUSH_MENUS as *const u8)),
        ) else {
            return false;
        };
        // SAFETY: these are the signatures of the two exports on the builds
        // let through above (one integer in and out; no arguments).
        let set: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set);
        let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
        set(ALLOW_DARK);
        flush();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_real_windows_build() {
        // The dev machine and CI are Windows 10 or 11.
        assert!(windows_build().is_some_and(|b| b >= 10240));
    }
}
