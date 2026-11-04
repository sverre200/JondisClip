//! `jondisclip-export trim <input> <output> <start> <end> <copy|exact>`
//! `jondisclip-export share <input> <output> <start> <end> <limit-bytes>` (ADR-0012)
//!
//! Started by the library window (ADR-0011). Prints one JSON object per line
//! on stdout: `{"progress":0.42}` while working, then `{"done":{…}}` or
//! `{"error":"…"}`. Exit code 0 on success. Killing it is the cancel path;
//! the caller removes `<output>.partial`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use jondisclip_export::{Mode, Share, Trim, share, trim};

fn emit(v: serde_json::Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

enum Job {
    Trim(Trim),
    Share(Share),
}

const USAGE: &str = "usage: jondisclip-export trim <input> <output> <start> <end> <copy|exact>
       jondisclip-export share <input> <output> <start> <end> <limit-bytes>";

fn parse(args: &[String]) -> Result<Job, String> {
    let [cmd, input, output, start, end, last] = args else {
        return Err(USAGE.into());
    };
    let num = |s: &str| s.parse::<f64>().map_err(|_| format!("not a number: {s}"));
    let (input, output, start, end) = (
        PathBuf::from(input),
        PathBuf::from(output),
        num(start)?,
        num(end)?,
    );
    match cmd.as_str() {
        "trim" => Ok(Job::Trim(Trim {
            input,
            output,
            start,
            end,
            mode: match last.as_str() {
                "copy" => Mode::Copy,
                "exact" => Mode::Exact,
                m => return Err(format!("unknown mode {m}")),
            },
        })),
        "share" => Ok(Job::Share(Share {
            input,
            output,
            start,
            end,
            limit_bytes: last.parse().map_err(|_| format!("not a size: {last}"))?,
        })),
        c => Err(format!("unknown command {c}")),
    }
}

/// Priority follows the user's attention. While the library window is in
/// front, the export runs at below-normal priority (set by the window) at full
/// speed. As soon as something else is in front (typically the game), the
/// process switches to background mode: lowest CPU, IO and memory priority,
/// so the game and the recorder always win. Measured: background mode makes
/// an exact 20 s cut ~3× slower (5.3 → 16–17 s), hence not always on.
struct Attention {
    ui_pid: Option<u32>,
    background: bool,
    checked: Instant,
}

impl Attention {
    fn new() -> Self {
        // Set by the window; a manual CLI run without it stays in background mode.
        let ui_pid = std::env::var("JC_UI_PID").ok().and_then(|v| v.parse().ok());
        let mut a = Self {
            ui_pid,
            background: false,
            checked: Instant::now(),
        };
        a.apply(ui_pid.is_none() || !a.ui_in_front());
        a
    }

    fn ui_in_front(&self) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        let Some(ui) = self.ui_pid else { return false };
        let mut pid = 0u32;
        unsafe {
            let h = GetForegroundWindow();
            if h.0.is_null() {
                return false;
            }
            GetWindowThreadProcessId(h, Some(&mut pid));
        }
        pid == ui
    }

    fn apply(&mut self, background: bool) {
        use windows::Win32::System::Threading::{
            GetCurrentProcess, PROCESS_MODE_BACKGROUND_BEGIN, PROCESS_MODE_BACKGROUND_END,
            SetPriorityClass,
        };
        if background == self.background {
            return;
        }
        let mode = if background {
            PROCESS_MODE_BACKGROUND_BEGIN
        } else {
            PROCESS_MODE_BACKGROUND_END
        };
        if unsafe { SetPriorityClass(GetCurrentProcess(), mode) }.is_ok() {
            self.background = background;
        }
        // In the background (the game is in front) encode no faster than
        // real time; with the window in front, 4×.
        jondisclip_export::set_max_speed(if background { 1.0 } else { 4.0 });
    }

    /// Re-checks at most twice a second.
    fn tick(&mut self) {
        if self.ui_pid.is_none() || self.checked.elapsed().as_millis() < 500 {
            return;
        }
        self.checked = Instant::now();
        let front = self.ui_in_front();
        self.apply(!front);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let job = match parse(&args) {
        Ok(j) => j,
        Err(e) => {
            emit(serde_json::json!({ "error": e }));
            std::process::exit(2);
        }
    };
    let mut last = Instant::now();
    let started = Instant::now();
    let mut attention = Attention::new();
    let mut report = |p: f64| {
        attention.tick();
        // At most ~10 updates a second.
        if last.elapsed().as_millis() >= 100 || p >= 1.0 {
            last = Instant::now();
            emit(serde_json::json!({ "progress": (p * 1000.0).round() / 1000.0 }));
        }
    };
    let result = match &job {
        Job::Trim(t) => trim(t, &mut report).map(|o| serde_json::to_value(o).unwrap_or_default()),
        Job::Share(sh) => {
            share(sh, &mut report).map(|o| serde_json::to_value(o).unwrap_or_default())
        }
    };
    match result {
        Ok(o) => emit(serde_json::json!({ "done": o, "seconds": started.elapsed().as_secs_f64() })),
        Err(e) => {
            emit(serde_json::json!({ "error": e.to_string() }));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_arguments() {
        let Ok(Job::Trim(j)) = parse(&a(&[
            "trim",
            "C:/in clip.mp4",
            "C:/out.mp4",
            "1.5",
            "20",
            "exact",
        ])) else {
            panic!("trim job")
        };
        assert_eq!((j.start, j.end, j.mode), (1.5, 20.0, Mode::Exact));
        assert_eq!(j.input, PathBuf::from("C:/in clip.mp4"));
        assert!(parse(&a(&["trim", "a", "b", "x", "2", "copy"])).is_err());
        assert!(parse(&a(&["trim", "a", "b", "1", "2", "fast"])).is_err());
        assert!(parse(&a(&["cut", "a", "b", "1", "2", "copy"])).is_err());
        assert!(parse(&a(&["trim"])).is_err());
        let Ok(Job::Share(sh)) = parse(&a(&["share", "a.mp4", "b.mp4", "1", "21", "19922944"]))
        else {
            panic!("share job")
        };
        assert_eq!((sh.start, sh.end, sh.limit_bytes), (1.0, 21.0, 19_922_944));
        assert!(parse(&a(&["share", "a", "b", "1", "2", "19MB"])).is_err());
    }
}
