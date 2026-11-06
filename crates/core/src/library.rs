//! Clip library discovery (ADR-0010). There's no background indexer: the
//! settings/library window calls [`scan`] when it opens, and unchanged files are
//! served from a small JSON cache keyed by path, size and modified time.
//!
//! Layout written by the tray: `<save_dir>\<App>\<App> YYYY-MM-DD HH-MM-SS[ (n)].mp4`.
//! Files directly in `<save_dir>` are listed too. Originals are only ever read.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

/// What the MP4 headers say about a clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Probe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    /// Sample-entry fourcc of the video track: "avc1", "hvc1", "hev1", "av01"…
    pub codec: String,
    pub audio_tracks: u32,
}

/// One clip as the library shows it.
#[derive(Debug, Clone, Serialize)]
pub struct Clip {
    /// Stable for an unchanged file (hash of path, size and modified time).
    /// Also the thumbnail cache key.
    pub id: String,
    pub path: PathBuf,
    pub file_name: String,
    pub game: String,
    /// Local time "YYYY-MM-DD HH:MM:SS". Sorts as text.
    pub recorded: String,
    pub size: u64,
    pub probe: Option<Probe>,
    /// Why the clip can't be read (truncated, not an MP4…), if it can't.
    pub error: Option<String>,
    /// A cached thumbnail exists.
    pub thumb: bool,
    /// Made by the editor (`<stem> edit[ N].mp4`), not by the recorder.
    pub edit: bool,
    /// Content fingerprint (ADR-0014): identifies the clip for titles and
    /// folders even if it's renamed or moved outside JondisClip.
    pub key: String,
    /// The directory directly under the clips root it's in (None: the root).
    pub dir: Option<String>,
    /// Thumbnail cache key: the fingerprint, so a moved clip keeps its
    /// thumbnail (ADR-0015); the path id while the file couldn't be read.
    pub thumb_key: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    version: u32,
    entries: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    size: u64,
    mtime: u64,
    probe: Result<Probe, String>,
    /// Content fingerprint; None if the file couldn't be read.
    #[serde(default)]
    key: Option<String>,
}

/// 2: adds the content fingerprint (a one-time re-read of every clip).
const INDEX_VERSION: u32 = 2;

/// Where the index and thumbnails live.
pub struct Cache {
    pub dir: PathBuf,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
    fn index_file(&self) -> PathBuf {
        self.dir.join("index.json")
    }
    pub fn thumbs_dir(&self) -> PathBuf {
        self.dir.join("thumbs")
    }
    pub fn thumb_path(&self, id: &str) -> PathBuf {
        self.thumbs_dir().join(format!("{id}.jpg"))
    }
}

/// Result of a scan plus how much work it did (for tests and logs).
pub struct Scan {
    pub clips: Vec<Clip>,
    /// Every directory directly under the root (including empty ones).
    pub dirs: Vec<String>,
    /// Files whose headers had to be read (new or changed).
    pub probed: usize,
    pub pruned_thumbs: usize,
}

/// Lists every clip under `root`, newest first. `local_time` turns a file's
/// modified time (seconds since 1970, UTC) into a local "YYYY-MM-DD HH:MM:SS"
/// for clips whose name has no timestamp.
pub fn scan(root: &Path, cache: &Cache, local_time: &dyn Fn(u64) -> String) -> Scan {
    // One scan at a time per process: they share the index file.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut index: Index = std::fs::read(cache.index_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        // v1 indexes are kept: their entries only lack fingerprints, which
        // are filled in below (no full re-probe after the upgrade).
        .filter(|i: &Index| i.version == INDEX_VERSION || i.version == 1)
        .unwrap_or_default();

    let mut files = Vec::new();
    let mut dirs = Vec::new();
    collect(root, None, &mut files);
    walk_dirs(root, None, 1, &mut dirs, &mut files);
    dirs.sort_by_key(|d| d.to_lowercase());

    // Cached thumbnails, listed once (used for the flag and for pruning).
    let thumbs: HashSet<String> = std::fs::read_dir(cache.thumbs_dir())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();

    let mut probed = 0;
    let mut fingerprinted = 0;
    let mut fresh = BTreeMap::new();
    let mut clips = Vec::with_capacity(files.len());
    for (path, rel_dir, size, mtime) in files {
        // The top-level directory names the game (the recorder's layout).
        let folder_game = rel_dir
            .as_deref()
            .map(|d| d.split('\\').next().unwrap_or(d).to_owned());
        let mut transient = false;
        let key = path.to_string_lossy().into_owned();
        let entry = match index.entries.remove(&key) {
            Some(mut e) if e.size == size && e.mtime == mtime => {
                if e.key.is_none() {
                    e.key = fingerprint(&path, size).ok();
                    fingerprinted += 1;
                }
                e
            }
            _ => {
                probed += 1;
                let probe = probe_file(&path);
                // A busy file (antivirus, backup) isn't broken: don't remember that.
                transient |= matches!(&probe, Err(e) if e.is_transient());
                let key = fingerprint(&path, size).ok();
                transient |= key.is_none();
                Entry {
                    size,
                    mtime,
                    probe: probe.map_err(|e| e.to_string()),
                    key,
                }
            }
        };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parsed = parse_name(&file_name);
        let dir = rel_dir;
        let game = folder_game
            .or_else(|| parsed.as_ref().map(|p| p.0.clone()))
            .unwrap_or_else(|| "Other".into());
        let recorded = parsed
            .map(|p| p.1)
            .unwrap_or_else(|| local_time(mtime / 1_000_000_000));
        let id = clip_id(&key, size, mtime);
        let thumb_key = entry.key.clone().unwrap_or_else(|| id.clone());
        let thumb = thumbs.contains(&format!("{thumb_key}.jpg"));
        let edit = is_edit_name(&file_name);
        // No fingerprint (file busy): an empty key, which organising refuses
        // until a later scan can read the file.
        let clip_key = entry.key.clone().unwrap_or_default();
        clips.push(Clip {
            id,
            path,
            file_name,
            game,
            recorded,
            size,
            probe: entry.probe.clone().ok(),
            error: entry.probe.clone().err(),
            thumb,
            edit,
            key: clip_key,
            dir,
            thumb_key,
        });
        if !transient {
            fresh.insert(key, entry);
        }
    }

    // Write the index only when something changed (new, changed or removed files).
    let changed = probed > 0
        || fingerprinted > 0
        || !index.entries.is_empty()
        || index.version != INDEX_VERSION;
    if changed {
        let _ = std::fs::create_dir_all(&cache.dir);
        let out = Index {
            version: INDEX_VERSION,
            entries: fresh,
        };
        if let Ok(bytes) = serde_json::to_vec(&out) {
            let tmp = cache
                .index_file()
                .with_extension(format!("{}.tmp", std::process::id()));
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, cache.index_file());
            }
        }
    }

    let pruned_thumbs = prune_thumbs(cache, &clips, &thumbs);
    clips.sort_by(|a, b| {
        b.recorded
            .cmp(&a.recorded)
            // Same second: "(10)" after "(9)" after "(2)" after the plain name.
            .then_with(|| b.file_name.len().cmp(&a.file_name.len()))
            .then_with(|| b.file_name.cmp(&a.file_name))
    });
    Scan {
        clips,
        dirs,
        probed,
        pruned_thumbs,
    }
}

type Found = (PathBuf, Option<String>, u64, u64);

/// How deep folders go under the clips root (`A\B\C`).
pub const MAX_DEPTH: usize = 3;

/// Directories under `dir` (relative paths with `\`, up to [`MAX_DEPTH`]) and
/// the clips in them. Hidden (`.name`) directories are skipped, and links
/// below the top level aren't followed (no loops).
fn walk_dirs(
    dir: &Path,
    rel: Option<&str>,
    depth: usize,
    dirs: &mut Vec<String>,
    files: &mut Vec<Found>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for d in rd.flatten() {
        let Ok(t) = d.file_type() else { continue };
        let name = d.file_name().to_string_lossy().into_owned();
        if !t.is_dir() && !(depth == 1 && t.is_symlink() && d.path().is_dir()) {
            continue;
        }
        if name.starts_with('.') || (depth > 1 && t.is_symlink()) {
            continue;
        }
        let here = match rel {
            Some(r) => format!("{r}\\{name}"),
            None => name,
        };
        collect(&d.path(), Some(&here), files);
        dirs.push(here.clone());
        walk_dirs(&d.path(), Some(&here), depth + 1, dirs, files);
    }
}

fn collect(dir: &Path, game: Option<&str>, out: &mut Vec<Found>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        let is_mp4 = path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("mp4"));
        let Ok(meta) = e.metadata() else { continue };
        if !is_mp4 || !meta.is_file() {
            continue;
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        out.push((path, game.map(str::to_owned), meta.len(), mtime));
    }
}

/// Deletes cached thumbnails whose clip is gone or changed, and leftover
/// `<id>.jpg.tmp` files. Only touches our own cache folder, and only files
/// named after a clip id.
fn prune_thumbs(cache: &Cache, clips: &[Clip], thumbs: &HashSet<String>) -> usize {
    let live: HashSet<&str> = clips.iter().map(|c| c.thumb_key.as_str()).collect();
    let mut n = 0;
    for name in thumbs {
        let (id, stale_tmp) = match name.strip_suffix(".jpg.tmp") {
            Some(id) => (id, true),
            None => match name.strip_suffix(".jpg") {
                Some(id) => (id, false),
                None => continue,
            },
        };
        if is_clip_id(id)
            && (stale_tmp || !live.contains(id))
            && std::fs::remove_file(cache.thumbs_dir().join(name)).is_ok()
        {
            n += 1;
        }
    }
    n
}

/// Bytes read from each end of a file for its fingerprint.
const FP_SPAN: u64 = 64 * 1024;

/// Content fingerprint: FNV-1a 64 over the size and the first and last
/// 64 KB, as 16 hex digits. Survives renames and moves; reads 128 KB once
/// per new or changed file (cached in the index).
pub fn fingerprint(path: &Path, size: u64) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(&size.to_le_bytes());
    let mut buf = Vec::with_capacity(FP_SPAN as usize);
    (&mut f).take(FP_SPAN).read_to_end(&mut buf)?;
    eat(&buf);
    let mut buf = vec![0u8; FP_SPAN as usize];
    if size > FP_SPAN * 2 {
        f.seek(SeekFrom::Start(size - FP_SPAN))?;
        f.read_exact(&mut buf)?;
        eat(&buf);
    } else if size > FP_SPAN {
        let mut rest = Vec::new();
        f.read_to_end(&mut rest)?;
        eat(&rest);
    }
    Ok(format!("{h:016x}"))
}

/// FNV-1a 64 of the path (case-folded, as Windows paths are) plus size and
/// modified time, as 16 hex digits.
pub fn clip_id(path: &str, size: u64, mtime: u64) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    for c in path.to_lowercase().bytes() {
        eat(c);
    }
    for b in size.to_le_bytes().into_iter().chain(mtime.to_le_bytes()) {
        eat(b);
    }
    format!("{h:016x}")
}

pub fn is_clip_id(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Between a clip's title and its recorder name in a renamed file:
/// `Our Base Got Raided — DayZ 2026-11-04 14-35-41.mp4`.
pub const TITLE_SEP: &str = " — ";

/// A recorder file name taken apart: `[<title> — ]<app> <stamp>[ edit[ N]]`,
/// ignoring " (n)" clash suffixes.
struct NameParts<'a> {
    title: Option<&'a str>,
    /// As written (may be empty: "Other").
    app: &'a str,
    /// As written: `YYYY-MM-DD HH-MM-SS`, `DD.MM.YYYY …` or `MM-DD-YYYY …`
    stamp: &'a str,
    /// The same moment as `YYYY-MM-DD HH:MM:SS` (sorts as text).
    ymd: String,
    edit: Option<u32>,
}

fn name_parts(file_name: &str) -> Option<NameParts<'_>> {
    let stem = file_name.strip_suffix(".mp4").or_else(|| {
        let n = file_name.len().checked_sub(4)?;
        file_name
            .get(n..)?
            .eq_ignore_ascii_case(".mp4")
            .then(|| &file_name[..n])
    })?;
    // " (n)" from a name clash (recorder or a move), then " edit[ N]", then
    // possibly another " (n)" from the recorder.
    let (stem, edit) = split_edit(strip_dup(stem));
    let stem = strip_dup(stem);
    // The stamp is the last 19 bytes (ASCII): "YYYY-MM-DD HH-MM-SS",
    // "DD.MM.YYYY HH-MM-SS" or "MM-DD-YYYY HH-MM-SS" (the date format setting).
    let n = stem.len().checked_sub(19)?;
    let (app, stamp) = (stem.get(..n)?, stem.get(n..)?);
    let ymd = stamp_ymd(stamp)?;
    let app = app.strip_suffix(' ').unwrap_or(app);
    // A title in front (set in JondisClip). Game names don't contain the
    // separator; titles may, so split at the last one.
    let (title, app) = match app.rsplit_once(TITLE_SEP) {
        Some((t, a)) if !t.trim().is_empty() => (Some(t.trim()), a),
        _ => (None, app),
    };
    Some(NameParts {
        title,
        app: app.trim(),
        stamp,
        ymd,
        edit,
    })
}

/// A file-name stamp in any of the three date formats → `YYYY-MM-DD HH:MM:SS`.
fn stamp_ymd(stamp: &str) -> Option<String> {
    let b = stamp.as_bytes();
    if b.len() != 19 || !stamp.is_ascii() {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        let s = &stamp[r];
        if s.bytes().all(|c| c.is_ascii_digit()) {
            s.parse().ok()
        } else {
            None
        }
    };
    let (y, m, d) = match (b[2], b[4], b[5], b[7]) {
        (_, b'-', _, b'-') => (num(0..4)?, num(5..7)?, num(8..10)?),
        (b'.', _, b'.', _) => (num(6..10)?, num(3..5)?, num(0..2)?),
        (b'-', _, b'-', _) => (num(6..10)?, num(0..2)?, num(3..5)?),
        _ => return None,
    };
    if b[10] != b' ' || b[13] != b'-' || b[16] != b'-' {
        return None;
    }
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}"))
}

/// A recording time as written in new file names, per the date format
/// setting: "ymd" `2026-11-06 21-03-11`, "dmy" `06.11.2026 21-03-11`,
/// "mdy" `11-06-2026 21-03-11`. The time always uses dashes (no `:` in
/// Windows file names).
pub fn file_stamp(date_format: &str, y: u16, mo: u16, d: u16, h: u16, mi: u16, s: u16) -> String {
    match date_format {
        "dmy" => format!("{d:02}.{mo:02}.{y:04} {h:02}-{mi:02}-{s:02}"),
        "mdy" => format!("{mo:02}-{d:02}-{y:04} {h:02}-{mi:02}-{s:02}"),
        _ => format!("{y:04}-{mo:02}-{d:02} {h:02}-{mi:02}-{s:02}"),
    }
}

/// `"DayZ 2026-11-04 14-35-41 (2).mp4"` → `("DayZ", "2026-11-04 14:35:41")`.
/// A title in front (`"Raid — DayZ 2026-…"`) is skipped.
pub fn parse_name(file_name: &str) -> Option<(String, String)> {
    let p = name_parts(file_name)?;
    let app = if p.app.is_empty() { "Other" } else { p.app };
    Some((app.to_owned(), p.ymd))
}

/// The title in front of a recorder name (`"Raid — DayZ 2026-….mp4"` →
/// `"Raid"`), if the file was renamed with one.
pub fn parse_title(file_name: &str) -> Option<String> {
    name_parts(file_name)?.title.map(str::to_owned)
}

/// The file name for a clip given (or, with None, cleared of) a title,
/// keeping what the library reads from the name: game, time and the edit
/// suffix. `title_stem` must already be a safe file name stem.
/// `Raid — DayZ 2026-11-04 14-35-41 edit.mp4`; a file without a recorder
/// name becomes `<title>.mp4` (and is left alone when the title is cleared).
pub fn titled_name(file_name: &str, title_stem: Option<&str>) -> String {
    let Some(p) = name_parts(file_name) else {
        return match title_stem {
            Some(t) => format!("{t}.mp4"),
            None => file_name.to_owned(),
        };
    };
    let app = if p.app.is_empty() { "Other" } else { p.app };
    let edit = match p.edit {
        None => String::new(),
        Some(1) => " edit".into(),
        Some(n) => format!(" edit {n}"),
    };
    match title_stem {
        Some(t) => format!("{t}{TITLE_SEP}{app} {}{edit}.mp4", p.stamp),
        None => format!("{app} {}{edit}.mp4", p.stamp),
    }
}

fn file_stem(file_name: &str) -> &str {
    match file_name.len().checked_sub(4) {
        Some(n)
            if file_name
                .get(n..)
                .is_some_and(|x| x.eq_ignore_ascii_case(".mp4")) =>
        {
            &file_name[..n]
        }
        _ => file_name,
    }
}

/// Drops one " (n)" duplicate suffix: `"X (2)"` → `"X"`.
pub fn strip_dup(stem: &str) -> &str {
    match stem.rsplit_once(" (") {
        Some((s, n))
            if n.ends_with(')')
                && n.len() > 1
                && n[..n.len() - 1].bytes().all(|b| b.is_ascii_digit()) =>
        {
            s
        }
        _ => stem,
    }
}

/// Made by the editor? (`… edit.mp4`, `… edit 2.mp4`, also after a clash
/// rename: `… edit (2).mp4`.) The Edit badge depends only on the file name,
/// so it survives moves and folders.
pub fn is_edit_name(file_name: &str) -> bool {
    split_edit(strip_dup(file_stem(file_name))).1.is_some()
}

/// `dir\file_name`, or `dir\stem (2).ext`, `(3)`… if that's taken
/// (`taken` decides; the caller passes a filesystem check). Never returns a
/// name that `taken` reports as used.
pub fn free_path(dir: &Path, file_name: &str, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(file_name);
    if !taken(&first) {
        return first;
    }
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (strip_dup(s).to_string(), format!(".{e}")),
        None => (strip_dup(file_name).to_string(), String::new()),
    };
    let mut n = 2;
    loop {
        let p = dir.join(format!("{stem} ({n}){ext}"));
        if !taken(&p) {
            return p;
        }
        n += 1;
    }
}

/// `"X edit"` → `("X", Some(1))`, `"X edit 3"` → `("X", Some(3))`, else `(stem, None)`.
pub fn split_edit(stem: &str) -> (&str, Option<u32>) {
    if let Some(base) = stem.strip_suffix(" edit") {
        return (base, Some(1));
    }
    if let Some((rest, n)) = stem.rsplit_once(' ')
        && let Some(base) = rest.strip_suffix(" edit")
        && !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit())
        && let Ok(n) = n.parse::<u32>()
        && n >= 2
    {
        return (base, Some(n));
    }
    (stem, None)
}

/// A free name for an edit of `original`, in the same folder:
/// `<stem> edit.mp4`, `<stem> edit 2.mp4`, … Editing an edit numbers from the
/// same original name. Never returns an existing path.
pub fn edit_path(original: &Path) -> PathBuf {
    let name = original
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = split_edit(file_stem(&name)).0.to_owned();
    let dir = original.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut n = 1u32;
    loop {
        let candidate = if n == 1 {
            dir.join(format!("{base} edit.mp4"))
        } else {
            dir.join(format!("{base} edit {n}.mp4"))
        };
        let partial = candidate.with_extension("mp4.partial");
        if !candidate.exists() && !partial.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// Unfinished editor exports (`<stem> edit[ N].mp4.partial`) left behind when
/// the window closed mid-export. The recorder's own `.partial` files (no
/// " edit" suffix) are never listed: it may be writing one right now.
pub fn stale_edit_partials(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    if let Ok(rd) = std::fs::read_dir(root) {
        dirs.extend(
            rd.flatten()
                .filter(|d| d.file_type().is_ok_and(|t| t.is_dir()))
                .map(|d| d.path()),
        );
    }
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".mp4.partial")
                && split_edit(stem).1.is_some()
                && e.file_type().is_ok_and(|t| t.is_file())
            {
                out.push(e.path());
            }
        }
    }
    out
}

// ---------------------------------------------------------------- keyframes

/// Presentation times (seconds) of the video track's sync samples, from the
/// MP4 sample tables. No decoding. Assumes no B-frames (our recordings have
/// none), so decode order is display order.
pub fn keyframes_file(path: &Path) -> Result<Vec<f64>, ProbeError> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    keyframes(&mut std::io::BufReader::new(&mut f), len)
}

#[derive(Default)]
struct KfTrack {
    handler: [u8; 4],
    timescale: u32,
    /// (sample count, delta)
    stts: Vec<(u32, u32)>,
    /// 1-based sample numbers; `None` = every sample is a sync sample.
    stss: Option<Vec<u32>>,
}

/// Largest sample table we'll read (a 5-minute clip's is a few KB).
const MAX_TABLE: u64 = 16 * 1024 * 1024;

pub fn keyframes<R: Read + Seek>(r: &mut R, len: u64) -> Result<Vec<f64>, ProbeError> {
    let mut magic = [0u8; 8];
    r.seek(SeekFrom::Start(0))?;
    if len < 8 || r.read_exact(&mut magic).is_err() || &magic[4..] != b"ftyp" {
        return Err(ProbeError::NotMp4("no ftyp box"));
    }
    let mut pos = 0;
    let mut tracks: Vec<KfTrack> = Vec::new();
    let mut boxes = 0u32;
    while pos + 8 <= len {
        let (kind, start, end) = header(r, pos, len)?;
        if &kind == b"moov" {
            kf_walk(r, start, end, &mut tracks, 0, &mut boxes)?;
            break;
        }
        pos = end;
    }
    let t = tracks
        .iter()
        .find(|t| &t.handler == b"vide")
        .ok_or(ProbeError::NotMp4("no video track"))?;
    if t.timescale == 0 {
        return Err(ProbeError::NotMp4("no media timescale"));
    }
    // Sample number (1-based) → decode time, walking stts once.
    let mut out = Vec::new();
    let mut sync = t.stss.as_ref().map(|v| v.iter().copied().peekable());
    let (mut sample, mut time) = (1u64, 0u64);
    for &(count, delta) in &t.stts {
        for _ in 0..count {
            let is_sync = match sync.as_mut() {
                None => true,
                Some(it) => {
                    while it.peek().is_some_and(|&n| u64::from(n) < sample) {
                        it.next();
                    }
                    it.peek().is_some_and(|&n| u64::from(n) == sample)
                }
            };
            if is_sync {
                out.push(time as f64 / f64::from(t.timescale));
            }
            sample += 1;
            time += u64::from(delta);
        }
    }
    Ok(out)
}

fn kf_walk<R: Read + Seek>(
    r: &mut R,
    mut pos: u64,
    end: u64,
    tracks: &mut Vec<KfTrack>,
    depth: u32,
    boxes: &mut u32,
) -> Result<(), ProbeError> {
    if depth > 8 {
        return Err(ProbeError::NotMp4("boxes nested too deep"));
    }
    while pos + 8 <= end {
        let (kind, start, box_end) = header(r, pos, end)?;
        *boxes += 1;
        if *boxes > MAX_BOXES {
            return Err(ProbeError::NotMp4("too many boxes"));
        }
        match &kind {
            b"trak" => {
                if tracks.len() >= MAX_TRACKS {
                    return Err(ProbeError::NotMp4("too many tracks"));
                }
                tracks.push(KfTrack::default());
                kf_walk(r, start, box_end, tracks, depth + 1, boxes)?;
            }
            b"mdia" | b"minf" | b"stbl" => kf_walk(r, start, box_end, tracks, depth + 1, boxes)?,
            b"hdlr" => {
                let b = read_payload(r, start, box_end, 12, 12)?;
                if let Some(t) = tracks.last_mut() {
                    t.handler.copy_from_slice(&b[8..12]);
                }
            }
            b"mdhd" => {
                let b = read_payload(r, start, box_end, 16, 24)?;
                let ts = if b[0] == 1 {
                    if b.len() < 24 {
                        return Err(ProbeError::NotMp4("box too short"));
                    }
                    be32(&b[20..])
                } else {
                    be32(&b[12..])
                };
                if let Some(t) = tracks.last_mut() {
                    t.timescale = ts;
                }
            }
            b"stts" | b"stss" => {
                if box_end - start > MAX_TABLE {
                    return Err(ProbeError::NotMp4("sample table too large"));
                }
                let b = read_payload(r, start, box_end, 8, (box_end - start) as usize)?;
                let n = be32(&b[4..]) as usize;
                let per = if &kind == b"stts" { 8 } else { 4 };
                if b.len() < 8 + n.saturating_mul(per) {
                    return Err(ProbeError::NotMp4("sample table cut off"));
                }
                let body = &b[8..8 + n * per];
                if let Some(t) = tracks.last_mut() {
                    if &kind == b"stts" {
                        t.stts = body
                            .as_chunks::<8>()
                            .0
                            .iter()
                            .map(|c| (be32(c), be32(&c[4..])))
                            .collect();
                    } else {
                        t.stss = Some(body.as_chunks::<4>().0.iter().map(|c| be32(c)).collect());
                    }
                }
            }
            _ => {}
        }
        pos = box_end;
    }
    Ok(())
}

// ---------------------------------------------------------------- MP4 headers

#[derive(Debug)]
pub enum ProbeError {
    Io(io::Error),
    NotMp4(&'static str),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::Io(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                f.write_str("the file is cut off (incomplete recording?)")
            }
            ProbeError::Io(e) => write!(f, "couldn't read the file: {e}"),
            ProbeError::NotMp4(why) => write!(f, "not a playable MP4 ({why})"),
        }
    }
}

impl ProbeError {
    /// IO trouble that may go away (file locked by another program), as
    /// opposed to a file that is really broken.
    pub fn is_transient(&self) -> bool {
        matches!(self, ProbeError::Io(e) if e.kind() != io::ErrorKind::UnexpectedEof)
    }
}

impl From<io::Error> for ProbeError {
    fn from(e: io::Error) -> Self {
        ProbeError::Io(e)
    }
}

pub fn probe_file(path: &Path) -> Result<Probe, ProbeError> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    probe(&mut f, len)
}

#[derive(Default)]
struct Track {
    handler: [u8; 4],
    codec: [u8; 4],
    width: u32,
    height: u32,
}

#[derive(Default)]
struct Moov {
    timescale: u32,
    duration: u64,
    tracks: Vec<Track>,
    /// Boxes visited, to bound work on garbage files.
    boxes: u32,
}

const MAX_BOXES: u32 = 10_000;
const MAX_TRACKS: usize = 64;

/// Reads only the boxes it needs, seeking past media data and sample tables.
pub fn probe<R: Read + Seek>(r: &mut R, len: u64) -> Result<Probe, ProbeError> {
    let mut magic = [0u8; 8];
    r.seek(SeekFrom::Start(0))?;
    if len < 8 || r.read_exact(&mut magic).is_err() || &magic[4..] != b"ftyp" {
        return Err(ProbeError::NotMp4("no ftyp box"));
    }
    let mut found = Moov::default();
    let mut saw_moov = false;
    let mut pos = 0;
    while pos + 8 <= len {
        let (kind, start, end) = header(r, pos, len)?;
        if &kind == b"moov" {
            saw_moov = true;
            walk(r, start, end, &mut found, 0)?;
            break;
        }
        if pos == 0 && &kind != b"ftyp" {
            return Err(ProbeError::NotMp4("no ftyp box"));
        }
        pos = end;
    }
    if !saw_moov {
        return Err(ProbeError::NotMp4("no movie header"));
    }
    if found.timescale == 0 {
        return Err(ProbeError::NotMp4("no duration"));
    }
    let video = found.tracks.iter().find(|t| &t.handler == b"vide");
    let Some(video) = video else {
        return Err(ProbeError::NotMp4("no video track"));
    };
    Ok(Probe {
        duration: found.duration as f64 / f64::from(found.timescale),
        width: video.width,
        height: video.height,
        codec: String::from_utf8_lossy(&video.codec).into_owned(),
        audio_tracks: found
            .tracks
            .iter()
            .filter(|t| &t.handler == b"soun")
            .count() as u32,
    })
}

/// Box header at `pos`: (type, payload start, end). Validates the size.
fn header<R: Read + Seek>(
    r: &mut R,
    pos: u64,
    limit: u64,
) -> Result<([u8; 4], u64, u64), ProbeError> {
    r.seek(SeekFrom::Start(pos))?;
    let mut h = [0u8; 8];
    r.read_exact(&mut h)?;
    let size32 = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
    let kind = [h[4], h[5], h[6], h[7]];
    let (start, end) = match size32 {
        0 => (pos + 8, limit),
        1 => {
            let mut l = [0u8; 8];
            r.read_exact(&mut l)?;
            (pos + 16, pos.saturating_add(u64::from_be_bytes(l)))
        }
        n => (pos + 8, pos + u64::from(n)),
    };
    if end < start || end > limit {
        return Err(if end > limit {
            ProbeError::Io(io::ErrorKind::UnexpectedEof.into())
        } else {
            ProbeError::NotMp4("bad box size")
        });
    }
    Ok((kind, start, end))
}

fn walk<R: Read + Seek>(
    r: &mut R,
    mut pos: u64,
    end: u64,
    found: &mut Moov,
    depth: u32,
) -> Result<(), ProbeError> {
    if depth > 8 {
        return Err(ProbeError::NotMp4("boxes nested too deep"));
    }
    while pos + 8 <= end {
        let (kind, start, box_end) = header(r, pos, end)?;
        found.boxes += 1;
        if found.boxes > MAX_BOXES {
            return Err(ProbeError::NotMp4("too many boxes"));
        }
        match &kind {
            b"trak" => {
                if found.tracks.len() >= MAX_TRACKS {
                    return Err(ProbeError::NotMp4("too many tracks"));
                }
                found.tracks.push(Track::default());
                walk(r, start, box_end, found, depth + 1)?;
            }
            b"mdia" | b"minf" | b"stbl" => walk(r, start, box_end, found, depth + 1)?,
            b"mvhd" => {
                let b = read_payload(r, start, box_end, 20, 32)?;
                if b[0] == 1 && b.len() < 32 {
                    return Err(ProbeError::NotMp4("box too short"));
                }
                (found.timescale, found.duration) = if b[0] == 1 {
                    (be32(&b[20..]), be64(&b[24..]))
                } else {
                    (be32(&b[12..]), u64::from(be32(&b[16..])))
                };
            }
            b"tkhd" => {
                let b = read_payload(r, start, box_end, 4, 96)?;
                let at = if b[0] == 1 { 4 + 84 } else { 4 + 72 };
                if let Some(t) = found.tracks.last_mut()
                    && b.len() >= at + 8
                {
                    t.width = be32(&b[at..]) >> 16;
                    t.height = be32(&b[at + 4..]) >> 16;
                }
            }
            b"hdlr" => {
                let b = read_payload(r, start, box_end, 12, 12)?;
                if let Some(t) = found.tracks.last_mut() {
                    t.handler.copy_from_slice(&b[8..12]);
                }
            }
            b"stsd" => {
                let b = read_payload(r, start, box_end, 16, 16)?;
                if let Some(t) = found.tracks.last_mut() {
                    t.codec.copy_from_slice(&b[12..16]);
                }
            }
            _ => {}
        }
        pos = box_end;
    }
    Ok(())
}

/// Up to `max` bytes of a box payload, at least `min`.
fn read_payload<R: Read + Seek>(
    r: &mut R,
    start: u64,
    end: u64,
    min: usize,
    max: usize,
) -> Result<Vec<u8>, ProbeError> {
    let n = ((end - start) as usize).min(max);
    if n < min {
        return Err(ProbeError::NotMp4("box too short"));
    }
    r.seek(SeekFrom::Start(start))?;
    let mut b = vec![0u8; n];
    r.read_exact(&mut b)?;
    Ok(b)
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
fn be64(b: &[u8]) -> u64 {
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    pub fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    fn full(version: u8, rest: &[u8]) -> Vec<u8> {
        let mut v = vec![version, 0, 0, 0];
        v.extend_from_slice(rest);
        v
    }

    fn mvhd_v0(timescale: u32, duration: u32) -> Vec<u8> {
        let mut p = vec![0u8; 8];
        p.extend(timescale.to_be_bytes());
        p.extend(duration.to_be_bytes());
        p.extend([0u8; 80]);
        bx(b"mvhd", &full(0, &p))
    }

    fn trak(handler: &[u8; 4], codec: &[u8; 4], w: u32, h: u32) -> Vec<u8> {
        let mut tk = vec![0u8; 72];
        tk.extend((w << 16).to_be_bytes());
        tk.extend((h << 16).to_be_bytes());
        let tkhd = bx(b"tkhd", &full(0, &tk));
        let mut hd = vec![0u8; 4];
        hd.extend_from_slice(handler);
        hd.extend([0u8; 13]);
        let hdlr = bx(b"hdlr", &full(0, &hd));
        let mut sd = 1u32.to_be_bytes().to_vec();
        sd.extend(bx(codec, &[0u8; 70]));
        let stsd = bx(b"stsd", &full(0, &sd));
        let stsz = bx(b"stsz", &[0u8; 4000]); // big table we must skip
        let stbl = bx(b"stbl", &[stsd, stsz].concat());
        let minf = bx(b"minf", &stbl);
        let mdia = bx(b"mdia", &[bx(b"mdhd", &[0u8; 24]), hdlr, minf].concat());
        bx(b"trak", &[tkhd, mdia].concat())
    }

    /// A small but structurally real MP4: 3 audio tracks, 1080p H.264, 300 s.
    pub fn sample_mp4(seconds: u32, moov_first: bool) -> Vec<u8> {
        let ftyp = bx(b"ftyp", b"isomiso2avc1mp41");
        let moov = bx(
            b"moov",
            &[
                mvhd_v0(1000, seconds * 1000),
                trak(b"vide", b"avc1", 1920, 1080),
                trak(b"soun", b"mp4a", 0, 0),
                trak(b"soun", b"mp4a", 0, 0),
                trak(b"soun", b"mp4a", 0, 0),
            ]
            .concat(),
        );
        let mdat = bx(b"mdat", &vec![0xAB; 5000]);
        if moov_first {
            [ftyp, moov, mdat].concat()
        } else {
            [ftyp, mdat, moov].concat()
        }
    }

    fn probe_bytes(b: &[u8]) -> Result<Probe, ProbeError> {
        probe(&mut Cursor::new(b), b.len() as u64)
    }

    #[test]
    fn reads_duration_resolution_and_tracks() {
        for moov_first in [true, false] {
            let p = probe_bytes(&sample_mp4(300, moov_first)).unwrap();
            assert_eq!(
                p,
                Probe {
                    duration: 300.0,
                    width: 1920,
                    height: 1080,
                    codec: "avc1".into(),
                    audio_tracks: 3
                }
            );
        }
    }

    #[test]
    fn reads_version1_mvhd_and_64bit_box_sizes() {
        let mut p = vec![0u8; 16];
        p.extend(90_000u32.to_be_bytes());
        p.extend((90_000u64 * 42).to_be_bytes());
        p.extend([0u8; 80]);
        let mvhd = bx(b"mvhd", &full(1, &p));
        let inner = [mvhd, trak(b"vide", b"hvc1", 2560, 1440)].concat();
        // moov with a 64-bit "largesize" header.
        let mut moov = 1u32.to_be_bytes().to_vec();
        moov.extend_from_slice(b"moov");
        moov.extend(((inner.len() + 16) as u64).to_be_bytes());
        moov.extend(inner);
        let file = [bx(b"ftyp", b"isom"), moov].concat();
        let p = probe_bytes(&file).unwrap();
        assert_eq!(
            (p.duration, p.width, p.codec.as_str(), p.audio_tracks),
            (42.0, 2560, "hvc1", 0)
        );
    }

    #[test]
    fn reads_resolution_from_version1_track_header() {
        let mut tk = vec![0u8; 84];
        tk.extend((2560u32 << 16).to_be_bytes());
        tk.extend((1440u32 << 16).to_be_bytes());
        let tkhd = bx(b"tkhd", &full(1, &tk));
        let mut hd = vec![0u8; 4];
        hd.extend_from_slice(b"vide");
        hd.extend([0u8; 13]);
        let mdia = bx(b"mdia", &bx(b"hdlr", &full(0, &hd)));
        let trak = bx(b"trak", &[tkhd, mdia].concat());
        let moov = bx(b"moov", &[mvhd_v0(1000, 1000), trak].concat());
        let file = [bx(b"ftyp", b"isom"), moov].concat();
        let p = probe_bytes(&file).unwrap();
        assert_eq!((p.width, p.height), (2560, 1440));
    }

    #[test]
    fn bounds_work_on_files_with_huge_box_counts() {
        let many: Vec<u8> = (0..20_000).flat_map(|_| bx(b"free", &[])).collect();
        let moov = bx(b"moov", &[mvhd_v0(1000, 1000), many].concat());
        let file = [bx(b"ftyp", b"isom"), moov].concat();
        assert!(
            probe_bytes(&file)
                .unwrap_err()
                .to_string()
                .contains("too many")
        );
        let traks: Vec<u8> = (0..100).flat_map(|_| bx(b"trak", &[])).collect();
        let moov = bx(b"moov", &[mvhd_v0(1000, 1000), traks].concat());
        let file = [bx(b"ftyp", b"isom"), moov].concat();
        assert!(
            probe_bytes(&file)
                .unwrap_err()
                .to_string()
                .contains("too many tracks")
        );
        assert!(!ProbeError::NotMp4("x").is_transient());
        assert!(!ProbeError::Io(io::ErrorKind::UnexpectedEof.into()).is_transient());
        assert!(ProbeError::Io(io::ErrorKind::PermissionDenied.into()).is_transient());
    }

    #[test]
    fn rejects_broken_files_with_a_reason() {
        let good = sample_mp4(10, false);
        let cut = &good[..good.len() - 300]; // recording cut off mid-moov
        assert!(
            probe_bytes(cut)
                .unwrap_err()
                .to_string()
                .contains("cut off")
        );
        assert!(matches!(
            probe_bytes(b"hello world, not a video at all"),
            Err(ProbeError::NotMp4(_))
        ));
        assert!(probe_bytes(&[]).is_err());
        let no_moov = [bx(b"ftyp", b"isom"), bx(b"mdat", &[0; 100])].concat();
        assert!(
            probe_bytes(&no_moov)
                .unwrap_err()
                .to_string()
                .contains("movie header")
        );
        let audio_only = [
            bx(b"ftyp", b"isom"),
            bx(
                b"moov",
                &[mvhd_v0(1000, 5000), trak(b"soun", b"mp4a", 0, 0)].concat(),
            ),
        ]
        .concat();
        assert!(
            probe_bytes(&audio_only)
                .unwrap_err()
                .to_string()
                .contains("video")
        );
        // A box claiming to be smaller than its header.
        let mut bad = bx(b"ftyp", b"isom");
        bad.extend([0, 0, 0, 4]);
        bad.extend(b"moov");
        bad.extend([0u8; 8]);
        assert!(probe_bytes(&bad).is_err());
        // Self-nesting garbage doesn't recurse forever.
        let mut deep = bx(b"stbl", &[]);
        for _ in 0..20 {
            deep = bx(b"stbl", &deep);
        }
        let file = [
            bx(b"ftyp", b"isom"),
            bx(b"moov", &[mvhd_v0(1000, 1000), bx(b"trak", &deep)].concat()),
        ]
        .concat();
        assert!(probe_bytes(&file).is_err());
    }

    #[test]
    fn parses_names_including_odd_ones() {
        let p = |n: &str| parse_name(n);
        assert_eq!(
            p("DayZ 2026-11-04 14-35-41.mp4"),
            Some(("DayZ".into(), "2026-11-04 14:35:41".into()))
        );
        assert_eq!(
            p("DayZ 2026-11-04 14-35-41 (2).mp4").unwrap().1,
            "2026-11-04 14:35:41"
        );
        assert_eq!(
            p("Escape From Tarkov 2026-01-02 03-04-05.MP4").unwrap().0,
            "Escape From Tarkov"
        );
        assert_eq!(
            p("Ærøskøbing Spill 2026-01-02 03-04-05.mp4").unwrap().0,
            "Ærøskøbing Spill"
        );
        assert_eq!(p("2026-01-02 03-04-05.mp4").unwrap().0, "Other");
        assert_eq!(p("my funny clip.mp4"), None);
        assert_eq!(p("DayZ 2026-11-04 14-35-4x.mp4"), None);
        assert_eq!(p("DayZ 2026-11-04 14-35-41 (two).mp4"), None);
        assert_eq!(p("ø.mp4"), None); // shorter than a stamp, multi-byte: no panic
        assert_eq!(p("ææææææææææææææææææææ.mp4"), None);
    }

    #[test]
    fn every_date_format_reads_back_the_same_time() {
        let want = Some(("DayZ".to_string(), "2026-11-04 14:35:41".to_string()));
        for f in ["ymd", "dmy", "mdy"] {
            let name = format!("DayZ {} edit.mp4", file_stamp(f, 2026, 11, 4, 14, 35, 41));
            assert_eq!(parse_name(&name), want, "{name}");
            assert!(is_edit_name(&name));
            let titled = titled_name(&name, Some("Raid"));
            assert_eq!(parse_name(&titled), want, "{titled}");
            assert_eq!(parse_title(&titled).as_deref(), Some("Raid"));
        }
        assert_eq!(
            file_stamp("dmy", 2026, 11, 4, 9, 5, 1),
            "04.11.2026 09-05-01"
        );
        assert_eq!(
            file_stamp("mdy", 2026, 11, 4, 9, 5, 1),
            "11-04-2026 09-05-01"
        );
        // Not dates: month 13, mixed separators.
        assert_eq!(parse_name("DayZ 13-04-2026 14-35-41.mp4"), None);
        assert_eq!(parse_name("DayZ 04.11-2026 14-35-41.mp4"), None);
    }

    #[test]
    fn titles_in_file_names_keep_game_time_and_edit() {
        let n = "Raid — DayZ 2026-11-04 14-35-41 edit 2 (3).mp4";
        assert_eq!(
            parse_name(n),
            Some(("DayZ".into(), "2026-11-04 14:35:41".into()))
        );
        assert_eq!(parse_title(n).as_deref(), Some("Raid"));
        assert!(is_edit_name(n));
        // A title may contain the separator; the game can't.
        let n = "A — B — Escape From Tarkov 2026-01-02 03-04-05.mp4";
        assert_eq!(parse_title(n).as_deref(), Some("A — B"));
        assert_eq!(parse_name(n).unwrap().0, "Escape From Tarkov");
        assert_eq!(parse_title("DayZ 2026-11-04 14-35-41.mp4"), None);
        assert_eq!(parse_title("my clip.mp4"), None);

        let raw = "DayZ 2026-11-04 14-35-41 edit (2).mp4";
        let titled = titled_name(raw, Some("Raid"));
        assert_eq!(titled, "Raid — DayZ 2026-11-04 14-35-41 edit.mp4");
        assert_eq!(
            titled_name(&titled, Some("Other title")),
            "Other title — DayZ 2026-11-04 14-35-41 edit.mp4"
        );
        assert_eq!(
            titled_name(&titled, None),
            "DayZ 2026-11-04 14-35-41 edit.mp4"
        );
        assert_eq!(
            titled_name("2026-11-04 14-35-41 edit 3.mp4", Some("X")),
            "X — Other 2026-11-04 14-35-41 edit 3.mp4"
        );
        assert_eq!(titled_name("my clip.mp4", Some("Raid")), "Raid.mp4");
        assert_eq!(titled_name("Raid.mp4", None), "Raid.mp4");
        // Editing a titled clip keeps the title in the edit's name.
        assert_eq!(
            edit_path(Path::new(r"C:\c\Raid — DayZ 2026-11-04 14-35-41.mp4"))
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "Raid — DayZ 2026-11-04 14-35-41 edit.mp4"
        );
    }

    #[test]
    fn clash_suffixes_keep_the_edit_badge_and_recording_time() {
        assert!(is_edit_name("DayZ 2026-11-04 18-31-24 edit (2).mp4"));
        assert!(is_edit_name("DayZ 2026-11-04 18-31-24 edit 2 (3).mp4"));
        assert!(!is_edit_name("DayZ 2026-11-04 18-31-24 (2).mp4"));
        assert_eq!(
            parse_name("DayZ 2026-11-04 18-31-24 edit (2).mp4").unwrap(),
            ("DayZ".into(), "2026-11-04 18:31:24".into())
        );
        assert_eq!(
            parse_name("DayZ 2026-11-04 18-31-24 (2) edit (3).mp4")
                .unwrap()
                .1,
            "2026-11-04 18:31:24"
        );
        let taken = |p: &Path| {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            n == "a edit.mp4" || n == "a edit (2).mp4"
        };
        assert_eq!(
            free_path(Path::new("D"), "b.mp4", &taken),
            Path::new("D").join("b.mp4")
        );
        assert_eq!(
            free_path(Path::new("D"), "a edit.mp4", &taken),
            Path::new("D").join("a edit (3).mp4")
        );
        assert_eq!(
            free_path(Path::new("D"), "a edit (2).mp4", &taken),
            Path::new("D").join("a edit (3).mp4")
        );
    }

    #[test]
    fn edit_names_keep_the_recording_time_and_never_collide() {
        assert_eq!(
            parse_name("DayZ 2026-11-04 14-35-41 edit.mp4").unwrap().1,
            "2026-11-04 14:35:41"
        );
        assert_eq!(
            parse_name("DayZ 2026-11-04 14-35-41 (2) edit 3.mp4")
                .unwrap()
                .0,
            "DayZ"
        );
        assert_eq!(split_edit("X edit"), ("X", Some(1)));
        assert_eq!(split_edit("X edit 12"), ("X", Some(12)));
        assert_eq!(split_edit("X edit 1"), ("X edit 1", None));
        assert_eq!(split_edit("X edited"), ("X edited", None));
        assert_eq!(split_edit("credit 2"), ("credit 2", None));

        let t = TempDir::new("editname");
        let orig = t.0.join("DayZ 2026-11-04 14-35-41.mp4");
        std::fs::write(&orig, b"x").unwrap();
        let first = edit_path(&orig);
        assert_eq!(
            first.file_name().unwrap(),
            "DayZ 2026-11-04 14-35-41 edit.mp4"
        );
        std::fs::write(&first, b"x").unwrap();
        let second = edit_path(&orig);
        assert_eq!(
            second.file_name().unwrap(),
            "DayZ 2026-11-04 14-35-41 edit 2.mp4"
        );
        // An export in progress reserves its name too.
        std::fs::write(second.with_extension("mp4.partial"), b"x").unwrap();
        assert_eq!(
            edit_path(&orig).file_name().unwrap(),
            "DayZ 2026-11-04 14-35-41 edit 3.mp4"
        );
        // Editing an edit numbers from the original.
        assert_eq!(
            edit_path(&first).file_name().unwrap(),
            "DayZ 2026-11-04 14-35-41 edit 3.mp4"
        );
    }

    fn stbl_with(stts: &[(u32, u32)], stss: Option<&[u32]>) -> Vec<u8> {
        let mut t = (stts.len() as u32).to_be_bytes().to_vec();
        for (c, d) in stts {
            t.extend(c.to_be_bytes());
            t.extend(d.to_be_bytes());
        }
        let mut boxes = bx(b"stts", &full(0, &t));
        if let Some(stss) = stss {
            let mut v = (stss.len() as u32).to_be_bytes().to_vec();
            for n in stss {
                v.extend(n.to_be_bytes());
            }
            boxes.extend(bx(b"stss", &full(0, &v)));
        }
        bx(b"stbl", &boxes)
    }

    fn kf_file(stbl: Vec<u8>, timescale: u32) -> Vec<u8> {
        let mut hd = vec![0u8; 4];
        hd.extend_from_slice(b"vide");
        hd.extend([0u8; 13]);
        let mut md = vec![0u8; 8];
        md.extend(timescale.to_be_bytes());
        md.extend([0u8; 8]);
        let mdia = bx(
            b"mdia",
            &[
                bx(b"mdhd", &full(0, &md)),
                bx(b"hdlr", &full(0, &hd)),
                bx(b"minf", &stbl),
            ]
            .concat(),
        );
        let audio = trak(b"soun", b"mp4a", 0, 0);
        let moov = bx(
            b"moov",
            &[mvhd_v0(1000, 1000), audio, bx(b"trak", &mdia)].concat(),
        );
        [bx(b"ftyp", b"isom"), bx(b"mdat", &[0; 64]), moov].concat()
    }

    #[test]
    fn keyframe_times_come_from_the_sample_tables() {
        // 60 fps in a 90 kHz time base, keyframe every 60 samples (1 s), 3 s.
        let f = kf_file(stbl_with(&[(180, 1500)], Some(&[1, 61, 121])), 90_000);
        let k = keyframes(&mut Cursor::new(&f), f.len() as u64).unwrap();
        assert_eq!(k, vec![0.0, 1.0, 2.0]);
        // Variable frame durations (VFR capture) are summed correctly.
        let f = kf_file(
            stbl_with(&[(2, 1500), (1, 3000), (2, 1500)], Some(&[1, 4])),
            90_000,
        );
        let k = keyframes(&mut Cursor::new(&f), f.len() as u64).unwrap();
        assert_eq!(k.len(), 2);
        assert!((k[1] - 6000.0 / 90_000.0).abs() < 1e-9);
        // No stss: every sample is a keyframe.
        let f = kf_file(stbl_with(&[(3, 1500)], None), 90_000);
        assert_eq!(
            keyframes(&mut Cursor::new(&f), f.len() as u64)
                .unwrap()
                .len(),
            3
        );
        // A table claiming more entries than it holds is refused, not trusted.
        let mut bad = stbl_with(&[(3, 1500)], None);
        let at = bad.windows(4).position(|w| w == b"stts").unwrap() + 8;
        bad[at..at + 4].copy_from_slice(&1_000_000u32.to_be_bytes());
        let f = kf_file(bad, 90_000);
        assert!(keyframes(&mut Cursor::new(&f), f.len() as u64).is_err());
    }

    #[test]
    fn only_editor_partials_count_as_stale() {
        let t = TempDir::new("partials");
        std::fs::create_dir_all(t.0.join("DayZ")).unwrap();
        for n in [
            "DayZ/DayZ 2026-11-04 14-35-41 edit.mp4.partial",
            "DayZ/DayZ 2026-11-04 14-35-41 edit 2.mp4.partial",
            "DayZ/DayZ 2026-11-04 14-35-41.mp4.partial", // the recorder's: never
            "DayZ/DayZ 2026-11-04 14-35-41 edit.mp4",    // finished: never
            "loose edit.mp4.partial",
        ] {
            std::fs::write(t.0.join(n), b"x").unwrap();
        }
        let mut found: Vec<_> = stale_edit_partials(&t.0)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                "DayZ 2026-11-04 14-35-41 edit 2.mp4.partial",
                "DayZ 2026-11-04 14-35-41 edit.mp4.partial",
                "loose edit.mp4.partial"
            ]
        );
    }

    #[test]
    fn ids_are_stable_and_change_with_the_file() {
        let a = clip_id(r"D:\Clips\DayZ\a.mp4", 10, 20);
        assert!(is_clip_id(&a));
        assert_eq!(a, clip_id(r"d:\clips\dayz\A.MP4", 10, 20));
        assert_ne!(a, clip_id(r"D:\Clips\DayZ\a.mp4", 11, 20));
        assert_ne!(a, clip_id(r"D:\Clips\DayZ\a.mp4", 10, 21));
        assert!(!is_clip_id("../../etc/passwd"));
    }

    pub struct TempDir(pub PathBuf);
    impl TempDir {
        pub fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "jc-test-{tag}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scans_subfolders_three_levels_deep_and_skips_hidden_ones() {
        let t = TempDir::new("nested");
        let root = t.0.join("clips");
        let cache = Cache::new(t.0.join("cache"));
        for dir in [
            "Edited\\DayZ\\Raids",
            "Edited\\Dota",
            ".hidden",
            "A\\B\\C\\TooDeep",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let put = |dir: &str, name: &str| {
            std::fs::write(root.join(dir).join(name), sample_mp4(5, true)).unwrap()
        };
        put("Edited\\DayZ", "DayZ 2026-11-04 18-31-24 edit.mp4");
        put("Edited\\DayZ\\Raids", "DayZ 2026-11-04 19-00-00.mp4");
        put(".hidden", "secret.mp4");
        put("A\\B\\C\\TooDeep", "deep.mp4");
        let s = scan(&root, &cache, &|_| "2026-01-01 00:00:00".into());
        let dir_of = |n: &str| {
            s.clips
                .iter()
                .find(|c| c.file_name == n)
                .map(|c| (c.dir.clone(), c.game.clone()))
        };
        assert_eq!(
            dir_of("DayZ 2026-11-04 18-31-24 edit.mp4"),
            Some((Some("Edited\\DayZ".into()), "Edited".into()))
        );
        assert_eq!(
            dir_of("DayZ 2026-11-04 19-00-00.mp4").unwrap().0.as_deref(),
            Some("Edited\\DayZ\\Raids")
        );
        assert!(dir_of("secret.mp4").is_none(), "hidden folders are skipped");
        assert!(
            dir_of("deep.mp4").is_none(),
            "deeper than three levels isn't scanned"
        );
        for d in [
            "Edited",
            "Edited\\DayZ",
            "Edited\\DayZ\\Raids",
            "Edited\\Dota",
            "A\\B\\C",
        ] {
            assert!(s.dirs.iter().any(|x| x == d), "{d} listed");
        }
        assert!(
            !s.dirs
                .iter()
                .any(|x| x.contains("TooDeep") || x.starts_with('.'))
        );
    }

    #[test]
    fn scans_folders_sorts_newest_first_and_caches() {
        let t = TempDir::new("scan");
        let root = t.0.join("clips with spaces & ünïcode");
        let cache = Cache::new(t.0.join("cache"));
        for (dir, name) in [
            ("DayZ", "DayZ 2026-11-04 14-35-41.mp4"),
            ("DayZ", "DayZ 2026-11-04 17-47-11.mp4"),
            ("DayZ", "DayZ 2026-11-04 17-47-11 (2).mp4"),
            (
                "Escape From Tarkov",
                "Escape From Tarkov 2026-10-01 09-00-00.mp4",
            ),
            ("", "loose clip.mp4"),
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join(name), sample_mp4(60, true)).unwrap();
        }
        std::fs::write(
            root.join("DayZ")
                .join("DayZ 2026-11-05 10-00-00.mp4.partial"),
            b"x",
        )
        .unwrap();
        std::fs::write(root.join("DayZ").join("notes.txt"), b"x").unwrap();
        std::fs::write(
            root.join("DayZ").join("DayZ 2026-11-03 10-00-00.mp4"),
            b"garbage",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("Empty")).unwrap();

        let lt = |_s: u64| "2026-01-01 00:00:00".to_string();
        let s = scan(&root, &cache, &lt);
        let names: Vec<_> = s.clips.iter().map(|c| c.file_name.as_str()).collect();
        assert_eq!(
            names,
            [
                "DayZ 2026-11-04 17-47-11 (2).mp4",
                "DayZ 2026-11-04 17-47-11.mp4",
                "DayZ 2026-11-04 14-35-41.mp4",
                "DayZ 2026-11-03 10-00-00.mp4",
                "Escape From Tarkov 2026-10-01 09-00-00.mp4",
                "loose clip.mp4",
            ]
        );
        assert_eq!(s.probed, 6);
        let broken = &s.clips[3];
        assert!(
            broken.probe.is_none() && broken.error.is_some(),
            "corrupt clips are listed, not hidden"
        );
        assert_eq!(s.clips[4].game, "Escape From Tarkov");
        assert_eq!(
            (s.clips[5].game.as_str(), s.clips[5].recorded.as_str()),
            ("Other", "2026-01-01 00:00:00")
        );
        assert_eq!(s.clips[0].probe.as_ref().unwrap().duration, 60.0);

        // Second scan reads no headers; a changed file is read again.
        assert_eq!(scan(&root, &cache, &lt).probed, 0);
        std::fs::write(
            root.join("DayZ").join("DayZ 2026-11-04 14-35-41.mp4"),
            sample_mp4(20, false),
        )
        .unwrap();
        let s3 = scan(&root, &cache, &lt);
        assert_eq!(s3.probed, 1);
        let c = s3
            .clips
            .iter()
            .find(|c| c.file_name == "DayZ 2026-11-04 14-35-41.mp4")
            .unwrap();
        assert_eq!(c.probe.as_ref().unwrap().duration, 20.0);
    }

    #[test]
    fn fingerprints_follow_content_not_names() {
        let t = TempDir::new("fp");
        let a = t.0.join("a.mp4");
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
        std::fs::write(&a, &data).unwrap();
        let fa = fingerprint(&a, data.len() as u64).unwrap();
        // Renamed/moved: same key.
        let b = t.0.join("sub").join("renamed.mp4");
        std::fs::create_dir_all(b.parent().unwrap()).unwrap();
        std::fs::rename(&a, &b).unwrap();
        assert_eq!(fingerprint(&b, data.len() as u64).unwrap(), fa);
        // Different content (end changed): different key.
        let mut other = data.clone();
        *other.last_mut().unwrap() ^= 1;
        let c = t.0.join("c.mp4");
        std::fs::write(&c, &other).unwrap();
        assert_ne!(fingerprint(&c, other.len() as u64).unwrap(), fa);
        // Small files (under two spans) are covered completely.
        let s = t.0.join("s.mp4");
        std::fs::write(&s, &data[..100_000]).unwrap();
        let mut s2 = data[..100_000].to_vec();
        s2[90_000] ^= 1;
        let s2p = t.0.join("s2.mp4");
        std::fs::write(&s2p, &s2).unwrap();
        assert_ne!(
            fingerprint(&s, 100_000).unwrap(),
            fingerprint(&s2p, 100_000).unwrap()
        );
        assert!(is_clip_id(&fa));
    }

    #[test]
    fn missing_root_is_an_empty_library() {
        let t = TempDir::new("missing");
        let s = scan(&t.0.join("nope"), &Cache::new(t.0.join("cache")), &|_| {
            String::new()
        });
        assert!(s.clips.is_empty());
    }

    #[test]
    fn thumbnail_cache_tracks_clips_and_prunes_orphans() {
        let t = TempDir::new("thumbs");
        let root = t.0.join("clips");
        let cache = Cache::new(t.0.join("cache"));
        std::fs::create_dir_all(root.join("DayZ")).unwrap();
        let clip = root.join("DayZ").join("DayZ 2026-11-04 14-35-41.mp4");
        std::fs::write(&clip, sample_mp4(30, true)).unwrap();
        let s = scan(&root, &cache, &|_| String::new());
        let id = s.clips[0].thumb_key.clone();
        assert!(!s.clips[0].thumb);

        std::fs::create_dir_all(cache.thumbs_dir()).unwrap();
        std::fs::write(cache.thumb_path(&id), b"jpg").unwrap();
        std::fs::write(cache.thumb_path("00000000deadbeef"), b"orphan").unwrap();
        std::fs::write(cache.thumbs_dir().join("keep-me.txt"), b"not ours").unwrap();
        let tmp = cache.thumbs_dir().join(format!("{id}.jpg.tmp"));
        std::fs::write(&tmp, b"crashed mid-write").unwrap();
        let s = scan(&root, &cache, &|_| String::new());
        assert!(s.clips[0].thumb);
        assert_eq!(s.pruned_thumbs, 2);
        assert!(cache.thumb_path(&id).exists() && !tmp.exists());
        assert!(cache.thumbs_dir().join("keep-me.txt").exists());

        // Deleting the clip (by the user) drops its thumbnail on the next scan;
        // the original folder is otherwise untouched by scanning.
        std::fs::remove_file(&clip).unwrap();
        let s = scan(&root, &cache, &|_| String::new());
        assert!(s.clips.is_empty());
        assert_eq!(s.pruned_thumbs, 1);
        assert!(root.join("DayZ").is_dir());
    }

    #[test]
    fn scanning_never_modifies_originals() {
        let t = TempDir::new("ro");
        let root = t.0.join("clips");
        std::fs::create_dir_all(root.join("DayZ")).unwrap();
        let clip = root.join("DayZ").join("DayZ 2026-11-04 14-35-41.mp4");
        let bytes = sample_mp4(30, true);
        std::fs::write(&clip, &bytes).unwrap();
        let before = std::fs::metadata(&clip).unwrap().modified().unwrap();
        let _ = scan(&root, &Cache::new(t.0.join("cache")), &|_| String::new());
        assert_eq!(std::fs::read(&clip).unwrap(), bytes);
        assert_eq!(
            std::fs::metadata(&clip).unwrap().modified().unwrap(),
            before
        );
        assert_eq!(std::fs::read_dir(root.join("DayZ")).unwrap().count(), 1);
    }
}
