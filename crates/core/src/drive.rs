//! "Save to Drive" destination logic (ADR-0013). Pure and testable: the
//! window does the copying. JondisClip only writes into the folder Google
//! Drive for desktop syncs; Drive does the upload.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Folders Google Drive for desktop creates, if present: `X:\My Drive`
/// (streaming, the default) and `%USERPROFILE%\My Drive` (mirroring).
pub fn suggestions(exists: &dyn Fn(&Path) -> bool, home: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for letter in b'D'..=b'Z' {
        let p = PathBuf::from(format!(r"{}:\My Drive", letter as char));
        if exists(&p) {
            out.push(p);
        }
    }
    if let Some(h) = home {
        for name in ["My Drive", "Google Drive"] {
            let p = h.join(name);
            if exists(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// What to do for a clip of `size` bytes named `file_name` from `game`.
#[derive(Debug, PartialEq)]
pub enum Target {
    /// Write here (doesn't exist yet).
    Write(PathBuf),
    /// The same file (name and size) is already there: nothing to copy.
    AlreadyThere(PathBuf),
}

/// Size and modified time of a file.
pub type Stamp = (u64, Option<SystemTime>);

/// Same file? Size must match, and so must the modified time (our copies
/// carry the source's), within FAT/exFAT's 2 s precision.
fn same(a: Stamp, b: Stamp) -> bool {
    let close = match (a.1, b.1) {
        (Some(x), Some(y)) => x
            .duration_since(y)
            .or_else(|_| y.duration_since(x))
            .is_ok_and(|d| d.as_secs() <= 2),
        _ => false,
    };
    a.0 == b.0 && close
}

/// `<drive>\JondisClip\<game>\<file_name>`; ` (2)`… if a different file
/// already has that name. `source` = None for a trim (not written yet),
/// which never counts as "already there".
pub fn target(
    drive: &Path,
    game: &str,
    file_name: &str,
    source: Option<Stamp>,
    existing: &dyn Fn(&Path) -> Option<Stamp>,
) -> Target {
    // `game` may be a folder path (`Edited\DayZ`): each part is cleaned.
    let mut dir = drive.join("JondisClip");
    for part in game.split('\\') {
        dir.push(sanitize(part));
    }
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (file_name.to_string(), String::new()),
    };
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem}{ext}")
        } else {
            format!("{stem} ({n}){ext}")
        };
        let p = dir.join(name);
        match existing(&p) {
            None if existing(&p.with_extension("mp4.partial")).is_none() => {
                return Target::Write(p);
            }
            Some(s) if source.is_some_and(|src| same(s, src)) => return Target::AlreadyThere(p),
            _ => n += 1,
        }
    }
}

fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if r#"<>:"/\|?*"#.contains(c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let t = t.trim().trim_matches('.').to_string();
    if t.is_empty() { "Clips".into() } else { t }
}

/// True if either folder is inside the other (or they're the same), after
/// resolving links/junctions where the folders exist. Case-insensitive.
pub fn overlaps(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        PathBuf::from(
            p.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_lowercase()
                .trim_end_matches('\\'),
        )
    };
    if a.as_os_str().is_empty() || b.as_os_str().is_empty() {
        return false;
    }
    let (a, b) = (norm(a), norm(b));
    a.starts_with(&b) || b.starts_with(&a)
}

/// Leftover `.partial` files in our part of the Drive folder (a copy or
/// trim cut short by closing the window). Only `<drive>\JondisClip\…`.
pub fn stale_partials(drive: &Path) -> Vec<PathBuf> {
    // `JondisClip\<folder path, up to MAX_DEPTH>\*.mp4.partial`.
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() && !t.is_symlink() && depth < crate::library::MAX_DEPTH {
                walk(&e.path(), depth + 1, out);
            } else if t.is_file()
                && depth > 0
                && e.file_name()
                    .to_string_lossy()
                    .to_lowercase()
                    .ends_with(".mp4.partial")
            {
                out.push(e.path());
            }
        }
    }
    let mut out = Vec::new();
    walk(&drive.join("JondisClip"), 0, &mut out);
    out
}

/// Tries writing, reading back and deleting a small file in `dir`
/// (`.jondisclip-write-test-<pid>.tmp`). Ok(()) if all of that worked.
pub fn write_test(dir: &Path) -> Result<(), String> {
    let p = dir.join(format!(".jondisclip-write-test-{}.tmp", std::process::id()));
    let data = b"JondisClip write test";
    let res = (|| {
        std::fs::write(&p, data)?;
        let back = std::fs::read(&p)?;
        if back != data {
            return Err(std::io::Error::other("the file read back differently"));
        }
        Ok(())
    })();
    let removed = std::fs::remove_file(&p);
    match (res, removed) {
        (Err(e), _) => Err(match e.kind() {
            std::io::ErrorKind::PermissionDenied => {
                crate::tr!("JondisClip isn't allowed to write there.")
            }
            _ => crate::tr!("Writing there failed: {error}", error = e),
        }),
        (Ok(()), Err(e)) if e.kind() != std::io::ErrorKind::NotFound => Err(crate::tr!(
            "A test file couldn't be removed again: {error}",
            error = e
        )),
        _ => Ok(()),
    }
}

/// Why a Drive folder can't be used, if it can't. It must not overlap the
/// clips folder (copies would be taken for recordings) or JondisClip's own
/// folders (`own_dir`: buffer, library cache, share).
pub fn check_folder(
    drive: &Path,
    save_dir: &Path,
    own_dir: &Path,
    is_dir: &dyn Fn(&Path) -> bool,
) -> Result<(), String> {
    if drive.as_os_str().is_empty() {
        return Err(crate::tr!(
            "Choose your Google Drive folder in Settings → Google Drive first."
        ));
    }
    if !is_dir(drive) {
        return Err(crate::tr!(
            "Google Drive isn't available ({path} not found). Is Google Drive for desktop running and signed in?",
            path = drive.display()
        ));
    }
    if overlaps(drive, save_dir) {
        return Err(crate::tr!(
            "The Drive folder overlaps your clips folder. Choose a separate folder for Drive."
        ));
    }
    if overlaps(drive, own_dir) {
        return Err(crate::tr!(
            "The Drive folder can't be inside JondisClip's own folders."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn write_test_leaves_nothing_behind() {
        let d = std::env::temp_dir().join(format!("jc-drive-wt-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        write_test(&d).unwrap();
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
        assert!(write_test(&d.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn suggests_drive_for_desktop_folders_that_exist() {
        let have = [r"G:\My Drive", r"C:\Users\you\My Drive"];
        let exists = |p: &Path| have.iter().any(|h| Path::new(h) == p);
        let s = suggestions(&exists, Some(Path::new(r"C:\Users\you")));
        assert_eq!(
            s,
            [
                PathBuf::from(r"G:\My Drive"),
                PathBuf::from(r"C:\Users\you\My Drive")
            ]
        );
        assert!(suggestions(&|_| false, None).is_empty());
    }

    #[test]
    fn targets_go_under_jondisclip_by_game_and_avoid_duplicates() {
        let t0 = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000);
        let files: HashMap<PathBuf, Stamp> = [(
            PathBuf::from(r"G:\My Drive\JondisClip\DayZ\DayZ 2026-11-04 15-27-02.mp4"),
            (100, Some(t0)),
        )]
        .into();
        let size = |p: &Path| files.get(p).copied();
        let drive = Path::new(r"G:\My Drive");
        let name = "DayZ 2026-11-04 15-27-02.mp4";
        // Same name, size and time (within FAT's 2 s): already there.
        let later = t0 + std::time::Duration::from_secs(1);
        assert_eq!(
            target(drive, "DayZ", name, Some((100, Some(later))), &size),
            Target::AlreadyThere(PathBuf::from(
                r"G:\My Drive\JondisClip\DayZ\DayZ 2026-11-04 15-27-02.mp4"
            ))
        );
        let numbered = Target::Write(PathBuf::from(
            r"G:\My Drive\JondisClip\DayZ\DayZ 2026-11-04 15-27-02 (2).mp4",
        ));
        // Same size but a different time: a different file, so numbered.
        let other_day = t0 + std::time::Duration::from_secs(86_400);
        assert_eq!(
            target(drive, "DayZ", name, Some((100, Some(other_day))), &size),
            numbered
        );
        // Different size: numbered.
        assert_eq!(
            target(drive, "DayZ", name, Some((999, Some(t0))), &size),
            numbered
        );
        // A trim (size unknown) never counts as already there.
        assert!(matches!(
            target(drive, "DayZ", "DayZ 2026-11-04 15-27-02.mp4", None, &size),
            Target::Write(_)
        ));
        // Odd game names become safe folder names.
        assert_eq!(
            target(drive, "A:B/C", "x.mp4", None, &|_| None),
            Target::Write(PathBuf::from(r"G:\My Drive\JondisClip\A B C\x.mp4"))
        );
    }

    #[test]
    fn refuses_missing_unset_or_overlapping_folders() {
        let dirs = |p: &Path| p == Path::new(r"G:\My Drive") || p == Path::new(r"D:\Clips\Drive");
        let (save, share) = (
            Path::new(r"D:\Clips"),
            Path::new(r"C:\Users\you\AppData\Local\JondisClip\share"),
        );
        assert!(
            check_folder(Path::new(""), save, share, &dirs)
                .unwrap_err()
                .contains("Settings")
        );
        assert!(
            check_folder(Path::new(r"H:\My Drive"), save, share, &dirs)
                .unwrap_err()
                .contains("isn't available")
        );
        assert!(
            check_folder(Path::new(r"D:\Clips\Drive"), save, share, &dirs)
                .unwrap_err()
                .contains("overlaps")
        );
        assert!(check_folder(Path::new(r"G:\My Drive"), save, share, &dirs).is_ok());
        assert!(
            check_folder(Path::new(r"G:\My Drive"), Path::new(r"G:\"), share, &dirs).is_err(),
            "save folder containing Drive"
        );
        // Anything under JondisClip's own folder is refused, not just share.
        let own = Path::new(r"C:\Users\you\AppData\Local\JondisClip");
        let inside = |p: &Path| p.starts_with(own);
        assert!(check_folder(&own.join("buffer"), save, own, &inside).is_err());
        assert!(
            overlaps(Path::new(r"C:\A\b"), Path::new(r"c:\a")),
            "case-insensitive"
        );
        assert!(
            !overlaps(Path::new(r"C:\Ab"), Path::new(r"C:\A")),
            "whole components only"
        );
        assert!(!overlaps(Path::new(""), Path::new(r"C:\A")));
    }

    #[test]
    fn finds_only_our_leftover_partials() {
        let d = std::env::temp_dir().join(format!("jc-drive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("JondisClip").join("DayZ")).unwrap();
        std::fs::write(
            d.join("JondisClip").join("DayZ").join("a.mp4.partial"),
            b"x",
        )
        .unwrap();
        std::fs::write(d.join("JondisClip").join("DayZ").join("a.mp4"), b"x").unwrap();
        std::fs::write(d.join("someone else's.mp4.partial"), b"x").unwrap();
        let found = stale_partials(&d);
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("a.mp4.partial"));
        assert!(stale_partials(Path::new(r"Q:\nope")).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
