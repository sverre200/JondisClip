//! "Start with Windows": a value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. It's per user, needs
//! no admin rights, and is removed again when the setting is turned off.

use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
    RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "JondisClip";

pub fn apply(enabled: bool) -> windows::core::Result<()> {
    let mut key = HKEY::default();
    unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            Some(0),
            KEY_SET_VALUE,
            &mut key,
        )
        .ok()?;
    }
    let result = if enabled {
        let exe = std::env::current_exe()
            .map_err(|_| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
        let cmd = format!("\"{}\" --background", exe.display());
        let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: REG_SZ data is the UTF-16 string including its terminator.
        let bytes =
            unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
        unsafe { RegSetValueExW(key, &HSTRING::from(VALUE), Some(0), REG_SZ, Some(bytes)).ok() }
    } else {
        let r = unsafe { RegDeleteValueW(key, PCWSTR(HSTRING::from(VALUE).as_ptr())) };
        if r == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            r.ok()
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}
