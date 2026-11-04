//! Minimal local log file: `%LOCALAPPDATA%\JondisClip\jondisclip.log`.
//! It rotates to `.old` at 1 MB. It stays on this PC and is never uploaded
//! (no telemetry). It's written only from the tray thread and save waiters,
//! never from the capture path.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
const MAX_BYTES: u64 = 1_048_576;

pub fn init(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    if let Ok(mut p) = LOG_PATH.lock() {
        *p = Some(dir.join("jondisclip.log"));
    }
}

pub fn write(level: &str, msg: &str) {
    let Ok(guard) = LOG_PATH.lock() else { return };
    let Some(path) = guard.as_ref() else { return };
    if fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::rename(path, path.with_extension("log.old"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {level:5} {msg}", crate::naming::timestamp());
    }
    #[cfg(debug_assertions)]
    eprintln!("{level:5} {msg}");
}

macro_rules! info {
    ($($t:tt)*) => { $crate::log::write("INFO", &format!($($t)*)) };
}
macro_rules! warning {
    ($($t:tt)*) => { $crate::log::write("WARN", &format!($($t)*)) };
}
macro_rules! error {
    ($($t:tt)*) => { $crate::log::write("ERROR", &format!($($t)*)) };
}
pub(crate) use {error, info, warning};
