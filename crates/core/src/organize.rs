//! Library organisation (ADR-0015, building on ADR-0014).
//!
//! The filesystem decides which folder a clip is in: user folders are real
//! directories under the clips root. This document only *describes* them —
//! colour, sidebar order, the manual clip order — plus clip titles and which
//! directories are the recorder's game folders. Clips are identified by a
//! content fingerprint (`library::fingerprint`), so titles and order follow
//! a clip through moves and renames.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 2;
pub const MAX_TITLE: usize = 120;
pub const MAX_FOLDER_NAME: usize = 60;
/// The palette (muted tokens in the UI). Anything else is refused.
pub const COLORS: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "gray"];

/// A user folder: a directory under the clips root, up to
/// [`MAX_DEPTH`](crate::library::MAX_DEPTH) levels deep.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    /// Its path relative to the clips root, `\`-separated (`Edited\DayZ`).
    /// The filesystem is the truth for it.
    pub name: String,
    pub color: String,
    /// Clip keys in the user's custom order (display only; files never move
    /// to express it). May list clips that aren't in the folder (any more).
    #[serde(default)]
    pub order: Vec<String>,
}

/// Clips that belonged to a v1 (metadata-only) folder and haven't been moved
/// into its real directory yet. The library asks before moving anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub folder: String,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Organize {
    pub version: u32,
    /// User folders in sidebar order (including ones whose directory is
    /// missing right now: kept, not shown).
    #[serde(default)]
    pub folders: Vec<Folder>,
    /// Clip key → title.
    #[serde(default)]
    pub titles: BTreeMap<String, String>,
    /// Lower-case names of the recorder's game folders (learned on scans).
    #[serde(default)]
    pub game_dirs: BTreeSet<String>,
    #[serde(default)]
    pub pending: Vec<Pending>,
    /// Clip key → its copies in the Google Drive folder (ADR-0013), written
    /// by Save to Drive. Shown as "In Google Drive" while a copy exists.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub drive: BTreeMap<String, Vec<String>>,
}

/// Most Drive copies remembered per clip (oldest dropped first).
pub const MAX_DRIVE_COPIES: usize = 8;

#[derive(Debug, PartialEq)]
pub enum OrgError {
    Invalid(String),
    NotFound,
}

impl std::fmt::Display for OrgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrgError::Invalid(s) => f.write_str(s),
            OrgError::NotFound => f.write_str("that folder no longer exists"),
        }
    }
}

/// Trims, collapses whitespace and drops control characters.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn is_key(k: &str) -> bool {
    k.len() == 16 && k.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A folder name that is also a valid, unremarkable Windows directory name:
/// no `<>:"/\|?*` or control characters, no trailing dot/space, no reserved
/// device name, 1–60 characters.
pub fn folder_name(raw: &str) -> Result<String, OrgError> {
    let name = clean(raw);
    if name.is_empty() {
        return Err(OrgError::Invalid("Give the folder a name.".into()));
    }
    if name.chars().count() > MAX_FOLDER_NAME {
        return Err(OrgError::Invalid(format!(
            "Folder names can be up to {MAX_FOLDER_NAME} characters."
        )));
    }
    if let Some(c) = name.chars().find(|c| r#"<>:"/\|?*"#.contains(*c)) {
        return Err(OrgError::Invalid(format!("Folder names can't contain {c}")));
    }
    if name.ends_with('.') {
        return Err(OrgError::Invalid(
            "Folder names can't end with a dot.".into(),
        ));
    }
    let base = name.split('.').next().unwrap_or("").trim().to_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if reserved.contains(&base.as_str()) {
        return Err(OrgError::Invalid(format!(
            "\"{name}\" is reserved by Windows."
        )));
    }
    Ok(name)
}

/// A folder path: one to [`MAX_DEPTH`](crate::library::MAX_DEPTH) valid
/// names, separated by `\` (or `/`), returned `\`-separated.
pub fn folder_path(raw: &str) -> Result<String, OrgError> {
    let parts: Vec<&str> = raw.split(['\\', '/']).collect();
    if parts.len() > crate::library::MAX_DEPTH {
        return Err(OrgError::Invalid(format!(
            "Folders go up to {} levels deep.",
            crate::library::MAX_DEPTH
        )));
    }
    let names = parts
        .iter()
        .map(|p| folder_name(p))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names.join("\\"))
}

/// `A\B\C` → `Some("A\B")`; a top-level folder has no parent.
pub fn parent_of(path: &str) -> Option<&str> {
    path.rsplit_once('\\').map(|(p, _)| p)
}

/// The last part of a path: what the sidebar shows.
pub fn leaf_of(path: &str) -> &str {
    path.rsplit_once('\\').map_or(path, |(_, l)| l)
}

/// Is `path` inside `ancestor` (not equal)? Case-insensitive, like Windows.
pub fn is_within(path: &str, ancestor: &str) -> bool {
    path.len() > ancestor.len() + 1
        && path.is_char_boundary(ancestor.len())
        && path[..ancestor.len()].eq_ignore_ascii_case(ancestor)
        && path[ancestor.len()..].starts_with('\\')
}

/// How many levels deep a path is (`A` = 1).
pub fn depth_of(path: &str) -> usize {
    path.matches('\\').count() + 1
}

/// What a scan found in the clips root, for reconciliation.
pub struct Found<'a> {
    /// Every directory under the root, as relative paths (`A`, `A\B`).
    pub dirs: &'a [String],
    /// Directory path → (file name, clip key) of the clips directly in it.
    pub clips: &'a HashMap<String, Vec<(String, String)>>,
    /// Recorder app name parsed from a file name (`parse_name`), if any.
    pub app_of: &'a dyn Fn(&str) -> Option<String>,
}

impl Organize {
    pub fn new() -> Self {
        Self {
            version: VERSION,
            ..Default::default()
        }
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.folders
            .iter()
            .position(|f| f.name.eq_ignore_ascii_case(name))
    }

    pub fn folder(&self, name: &str) -> Option<&Folder> {
        self.find(name).map(|i| &self.folders[i])
    }

    /// Only top-level directories can be the recorder's game folders.
    pub fn is_game_dir(&self, name: &str) -> bool {
        !name.contains('\\')
            && self.game_dirs.contains(&name.to_lowercase())
            && self.find(name).is_none()
    }

    /// The folder and every folder inside it.
    pub fn subtree(&self, name: &str) -> Vec<&Folder> {
        self.folders
            .iter()
            .filter(|f| f.name.eq_ignore_ascii_case(name) || is_within(&f.name, name))
            .collect()
    }

    fn check_color(color: &str) -> Result<String, OrgError> {
        COLORS
            .iter()
            .find(|c| **c == color)
            .map(|c| c.to_string())
            .ok_or_else(|| OrgError::Invalid(format!("unknown colour {color}")))
    }

    /// Registers a folder (its directory is created by the caller first).
    /// A subfolder's parent must be a registered folder.
    pub fn add_folder(&mut self, name: &str, color: &str) -> Result<(), OrgError> {
        let name = folder_path(name)?;
        let color = Self::check_color(color)?;
        if self.find(&name).is_some() {
            return Err(OrgError::Invalid(format!(
                "There's already a folder called \"{}\" there.",
                leaf_of(&name)
            )));
        }
        if let Some(parent) = parent_of(&name)
            && self.find(parent).is_none()
        {
            return Err(OrgError::NotFound);
        }
        self.folders.push(Folder {
            name,
            color,
            order: Vec::new(),
        });
        Ok(())
    }

    /// Records a rename or move that already happened on disk (`from` and
    /// `to` are full paths). Folders inside it move along.
    pub fn renamed(&mut self, from: &str, to: &str) -> Result<(), OrgError> {
        self.find(from).ok_or(OrgError::NotFound)?;
        let from = self.folders[self.find(from).unwrap_or_default()]
            .name
            .clone();
        let to = folder_path(to)?;
        if let Some(j) = self.find(&to)
            && !self.folders[j].name.eq_ignore_ascii_case(&from)
        {
            return Err(OrgError::Invalid(format!(
                "There's already a folder called \"{}\" there.",
                leaf_of(&to)
            )));
        }
        let moved = |name: &str| -> Option<String> {
            if name.eq_ignore_ascii_case(&from) {
                Some(to.clone())
            } else if is_within(name, &from) {
                Some(format!("{to}{}", &name[from.len()..]))
            } else {
                None
            }
        };
        for f in &mut self.folders {
            if let Some(n) = moved(&f.name) {
                f.name = n;
            }
        }
        for p in &mut self.pending {
            if let Some(n) = moved(&p.folder) {
                p.folder = n;
            }
        }
        Ok(())
    }

    pub fn recolor(&mut self, name: &str, color: &str) -> Result<(), OrgError> {
        let color = Self::check_color(color)?;
        let i = self.find(name).ok_or(OrgError::NotFound)?;
        self.folders[i].color = color;
        Ok(())
    }

    /// Forgets a folder and the folders inside it (after the caller emptied
    /// or removed their directories).
    pub fn remove_folder(&mut self, name: &str) -> Result<Folder, OrgError> {
        let i = self.find(name).ok_or(OrgError::NotFound)?;
        let gone = |n: &str| n.eq_ignore_ascii_case(name) || is_within(n, name);
        self.pending.retain(|p| !gone(&p.folder));
        let f = self.folders.remove(i);
        self.folders.retain(|x| !is_within(&x.name, name));
        Ok(f)
    }

    /// Sidebar order. Unknown names are ignored; folders missing from `names`
    /// keep their relative order at the end.
    pub fn reorder_folders(&mut self, names: &[String]) {
        let mut out = Vec::with_capacity(self.folders.len());
        for n in names {
            if let Some(i) = self.find(n) {
                out.push(self.folders.remove(i));
            }
        }
        out.append(&mut self.folders);
        self.folders = out;
    }

    /// Custom order inside a folder: puts `keys` before `before` (or at the
    /// end). `present` is the folder's current content (from the scan), so
    /// the order list is completed with clips it didn't know yet. Display
    /// only: no file is touched.
    pub fn arrange(
        &mut self,
        folder: &str,
        present: &[String],
        keys: &[String],
        before: Option<&str>,
    ) -> Result<(), OrgError> {
        if keys.iter().any(|k| !is_key(k)) {
            return Err(OrgError::Invalid("bad clip key".into()));
        }
        let i = self.find(folder).ok_or(OrgError::NotFound)?;
        let f = &mut self.folders[i];
        // Start from the current display order: known order first, then the
        // rest as found.
        let mut list: Vec<String> = f
            .order
            .iter()
            .filter(|k| present.contains(k))
            .cloned()
            .collect();
        for k in present {
            if !list.contains(k) {
                list.push(k.clone());
            }
        }
        // A `before` that is itself moving: anchor on the next one that stays.
        let anchor = match before {
            Some(b) if keys.iter().any(|k| k == b) => {
                let at = list.iter().position(|c| c == b);
                at.and_then(|at| list[at..].iter().find(|c| !keys.contains(c)).cloned())
            }
            b => b.map(str::to_owned),
        };
        list.retain(|c| !keys.contains(c));
        let at = anchor
            .and_then(|b| list.iter().position(|c| *c == b))
            .unwrap_or(list.len());
        let mut seen = std::collections::HashSet::new();
        let moving: Vec<String> = keys
            .iter()
            .filter(|k| present.contains(k) && seen.insert(k.as_str()))
            .cloned()
            .collect();
        list.splice(at..at, moving);
        f.order = list;
        Ok(())
    }

    /// After clips moved into `folder` on disk: append them to its order.
    pub fn arrived(&mut self, folder: &str, keys: &[String]) {
        if let Some(i) = self.find(folder) {
            let f = &mut self.folders[i];
            f.order.retain(|k| !keys.contains(k));
            f.order.extend(keys.iter().filter(|k| is_key(k)).cloned());
        }
        for p in &mut self.pending {
            p.keys.retain(|k| !keys.contains(k));
        }
        self.pending.retain(|p| !p.keys.is_empty());
    }

    /// Sets (or with an empty title, clears) a clip's title.
    pub fn set_title(&mut self, key: &str, title: &str) -> Result<(), OrgError> {
        if !is_key(key) {
            return Err(OrgError::Invalid("bad clip key".into()));
        }
        let t = clean(title);
        if t.chars().count() > MAX_TITLE {
            return Err(OrgError::Invalid(format!(
                "Titles can be up to {MAX_TITLE} characters."
            )));
        }
        if t.is_empty() {
            self.titles.remove(key);
        } else {
            self.titles.insert(key.into(), t);
        }
        Ok(())
    }

    /// Remembers that clip `key` has a copy at `path` in the Drive folder.
    pub fn mark_in_drive(&mut self, key: &str, path: &str) -> Result<(), OrgError> {
        if !is_key(key) || path.trim().is_empty() {
            return Err(OrgError::Invalid("bad clip key or path".into()));
        }
        let copies = self.drive.entry(key.into()).or_default();
        copies.retain(|p| !p.eq_ignore_ascii_case(path));
        copies.push(path.into());
        if copies.len() > MAX_DRIVE_COPIES {
            copies.remove(0);
        }
        Ok(())
    }

    /// The first remembered Drive copy of `key` that `exists`.
    pub fn drive_copy(&self, key: &str, exists: &dyn Fn(&Path) -> bool) -> Option<&str> {
        self.drive
            .get(key)?
            .iter()
            .rev()
            .find(|p| exists(Path::new(p)))
            .map(String::as_str)
    }

    /// Forgets Drive copies that are gone. Call it only while the Drive
    /// folder is reachable: when Drive isn't running everything looks gone.
    /// Returns true if anything changed.
    pub fn prune_drive(&mut self, exists: &dyn Fn(&Path) -> bool) -> bool {
        let before = self.drive.clone();
        for copies in self.drive.values_mut() {
            copies.retain(|p| exists(Path::new(p)));
        }
        self.drive.retain(|_, v| !v.is_empty());
        before != self.drive
    }

    /// Drops the pending move for `folder` (the user chose to leave the
    /// clips where they are, or they've been moved).
    pub fn resolve_pending(&mut self, folder: &str) {
        self.pending
            .retain(|p| !p.folder.eq_ignore_ascii_case(folder));
    }

    /// Brings the description in line with what's on disk (ADR-0015 §5).
    /// Returns true if anything changed (so the caller saves).
    pub fn reconcile(&mut self, found: &Found) -> bool {
        let before = self.clone();
        let exists = |name: &str| found.dirs.iter().any(|d| d.eq_ignore_ascii_case(name));
        let own_files = |name: &str| -> &[(String, String)] {
            found
                .clips
                .iter()
                .find(|(d, _)| d.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_slice())
                .unwrap_or(&[])
        };
        // Clip keys in a directory and everything inside it.
        let keys_under = |name: &str| -> Vec<String> {
            found
                .clips
                .iter()
                .filter(|(d, _)| d.eq_ignore_ascii_case(name) || is_within(d, name))
                .flat_map(|(_, v)| v.iter().map(|(_, k)| k.clone()))
                .filter(|k| !k.is_empty())
                .collect()
        };
        // 1. Learn game folders: a top-level directory holding a recorder
        //    clip of the same app, unless it's a registered user folder.
        for d in found.dirs.iter().filter(|d| !d.contains('\\')) {
            if self.find(d).is_some() {
                continue;
            }
            if own_files(d)
                .iter()
                .any(|(f, _)| (found.app_of)(f).is_some_and(|a| a.eq_ignore_ascii_case(d)))
            {
                self.game_dirs.insert(d.to_lowercase());
            }
        }
        // 2. Unknown directories, parents first: a top-level one holding
        //    clips (anywhere inside it), or any directory inside a user
        //    folder. Renamed/moved folders are recognised by their clips.
        let mut dirs: Vec<&String> = found.dirs.iter().collect();
        dirs.sort_by_key(|d| depth_of(d));
        for d in dirs {
            if self.find(d).is_some() || self.game_dirs.contains(&d.to_lowercase()) {
                continue;
            }
            match parent_of(d) {
                Some(parent) if self.find(parent).is_none() => continue,
                None if keys_under(d).is_empty() => continue,
                _ => {}
            }
            let here = keys_under(d);
            // A registered folder whose directory is gone, with its clips
            // here: renamed or moved outside the app. Keep its colour/order.
            let renamed = (!here.is_empty())
                .then(|| {
                    self.folders
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| !exists(&f.name))
                        .map(|(i, f)| {
                            let n = self
                                .subtree(&f.name)
                                .iter()
                                .flat_map(|x| x.order.iter())
                                .filter(|k| here.contains(k))
                                .count();
                            (i, n)
                        })
                        .filter(|(_, n)| *n > 0)
                        // Most shared clips; on a tie the folder at the same
                        // depth, then the outermost (a parent before its child).
                        .max_by_key(|(i, n)| {
                            let dep = depth_of(&self.folders[*i].name);
                            (*n, dep == depth_of(d), std::cmp::Reverse(dep))
                        })
                })
                .flatten();
            match renamed {
                Some((i, _)) => {
                    let old = self.folders[i].name.clone();
                    let _ = self.renamed(&old, d);
                }
                None => self.folders.push(Folder {
                    name: d.clone(),
                    color: "gray".into(),
                    order: own_files(d)
                        .iter()
                        .map(|(_, k)| k.clone())
                        .filter(|k| !k.is_empty())
                        .collect(),
                }),
            }
        }
        *self != before
    }

    /// Repairs anything a hand edit could leave behind.
    fn normalize(mut self) -> Self {
        self.version = VERSION;
        let mut seen = std::collections::HashSet::new();
        self.folders
            .retain(|f| folder_path(&f.name).is_ok() && seen.insert(f.name.to_lowercase()));
        for f in &mut self.folders {
            if !COLORS.contains(&f.color.as_str()) {
                f.color = "gray".into();
            }
            let mut s = std::collections::HashSet::new();
            f.order.retain(|c| is_key(c) && s.insert(c.clone()));
        }
        self.titles.retain(|k, v| is_key(k) && !clean(v).is_empty());
        self.pending.retain(|p| !p.keys.is_empty());
        self.drive.retain(|k, v| {
            v.retain(|p| !p.trim().is_empty());
            is_key(k) && !v.is_empty()
        });
        self
    }

    /// Reads a document of any known version. v1 (folders were metadata
    /// only) becomes v2: each folder keeps its colour and order, and its
    /// clips are listed as `pending` — nothing is moved without asking.
    fn from_json(bytes: &[u8]) -> Result<Organize, String> {
        let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let version = v.get("version").and_then(|x| x.as_u64()).unwrap_or(1) as u32;
        if version >= 2 {
            let mut o: Organize = serde_json::from_value(v).map_err(|e| e.to_string())?;
            o.version = version;
            return Ok(o);
        }
        #[derive(Deserialize)]
        struct V1Folder {
            name: String,
            color: String,
            #[serde(default)]
            clips: Vec<String>,
        }
        #[derive(Deserialize)]
        struct V1 {
            #[serde(default)]
            folders: Vec<V1Folder>,
            #[serde(default)]
            titles: BTreeMap<String, String>,
        }
        let old: V1 = serde_json::from_value(v).map_err(|e| e.to_string())?;
        let mut o = Organize::new();
        o.titles = old.titles;
        for f in old.folders {
            // A v1 name may not be a valid directory name: make it one.
            let name = folder_name(&f.name)
                .or_else(|_| {
                    folder_name(
                        f.name
                            .replace(|c: char| r#"<>:"/\|?*"#.contains(c), " ")
                            .trim_end_matches('.'),
                    )
                })
                .unwrap_or_else(|_| "Folder".into());
            if o.find(&name).is_some() {
                continue;
            }
            let keys: Vec<String> = f.clips.into_iter().filter(|k| is_key(k)).collect();
            if !keys.is_empty() {
                o.pending.push(Pending {
                    folder: name.clone(),
                    keys: keys.clone(),
                });
            }
            o.folders.push(Folder {
                name,
                color: f.color,
                order: keys,
            });
        }
        Ok(o)
    }
}

/// Where it's stored, with a backup next to it.
pub struct Store {
    pub path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn backup(&self) -> PathBuf {
        self.path.with_extension("json.bak")
    }

    /// Loads the document.
    /// - Missing: an empty library (or the backup, if only that exists).
    /// - Busy (antivirus, backup tools): retried for ~250 ms, then an error.
    ///   A locked file never falls back to the older backup.
    /// - Damaged (doesn't parse): the backup, if it parses; else an error,
    ///   so a broken file is never silently replaced.
    /// - Made by a newer JondisClip: an error (read-only), so an older build
    ///   never "repairs" data it doesn't understand.
    pub fn load(&self) -> Result<Organize, String> {
        self.load_checked().map(|(o, _)| o)
    }

    /// Like `load`, plus whether the main file itself was good (only then
    /// may it become the backup on the next save).
    fn load_checked(&self) -> Result<(Organize, bool), String> {
        enum Read {
            Missing,
            Busy(String),
            Damaged(String),
            Newer(u32),
            Ok(Organize),
        }
        let read = |p: &Path| -> Read {
            let mut last = String::new();
            for attempt in 0..6 {
                match std::fs::read(p) {
                    Ok(b) => {
                        return match Organize::from_json(&b) {
                            Ok(o) if o.version > VERSION => Read::Newer(o.version),
                            Ok(o) => Read::Ok(o.normalize()),
                            Err(e) => Read::Damaged(format!("{} is damaged: {e}", p.display())),
                        };
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Read::Missing,
                    Err(e) => {
                        last = format!("couldn't read {}: {e}", p.display());
                        if attempt < 5 {
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                    }
                }
            }
            Read::Busy(last)
        };
        let newer = |v: u32| {
            format!(
                "Your folders were saved by a newer JondisClip (format {v}); this version won't change them."
            )
        };
        match read(&self.path) {
            Read::Ok(o) => Ok((o, true)),
            Read::Newer(v) => Err(newer(v)),
            Read::Busy(e) => Err(format!("{e}. Try again in a moment.")),
            Read::Missing => match read(&self.backup()) {
                Read::Ok(o) => Ok((o, false)),
                Read::Newer(v) => Err(newer(v)),
                _ => Ok((Organize::new(), false)),
            },
            Read::Damaged(main) => match read(&self.backup()) {
                Read::Ok(o) => Ok((o, false)),
                Read::Newer(v) => Err(newer(v)),
                _ => Err(main),
            },
        }
    }

    /// Writes atomically: temp file (flushed to disk) then rename. The old
    /// main file becomes the backup only if it was a good one, so a damaged
    /// file can never replace a good backup.
    pub fn save(&self, o: &Organize) -> Result<(), String> {
        let main_ok = self.load_checked().map(|(_, ok)| ok).unwrap_or(false);
        self.save_inner(o, main_ok)
    }

    fn save_inner(&self, o: &Organize, rotate: bool) -> Result<(), String> {
        use std::io::Write;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(o).map_err(|e| e.to_string())?;
        let tmp = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        let written = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()
        })();
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("couldn't save the library: {e}"));
        }
        if rotate && self.path.exists() {
            let _ = std::fs::copy(&self.path, self.backup());
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("couldn't save the library: {e}")
        })
    }

    /// Load, change, save — one step, so callers can't forget to save.
    pub fn update<T>(
        &self,
        f: impl FnOnce(&mut Organize) -> Result<T, OrgError>,
    ) -> Result<(Organize, T), String> {
        let (mut o, main_ok) = self.load_checked()?;
        let out = f(&mut o).map_err(|e| e.to_string())?;
        self.save_inner(&o, main_ok)?;
        Ok((o, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "00000000000000aa";
    const B: &str = "00000000000000bb";
    const C: &str = "00000000000000cc";
    const D: &str = "00000000000000dd";

    fn keys(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn app(f: &str) -> Option<String> {
        // "DayZ 2026-…" → DayZ (enough for these tests)
        f.split(' ')
            .next()
            .filter(|_| f.contains(" 2026-"))
            .map(str::to_owned)
    }

    #[test]
    fn folder_names_are_valid_windows_directory_names() {
        assert_eq!(folder_name("  Funny   Moments ").unwrap(), "Funny Moments");
        assert_eq!(folder_name("Ærø 😂 raids").unwrap(), "Ærø 😂 raids");
        for bad in [
            "",
            "   ",
            "a/b",
            "a\\b",
            "what?",
            "x*",
            "a:b",
            "<x>",
            "pipe|",
            "quote\"",
            "dot.",
            "CON",
            "con.txt",
            "LPT1",
            &"x".repeat(61),
        ] {
            assert!(folder_name(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(
            folder_name("Con artists").is_ok(),
            "only exact device names are reserved"
        );
    }

    #[test]
    fn folders_are_unique_and_coloured_from_the_palette() {
        let mut o = Organize::new();
        o.add_folder("Funny Moments", "purple").unwrap();
        assert!(o.add_folder("funny moments", "red").is_err());
        assert!(o.add_folder("Raids", "pink").is_err());
        o.renamed("Funny Moments", "Funny").unwrap();
        assert_eq!(o.folders[0].name, "Funny");
        o.recolor("funny", "green").unwrap();
        assert_eq!(o.folders[0].color, "green");
        assert_eq!(o.recolor("gone", "red"), Err(OrgError::NotFound));
        o.remove_folder("Funny").unwrap();
        assert!(o.folders.is_empty());
    }

    #[test]
    fn arranging_only_changes_the_order_and_keeps_new_clips() {
        let mut o = Organize::new();
        o.add_folder("Edited", "red").unwrap();
        let present = keys(&[A, B, C]);
        // Stuck (C) to the front.
        o.arrange("Edited", &present, &keys(&[C]), Some(A)).unwrap();
        assert_eq!(o.folders[0].order, keys(&[C, A, B]));
        // A clip that appeared on disk is added at the end of the order.
        let present = keys(&[A, B, C, D]);
        o.arrange("Edited", &present, &keys(&[A]), None).unwrap();
        assert_eq!(o.folders[0].order, keys(&[C, B, D, A]));
        // A selection dropped inside itself stays put.
        o.arrange("Edited", &present, &keys(&[C, B]), Some(B))
            .unwrap();
        assert_eq!(o.folders[0].order, keys(&[C, B, D, A]));
        // Clips that left the folder disappear from the order on the next arrange.
        o.arrange("Edited", &keys(&[A, C]), &keys(&[A]), Some(C))
            .unwrap();
        assert_eq!(o.folders[0].order, keys(&[A, C]));
        assert!(
            o.arrange("Edited", &present, &keys(&["bad"]), None)
                .is_err()
        );
    }

    #[test]
    fn reconcile_learns_games_registers_folders_and_follows_renames() {
        let mut o = Organize::new();
        o.add_folder("Raids", "red").unwrap();
        o.folders[0].order = keys(&[A, B]);
        let dirs: Vec<String> = ["DayZ", "Claude", "Raids", "Explorer Folder", "Empty"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut clips = HashMap::new();
        clips.insert(
            "DayZ".to_string(),
            vec![("DayZ 2026-11-04 18-31-24.mp4".to_string(), C.to_string())],
        );
        clips.insert(
            "Raids".to_string(),
            vec![("DayZ 2026-11-04 10-00-00.mp4".to_string(), A.to_string())],
        );
        clips.insert(
            "Explorer Folder".to_string(),
            vec![("my clip.mp4".to_string(), D.to_string())],
        );
        let found = Found {
            dirs: &dirs,
            clips: &clips,
            app_of: &app,
        };
        assert!(o.reconcile(&found));
        assert!(o.is_game_dir("DayZ"), "recorder folder learned");
        assert!(
            !o.is_game_dir("Raids"),
            "a user folder is never a game folder"
        );
        assert!(
            o.folder("Explorer Folder").is_some(),
            "folder made in Explorer with clips: registered"
        );
        assert!(
            o.folder("Empty").is_none(),
            "empty unknown folders aren't adopted"
        );
        assert!(
            o.folder("Claude").is_none(),
            "an empty game folder isn't a user folder"
        );
        assert!(!o.reconcile(&found), "idempotent");

        // "Raids" renamed in Explorer to "Base Raids": colour and order follow.
        let dirs2: Vec<String> = ["DayZ", "Base Raids", "Explorer Folder"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut clips2 = clips.clone();
        clips2.remove("Raids");
        clips2.insert(
            "Base Raids".to_string(),
            vec![("DayZ 2026-11-04 10-00-00.mp4".to_string(), A.to_string())],
        );
        o.reconcile(&Found {
            dirs: &dirs2,
            clips: &clips2,
            app_of: &app,
        });
        let f = o.folder("Base Raids").expect("renamed");
        assert_eq!((f.color.as_str(), f.order.clone()), ("red", keys(&[A, B])));
        assert!(o.folder("Raids").is_none());

        // A folder whose directory vanished (drive offline) is kept, not purged.
        o.add_folder("Offline", "blue").unwrap();
        o.reconcile(&Found {
            dirs: &dirs2,
            clips: &clips2,
            app_of: &app,
        });
        assert!(o.folder("Offline").is_some());
    }

    #[test]
    fn v1_documents_migrate_without_moving_anything() {
        let v1 = br#"{"version":1,"folders":[{"id":"f1","name":"Edited","color":"red","clips":["00000000000000aa","00000000000000bb"]},
            {"id":"f2","name":"What? Bad/Name.","color":"blue","clips":[]}],"titles":{"00000000000000aa":"BridgeIncident"}}"#;
        let o = Organize::from_json(v1).unwrap().normalize();
        assert_eq!(o.version, 2);
        assert_eq!(o.folders[0].name, "Edited");
        assert_eq!(o.folders[0].order, keys(&[A, B]));
        assert_eq!(
            o.pending,
            vec![Pending {
                folder: "Edited".into(),
                keys: keys(&[A, B])
            }]
        );
        assert_eq!(
            o.folders[1].name, "What Bad Name",
            "made a valid directory name"
        );
        assert_eq!(o.titles[A], "BridgeIncident");
        // Moving one clip in resolves it from the pending list.
        let mut o = o;
        o.arrived("Edited", &keys(&[A]));
        assert_eq!(o.pending[0].keys, keys(&[B]));
        o.resolve_pending("Edited");
        assert!(o.pending.is_empty());
    }

    #[test]
    fn store_migrates_v1_keeps_backups_and_never_overwrites_newer_or_broken() {
        let d = std::env::temp_dir().join(format!("jc-org-v2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let s = Store::new(d.join("organize.json"));
        assert_eq!(s.load().unwrap(), Organize::new());
        std::fs::write(&s.path, br#"{"version":1,"folders":[{"id":"f","name":"Edited","color":"red","clips":["00000000000000aa"]}],"titles":{}}"#).unwrap();
        let (o, _) = s.update(|o| o.recolor("Edited", "purple")).unwrap();
        assert_eq!(
            (o.version, o.folders[0].color.as_str(), o.pending.len()),
            (2, "purple", 1)
        );
        // Newer: read-only.
        let newer = br#"{"version":99,"folders":[]}"#;
        std::fs::write(&s.path, newer).unwrap();
        assert!(s.load().unwrap_err().contains("newer"));
        assert!(s.update(|o| o.add_folder("X", "red")).is_err());
        assert_eq!(std::fs::read(&s.path).unwrap(), newer);
        // Damaged main: the good backup is used and kept.
        std::fs::write(&s.path, b"{ broken").unwrap();
        let bak = std::fs::read(s.path.with_extension("json.bak")).unwrap();
        s.update(|o| o.add_folder("Funny", "green")).unwrap();
        assert_eq!(
            std::fs::read(s.path.with_extension("json.bak")).unwrap(),
            bak
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn titles_are_cleaned_limited_and_clearable() {
        let mut o = Organize::new();
        o.set_title(A, "  The\tWorst   Grenade\nEver ").unwrap();
        assert_eq!(o.titles[A], "The Worst Grenade Ever");
        assert!(o.set_title(A, &"x".repeat(121)).is_err());
        o.set_title(A, "   ").unwrap();
        assert!(!o.titles.contains_key(A));
    }

    #[test]
    fn drive_copies_are_remembered_shown_and_pruned() {
        let mut o = Organize::new();
        let a = r"G:\My Drive\JondisClip\DayZ\Sniper.mp4";
        let b = r"G:\My Drive\JondisClip\DayZ\Sniper edit.mp4";
        o.mark_in_drive(A, a).unwrap();
        o.mark_in_drive(A, b).unwrap();
        o.mark_in_drive(A, &a.to_uppercase()).unwrap(); // same file: no duplicate
        assert_eq!(o.drive[A].len(), 2);
        assert!(o.mark_in_drive("nope", a).is_err());
        // Shown while a copy exists (newest first).
        assert_eq!(o.drive_copy(A, &|_| true), Some(a.to_uppercase().as_str()));
        assert_eq!(
            o.drive_copy(A, &|p| p.ends_with("Sniper edit.mp4")),
            Some(b)
        );
        assert_eq!(o.drive_copy(A, &|_| false), None);
        assert_eq!(o.drive_copy(B, &|_| true), None);
        // Pruning forgets the ones that are gone, and the clip once all are.
        assert!(o.prune_drive(&|p| p.ends_with("Sniper edit.mp4")));
        assert_eq!(o.drive[A], vec![b.to_string()]);
        assert!(!o.prune_drive(&|_| true));
        assert!(o.prune_drive(&|_| false));
        assert!(o.drive.is_empty());
        // Bounded per clip.
        for i in 0..20 {
            o.mark_in_drive(A, &format!(r"G:\d\{i}.mp4")).unwrap();
        }
        assert_eq!(o.drive[A].len(), MAX_DRIVE_COPIES);
        assert_eq!(o.drive[A][0], r"G:\d\12.mp4");
    }

    #[test]
    fn drive_copies_survive_a_save_and_load() {
        let d = std::env::temp_dir().join(format!("jc-org-drive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let s = Store::new(d.join("organize.json"));
        s.update(|o| o.mark_in_drive(A, r"G:\My Drive\JondisClip\DayZ\x.mp4"))
            .unwrap();
        let o = s.load().unwrap();
        assert_eq!(
            o.drive[A],
            vec![r"G:\My Drive\JondisClip\DayZ\x.mp4".to_string()]
        );
        // A document without the field (older builds) loads with none.
        std::fs::write(&s.path, br#"{"version":2,"folders":[]}"#).unwrap();
        assert!(s.load().unwrap().drive.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn folder_order_follows_the_given_names() {
        let mut o = Organize::new();
        for n in ["A", "B", "C"] {
            o.add_folder(n, "gray").unwrap();
        }
        o.reorder_folders(&keys(&["c", "A", "ghost"]));
        let names: Vec<_> = o.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["C", "A", "B"]);
    }

    #[test]
    fn folder_paths_nest_up_to_three_levels() {
        assert_eq!(folder_path("Edited/ DayZ ").unwrap(), "Edited\\DayZ");
        assert_eq!(folder_path("A\\B\\C").unwrap(), "A\\B\\C");
        assert!(folder_path("A\\B\\C\\D").is_err());
        assert!(folder_path("A\\\\B").is_err(), "empty part");
        assert!(folder_path("A\\CON").is_err());
        assert_eq!(parent_of("A\\B\\C"), Some("A\\B"));
        assert_eq!(leaf_of("A\\B"), "B");
        assert!(is_within("Edited\\DayZ", "edited"));
        assert!(!is_within("EditedX\\DayZ", "Edited"));
        assert!(!is_within("Edited", "Edited"));
    }

    #[test]
    fn subfolders_need_a_parent_and_follow_it_on_rename_and_delete() {
        let mut o = Organize::new();
        assert_eq!(o.add_folder("Edited\\DayZ", "red"), Err(OrgError::NotFound));
        o.add_folder("Edited", "red").unwrap();
        o.add_folder("Edited\\DayZ", "green").unwrap();
        o.add_folder("Edited\\DayZ\\Raids", "blue").unwrap();
        assert!(o.add_folder("edited\\dayz", "red").is_err(), "duplicate");
        o.pending.push(Pending {
            folder: "Edited\\DayZ".into(),
            keys: keys(&[A]),
        });
        o.renamed("Edited", "Edits").unwrap();
        let names: Vec<_> = o.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Edits", "Edits\\DayZ", "Edits\\DayZ\\Raids"]);
        assert_eq!(o.pending[0].folder, "Edits\\DayZ");
        // Moving a folder into another one carries its subfolders along.
        o.add_folder("Dota", "orange").unwrap();
        o.renamed("Edits\\DayZ", "Dota\\DayZ").unwrap();
        assert!(o.folder("Dota\\DayZ\\Raids").is_some());
        assert!(o.renamed("Dota\\DayZ", "Edits").is_err(), "name taken");
        o.remove_folder("Dota").unwrap();
        let names: Vec<_> = o.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Edits"]);
        assert!(o.pending.is_empty());
        assert!(!o.is_game_dir("Edits\\DayZ"));
    }

    #[test]
    fn reconcile_adopts_subfolders_and_follows_a_parent_renamed_in_explorer() {
        let mut o = Organize::new();
        o.add_folder("Edited", "purple").unwrap();
        o.add_folder("Edited\\DayZ", "green").unwrap();
        o.folders[1].order = keys(&[A, B]);
        o.game_dirs.insert("dayz".into());
        let dirs: Vec<String> = [
            "DayZ",
            "DayZ\\Old",
            "Edited",
            "Edited\\DayZ",
            "Edited\\Dota",
            "Edited\\DayZ\\Best",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut clips = HashMap::new();
        clips.insert(
            "Edited\\DayZ".to_string(),
            vec![
                ("x.mp4".to_string(), A.to_string()),
                ("y.mp4".to_string(), B.to_string()),
            ],
        );
        clips.insert(
            "DayZ\\Old".to_string(),
            vec![("z.mp4".to_string(), C.to_string())],
        );
        let found = Found {
            dirs: &dirs,
            clips: &clips,
            app_of: &app,
        };
        o.reconcile(&found);
        assert!(
            o.folder("Edited\\Dota").is_some(),
            "empty subfolder made in Explorer: adopted"
        );
        assert!(o.folder("Edited\\DayZ\\Best").is_some());
        assert!(
            o.folder("DayZ\\Old").is_none(),
            "nothing inside a game folder"
        );
        assert!(!o.reconcile(&found), "idempotent");

        // "Edited" renamed to "Edits" in Explorer: it has no clips of its
        // own, but its subfolder does, so colours and order follow.
        let dirs2: Vec<String> = [
            "DayZ",
            "Edits",
            "Edits\\DayZ",
            "Edits\\Dota",
            "Edits\\DayZ\\Best",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut clips2 = HashMap::new();
        clips2.insert(
            "Edits\\DayZ".to_string(),
            vec![
                ("x.mp4".to_string(), A.to_string()),
                ("y.mp4".to_string(), B.to_string()),
            ],
        );
        o.reconcile(&Found {
            dirs: &dirs2,
            clips: &clips2,
            app_of: &app,
        });
        assert_eq!(o.folder("Edits").map(|f| f.color.as_str()), Some("purple"));
        let f = o.folder("Edits\\DayZ").expect("followed");
        assert_eq!(
            (f.color.as_str(), f.order.clone()),
            ("green", keys(&[A, B]))
        );
        assert!(o.folder("Edits\\Dota").is_some());
        assert!(o.folder("Edited").is_none());
    }
}
