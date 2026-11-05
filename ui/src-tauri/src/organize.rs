//! Library organisation with real folders (ADR-0015).
//!
//! The filesystem decides membership: a user folder is a directory directly
//! under the clips root, and putting a clip in a folder moves the MP4 there
//! (an instant same-volume rename; never an overwrite). Colours, sidebar
//! order, the manual clip order and titles are metadata in
//! `%LOCALAPPDATA%\JondisClip\library\organize.json`. Every scan reconciles
//! the metadata with what's on disk (Explorer changes show up).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use jondisclip_core::library::{self as lib, Clip};
use jondisclip_core::organize::{Found, OrgError, Organize, Pending, Store, folder_name};
use jondisclip_core::paths;
use serde::{Deserialize, Serialize};

use crate::library;

/// One organisation change (metadata and file operations) at a time.
static LOCK: Mutex<()> = Mutex::new(());

fn store() -> Store {
    Store::new(paths::local_dir().join("library").join("organize.json"))
}

fn root() -> PathBuf {
    let text = std::fs::read_to_string(paths::settings_file()).unwrap_or_default();
    let (settings, _) = jondisclip_core::settings::Settings::from_toml(&text);
    settings.save_dir_or(&paths::default_save_dir())
}

/// Runs `f` on a blocking thread with background IO priority, holding the lock.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        library::in_background(f)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn meta<T>(f: impl FnOnce(&mut Organize) -> Result<T, OrgError>) -> Result<(Organize, T), String> {
    store().update(f)
}

// ------------------------------------------------------------------ view

#[derive(Serialize)]
pub struct FolderView {
    pub name: String,
    pub color: String,
    pub count: usize,
    /// Content keys in custom order (complete for the folder's clips).
    pub order: Vec<String>,
}

#[derive(Serialize)]
pub struct ClipView {
    #[serde(flatten)]
    pub clip: Clip,
    /// The user folder it's in (None: "Not in a folder").
    pub folder: Option<String>,
    pub title: Option<String>,
}

#[derive(Serialize)]
pub struct Library {
    pub root: String,
    pub root_ok: bool,
    pub clips: Vec<ClipView>,
    pub folders: Vec<FolderView>,
    /// v1 folders whose clips haven't been moved into the real folder yet.
    pub pending: Vec<PendingView>,
}

#[derive(Serialize)]
pub struct PendingView {
    pub folder: String,
    pub color: String,
    /// Ids of the clips (present right now) waiting to be moved.
    pub ids: Vec<String>,
}

/// Scans, reconciles the metadata with the disk, and builds the view.
pub fn load(root: PathBuf) -> Result<Library, String> {
    let root_ok = root.is_dir();
    let mut lv = library::scan(root.clone());
    let mut org = store().load()?;
    if root_ok {
        let mut by_dir: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for c in &lv.clips {
            if let Some(d) = &c.dir {
                by_dir
                    .entry(d.clone())
                    .or_default()
                    .push((c.file_name.clone(), c.key.clone()));
            }
        }
        let app = |f: &str| lib::parse_name(f).map(|p| p.0);
        let changed = org.reconcile(&Found {
            dirs: &lv.dirs,
            clips: &by_dir,
            app_of: &app,
        });
        // Folders created in JondisClip whose directory is gone stay in the
        // metadata (invisible) — e.g. a disconnected drive.
        if changed {
            org = store()
                .update(|o| {
                    o.reconcile(&Found {
                        dirs: &lv.dirs,
                        clips: &by_dir,
                        app_of: &app,
                    });
                    Ok(())
                })?
                .0;
        }
    }
    let exists = |n: &str| lv.dirs.iter().any(|d| d.eq_ignore_ascii_case(n));
    let user = |dir: &Option<String>| -> Option<String> {
        let d = dir.as_ref()?;
        org.folder(d)
            .filter(|_| !org.is_game_dir(d))
            .map(|f| f.name.clone())
    };
    let mut clips: Vec<ClipView> = std::mem::take(&mut lv.clips)
        .into_iter()
        .map(|mut c| {
            let folder = user(&c.dir);
            if folder.is_some() {
                // In a user folder the game comes from the file name.
                c.game = lib::parse_name(&c.file_name)
                    .map(|p| p.0)
                    .unwrap_or_else(|| "Other".into());
            }
            let title = org.titles.get(&c.key).cloned();
            ClipView {
                clip: c,
                folder,
                title,
            }
        })
        .collect();
    clips.sort_by(|a, b| b.clip.recorded.cmp(&a.clip.recorded));
    let folders = org
        .folders
        .iter()
        .filter(|f| exists(&f.name))
        .map(|f| {
            let here: Vec<&ClipView> = clips
                .iter()
                .filter(|c| c.folder.as_deref() == Some(f.name.as_str()))
                .collect();
            let keys: Vec<String> = here.iter().map(|c| c.clip.key.clone()).collect();
            let mut order: Vec<String> = f
                .order
                .iter()
                .filter(|k| keys.contains(k))
                .cloned()
                .collect();
            for k in keys {
                if !order.contains(&k) {
                    order.push(k);
                }
            }
            FolderView {
                name: f.name.clone(),
                color: f.color.clone(),
                count: here.len(),
                order,
            }
        })
        .collect();
    let pending = org
        .pending
        .iter()
        .map(|p: &Pending| PendingView {
            folder: p.folder.clone(),
            color: org
                .folder(&p.folder)
                .map_or_else(|| "gray".to_string(), |f| f.color.clone()),
            ids: clips
                .iter()
                .filter(|c| {
                    p.keys.contains(&c.clip.key) && c.folder.as_deref() != Some(p.folder.as_str())
                })
                .map(|c| c.clip.id.clone())
                .collect(),
        })
        .filter(|p| !p.ids.is_empty())
        .collect();
    Ok(Library {
        root: lv.root,
        root_ok,
        clips,
        folders,
        pending,
    })
}

// ------------------------------------------------------------------ files

/// `p` resolved, and inside `root`.
fn inside(root: &Path, p: &Path) -> Result<PathBuf, String> {
    let r = std::fs::canonicalize(root)
        .map_err(|e| format!("The clips folder isn't available: {e}"))?;
    let c = std::fs::canonicalize(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if !c.starts_with(&r) {
        return Err(format!("{} is outside the clips folder", p.display()));
    }
    Ok(c)
}

/// Is there any entry (file or folder) called `name` in `root`, ignoring case?
fn entry_exists(root: &Path, name: &str) -> bool {
    std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        })
        .unwrap_or(false)
}

/// Moves `src` into `dir` without ever overwriting (` (2)`… on a clash).
/// Same volume: one atomic rename. Another volume (via a junction): copy to
/// `.partial`, verify size and fingerprint, rename, and only then delete the
/// source. Returns the new path.
pub fn move_clip(src: &Path, dir: &Path) -> Result<PathBuf, String> {
    let name = src
        .file_name()
        .ok_or("bad clip path")?
        .to_string_lossy()
        .into_owned();
    if src.parent().is_some_and(|p| same_dir(p, dir)) {
        return Ok(src.to_path_buf());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    let dst = lib::free_path(dir, &name, &|p| {
        p.exists() || p.with_extension("mp4.partial").exists()
    });
    match crate::export::move_no_replace(src, &dst) {
        Ok(()) => Ok(dst),
        // ERROR_NOT_SAME_DEVICE: copy, verify, then remove the source.
        Err(e) if e.raw_os_error() == Some(17) => cross_volume(src, &dst),
        Err(e) => Err(match e.raw_os_error() {
            Some(32) | Some(33) => {
                format!("{name} is in use (playing or being exported?). Close it and try again.")
            }
            Some(5) => format!("Windows denied moving {name} (permissions)."),
            Some(80) | Some(183) => {
                format!("{} already exists; nothing was overwritten.", dst.display())
            }
            _ => format!("couldn't move {name}: {e}"),
        }),
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn cross_volume(src: &Path, dst: &Path) -> Result<PathBuf, String> {
    let tmp = dst.with_extension("mp4.partial");
    let fail = |e: String| {
        let _ = std::fs::remove_file(&tmp);
        e
    };
    let size = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
    std::fs::copy(src, &tmp).map_err(|e| fail(format!("couldn't copy: {e}")))?;
    let ok = std::fs::metadata(&tmp).map(|m| m.len()).ok() == Some(size)
        && lib::fingerprint(&tmp, size).ok() == lib::fingerprint(src, size).ok();
    if !ok {
        return Err(fail(
            "the copy didn't verify; the original is untouched".into(),
        ));
    }
    crate::export::move_no_replace(&tmp, dst)
        .map_err(|e| fail(format!("couldn't finish the copy: {e}")))?;
    // Explicit move, copy verified: now the original may go.
    if let Err(e) = std::fs::remove_file(src) {
        return Err(format!(
            "Copied to {}, but the original couldn't be removed ({e}); you now have both.",
            dst.display()
        ));
    }
    Ok(dst.to_path_buf())
}

/// Where "Not in a folder" puts a clip: its game folder (from the file name),
/// or the root for files without a recorder name.
fn home_dir(root: &Path, file_name: &str) -> PathBuf {
    match lib::parse_name(file_name) {
        Some((app, _)) if folder_name(&app).is_ok() && app != "Other" => root.join(app),
        _ => root.to_path_buf(),
    }
}

/// A clip id from the latest scan, checked to still be that exact file.
fn clip_path(id: &str) -> Result<PathBuf, String> {
    let p = library::path_of(id)?;
    let m = std::fs::metadata(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    if lib::clip_id(&p.to_string_lossy(), m.len(), mtime) != id {
        return Err(format!(
            "{} changed since the library was loaded. Refresh and try again.",
            p.display()
        ));
    }
    Ok(p)
}

#[derive(Deserialize, Clone)]
pub struct Item {
    pub id: String,
    pub key: String,
}

#[derive(Serialize, Default)]
pub struct MoveResult {
    pub moved: usize,
    pub failed: Vec<String>,
}

/// Moves clips into user folder `to` (None: back to their game folders).
fn move_items(root: &Path, items: &[Item], to: Option<&str>) -> Result<MoveResult, String> {
    if crate::export::busy() {
        return Err("Wait for the running export to finish first.".into());
    }
    let org = store().load()?;
    let dest = match to {
        Some(name) => {
            let f = org
                .folder(name)
                .ok_or("That folder no longer exists. Refresh and try again.")?;
            let d = root.join(&f.name);
            if !d.is_dir() {
                return Err(format!(
                    "{} isn't on disk any more. Refresh and try again.",
                    d.display()
                ));
            }
            Some(d)
        }
        None => None,
    };
    let mut res = MoveResult::default();
    let mut arrived = Vec::new();
    for it in items {
        let r = (|| {
            let src = inside(root, &clip_path(&it.id)?)?;
            let name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let dir = dest.clone().unwrap_or_else(|| home_dir(root, &name));
            move_clip(&src, &dir)
        })();
        match r {
            Ok(_) => {
                res.moved += 1;
                arrived.push(it.key.clone());
            }
            Err(e) => res.failed.push(e),
        }
    }
    if let Some(name) = to {
        let keys: Vec<String> = arrived.into_iter().filter(|k| !k.is_empty()).collect();
        meta(|o| {
            o.arrived(name, &keys);
            Ok(())
        })?;
    }
    Ok(res)
}

// ------------------------------------------------------------------ commands

#[tauri::command]
pub async fn library_load() -> Result<Library, String> {
    let root = root();
    tauri::async_runtime::spawn_blocking(move || {
        library::in_background(|| {
            // Left over if the window closed mid-export (the exporter dies
            // with it). Only one window runs, so none of these is in use.
            if !crate::export::busy() {
                for p in lib::stale_edit_partials(&root) {
                    let _ = std::fs::remove_file(p);
                }
            }
            let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            load(root)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Creates a real folder under the clips root and registers it.
#[tauri::command]
pub async fn folder_create(name: String, color: String) -> Result<(), String> {
    blocking(move || {
        let root = root();
        if !root.is_dir() {
            return Err(format!(
                "The clips folder {} isn't available.",
                root.display()
            ));
        }
        let name = folder_name(&name).map_err(|e| e.to_string())?;
        // Validate the metadata first, then create the directory.
        let mut probe = store().load()?;
        probe.add_folder(&name, &color).map_err(|e| e.to_string())?;
        if entry_exists(&root, &name) {
            // An existing directory that isn't a game folder (one kept after
            // a delete because it held other files, or made in Explorer) is
            // adopted under its own spelling instead of refused.
            let existing = std::fs::read_dir(&root).ok().and_then(|rd| {
                rd.flatten()
                    .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(&name))
            });
            return match existing {
                Some(e)
                    if e.file_type().is_ok_and(|t| t.is_dir())
                        && !probe.is_game_dir(&e.file_name().to_string_lossy()) =>
                {
                    let on_disk = e.file_name().to_string_lossy().into_owned();
                    meta(|o| o.add_folder(&on_disk, &color)).map(drop)
                }
                _ => Err(format!(
                    "There's already something called \"{name}\" in {}.",
                    root.display()
                )),
            };
        }
        std::fs::create_dir(root.join(&name))
            .map_err(|e| format!("couldn't create the folder: {e}"))?;
        meta(|o| o.add_folder(&name, &color)).map(drop)
    })
    .await
}

/// Renames the folder's directory (never over an existing one), then the metadata.
#[tauri::command]
pub async fn folder_rename(name: String, to: String) -> Result<(), String> {
    blocking(move || {
        let root = root();
        let to = folder_name(&to).map_err(|e| e.to_string())?;
        let org = store().load()?;
        let f = org.folder(&name).ok_or("That folder no longer exists.")?;
        let from = root.join(&f.name);
        if !from.is_dir() {
            return Err(format!("{} isn't on disk any more. Refresh and try again.", from.display()));
        }
        if !to.eq_ignore_ascii_case(&f.name) && entry_exists(&root, &to) {
            return Err(format!("There's already something called \"{to}\" in {}.", root.display()));
        }
        crate::export::move_no_replace(&from, &root.join(&to)).map_err(|e| match e.raw_os_error() {
            Some(5) | Some(32) => "A clip in this folder is in use (playing or exporting?). Close it and try again.".to_string(),
            _ => format!("couldn't rename the folder: {e}"),
        })?;
        let old = f.name.clone();
        meta(|o| o.renamed(&old, &to)).map(drop)
    })
    .await
}

#[tauri::command]
pub async fn folder_color(name: String, color: String) -> Result<(), String> {
    blocking(move || meta(|o| o.recolor(&name, &color)).map(drop)).await
}

#[derive(Serialize)]
pub struct DeleteFolderResult {
    pub moved: usize,
    pub failed: Vec<String>,
    /// The directory was kept because it still holds other files.
    pub kept_dir: bool,
}

/// Deletes a folder safely: its clips go back to their game folders first;
/// the directory is removed only if it's then empty. Never deletes a clip.
#[tauri::command]
pub async fn folder_delete(name: String) -> Result<DeleteFolderResult, String> {
    blocking(move || {
        if crate::export::busy() {
            return Err("Wait for the running export to finish first.".into());
        }
        let root = root();
        let org = store().load()?;
        let f = org.folder(&name).ok_or("That folder no longer exists.")?;
        let dir = root.join(&f.name);
        let mut moved = 0;
        let mut failed = Vec::new();
        if dir.is_dir() {
            let clips: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map_err(|e| e.to_string())?
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4"))
                })
                .collect();
            for c in clips {
                let name = c
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match move_clip(&c, &home_dir(&root, &name)) {
                    Ok(_) => moved += 1,
                    Err(e) => failed.push(e),
                }
            }
        }
        if !failed.is_empty() {
            return Ok(DeleteFolderResult {
                moved,
                failed,
                kept_dir: true,
            });
        }
        // remove_dir only succeeds on an empty directory.
        let kept_dir = dir.is_dir() && std::fs::remove_dir(&dir).is_err();
        let fname = f.name.clone();
        meta(|o| o.remove_folder(&fname).map(drop))?;
        Ok(DeleteFolderResult {
            moved,
            failed,
            kept_dir,
        })
    })
    .await
}

#[tauri::command]
pub async fn folders_reorder(names: Vec<String>) -> Result<(), String> {
    blocking(move || {
        meta(|o| {
            o.reorder_folders(&names);
            Ok(())
        })
        .map(drop)
    })
    .await
}

/// Moves clips (real files) into a folder, or back to their game folders.
#[tauri::command]
pub async fn clips_move(items: Vec<Item>, to: Option<String>) -> Result<MoveResult, String> {
    blocking(move || move_items(&root(), &items, to.as_deref())).await
}

/// Custom order inside a folder (metadata only: no file moves).
#[tauri::command]
pub async fn clips_arrange(
    folder: String,
    present: Vec<String>,
    keys: Vec<String>,
    before: Option<String>,
) -> Result<(), String> {
    blocking(move || meta(|o| o.arrange(&folder, &present, &keys, before.as_deref())).map(drop))
        .await
}

#[tauri::command]
pub async fn clip_title(key: String, title: String) -> Result<(), String> {
    blocking(move || meta(|o| o.set_title(&key, &title)).map(drop)).await
}

/// The one-time question for v1 folders: move their clips in, or leave them.
#[tauri::command]
pub async fn pending_resolve(
    folder: String,
    items: Vec<Item>,
    move_them: bool,
) -> Result<MoveResult, String> {
    blocking(move || {
        let res = if move_them {
            let root = root();
            // A folder from before ADR-0015 has no directory yet: make it
            // (not recursively: a missing clips root stays an error).
            let name = folder_name(&folder).map_err(|e| e.to_string())?;
            let dir = root.join(&name);
            if !dir.is_dir() {
                if dir.exists() {
                    return Err(format!(
                        "There's a file called {name} in your clips folder, so the folder can't be made."
                    ));
                }
                std::fs::create_dir(&dir)
                    .map_err(|e| format!("Couldn't make the folder {}: {e}", dir.display()))?;
            }
            move_items(&root, &items, Some(&name))?
        } else {
            MoveResult::default()
        };
        if res.failed.is_empty() {
            meta(|o| {
                o.resolve_pending(&folder);
                Ok(())
            })?;
        }
        Ok(res)
    })
    .await
}

/// Moves clips to the Recycle Bin. The page asks first; Windows itself
/// warns if a volume has no Recycle Bin (the delete would be permanent).
/// Titles are kept, so a restored clip comes back named.
#[tauri::command]
pub async fn clips_delete(app: tauri::AppHandle, ids: Vec<String>) -> Result<usize, String> {
    if crate::export::busy() {
        return Err("Wait for the running export to finish first.".into());
    }
    let paths: Vec<PathBuf> = ids
        .iter()
        .map(|id| clip_path(id))
        .collect::<Result<_, _>>()?;
    if paths.iter().any(|p| !p.is_absolute()) {
        return Err("bad clip path".into());
    }
    let owner = crate::export::main_hwnd(&app);
    tauri::async_runtime::spawn_blocking(move || recycle(&paths, owner))
        .await
        .map_err(|e| e.to_string())?
}

fn recycle(paths: &[PathBuf], owner: Option<isize>) -> Result<usize, String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };
    if paths.is_empty() {
        return Ok(0);
    }
    let mut from: Vec<u16> = Vec::new();
    for p in paths {
        from.extend(p.as_os_str().to_string_lossy().encode_utf16());
        from.push(0);
    }
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        hwnd: HWND(owner.unwrap_or(0) as *mut _),
        wFunc: FO_DELETE,
        pFrom: windows::core::PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_WANTNUKEWARNING.0) as u16,
        ..Default::default()
    };
    let r = unsafe { SHFileOperationW(&mut op) };
    let gone = paths.iter().filter(|p| !p.exists()).count();
    if gone == paths.len() {
        return Ok(gone);
    }
    let why = if op.fAnyOperationsAborted.as_bool() {
        "cancelled"
    } else {
        "a file was in use or couldn't be recycled"
    };
    let _ = r;
    Err(if gone == 0 {
        format!("Nothing was deleted ({why}).")
    } else {
        format!(
            "{gone} of {} clips were moved to the Recycle Bin ({why}).",
            paths.len()
        )
    })
}

/// Puts a clip's file on the clipboard (Explorer-style "Copy").
#[tauri::command]
pub async fn clip_copy_file(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let path = library::path_of(&id)?;
    let owner = crate::export::main_hwnd(&app);
    tauri::async_runtime::spawn_blocking(move || crate::clipboard::copy_file(&path, owner))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("jc-fs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }

    #[test]
    fn moves_are_renames_that_never_overwrite() {
        let t = tmp("move");
        let game = t.0.join("DayZ");
        let folder = t.0.join("Funny Moments");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        let a = game.join("DayZ 2026-11-04 18-31-24 edit.mp4");
        std::fs::write(&a, b"clip A").unwrap();
        // Something with the same name already in the folder: kept as is.
        std::fs::write(
            folder.join("DayZ 2026-11-04 18-31-24 edit.mp4"),
            b"other clip",
        )
        .unwrap();
        let moved = move_clip(&a, &folder).unwrap();
        assert_eq!(
            moved.file_name().unwrap(),
            "DayZ 2026-11-04 18-31-24 edit (2).mp4"
        );
        assert_eq!(std::fs::read(&moved).unwrap(), b"clip A");
        assert_eq!(
            std::fs::read(folder.join("DayZ 2026-11-04 18-31-24 edit.mp4")).unwrap(),
            b"other clip"
        );
        assert!(!a.exists());
        assert!(
            lib::is_edit_name(&moved.file_name().unwrap().to_string_lossy()),
            "Edit badge kept"
        );
        // Back home ("Not in a folder"): into the game folder from the file name.
        let home = home_dir(&t.0, &moved.file_name().unwrap().to_string_lossy());
        assert_eq!(home, game);
        let back = move_clip(&moved, &home).unwrap();
        assert_eq!(std::fs::read(&back).unwrap(), b"clip A");
        // Moving into the folder it's already in: no-op.
        assert_eq!(move_clip(&back, &game).unwrap(), back);
        // Special characters survive.
        let odd = game.join("Ærø 😂 'clip' & co.mp4");
        std::fs::write(&odd, b"odd").unwrap();
        let o2 = move_clip(&odd, &folder).unwrap();
        assert_eq!(std::fs::read(o2).unwrap(), b"odd");
    }

    #[test]
    fn missing_or_locked_sources_fail_cleanly() {
        let t = tmp("fail");
        let folder = t.0.join("F");
        std::fs::create_dir_all(&folder).unwrap();
        assert!(move_clip(&t.0.join("gone.mp4"), &folder).is_err());
        // A file open without delete sharing can't be moved: reported, untouched.
        use std::os::windows::fs::OpenOptionsExt;
        let locked = t.0.join("locked.mp4");
        std::fs::write(&locked, b"x").unwrap();
        let _h = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&locked)
            .unwrap(); // FILE_SHARE_READ only
        let e = move_clip(&locked, &folder).unwrap_err();
        assert!(e.contains("in use"), "{e}");
        assert!(locked.exists());
    }

    #[test]
    fn home_is_the_game_folder_or_the_root() {
        let r = Path::new(r"D:\Clips");
        assert_eq!(home_dir(r, "DayZ 2026-11-04 18-31-24.mp4"), r.join("DayZ"));
        assert_eq!(home_dir(r, "my own clip.mp4"), r.to_path_buf());
    }

    #[test]
    fn entry_names_compare_ignoring_case() {
        let t = tmp("names");
        std::fs::create_dir_all(t.0.join("Funny Moments")).unwrap();
        assert!(entry_exists(&t.0, "funny moments"));
        assert!(!entry_exists(&t.0, "Raids"));
    }
}
