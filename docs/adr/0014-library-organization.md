# ADR-0014: Library folders, titles and order are metadata, keyed by a content fingerprint

Status: **Proposed** (2026-11-04). Implemented (ROADMAP Phase 4).

## Context
The owner wants personal folders (with colours), clip titles, drag-and-drop moving and a manual order. Recordings live at `<save>\<Game>\<Game> <stamp>.mp4`. That layout matters:
- The library derives the game from it.
- Explorer shows the clips that way.
- Edits are written next to their originals.
- Thumbnails, the index cache, Discord shares and Drive copies all refer to paths.

## Options
- **Real folders** (move files into `<save>\Funny Moments\…`):
  - The folders are visible in Explorer. That's the only advantage.
  - Every move is a file operation, slow and risky across volumes, and may happen while the recorder writes nearby.
  - It breaks the game folders (the game has to be recovered from the name) and invalidates thumbnails and cache entries.
  - A clip can only be in one place, and a failed or partial move puts the one thing we must never risk, the recordings, at risk.
- **Metadata** (chosen): organisation is a small JSON document. Recordings never move, nothing touches the save folder, and every action is instant and undoable.

## Decision
1. **`%LOCALAPPDATA%\JondisClip\library\organize.json`** (version 1). Each write is atomic (temp file + rename) and keeps a `.bak`. If the file is unreadable, the window falls back to the `.bak` before starting empty, and never overwrites an unreadable file silently.
   ```json
   { "version": 1,
     "folders": [ { "id": "f1a2…", "name": "DayZ Raids", "color": "red", "clips": ["<key>", "<key>"] } ],
     "titles":  { "<key>": "Our Base Got Raided" } }
   ```
   - Folder order is the array order.
   - **A folder's `clips` array is both its membership and its custom order.** A clip is in at most one folder.
   - Moving a clip removes it from every other folder first.
2. **Clip identity = a content fingerprint:** FNV-1a 64 over the size plus the first and last 64 KB, as 16 hex digits.
   - It's computed once per new or changed file during the scan and cached in the library index.
   - A clip renamed or moved outside JondisClip keeps its title and folder.
   - A clip deleted and restored from the Recycle Bin gets them back.
   - Two byte-identical copies share one key, which is acceptable: they are the same clip.
3. **Missing clips aren't purged.** Entries whose file isn't present right now (a disconnected drive, a file deleted outside the app) stay in the JSON, invisible, so nothing is lost if the file comes back. Folder counts show present clips only.
4. *(Superseded by ADR-0017: titles now rename the file.)* **Titles are display names only.** The file name never changes. Search matches titles, game and file name. Titles are trimmed, limited to 120 characters, and stripped of control characters. Folder names are 1–60 characters, unique ignoring case.
5. **Colours** come from a fixed palette: red, orange, yellow, green, blue, purple and grey. They're muted tokens shown as a small dot or icon tint, never fills.
6. **Deleting:**
   - **Deleting a folder** never deletes recordings. Its clips go back to "All clips", and the dialog says so.
   - **Deleting a clip** is an explicit action with a confirmation dialog. It goes to the **Recycle Bin** (`SHFileOperationW` with `FOF_ALLOWUNDO`, plus Windows' own warning if the volume has no Recycle Bin). The clip's metadata is kept for a restore. (*Superseded in part by ADR-0019:* there's now also a separate, confirmed Delete permanently on Shift+Delete.)
7. **Manual order:**
   - "Custom order" is a sort mode **inside a folder**, and it's the default there.
   - Dragging within the grid reorders only when the view shows the folder's complete list: custom sort, no search, no game filter. Otherwise the UI says "Clear the search and game filter to reorder". Dragging onto a folder always works.
   - "All clips" keeps its chronological sorts.
8. **Drag-and-drop is pointer-based** (no HTML5 DnD, no library):
   - A 6 px threshold separates a click from a drag.
   - A small ghost shows the thumbnail and a count.
   - Folders in the sidebar highlight as drop targets.
   - An insertion bar shows where reordered clips will land.
   - Multi-select uses Ctrl/Shift-click and moves together.

## Consequences
- Organising costs one small JSON write. There's no scanning and no file IO in the save folder, and nothing runs while the window is closed.
- The folders exist only in JondisClip, not in Explorer. Exporting a folder as real files could be added later as an explicit action.
- The first scan after this change reads 128 KB per clip once, to fingerprint it (the index version is bumped).
