//! Clip file naming: `<save dir>\<App>\<App> 2026-11-04 21-03-11.mp4`.
//!
//! "App" is the foreground program when the hotkey is pressed (usually the
//! game), named from its executable, with a few friendly names.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::PWSTR;

/// Known executables: (exe stem, lowercase) → display name.
const KNOWN: &[(&str, &str)] = &[
    ("dayz_x64", "DayZ"),
    ("dayz_be", "DayZ"),
    ("dayz", "DayZ"),
    ("cs2", "Counter-Strike 2"),
    ("valorant-win64-shipping", "VALORANT"),
    ("fortniteclient-win64-shipping", "Fortnite"),
    ("r5apex", "Apex Legends"),
    ("rocketleague", "Rocket League"),
    ("eldenring", "Elden Ring"),
    ("explorer", "Desktop"),
    ("jondisclip", "Desktop"),
    ("chrome", "Chrome"),
    ("msedge", "Edge"),
    ("firefox", "Firefox"),
    ("discord", "Discord"),
    ("steam", "Steam"),
    ("steamwebhelper", "Steam"),
    ("code", "VS Code"),
    ("claude", "Claude"),
    ("obs64", "OBS"),
];

/// Friendly name for an executable stem ("DayZ_x64" → "DayZ").
pub fn app_name_from_exe(stem: &str) -> String {
    let lower = stem.to_ascii_lowercase();
    if let Some((_, name)) = KNOWN.iter().find(|(k, _)| *k == lower) {
        return (*name).to_string();
    }
    // Strip common engine and arch suffixes.
    let mut name = stem.to_string();
    for suffix in [
        "-Win64-Shipping",
        "_x64",
        "-x64",
        "_x86",
        "64",
        "_DX12",
        "_DX11",
        "_BE",
    ] {
        if name.len() > suffix.len()
            && name
                .to_ascii_lowercase()
                .ends_with(&suffix.to_ascii_lowercase())
        {
            name.truncate(name.len() - suffix.len());
            break;
        }
    }
    // Executables are often lowercase ("notepad"): capitalise for folder names.
    let mut chars = name.chars();
    let name = match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        None => name,
    };
    sanitize(&name)
}

/// Makes a string safe as a Windows file or folder name.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut s = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    s = s.trim_matches(|c| c == '.' || c == ' ').to_string();
    if s.chars().count() > 60 {
        s = s
            .chars()
            .take(60)
            .collect::<String>()
            .trim_end()
            .to_string();
    }
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "LPT1", "LPT2",
    ];
    if s.is_empty() || reserved.iter().any(|r| r.eq_ignore_ascii_case(&s)) {
        return "Clip".into();
    }
    s
}

/// Local time for a clip file name, in the date format setting.
pub fn file_timestamp(date_format: &str) -> String {
    let t = unsafe { GetLocalTime() };
    jondisclip_core::library::file_stamp(
        date_format,
        t.wYear,
        t.wMonth,
        t.wDay,
        t.wHour,
        t.wMinute,
        t.wSecond,
    )
}

/// Local time as "2026-11-04 21-03-11" (logs).
pub fn timestamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}-{:02}-{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// `<dir>\<app>\<app> <stamp>.mp4`, with " (2)" etc. if that name exists.
pub fn clip_path(save_dir: &Path, app: &str, stamp: &str) -> PathBuf {
    let app = sanitize(app);
    let folder = save_dir.join(&app);
    let mut path = folder.join(format!("{app} {stamp}.mp4"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{app} {stamp} ({n}).mp4"));
        n += 1;
    }
    path
}

/// Name of the program that owns the foreground window, if any.
pub fn foreground_app() -> Option<String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; MAX_PATH as usize * 2];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        ok.ok()?;
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        let stem = Path::new(&full).file_stem()?.to_string_lossy().into_owned();
        Some(app_name_from_exe(&stem))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_derived_names() {
        assert_eq!(app_name_from_exe("DayZ_x64"), "DayZ");
        assert_eq!(app_name_from_exe("explorer"), "Desktop");
        assert_eq!(app_name_from_exe("MyGame-Win64-Shipping"), "MyGame");
        assert_eq!(app_name_from_exe("Witcher3_x64"), "Witcher3");
        assert_eq!(app_name_from_exe("notepad"), "Notepad");
        assert_eq!(app_name_from_exe("chrome"), "Chrome");
    }

    #[test]
    fn sanitizes_file_names() {
        assert_eq!(sanitize("a<b>c:d"), "a b c d");
        assert_eq!(sanitize("  ..hidden.. "), "hidden");
        assert_eq!(sanitize(""), "Clip");
        assert_eq!(sanitize("CON"), "Clip");
        assert_eq!(sanitize(&"x".repeat(100)).len(), 60);
    }

    #[test]
    fn clip_path_is_unique() {
        let dir = std::env::temp_dir().join(format!("jc-naming-test-{}", std::process::id()));
        let p1 = clip_path(&dir, "DayZ", "2026-11-04 21-03-11");
        assert!(p1.ends_with("DayZ/DayZ 2026-11-04 21-03-11.mp4"));
        std::fs::create_dir_all(p1.parent().unwrap()).unwrap();
        std::fs::write(&p1, b"x").unwrap();
        let p2 = clip_path(&dir, "DayZ", "2026-11-04 21-03-11");
        assert!(p2.ends_with("DayZ/DayZ 2026-11-04 21-03-11 (2).mp4"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
