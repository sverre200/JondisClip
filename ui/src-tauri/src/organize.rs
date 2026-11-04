//! Library organisation commands (ADR-0014): folders, colours, titles and
//! manual order live in `%LOCALAPPDATA%\JondisClip\library\organize.json`.
//! Nothing here touches recordings except the explicit "Delete" (to the
//! Recycle Bin, after the page confirmed it).

use std::sync::Mutex;

use jondisclip_core::organize::{OrgError, Organize, Store};
use jondisclip_core::paths;

use crate::library;

/// One change at a time (load → change → save).
static LOCK: Mutex<()> = Mutex::new(());

fn store() -> Store {
    Store::new(paths::local_dir().join("library").join("organize.json"))
}

/// Load → change → save, on a blocking thread (file IO stays off the UI
/// thread), one at a time.
async fn update<T: Send + 'static>(
    f: impl FnOnce(&mut Organize) -> Result<T, OrgError> + Send + 'static,
) -> Result<Organize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        store().update(f).map(|(o, _)| o)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A fresh folder id (time + counter; only needs to be unique in this file).
fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("f{:x}{:x}", t, N.fetch_add(1, Ordering::Relaxed))
}

#[tauri::command]
pub async fn organize_get() -> Result<Organize, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        store().load()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn folder_create(name: String, color: String) -> Result<Organize, String> {
    let id = new_id();
    update(move |o| o.create_folder(&id, &name, &color)).await
}

#[tauri::command]
pub async fn folder_rename(id: String, name: String) -> Result<Organize, String> {
    update(move |o| o.rename_folder(&id, &name)).await
}

#[tauri::command]
pub async fn folder_color(id: String, color: String) -> Result<Organize, String> {
    update(move |o| o.recolor_folder(&id, &color)).await
}

/// Deletes the folder only; its clips stay in the library.
#[tauri::command]
pub async fn folder_delete(id: String) -> Result<Organize, String> {
    update(move |o| o.delete_folder(&id).map(drop)).await
}

#[tauri::command]
pub async fn folders_reorder(ids: Vec<String>) -> Result<Organize, String> {
    update(move |o| {
        o.reorder_folders(&ids);
        Ok(())
    })
    .await
}

/// Moves clips (by key) into a folder (`to` = None: out of every folder),
/// before `before` when given.
#[tauri::command]
pub async fn clips_move(
    keys: Vec<String>,
    to: Option<String>,
    before: Option<String>,
) -> Result<Organize, String> {
    update(move |o| o.move_clips(&keys, to.as_deref(), before.as_deref())).await
}

#[tauri::command]
pub async fn clip_title(key: String, title: String) -> Result<Organize, String> {
    update(move |o| o.set_title(&key, &title)).await
}

/// Moves clips to the Recycle Bin. The page asks first; Windows itself
/// warns if a volume has no Recycle Bin (the delete would be permanent).
/// Titles and folders are kept, so a restored clip comes back organised.
#[tauri::command]
pub async fn clips_delete(app: tauri::AppHandle, ids: Vec<String>) -> Result<usize, String> {
    if crate::export::busy() {
        return Err("Wait for the running export to finish first.".into());
    }
    let paths: Vec<_> = ids
        .iter()
        .map(|id| library::path_of(id))
        .collect::<Result<_, _>>()?;
    let owner = crate::export::main_hwnd(&app);
    tauri::async_runtime::spawn_blocking(move || recycle(&paths, owner))
        .await
        .map_err(|e| e.to_string())?
}

fn recycle(paths: &[std::path::PathBuf], owner: Option<isize>) -> Result<usize, String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };
    if paths.is_empty() {
        return Ok(0);
    }
    // Double-NUL-terminated list of full paths.
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
        // Undo = Recycle Bin. No second "are you sure" (we asked), but keep
        // Windows' warning when a file can't be recycled.
        fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_WANTNUKEWARNING.0) as u16,
        ..Default::default()
    };
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 {
        return Err(format!(
            "Windows couldn't move the clip to the Recycle Bin (error {r})."
        ));
    }
    if op.fAnyOperationsAborted.as_bool() {
        return Err("Cancelled: nothing was deleted.".into());
    }
    Ok(paths.iter().filter(|p| !p.exists()).count())
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

    #[test]
    fn ids_are_unique() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert!(a.starts_with('f'));
    }

    /// Puts a small temp file in the real Recycle Bin: run by hand with
    /// `cargo test -p jondisclip-ui -- --ignored recycling`.
    #[test]
    #[ignore]
    fn recycling_moves_files_out_and_never_touches_others() {
        let d = std::env::temp_dir().join(format!("jc-recycle-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let gone = d.join("delete me.mp4");
        let kept = d.join("keep me.mp4");
        std::fs::write(&gone, b"x").unwrap();
        std::fs::write(&kept, b"y").unwrap();
        assert_eq!(recycle(std::slice::from_ref(&gone), None).unwrap(), 1);
        assert!(!gone.exists());
        assert_eq!(std::fs::read(&kept).unwrap(), b"y");
        assert_eq!(recycle(&[], None).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&d);
    }
}
