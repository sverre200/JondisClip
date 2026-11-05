# ADR-0015: Library folders are real directories; order, colours and titles stay metadata

Status: **Proposed** (2026-11-05). This supersedes ADR-0014's "folders are metadata only". Titles, colours and manual order remain metadata as ADR-0014 decided.

## Context
The owner wants Explorer to match the app: a folder created in JondisClip is a real directory under the clips root, dragging a clip there moves the MP4, renames rename the directory, and changes made in Explorer show up in the app. Automatic recordings must keep landing in `<root>\<Game>\` exactly as before.

## Decision
1. **The filesystem decides membership.** Directly under the clips root there are two kinds of directory:
   - **Game folders:** the recorder's `<App>` folders. A directory is learned as a game folder once it holds a recorder-named clip of the same app (`DayZ\DayZ 2026-….mp4`). Learned names are remembered in `game_dirs`.
   - **User folders:** everything else that holds clips, plus every folder created in JondisClip.
   
   Explorer-made empty folders appear once they contain a clip or are created or registered in the app. Clips in a game folder, or loose in the root, are shown as **"Not in a folder"**. The game for a clip in a user folder comes from its file name.
2. **Metadata (organize.json v2) only describes, it never decides:**
   - `folders: [{name, color, order[]}]` in sidebar order, where `name` is the directory name
   - `titles: {key → title}`
   - `game_dirs`
   - `pending` (see 6)
   
   `order` holds content keys. Reordering never touches files.
3. **Clip identity stays the content fingerprint** (ADR-0014). Titles, order and **thumbnails** (now cached by fingerprint rather than by path id) follow a clip through moves, so nothing regenerates.
4. **File operations** (window process, background IO, one at a time):
   - **Move clip:** `MoveFileExW` without `REPLACE_EXISTING` (atomic and instant on the same volume).
     - On a name clash the file gets ` (2)`, ` (3)`… and nothing is ever overwritten.
     - Across volumes (only possible through junctions): copy to `.partial`, verify size and fingerprint, rename, and only then delete the source.
     - "Not in a folder" moves a clip back to `<root>\<Game>\` (created if missing).
     - Refused while an export is running.
   - **Create folder:** a validated Windows name (no `<>:"/\|?*`, no reserved device names, no trailing dot or space), unique ignoring case among all entries in the root. `create_dir` fails rather than adopt an existing name.
   - **Rename folder:** a directory rename without replace. It fails cleanly if a file inside is in use.
   - **Delete folder:** its clips go back to their game folders first. Then the directory is removed **only if empty**. Anything else in it is kept and reported. No recording is ever deleted.
5. **Reconciliation on every scan** (when the window opens or refreshes; there is still no watcher or background service):
   - Learn game folders.
   - Register unknown user folders that hold clips.
   - When a registered folder's directory is gone and an unknown directory holds clips from its `order`, treat it as **renamed outside the app** and carry over the colour and order.
   - A registered folder whose directory is missing is kept in metadata, invisible, in case the drive comes back. Nothing is purged automatically.
   - If the clips root itself is unavailable, nothing is reconciled.
6. **Migration from v1 metadata folders:**
   - Each v1 folder becomes a v2 folder of the same (validated) name, and its directory is created if missing.
   - Its clips are **not moved automatically**. They're listed in `pending`, and the library asks once: "Move N clips into ‹folder›?" The owner chooses Move or Leave. Leave keeps the clips where they are and only drops the pending list.
   - The migration is idempotent.

## Consequences
- Explorer and the app always agree on what's in which folder. Only the visual order (and colours and titles) live in JondisClip.
- **Moving a recording** is now a real file operation:
  - It's only ever done on an explicit drag or menu choice.
  - It never overwrites.
  - It fails visibly: a file in use or a permission problem is reported, and the clip stays where it was.
- A user folder sharing its name with a game the recorder later records goes on working: those recordings simply land in that folder.

## Verification (2026-11-05)

These checks ran against a throwaway profile: `APPDATA`/`LOCALAPPDATA` pointed at `target\jc-sandbox` and its own clips root, with ffmpeg-generated sample clips. The real library on `D:\` was not touched. The release `jondisclip-ui.exe` was driven over DevTools.

Verified:
- **v1 migration.** The `organize.json` folder "Edited" became a prompt. Nothing moved until Move was clicked; then the directory was created and both clips moved in, keeping their title and Edit badge.
- **Explorer folders.**
  - A folder made in Explorer that holds clips is adopted.
  - A folder renamed in Explorer keeps its colour and order.
  - An existing non-game directory is adopted when you create a folder with its name.
- **Folder names.** Invalid names (`?`, reserved `CON`, the name of an existing game folder) are refused.
- **Moves never overwrite.**
  - A move onto an existing name became `… (2).mp4`. The existing file kept its size.
  - A clip held open with an exclusive handle is reported as in use and stays where it was.
- **Deleting a folder** moves its clips back to their game folders. The directory is kept when it holds other files.
- **Custom order** changes only the metadata. No files moved.
- **No lost content.** After all operations, the SHA-1 of every original sample file was still present on disk.
- **Thumbnails** kept their timestamps through every move, so none were regenerated.

Not verified:
- Cross-volume moves (copy, verify, delete). These are covered by code review only. There's no second volume under the clips root on the dev machine.
- The real library's migration. It runs when the owner first opens the new build.
