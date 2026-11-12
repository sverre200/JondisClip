//! Updates from GitHub Releases (ADR-0020).
//!
//! The check is one anonymous HTTPS GET for the latest release's version; it
//! sends nothing about this PC or its clips. Installing is always the user's
//! click. Windows' own `curl.exe` and `tar.exe` (in System32 since Windows 10
//! 1803) do the network and unzip work, so the tray gains no HTTP or zip code
//! and stays small. Nothing here runs on the capture path.

use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `owner/name` of the GitHub repository that publishes the releases.
pub const REPO: &str = "sverre200/JondisClip";
/// Release assets must come from here; anything else in the API answer is refused.
const DOWNLOAD_PREFIX: &str = "https://github.com/sverre200/JondisClip/releases/download/";
const ASSET_SUFFIX: &str = "-win64.zip";
/// Every file a release must contain before it's allowed to replace anything.
const REQUIRED: [&str; 3] = [
    "jondisclip.exe",
    "jondisclip-ui.exe",
    "jondisclip-export.exe",
];
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Window class of the tray's hidden window, and the message the settings
/// window posts to it for "Install update" (`WM_APP + 6`).
pub const TRAY_WINDOW_CLASS: &str = "JondisClipTray";
pub const WM_INSTALL_UPDATE: u32 = 0x8000 + 6;
/// The settings window's "Check now" button (`WM_APP + 7`).
pub const WM_CHECK_UPDATE: u32 = 0x8000 + 7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Without the leading "v", e.g. "0.1.1".
    pub version: String,
    pub zip_url: String,
}

/// "v0.1.1" or "0.1.1" as (major, minor, patch).
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim();
    let mut parts = s.strip_prefix('v').unwrap_or(s).split('.');
    let v = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(v)
}

/// The release in a GitHub `releases/latest` answer, if it's newer than `current`.
pub fn newer_release(json: &str, current: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let latest = parse_version(v.get("tag_name")?.as_str()?)?;
    if latest <= parse_version(current)? {
        return None;
    }
    let zip_url = v.get("assets")?.as_array()?.iter().find_map(|a| {
        let url = a.get("browser_download_url")?.as_str()?;
        let name = url.strip_prefix(DOWNLOAD_PREFIX)?;
        // Only our own plain file names: nothing that curl could read as an option or a path.
        let plain = name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/.-_".contains(c));
        (plain && !name.contains("..") && name.ends_with(ASSET_SUFFIX)).then(|| url.to_string())
    })?;
    Some(Release {
        version: format!("{}.{}.{}", latest.0, latest.1, latest.2),
        zip_url,
    })
}

fn system_tool(name: &str) -> PathBuf {
    // The full path, so a `curl.exe` dropped next to the app is never picked up.
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(root).join("System32").join(name)
}

fn run(tool: &str, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
    let out = Command::new(system_tool(tool))
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("{tool}: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("{tool} failed ({}): {}", out.status, err.trim()))
    }
}

/// Asks GitHub for the latest release. `Ok(None)` means up to date.
pub fn check(current: &str) -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let out = run(
        "curl.exe",
        &[
            "--silent".as_ref(),
            "--show-error".as_ref(),
            "--fail".as_ref(),
            "--location".as_ref(),
            "--proto".as_ref(),
            "=https".as_ref(),
            "--max-time".as_ref(),
            "20".as_ref(),
            "--user-agent".as_ref(),
            "JondisClip".as_ref(),
            "--header".as_ref(),
            "Accept: application/vnd.github+json".as_ref(),
            url.as_ref(),
        ],
    )?;
    Ok(newer_release(&String::from_utf8_lossy(&out), current))
}

/// Downloads and unpacks `release` under `work_dir` (wiped first). Returns the
/// folder holding the new files. Nothing in the install folder is touched.
pub fn download(release: &Release, work_dir: &Path) -> Result<PathBuf, String> {
    let _ = fs::remove_dir_all(work_dir);
    fs::create_dir_all(work_dir).map_err(|e| format!("{}: {e}", work_dir.display()))?;
    let zip = work_dir.join("update.zip");
    run(
        "curl.exe",
        &[
            "--silent".as_ref(),
            "--show-error".as_ref(),
            "--fail".as_ref(),
            "--location".as_ref(),
            "--proto".as_ref(),
            "=https".as_ref(),
            "--max-time".as_ref(),
            "900".as_ref(),
            "--user-agent".as_ref(),
            "JondisClip".as_ref(),
            "--output".as_ref(),
            zip.as_os_str(),
            release.zip_url.as_ref(),
        ],
    )?;
    run(
        "tar.exe",
        &[
            "-xf".as_ref(),
            zip.as_os_str(),
            "-C".as_ref(),
            work_dir.as_os_str(),
        ],
    )?;
    let _ = fs::remove_file(&zip);
    // The zip holds one top folder ("JondisClip"); accept a flat zip too.
    let staged = [work_dir.join("JondisClip"), work_dir.to_path_buf()]
        .into_iter()
        .find(|d| REQUIRED.iter().all(|f| d.join(f).is_file()))
        .ok_or("the downloaded update is missing files")?;
    Ok(staged)
}

fn files_under(dir: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir.join(rel))? {
        let entry = entry?;
        let rel = rel.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            files_under(dir, &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

fn old_name(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// Replaces the files in `install_dir` with those in `staged`. Files in use
/// (the running exes and their DLLs) can't be overwritten on Windows but can
/// be renamed, so each one is moved aside to `<name>.old` first; [`cleanup`]
/// removes those on the next start. If anything fails, every file is put back.
pub fn apply(staged: &Path, install_dir: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    files_under(staged, Path::new(""), &mut files).map_err(|e| e.to_string())?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut written: Vec<PathBuf> = Vec::new();
    let result = (|| -> io::Result<()> {
        for rel in &files {
            let dest = install_dir.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            if dest.exists() {
                let old = old_name(&dest);
                if old.exists() {
                    fs::remove_file(&old)?;
                }
                fs::rename(&dest, &old)?;
                moved.push((old, dest.clone()));
            }
            written.push(dest.clone());
            fs::copy(staged.join(rel), &dest)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        for dest in &written {
            let _ = fs::remove_file(dest);
        }
        for (old, dest) in &moved {
            let _ = fs::rename(old, dest);
        }
        return Err(format!("could not replace the app files: {e}"));
    }
    Ok(())
}

/// Removes what an earlier update left behind: `*.old` files in the install
/// folder and the download folder. Files still in use are skipped silently.
pub fn cleanup(install_dir: &Path, work_dir: &Path) {
    let _ = fs::remove_dir_all(work_dir);
    for dir in [install_dir.to_path_buf(), install_dir.join("licenses")] {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "old") {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// True for a developer build run from Cargo's `target` folder, which must
/// never replace itself with a packaged release.
pub fn is_dev_install(install_dir: &Path) -> bool {
    install_dir
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("target"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATEST: &str = r#"{
        "tag_name": "v0.2.0",
        "assets": [
            {"browser_download_url": "https://github.com/sverre200/JondisClip/releases/download/v0.2.0/notes.txt"},
            {"browser_download_url": "https://github.com/sverre200/JondisClip/releases/download/v0.2.0/JondisClip-0.2.0-win64.zip"}
        ]
    }"#;

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(parse_version("v0.1.10"), Some((0, 1, 10)));
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert!(parse_version("0.1.10") > parse_version("0.1.9"));
        for bad in ["", "1.2", "1.2.3.4", "v1.x.3", "latest"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn offers_only_newer_releases_from_our_repo() {
        let r = newer_release(LATEST, "0.1.0").unwrap();
        assert_eq!(r.version, "0.2.0");
        assert!(r.zip_url.ends_with("/v0.2.0/JondisClip-0.2.0-win64.zip"));
        assert_eq!(newer_release(LATEST, "0.2.0"), None);
        assert_eq!(newer_release(LATEST, "0.3.0"), None);
        assert_eq!(newer_release("not json", "0.1.0"), None);
        // A zip hosted anywhere else, or a name with odd characters, is refused.
        for url in [
            "https://example.com/JondisClip-9.9.9-win64.zip",
            "https://github.com/sverre200/JondisClip/releases/download/v9.9.9/../../x-win64.zip",
            "https://github.com/sverre200/JondisClip/releases/download/v9.9.9/a b-win64.zip",
        ] {
            let json =
                format!(r#"{{"tag_name":"v9.9.9","assets":[{{"browser_download_url":"{url}"}}]}}"#);
            assert_eq!(newer_release(&json, "0.1.0"), None, "{url}");
        }
    }

    #[test]
    fn apply_swaps_files_and_cleanup_removes_the_old_ones() {
        let root = std::env::temp_dir().join(format!("jc-update-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (staged, install, work) = (root.join("staged"), root.join("app"), root.join("work"));
        fs::create_dir_all(staged.join("licenses")).unwrap();
        fs::create_dir_all(&install).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(staged.join("jondisclip.exe"), b"new").unwrap();
        fs::write(staged.join("licenses").join("a.txt"), b"new licence").unwrap();
        fs::write(install.join("jondisclip.exe"), b"old").unwrap();
        fs::write(install.join("mine.txt"), b"keep").unwrap();

        apply(&staged, &install).unwrap();
        assert_eq!(fs::read(install.join("jondisclip.exe")).unwrap(), b"new");
        assert_eq!(
            fs::read(install.join("jondisclip.exe.old")).unwrap(),
            b"old"
        );
        assert!(install.join("licenses").join("a.txt").is_file());

        cleanup(&install, &work);
        assert!(!install.join("jondisclip.exe.old").exists());
        assert!(!work.exists());
        assert_eq!(fs::read(install.join("mine.txt")).unwrap(), b"keep");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dev_builds_are_recognised() {
        assert!(is_dev_install(Path::new(r"D:\proj\target\release")));
        assert!(!is_dev_install(Path::new(r"C:\Users\me\JondisClip")));
    }
}
