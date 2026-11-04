//! Export jobs (ADR-0011): one `jondisclip-export.exe` child at a time, at
//! below-normal priority, in a kill-on-close job object so it can never
//! outlive this window. Progress comes back as JSON lines and is forwarded to
//! the page as `export` events. Cancel kills the child and removes its
//! `.partial` file. The recorder is never involved.

use std::io::{BufRead, BufReader};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

struct Running {
    job: u32,
    child: Child,
    output: PathBuf,
    cancelled: bool,
}

fn running() -> &'static Mutex<Option<Running>> {
    static R: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Event {
    Progress {
        job: u32,
        progress: f64,
    },
    Done {
        job: u32,
        path: String,
        size: u64,
        start: f64,
        duration: f64,
        video: String,
        seconds: f64,
    },
    Error {
        job: u32,
        message: String,
    },
    Cancelled {
        job: u32,
    },
}

#[derive(Serialize)]
pub struct Started {
    pub job: u32,
    pub output: String,
}

/// A job object that kills its processes when this process exits (its last
/// handle closes), so an export never runs on after the window is gone.
fn kill_on_close_job() -> Option<HANDLE> {
    static JOB: OnceLock<usize> = OnceLock::new();
    let h = *JOB.get_or_init(|| unsafe {
        let Ok(job) = CreateJobObjectW(None, None) else {
            return 0;
        };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok.is_err() { 0 } else { job.0 as usize }
    });
    (h != 0).then_some(HANDLE(h as *mut _))
}

fn exporter() -> Result<PathBuf, String> {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("jondisclip-export.exe")))
        .filter(|p| p.exists())
        .ok_or_else(|| "jondisclip-export.exe wasn't found next to the app".into())
}

/// Free bytes on the volume holding `dir`.
fn free_space(dir: &Path) -> Option<u64> {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(
            &windows::core::HSTRING::from(dir.as_os_str()),
            Some(&mut free),
            None,
            None,
        )
    }
    .ok()
    .map(|_| free)
}

/// Checks the selection and the disk, then starts the export.
pub fn start(
    app: AppHandle,
    input: PathBuf,
    duration: f64,
    size: u64,
    start: f64,
    end: f64,
    exact: bool,
) -> Result<Started, String> {
    let valid =
        start.is_finite() && end.is_finite() && start >= 0.0 && end > start && start < duration;
    if !valid {
        return Err("Choose a section to save: the start must be before the end.".into());
    }
    let end = end.min(duration);
    let mut slot = running().lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() {
        return Err("An export is already running. Wait for it or cancel it first.".into());
    }
    let output = jondisclip_core::library::edit_path(&input);
    let dir = output.parent().map(Path::to_path_buf).unwrap_or_default();
    // Same bitrate for a copy; an exact cut targets 1.3× the video bitrate.
    let estimate = (size as f64 / duration.max(0.1)
        * (end - start + 1.0)
        * if exact { 1.35 } else { 1.0 }) as u64;
    // Leave room for the recorder: its next hotkey save (and possibly its
    // buffer) may need this drive too.
    const RESERVE: u64 = 2 * 1024 * 1024 * 1024;
    if let Some(free) = free_space(&dir)
        && free < estimate + estimate / 2 + RESERVE
    {
        return Err(format!(
            "Not enough free space on {} (about {} MB for the clip plus 2 GB kept free for recording).",
            dir.display(),
            estimate / 1_000_000 + 1
        ));
    }
    // Without the job object the exporter could outlive this window; refuse.
    let Some(job_object) = kill_on_close_job() else {
        return Err("couldn't set up the export safely (job object); try again".into());
    };
    let mut child = Command::new(exporter()?)
        .arg("trim")
        .arg(&input)
        .arg(&output)
        .arg(format!("{start:.6}"))
        .arg(format!("{end:.6}"))
        .arg(if exact { "exact" } else { "copy" })
        // Lets the exporter drop to background priority when we're not in front.
        .env("JC_UI_PID", std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS)
        .spawn()
        .map_err(|e| format!("couldn't start the exporter: {e}"))?;
    if unsafe { AssignProcessToJobObject(job_object, HANDLE(child.as_raw_handle())) }.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        remove_partial(&output);
        return Err("couldn't set up the export safely (job object); try again".into());
    }
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let job = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stdout = child.stdout.take().ok_or("no exporter output")?;
    *slot = Some(Running {
        job,
        child,
        output: output.clone(),
        cancelled: false,
    });
    drop(slot);

    std::thread::spawn(move || watch(app, job, stdout));
    Ok(Started {
        job,
        output: output.display().to_string(),
    })
}

/// Turns the exporter's output lines into events until it exits.
fn watch(app: AppHandle, job: u32, stdout: std::process::ChildStdout) {
    let mut last: Option<Event> = None;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(ev) = parse_line(job, &line) {
            if matches!(ev, Event::Progress { .. }) {
                let _ = app.emit("export", ev);
            } else {
                last = Some(ev);
            }
        }
    }
    // Stdout closed: the process ended (or was killed).
    let mut slot = running().lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut r) = slot.take().filter(|r| r.job == job) else {
        return;
    };
    drop(slot);
    let _ = r.child.wait();
    // A finished file wins over a cancel that arrived just after it.
    let ev = match last {
        Some(done @ Event::Done { .. }) => done,
        _ if r.cancelled => Event::Cancelled { job },
        Some(other) => other,
        None => Event::Error {
            job,
            message: "The export stopped unexpectedly.".into(),
        },
    };
    if !matches!(ev, Event::Done { .. }) {
        remove_partial(&r.output);
    }
    let _ = app.emit("export", ev);
}

/// True while an export runs in this window.
pub fn busy() -> bool {
    running()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
}

fn remove_partial(output: &Path) {
    let _ = std::fs::remove_file(output.with_extension("mp4.partial"));
}

pub fn parse_line(job: u32, line: &str) -> Option<Event> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if let Some(p) = v.get("progress").and_then(|p| p.as_f64()) {
        return Some(Event::Progress {
            job,
            progress: p.clamp(0.0, 1.0),
        });
    }
    if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
        return Some(Event::Error {
            job,
            message: e.to_string(),
        });
    }
    let d = v.get("done")?;
    Some(Event::Done {
        job,
        path: d.get("path")?.as_str()?.to_string(),
        size: d.get("size")?.as_u64()?,
        start: d.get("start")?.as_f64()?,
        duration: d.get("duration")?.as_f64()?,
        video: d.get("video")?.as_str()?.to_string(),
        seconds: v.get("seconds").and_then(|s| s.as_f64()).unwrap_or(0.0),
    })
}

/// Stops the running export, if any. Its partial file is removed when the
/// watcher sees the process end.
pub fn cancel() {
    let mut slot = running().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = slot.as_mut() {
        r.cancelled = true;
        let _ = r.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exporter_lines() {
        assert!(
            matches!(parse_line(1, r#"{"progress":0.5}"#), Some(Event::Progress { progress, .. }) if progress == 0.5)
        );
        assert!(
            matches!(parse_line(1, r#"{"progress":7}"#), Some(Event::Progress { progress, .. }) if progress == 1.0)
        );
        assert!(
            matches!(parse_line(2, r#"{"error":"disk full"}"#), Some(Event::Error { job: 2, message }) if message == "disk full")
        );
        let done = r#"{"done":{"path":"C:\\a edit.mp4","size":5,"start":1.0,"duration":2.0,"video":"copy"},"seconds":0.1}"#;
        assert!(matches!(
            parse_line(3, done),
            Some(Event::Done { size: 5, .. })
        ));
        assert!(parse_line(1, "not json").is_none());
        assert!(parse_line(1, r#"{"done":{"path":1}}"#).is_none());
        assert!(parse_line(1, "[mp4 @ 0x1] second pass").is_none());
    }

    #[test]
    fn removes_only_the_partial_file() {
        let d = std::env::temp_dir().join(format!("jc-partial-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let out = d.join("X edit.mp4");
        std::fs::write(out.with_extension("mp4.partial"), b"half").unwrap();
        let original = d.join("X.mp4");
        std::fs::write(&original, b"original").unwrap();
        remove_partial(&out);
        assert!(!out.with_extension("mp4.partial").exists());
        assert_eq!(std::fs::read(&original).unwrap(), b"original");
        let _ = std::fs::remove_dir_all(&d);
    }
}
