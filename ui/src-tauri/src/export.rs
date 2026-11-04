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
    share: bool,
}

/// What to make.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// `<name> edit.mp4` next to the original (ADR-0011).
    Trim { exact: bool },
    /// A file under `limit` bytes in the share folder, put on the clipboard (ADR-0012).
    Share { limit: u64 },
}

/// Where share files go: our own folder, out of the library, pruned.
pub fn share_dir() -> PathBuf {
    jondisclip_core::paths::local_dir().join("share")
}

/// Share files are only needed until Discord has uploaded them.
const SHARE_KEEP: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 3600);

/// Deletes share files older than two weeks. Only our own folder, only
/// `.mp4` / `.mp4.partial`.
pub fn prune_share_dir(dir: &Path, keep: std::time::Duration) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut n = 0;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        let ours = name.ends_with(".mp4") || name.ends_with(".mp4.partial");
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > keep);
        if ours && old && std::fs::remove_file(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

/// True if either folder is inside the other (or they're the same), so
/// pruning one could delete recordings in the other.
pub fn overlaps(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        PathBuf::from(
            p.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_lowercase(),
        )
    };
    let (a, b) = (norm(a), norm(b));
    a.starts_with(&b) || b.starts_with(&a)
}

/// Total size we let the share folder grow to before the oldest go.
const SHARE_CAP: u64 = 2_000_000_000;

/// Deletes the oldest share files until the folder is under `cap` bytes.
pub fn cap_share_dir(dir: &Path, cap: u64) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .to_lowercase()
                .ends_with(".mp4")
        })
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    files.sort();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    let mut n = 0;
    for (_, len, path) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
            n += 1;
        }
    }
    n
}

/// `<share dir>\<clip name>.mp4`, or `… (2).mp4` if that's taken. The name is
/// what Discord shows, so it stays the clip's own.
pub fn share_path(dir: &Path, input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clip".into());
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem}.mp4")
        } else {
            format!("{stem} ({n}).mp4")
        };
        let p = dir.join(name);
        if !p.exists() && !p.with_extension("mp4.partial").exists() {
            return p;
        }
        n += 1;
    }
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
        /// For a share: width/height/fps/copied/limit/attempts.
        share: Option<serde_json::Value>,
        /// For a share: the file is on the clipboard (or why it isn't).
        clipboard: Option<Result<(), String>>,
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
    kind: Kind,
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
    let (output, estimate) = match kind {
        Kind::Trim { exact } => (
            jondisclip_core::library::edit_path(&input),
            // Same bitrate for a copy; an exact cut targets 1.3× the video bitrate.
            (size as f64 / duration.max(0.1) * (end - start + 1.0) * if exact { 1.35 } else { 1.0 })
                as u64,
        ),
        Kind::Share { limit } => {
            let dir = share_dir();
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
            let (settings, _) = jondisclip_core::settings::Settings::from_toml(
                &std::fs::read_to_string(jondisclip_core::paths::settings_file())
                    .unwrap_or_default(),
            );
            let save = settings.save_dir_or(&jondisclip_core::paths::default_save_dir());
            if overlaps(&dir, &save) {
                return Err(format!(
                    "The save folder overlaps JondisClip's share folder ({}); choose another save folder.",
                    dir.display()
                ));
            }
            prune_share_dir(&dir, SHARE_KEEP);
            cap_share_dir(&dir, SHARE_CAP);
            (share_path(&dir, &input), limit)
        }
    };
    let dir = output.parent().map(Path::to_path_buf).unwrap_or_default();
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
    let (cmd, last) = match kind {
        Kind::Trim { exact } => (
            "trim",
            if exact {
                "exact".to_string()
            } else {
                "copy".to_string()
            },
        ),
        Kind::Share { limit } => ("share", limit.to_string()),
    };
    let mut child = Command::new(exporter()?)
        .arg(cmd)
        .arg(&input)
        .arg(&output)
        .arg(format!("{start:.6}"))
        .arg(format!("{end:.6}"))
        .arg(last)
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
        share: matches!(kind, Kind::Share { .. }),
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
        Some(Event::Done {
            job,
            path,
            size,
            start,
            duration,
            video,
            seconds,
            share,
            ..
        }) => {
            // A share goes straight onto the clipboard, ready for Ctrl+V.
            let clipboard = r
                .share
                .then(|| crate::clipboard::copy_file(Path::new(&path), main_hwnd(&app)));
            Event::Done {
                job,
                path,
                size,
                start,
                duration,
                video,
                seconds,
                share,
                clipboard,
            }
        }
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

/// Our window's handle, as the clipboard owner.
pub fn main_hwnd(app: &AppHandle) -> Option<isize> {
    use tauri::Manager;
    app.get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
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
    let share = d.get("limit").map(|_| d.clone());
    Some(Event::Done {
        job,
        path: d.get("path")?.as_str()?.to_string(),
        size: d.get("size")?.as_u64()?,
        start: d.get("start").and_then(|s| s.as_f64()).unwrap_or(0.0),
        duration: d.get("duration")?.as_f64()?,
        video: d.get("video")?.as_str()?.to_string(),
        seconds: v.get("seconds").and_then(|s| s.as_f64()).unwrap_or(0.0),
        share,
        clipboard: None,
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
    fn parses_share_results() {
        let done = r#"{"done":{"path":"C:\\s\\a.mp4","size":5,"limit":19000000,"duration":2.0,"width":1280,"height":720,"fps":60,"copied":false,"video":"h264_amf","attempts":1},"seconds":3}"#;
        let Some(Event::Done {
            share: Some(s),
            start,
            ..
        }) = parse_line(4, done)
        else {
            panic!("share done")
        };
        assert_eq!(s["height"], 720);
        assert_eq!(start, 0.0);
    }

    #[test]
    fn share_names_and_pruning_stay_in_our_folder() {
        let d = std::env::temp_dir().join(format!("jc-share-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let input = Path::new(r"D:\Clips\DayZ\DayZ 2026-11-04 15-27-02 edit.mp4");
        let first = share_path(&d, input);
        assert_eq!(
            first.file_name().unwrap(),
            "DayZ 2026-11-04 15-27-02 edit.mp4"
        );
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(
            share_path(&d, input).file_name().unwrap(),
            "DayZ 2026-11-04 15-27-02 edit (2).mp4"
        );
        std::fs::write(d.join("notes.txt"), b"keep").unwrap();
        // Nothing is old enough yet…
        assert_eq!(prune_share_dir(&d, SHARE_KEEP), 0);
        // …with a zero age limit only our .mp4 goes.
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(prune_share_dir(&d, std::time::Duration::ZERO), 1);
        assert!(d.join("notes.txt").exists());
        assert_eq!(prune_share_dir(Path::new(r"C:\no\such\dir"), SHARE_KEEP), 0);
        // Size cap: the oldest go first.
        let old = d.join("old.mp4");
        std::fs::write(&old, vec![0u8; 600]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let new = d.join("new.mp4");
        std::fs::write(&new, vec![0u8; 600]).unwrap();
        assert_eq!(cap_share_dir(&d, 1000), 1);
        assert!(!old.exists() && new.exists());
        // Overlap with the save folder is detected both ways.
        assert!(overlaps(&d, &d.join("sub")));
        assert!(overlaps(&d.join("sub"), &d));
        assert!(!overlaps(&d.join("a"), &d.join("b")));
        let _ = std::fs::remove_dir_all(&d);
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
