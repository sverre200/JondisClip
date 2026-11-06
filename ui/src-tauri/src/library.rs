//! Library backend (ADR-0010): scan on request, serve clips and thumbnails
//! over the `clip:` protocol (only IDs from the latest scan, with range
//! support), and store thumbnails the page renders. Nothing here runs unless
//! the window asks: there's no watcher and no background work.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Mutex, OnceLock};

use jondisclip_core::library::{self, Cache, Clip};
use jondisclip_core::paths;
use serde::Serialize;
use tauri::http::{Request, Response, StatusCode, header};

/// Clips from the latest scan: id → path. The protocol serves only these.
fn known() -> &'static Mutex<HashMap<String, PathBuf>> {
    static K: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    K.get_or_init(Default::default)
}

pub fn cache() -> Cache {
    Cache::new(paths::local_dir().join("library"))
}

/// Content keys of the latest scan: id → key (only clips that could be read).
fn known_keys() -> &'static Mutex<HashMap<String, String>> {
    static K: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    K.get_or_init(Default::default)
}

/// The content key of a clip from the latest scan.
pub fn key_of(id: &str) -> Option<String> {
    known_keys()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
}

/// Whether a clip other than `except` has content key `key` (a byte-identical
/// copy shares its title and folders, so they're kept for it).
pub fn key_held_by_others(key: &str, except: &[String]) -> bool {
    known_keys()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|(id, k)| k == key && !except.contains(id))
}

pub fn path_of(id: &str) -> Result<PathBuf, String> {
    known()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
        .ok_or_else(|| "that clip is no longer in the library; refresh and try again".into())
}

#[derive(Serialize)]
pub struct LibraryView {
    pub root: String,
    pub clips: Vec<Clip>,
    /// Every directory directly under the root (for folder reconciliation).
    #[serde(skip)]
    pub dirs: Vec<String>,
}

/// Thumbnail keys of the latest scan (content keys): the only ones saved.
fn known_thumbs() -> &'static Mutex<std::collections::HashSet<String>> {
    static K: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    K.get_or_init(Default::default)
}

pub fn scan(root: PathBuf) -> LibraryView {
    let s = library::scan(&root, &cache(), &local_time);
    let mut k = known().lock().unwrap_or_else(|e| e.into_inner());
    k.clear();
    k.extend(s.clips.iter().map(|c| (c.id.clone(), c.path.clone())));
    let mut ks = known_keys().lock().unwrap_or_else(|e| e.into_inner());
    ks.clear();
    ks.extend(
        s.clips
            .iter()
            .filter(|c| !c.key.is_empty())
            .map(|c| (c.id.clone(), c.key.clone())),
    );
    drop(ks);
    let mut t = known_thumbs().lock().unwrap_or_else(|e| e.into_inner());
    t.clear();
    t.extend(s.clips.iter().map(|c| c.thumb_key.clone()));
    LibraryView {
        root: root.display().to_string(),
        clips: s.clips,
        dirs: s.dirs,
    }
}

/// Saves a thumbnail JPEG the page drew, under the clip's thumbnail key
/// (its content fingerprint, so it survives moves).
pub fn save_thumb(id: &str, jpeg: &[u8]) -> Result<(), String> {
    let known = known_thumbs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(id);
    if !library::is_clip_id(id) || !known {
        return Err("unknown clip".into());
    }
    // A JPEG starts with FF D8 and a thumbnail is small: refuse anything else.
    if jpeg.len() < 4 || jpeg[..2] != [0xFF, 0xD8] || jpeg.len() > 2_000_000 {
        return Err("not a thumbnail".into());
    }
    let c = cache();
    std::fs::create_dir_all(c.thumbs_dir()).map_err(|e| e.to_string())?;
    let dst = c.thumb_path(id);
    let tmp = dst.with_extension("jpg.tmp"); // pruned on scan if left behind
    std::fs::write(&tmp, jpeg).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())
}

/// Local "YYYY-MM-DD HH:MM:SS" for seconds since 1970 (UTC), via Windows'
/// time zone rules (correct for past DST periods too).
fn local_time(secs: u64) -> String {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ft = (secs + 11_644_473_600) * 10_000_000;
    let ft = FILETIME {
        dwLowDateTime: ft as u32,
        dwHighDateTime: (ft >> 32) as u32,
    };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err()
            || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err()
        {
            local = utc;
        }
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute, local.wSecond
    )
}

/// Largest body per range response. The video element asks for more as it plays.
const CHUNK: u64 = 1024 * 1024;

/// Protocol requests waiting for a worker. Scrubbing can fire many; beyond
/// this they get 503 and the video element simply asks again.
const QUEUE: usize = 32;
const WORKERS: usize = 2;

type Job = (Request<Vec<u8>>, tauri::UriSchemeResponder);

/// Hands a `clip:` request to a small fixed pool (below-normal priority), so
/// scrubbing never turns into hundreds of threads and parallel disk reads on
/// the drive the recorder writes to.
pub fn enqueue(req: Request<Vec<u8>>, responder: tauri::UriSchemeResponder) {
    static TX: OnceLock<Mutex<SyncSender<Job>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = sync_channel::<Job>(QUEUE);
        let rx = std::sync::Arc::new(Mutex::new(rx));
        for i in 0..WORKERS {
            let rx = rx.clone();
            let _ = std::thread::Builder::new()
                .name(format!("jc-clip-io-{i}"))
                .spawn(move || {
                    below_normal_priority();
                    loop {
                        let job = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        let Ok((req, responder)) = job else { return };
                        responder.respond(serve(&req));
                    }
                });
        }
        Mutex::new(tx)
    });
    let tx = tx.lock().unwrap_or_else(|e| e.into_inner());
    match tx.try_send((req, responder)) {
        Ok(()) => {}
        Err(TrySendError::Full((_, responder)) | TrySendError::Disconnected((_, responder))) => {
            responder.respond(status(StatusCode::SERVICE_UNAVAILABLE));
        }
    }
}

fn below_normal_priority() {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

/// Runs `f` with background CPU and IO priority (scans and thumbnail writes),
/// so a library open during a game never competes with it for the disk.
pub fn in_background<T>(f: impl FnOnce() -> T) -> T {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        THREAD_MODE_BACKGROUND_END,
    };
    let began =
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) }.is_ok();
    let out = f();
    if began {
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END);
        }
    }
    out
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(code)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(Vec::new())
        .unwrap_or_default()
}

/// `clip:` protocol: `/v/<id>` streams a clip (HTTP ranges), `/t/<id>.jpg`
/// returns its cached thumbnail.
pub fn serve(req: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = req.uri().path();
    let result = if let Some(id) = path.strip_prefix("/v/") {
        serve_video(
            id,
            req.headers()
                .get(header::RANGE)
                .and_then(|v| v.to_str().ok()),
        )
    } else if let Some(id) = path
        .strip_prefix("/t/")
        .and_then(|p| p.strip_suffix(".jpg"))
    {
        serve_thumb(id)
    } else {
        Err(StatusCode::NOT_FOUND)
    };
    result.unwrap_or_else(status)
}

fn serve_thumb(id: &str) -> Result<Response<Vec<u8>>, StatusCode> {
    if !library::is_clip_id(id) {
        return Err(StatusCode::NOT_FOUND);
    }
    let body = std::fs::read(cache().thumb_path(id)).map_err(|_| StatusCode::NOT_FOUND)?;
    Response::builder()
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn serve_video(id: &str, range: Option<&str>) -> Result<Response<Vec<u8>>, StatusCode> {
    let path = path_of(id).map_err(|_| StatusCode::NOT_FOUND)?;
    let mut f = File::open(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    let len = f.metadata().map_err(|_| StatusCode::NOT_FOUND)?.len();
    let (start, end) = match range.map(|r| parse_range(r, len)) {
        None => (0, len.min(CHUNK).saturating_sub(1)),
        Some(Some(r)) => r,
        Some(None) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(Vec::new())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let end = end.min(start + CHUNK - 1);
    let mut body = vec![0u8; (end - start + 1) as usize];
    f.seek(SeekFrom::Start(start))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    f.read_exact(&mut body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(header::CONTENT_TYPE, "video/mp4")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
        .header(header::CONTENT_LENGTH, body.len().to_string())
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` → inclusive (start, end), or `None`
/// if unsatisfiable. Multiple ranges aren't used by media elements.
pub fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let spec = h.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if len == 0 {
        return None;
    }
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        if n == 0 {
            return None;
        }
        (len.saturating_sub(n), len - 1)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() {
            len - 1
        } else {
            b.parse::<u64>().ok()?.min(len - 1)
        };
        (start, end)
    };
    (start <= end && start < len).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-500", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-500", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=20-10", 100), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
        assert_eq!(parse_range("items=0-1", 100), None);
        assert_eq!(parse_range("bytes=abc", 100), None);
    }

    #[test]
    fn unknown_ids_and_paths_are_not_served() {
        let get = |uri: &str| {
            let req = Request::builder().uri(uri).body(Vec::new()).unwrap();
            serve(&req).status()
        };
        assert_eq!(
            get("http://clip.localhost/v/0123456789abcdef"),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get("http://clip.localhost/v/..%2F..%2Fwindows%2Fwin.ini"),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get("http://clip.localhost/t/../../settings.toml"),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get("http://clip.localhost/C:/Windows/win.ini"),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn rejects_bad_thumbnails() {
        assert!(save_thumb("../../evil", &[0xFF, 0xD8, 0, 0]).is_err());
        assert!(save_thumb("0123456789abcdef", &[0xFF, 0xD8, 0, 0]).is_err()); // not scanned
    }

    #[test]
    fn serves_ranges_of_a_scanned_clip() {
        let dir = std::env::temp_dir().join(format!("jc-serve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.mp4");
        let data: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
        std::fs::write(&file, &data).unwrap();
        known()
            .lock()
            .unwrap()
            .insert("00000000000000aa".into(), file.clone());
        let req = Request::builder()
            .uri("http://clip.localhost/v/00000000000000aa")
            .header(header::RANGE, "bytes=100-199")
            .body(Vec::new())
            .unwrap();
        let r = serve(&req);
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(r.body().as_slice(), &data[100..200]);
        assert_eq!(r.headers()[header::CONTENT_RANGE], "bytes 100-199/10000");
        known().lock().unwrap().remove("00000000000000aa");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
