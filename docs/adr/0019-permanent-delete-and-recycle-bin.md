# ADR-0019: Permanent delete and a Recycle Bin shortcut

Status: **Proposed** (2026-11-06). Supersedes ADR-0014's "it's never a permanent delete" (§ deleting a clip).

## Context
The owner wants to remove clips from the computer for good, without a second trip to the Recycle Bin. They also want to reach the Recycle Bin from the app to restore clips.

## Decision
- **Delete permanently…** is a second, separate action, next to the normal Delete… (Recycle Bin), which stays the default.
  - It's in the clip context menu, and on **Shift+Delete**, as in File Explorer. Plain Delete still asks to move clips to the Recycle Bin.
  - It always asks first, in its own dialog. The title says "permanently", the text says the file can't be restored, and the focus starts on Cancel.
  - Files are removed with `std::fs::remove_file`. Like normal delete, it's refused while an export is running, and every id must still be the exact file from the latest scan.
  - A file that can't be removed (in use) is reported; the rest are still deleted.
- **Metadata.** After a permanent delete, the clip's title, its place in folder orders, its Drive copies list and any pending move are forgotten, because it can't come back. A byte-identical copy still in the library keeps them (same content key). Its thumbnail is pruned on the next scan, as before.
- **Recycle Bin shortcut.** The folder sidebar has a **Recycle Bin** row under New folder. It opens the Windows Recycle Bin in File Explorer (`explorer shell:RecycleBinFolder`), where deleted clips can be restored. The app doesn't show or restore the Recycle Bin's contents itself.

## Consequences
- Non-negotiable 3 still holds: nothing is deleted automatically. Every delete is a user action with a confirmation dialog.
- A permanent delete can't be undone, even by Windows.
- **Verified 2026-11-06:**
  - A unit test checks that `Organize::forget` drops titles, orders, Drive copies and pending moves only for the deleted keys.
  - Browser preview: the menu shows both actions, Shift+Delete opens the permanent dialog with Cancel focused, and confirming removes the clip.
