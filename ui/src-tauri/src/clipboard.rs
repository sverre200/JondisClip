//! Puts files on the Windows clipboard the way Explorer's "Copy" does
//! (`CF_HDROP` + "Preferred DropEffect" = copy), so Ctrl+V in Discord, a chat
//! or a folder pastes the file itself (ADR-0012). Nothing is uploaded by us.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::core::w;

const CF_HDROP: u32 = 15;
const DROPEFFECT_COPY: u32 = 1;

/// The `DROPFILES` block for `paths`: a 20-byte header (pFiles = 20,
/// fWide = 1) followed by NUL-terminated UTF-16 paths and a final NUL.
pub fn dropfiles(paths: &[PathBuf]) -> Vec<u8> {
    let mut v = Vec::with_capacity(20 + paths.len() * 260 * 2);
    v.extend_from_slice(&20u32.to_le_bytes()); // pFiles: offset of the list
    v.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    v.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    v.extend_from_slice(&0u32.to_le_bytes()); // fNC
    v.extend_from_slice(&1u32.to_le_bytes()); // fWide: UTF-16
    for p in paths {
        for u in p.as_os_str().to_string_lossy().encode_utf16() {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v.extend_from_slice(&[0, 0]);
    }
    v.extend_from_slice(&[0, 0]);
    v
}

fn global(bytes: &[u8]) -> Result<HGLOBAL, String> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| e.to_string())?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err("couldn't lock clipboard memory".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Ok(h)
    }
}

/// Copies `path` (which must exist) to the clipboard as a file. `owner` is
/// our window: with no owner, `EmptyClipboard` leaves the clipboard ownerless
/// and `SetClipboardData` is documented to fail (it usually works anyway).
pub fn copy_file(path: &Path, owner: Option<isize>) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    if !path.is_file() {
        return Err(format!("{} doesn't exist", path.display()));
    }
    let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    // canonicalize gives \\?\C:\…; paste targets want the plain form.
    let plain = {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
        let wide = wide.strip_prefix(prefix.as_slice()).unwrap_or(&wide);
        PathBuf::from(std::ffi::OsString::from_wide(wide))
    };
    // Each handle is freed by us unless the clipboard took it.
    let drop = Owned::new(global(&dropfiles(&[plain]))?);
    let effect = Owned::new(global(&DROPEFFECT_COPY.to_le_bytes())?);
    unsafe {
        // Another program may hold the clipboard for a moment: retry briefly.
        let hwnd = owner.map(|h| HWND(h as *mut _));
        let mut opened = false;
        for _ in 0..20 {
            if OpenClipboard(hwnd).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        if !opened {
            return Err("the clipboard is busy; try again".into());
        }
        let result = (|| {
            EmptyClipboard().map_err(|e| e.to_string())?;
            SetClipboardData(CF_HDROP, Some(HANDLE(drop.0.0))).map_err(|e| e.to_string())?;
            drop.release(); // now the clipboard's
            let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            if fmt != 0 && SetClipboardData(fmt, Some(HANDLE(effect.0.0))).is_ok() {
                effect.release();
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// A global memory handle we free on drop unless it was handed over.
struct Owned(HGLOBAL, std::cell::Cell<bool>);

impl Owned {
    fn new(h: HGLOBAL) -> Self {
        Self(h, std::cell::Cell::new(false))
    }
    fn release(&self) {
        self.1.set(true);
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.1.get() {
            unsafe {
                let _ = GlobalFree(Some(self.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropfiles_layout_matches_the_win32_struct() {
        let b = dropfiles(&[PathBuf::from(r"C:\Clips\Dåy Z.mp4")]);
        assert_eq!(&b[0..4], &20u32.to_le_bytes(), "pFiles");
        assert_eq!(&b[16..20], &1u32.to_le_bytes(), "fWide");
        let units: Vec<u16> = b[20..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let text = String::from_utf16(&units[..units.len() - 2]).unwrap();
        assert_eq!(text, r"C:\Clips\Dåy Z.mp4", "non-ASCII and spaces survive");
        assert_eq!(&units[units.len() - 2..], &[0, 0], "double NUL terminator");
        let two = dropfiles(&[PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(two.len(), 20 + (2 + 2) * 2 + 2);
    }

    #[test]
    fn missing_files_are_refused() {
        assert!(copy_file(Path::new(r"C:\definitely\not\here.mp4"), None).is_err());
    }

    /// Touches the real clipboard (replaces its content): run by hand with
    /// `cargo test -p jondisclip-ui -- --ignored clipboard`.
    #[test]
    #[ignore]
    fn clipboard_round_trip() {
        use windows::Win32::System::DataExchange::GetClipboardData;
        use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
        let f = std::env::temp_dir().join("jc clipboard test.mp4");
        std::fs::write(&f, b"x").unwrap();
        copy_file(&f, None).unwrap();
        unsafe {
            OpenClipboard(None).unwrap();
            let h = GetClipboardData(CF_HDROP).unwrap();
            let n = DragQueryFileW(HDROP(h.0), u32::MAX, None);
            let mut buf = [0u16; 520];
            let len = DragQueryFileW(HDROP(h.0), 0, Some(&mut buf));
            let _ = CloseClipboard();
            assert_eq!(n, 1);
            let got = String::from_utf16_lossy(&buf[..len as usize]);
            assert!(got.ends_with("jc clipboard test.mp4"), "{got}");
        }
        let _ = std::fs::remove_file(&f);
    }
}
