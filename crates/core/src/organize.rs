//! Library organisation (ADR-0014): personal folders (with colour and a
//! manual clip order) and clip titles, as a small JSON document. Recordings
//! never move. Clips are identified by a content fingerprint
//! (`library::fingerprint`), so titles and folders survive renames and moves
//! outside JondisClip.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const MAX_TITLE: usize = 120;
pub const MAX_FOLDER_NAME: usize = 60;
/// The palette (muted tokens in the UI). Anything else is refused.
pub const COLORS: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "gray"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Clip keys: membership *and* custom order.
    #[serde(default)]
    pub clips: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Organize {
    pub version: u32,
    /// In display order.
    #[serde(default)]
    pub folders: Vec<Folder>,
    /// Clip key → title.
    #[serde(default)]
    pub titles: std::collections::BTreeMap<String, String>,
}

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

fn is_key(k: &str) -> bool {
    k.len() == 16 && k.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Organize {
    pub fn new() -> Self {
        Self {
            version: VERSION,
            ..Default::default()
        }
    }

    fn folder_mut(&mut self, id: &str) -> Result<&mut Folder, OrgError> {
        self.folders
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or(OrgError::NotFound)
    }

    fn check_name(&self, name: &str, except: Option<&str>) -> Result<String, OrgError> {
        let name = clean(name);
        if name.is_empty() {
            return Err(OrgError::Invalid("Give the folder a name.".into()));
        }
        if name.chars().count() > MAX_FOLDER_NAME {
            return Err(OrgError::Invalid(format!(
                "Folder names can be up to {MAX_FOLDER_NAME} characters."
            )));
        }
        let taken = self
            .folders
            .iter()
            .any(|f| Some(f.id.as_str()) != except && f.name.to_lowercase() == name.to_lowercase());
        if taken {
            return Err(OrgError::Invalid(format!(
                "There's already a folder called \"{name}\"."
            )));
        }
        Ok(name)
    }

    fn check_color(color: &str) -> Result<String, OrgError> {
        COLORS
            .iter()
            .find(|c| **c == color)
            .map(|c| c.to_string())
            .ok_or_else(|| OrgError::Invalid(format!("unknown colour {color}")))
    }

    /// Creates a folder at the end; returns its id. `id` must be fresh
    /// (the caller supplies randomness so this stays testable).
    pub fn create_folder(&mut self, id: &str, name: &str, color: &str) -> Result<String, OrgError> {
        let name = self.check_name(name, None)?;
        let color = Self::check_color(color)?;
        if self.folders.iter().any(|f| f.id == id) || id.is_empty() {
            return Err(OrgError::Invalid("folder id clash".into()));
        }
        self.folders.push(Folder {
            id: id.into(),
            name,
            color,
            clips: Vec::new(),
        });
        Ok(id.into())
    }

    pub fn rename_folder(&mut self, id: &str, name: &str) -> Result<(), OrgError> {
        let name = self.check_name(name, Some(id))?;
        self.folder_mut(id)?.name = name;
        Ok(())
    }

    pub fn recolor_folder(&mut self, id: &str, color: &str) -> Result<(), OrgError> {
        let color = Self::check_color(color)?;
        self.folder_mut(id)?.color = color;
        Ok(())
    }

    /// Removes the folder only. Its clips simply aren't in a folder any more;
    /// no recording is touched (ADR-0014).
    pub fn delete_folder(&mut self, id: &str) -> Result<Folder, OrgError> {
        let i = self
            .folders
            .iter()
            .position(|f| f.id == id)
            .ok_or(OrgError::NotFound)?;
        Ok(self.folders.remove(i))
    }

    /// Puts the folders in `ids` order. Unknown ids are ignored; folders
    /// missing from `ids` keep their relative order at the end.
    pub fn reorder_folders(&mut self, ids: &[String]) {
        let mut out = Vec::with_capacity(self.folders.len());
        for id in ids {
            if let Some(i) = self.folders.iter().position(|f| &f.id == id) {
                out.push(self.folders.remove(i));
            }
        }
        out.append(&mut self.folders);
        self.folders = out;
    }

    /// Moves `keys` into folder `to` (None = out of every folder), inserted
    /// before `before` if it's in that folder, else at the end. A clip is in
    /// at most one folder. Order of `keys` is kept.
    pub fn move_clips(
        &mut self,
        keys: &[String],
        to: Option<&str>,
        before: Option<&str>,
    ) -> Result<(), OrgError> {
        if keys.iter().any(|k| !is_key(k)) {
            return Err(OrgError::Invalid("bad clip key".into()));
        }
        if let Some(t) = to
            && !self.folders.iter().any(|f| f.id == t)
        {
            return Err(OrgError::NotFound);
        }
        // If `before` is one of the clips being moved (dropping a selection
        // inside itself), anchor on the next clip after it that stays put.
        let before: Option<String> = match (to, before) {
            (Some(t), Some(b)) if keys.iter().any(|k| k == b) => {
                self.folders.iter().find(|f| f.id == t).and_then(|f| {
                    let at = f.clips.iter().position(|c| c == b)?;
                    f.clips[at..].iter().find(|c| !keys.contains(c)).cloned()
                })
            }
            (_, b) => b.map(str::to_owned),
        };
        let before = before.as_deref();
        for f in &mut self.folders {
            f.clips.retain(|c| !keys.contains(c));
        }
        if let Some(t) = to {
            let f = self.folder_mut(t)?;
            let at = before
                .filter(|b| !keys.iter().any(|k| k == b))
                .and_then(|b| f.clips.iter().position(|c| c == b))
                .unwrap_or(f.clips.len());
            let mut seen = std::collections::HashSet::new();
            let unique: Vec<String> = keys
                .iter()
                .filter(|k| seen.insert(k.as_str()))
                .cloned()
                .collect();
            f.clips.splice(at..at, unique);
        }
        Ok(())
    }

    /// Which folder a clip is in, if any.
    pub fn folder_of(&self, key: &str) -> Option<&Folder> {
        self.folders
            .iter()
            .find(|f| f.clips.iter().any(|c| c == key))
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

    /// Repairs anything a hand edit or an old version could leave behind:
    /// duplicate ids/names, clips listed in two folders, bad colours.
    fn normalize(mut self) -> Self {
        self.version = VERSION;
        let mut seen_clips = std::collections::HashSet::new();
        let mut seen_ids = std::collections::HashSet::new();
        let mut seen_names = std::collections::HashSet::new();
        self.folders
            .retain(|f| seen_ids.insert(f.id.clone()) && !f.id.is_empty());
        for f in &mut self.folders {
            f.name = clean(&f.name);
            if f.name.is_empty() {
                f.name = "Folder".into();
            }
            let base = f.name.clone();
            let mut n = 2;
            while !seen_names.insert(f.name.to_lowercase()) {
                f.name = format!("{base} {n}");
                n += 1;
            }
            if !COLORS.contains(&f.color.as_str()) {
                f.color = "gray".into();
            }
            f.clips
                .retain(|c| is_key(c) && seen_clips.insert(c.clone()));
        }
        self.titles.retain(|k, v| is_key(k) && !clean(v).is_empty());
        self
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
                        return match serde_json::from_slice::<Organize>(&b) {
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

    fn keys(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn folders_are_validated_and_unique() {
        let mut o = Organize::new();
        o.create_folder("f1", "  DayZ   Raids ", "red").unwrap();
        assert_eq!(o.folders[0].name, "DayZ Raids");
        assert!(
            o.create_folder("f2", "dayz raids", "blue").is_err(),
            "names are unique ignoring case"
        );
        assert!(o.create_folder("f2", "", "blue").is_err());
        assert!(o.create_folder("f2", &"x".repeat(61), "blue").is_err());
        assert!(
            o.create_folder("f2", "Friends", "pink").is_err(),
            "palette only"
        );
        assert!(
            o.create_folder("f1", "Friends", "blue").is_err(),
            "id clash"
        );
        o.create_folder("f2", "Friends", "blue").unwrap();
        o.rename_folder("f2", "Best Kills").unwrap();
        assert!(o.rename_folder("f2", "DAYZ RAIDS").is_err());
        o.rename_folder("f1", "dayz raids").unwrap(); // its own name, other case: fine
        o.recolor_folder("f1", "green").unwrap();
        assert_eq!(o.rename_folder("nope", "x"), Err(OrgError::NotFound));
    }

    #[test]
    fn moving_keeps_one_folder_per_clip_and_the_order() {
        let mut o = Organize::new();
        o.create_folder("f1", "Raids", "red").unwrap();
        o.create_folder("f2", "Funny", "green").unwrap();
        o.move_clips(&keys(&[A, B]), Some("f1"), None).unwrap();
        o.move_clips(&keys(&[C]), Some("f1"), Some(B)).unwrap();
        assert_eq!(o.folders[0].clips, keys(&[A, C, B]), "inserted before B");
        // Reorder inside: move B before A.
        o.move_clips(&keys(&[B]), Some("f1"), Some(A)).unwrap();
        assert_eq!(o.folders[0].clips, keys(&[B, A, C]));
        // Between folders: leaves the first.
        o.move_clips(&keys(&[A, C]), Some("f2"), None).unwrap();
        assert_eq!(o.folders[0].clips, keys(&[B]));
        assert_eq!(o.folders[1].clips, keys(&[A, C]));
        assert_eq!(o.folder_of(A).unwrap().id, "f2");
        // Out of all folders.
        o.move_clips(&keys(&[A]), None, None).unwrap();
        assert!(o.folder_of(A).is_none());
        // Dropping onto itself, duplicates and bad input.
        o.move_clips(&keys(&[C, C]), Some("f2"), Some(C)).unwrap();
        assert_eq!(o.folders[1].clips, keys(&[C]));
        assert_eq!(
            o.move_clips(&keys(&[A]), Some("gone"), None),
            Err(OrgError::NotFound)
        );
        assert!(o.move_clips(&keys(&["../x"]), None, None).is_err());
    }

    #[test]
    fn moving_a_selection_inside_itself_keeps_its_place() {
        const D: &str = "00000000000000dd";
        let mut o = Organize::new();
        o.create_folder("f1", "Raids", "red").unwrap();
        o.move_clips(&keys(&[A, B, C, D]), Some("f1"), None)
            .unwrap();
        // Select A and B, drop "before B": stays where it is, not at the end.
        o.move_clips(&keys(&[A, B]), Some("f1"), Some(B)).unwrap();
        assert_eq!(o.folders[0].clips, keys(&[A, B, C, D]));
        // Select B and D, drop before D (the last): goes to the end.
        o.move_clips(&keys(&[B, D]), Some("f1"), Some(D)).unwrap();
        assert_eq!(o.folders[0].clips, keys(&[A, C, B, D]));
    }

    #[test]
    fn locked_newer_and_damaged_files_are_never_overwritten() {
        let d = std::env::temp_dir().join(format!("jc-org3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let s = Store::new(d.join("organize.json"));
        s.update(|o| o.create_folder("f1", "Raids", "red")).unwrap();
        s.update(|o| o.create_folder("f2", "Funny", "green"))
            .unwrap();
        // Newer format: refused, file untouched.
        let newer = br#"{"version":99,"folders":[],"titles":{},"future":true}"#;
        std::fs::write(&s.path, newer).unwrap();
        assert!(s.load().unwrap_err().contains("newer"));
        assert!(s.update(|o| o.create_folder("f3", "X", "red")).is_err());
        assert_eq!(std::fs::read(&s.path).unwrap(), newer);
        // Damaged main: falls back to the backup, and saving then does NOT
        // copy the damaged file over the good backup.
        std::fs::write(&s.path, b"{ broken").unwrap();
        let good_bak = std::fs::read(s.path.with_extension("json.bak")).unwrap();
        s.update(|o| o.create_folder("f3", "Friends", "blue"))
            .unwrap();
        assert_eq!(
            std::fs::read(s.path.with_extension("json.bak")).unwrap(),
            good_bak,
            "good backup kept"
        );
        assert!(
            s.load()
                .unwrap()
                .folders
                .iter()
                .any(|f| f.name == "Friends")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn deleting_a_folder_keeps_titles_and_never_lists_clips_elsewhere() {
        let mut o = Organize::new();
        o.create_folder("f1", "Raids", "red").unwrap();
        o.move_clips(&keys(&[A]), Some("f1"), None).unwrap();
        o.set_title(A, "Our Base Got Raided").unwrap();
        let removed = o.delete_folder("f1").unwrap();
        assert_eq!(removed.clips, keys(&[A]));
        assert!(o.folders.is_empty() && o.folder_of(A).is_none());
        assert_eq!(o.titles[A], "Our Base Got Raided");
    }

    #[test]
    fn titles_are_cleaned_limited_and_clearable() {
        let mut o = Organize::new();
        o.set_title(A, "  The\tWorst   Grenade\nEver ").unwrap();
        assert_eq!(o.titles[A], "The Worst Grenade Ever");
        o.set_title(A, "Ærlig talt 😂 — \"quotes\" / slashes: fine")
            .unwrap();
        assert!(o.titles[A].contains('😂'));
        assert!(o.set_title(A, &"x".repeat(121)).is_err());
        o.set_title(A, "   ").unwrap();
        assert!(!o.titles.contains_key(A));
    }

    #[test]
    fn folder_order_follows_the_given_ids() {
        let mut o = Organize::new();
        for (i, n) in ["A", "B", "C"].iter().enumerate() {
            o.create_folder(&format!("f{i}"), n, "gray").unwrap();
        }
        o.reorder_folders(&keys(&["f2", "f0", "ghost"]));
        let names: Vec<_> = o.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["C", "A", "B"]);
    }

    #[test]
    fn store_round_trips_atomically_and_survives_damage() {
        let d = std::env::temp_dir().join(format!("jc-org-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let s = Store::new(d.join("organize.json"));
        assert_eq!(
            s.load().unwrap(),
            Organize::new(),
            "missing file = empty library"
        );
        s.update(|o| o.create_folder("f1", "Raids", "red")).unwrap();
        s.update(|o| o.set_title(A, "Driving Disaster")).unwrap();
        let o = s.load().unwrap();
        assert_eq!(
            (o.folders.len(), o.titles[A].as_str()),
            (1, "Driving Disaster")
        );
        assert!(s.path.with_extension("json.bak").exists());
        // A damaged file falls back to the backup instead of an empty library.
        std::fs::write(&s.path, b"{ not json").unwrap();
        let o = s.load().unwrap();
        assert_eq!(o.folders.len(), 1, "backup used");
        // Both damaged: an error, never a silent reset.
        std::fs::write(s.path.with_extension("json.bak"), b"also broken").unwrap();
        assert!(s.load().is_err());
        assert!(
            s.update(|o| o.create_folder("f2", "X", "red")).is_err(),
            "refuses to overwrite"
        );
        assert_eq!(std::fs::read(&s.path).unwrap(), b"{ not json");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hand_edited_files_are_repaired_on_load() {
        let json = r#"{"version":1,"folders":[
            {"id":"f1","name":"Raids","color":"neon","clips":["00000000000000aa","00000000000000aa","bad"]},
            {"id":"f1","name":"Dup id","color":"red"},
            {"id":"f2","name":"raids","color":"red","clips":["00000000000000aa","00000000000000bb"]}],
            "titles":{"00000000000000aa":"  ","oops":"x","00000000000000bb":"Kept"}}"#;
        let d = std::env::temp_dir().join(format!("jc-org2-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let s = Store::new(d.join("organize.json"));
        std::fs::write(&s.path, json).unwrap();
        let o = s.load().unwrap();
        assert_eq!(o.folders.len(), 2);
        assert_eq!(o.folders[0].color, "gray");
        assert_eq!(o.folders[0].clips, keys(&[A]));
        assert_eq!(o.folders[1].name, "raids 2", "duplicate names made unique");
        assert_eq!(o.folders[1].clips, keys(&[B]), "a clip lives in one folder");
        assert_eq!(o.titles.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
