# ADR-0013: Save to Drive copies into the Google Drive for desktop folder; Drive does the upload

Status: **Proposed** (2026-11-04). Implemented (see ROADMAP Phase 4, stage 4). This implements ADR-0004 "Option A" as an explicit action rather than pointing the whole save folder at Drive.

## Context
The owner wants long-term storage of chosen clips in Google Drive, initiated by them, with no OAuth, cloud APIs, accounts or background uploads. Google Drive for desktop already syncs a local folder, usually `G:\My Drive`, or `%USERPROFILE%\My Drive` in mirror mode. Pointing the whole save folder at Drive would upload every 600 MB recording, so the user picks clips instead.

## Decision
1. **Setting `drive_dir`** (empty = off). Settings → Sharing → "Google Drive folder" has a folder picker and suggests the folders Drive for desktop creates, if they exist: `X:\My Drive` on any drive letter, or `%USERPROFILE%\My Drive`.
2. **Save to Drive** in the editor (shown once a folder is set):
   - **Whole clip:** a byte-for-byte **copy** of the recording into `<drive_dir>\JondisClip\<Game>\<file name>`.
   - **Selection:** a trim written **directly** into that folder by the exporter (ADR-0011: lossless or exact, all tracks). No extra local file is made.
   - **No redundant copies.** If a file with the same name and size is already there, nothing is copied ("Already in your Drive folder"). A different file with that name gets ` (2)`.
3. **Checks before writing:**
   - The folder exists. If it doesn't, the message is "Google Drive isn't available: is Google Drive for desktop running and signed in?"
   - There's free space for the file plus a reserve on the destination volume: 2 GB if it's the recorder's drive, otherwise 256 MB.
   - The destination is not inside the save folder and not the share folder, so the copy can't be mistaken for a recording or get pruned.
4. **Copying:**
   - Done in the window process on a background-IO-priority thread, in 4 MB chunks, with progress and cancel.
   - Written to a `.partial` name and renamed when complete (a hard link, never overwriting), so Drive only ever syncs whole files.
   - Cancel or an error removes the `.partial`. If the window closes mid-copy, the leftover `JondisClip\**\*.mp4.partial` in the Drive folder is removed the next time the window checks Drive. Drive may sync a `.partial` meanwhile. **The original is only read.**
5. **Wording:** the result says **"Copied to your Google Drive folder. Google Drive for desktop uploads it."** JondisClip never claims the upload happened. It can't know, and it doesn't ask Drive.

## Review outcomes (perf-reviewer, 2026-11-04)
- **Free space** is compared per volume (volume GUID, so mount points, SUBST and `\\?\` paths all match). Drive's streaming drive is a non-NTFS/ReFS volume that caches files in `%LOCALAPPDATA%` until it uploads them, so the copy also requires the file plus 2 GB free there, on the recorder's volume.
- **"Already there"** means same name, same size *and* same modified time (within 2 s). Our copies keep the source's time, so a different file of the same size is never mistaken for it.
- **Never overwrite:** a hard link, or if the volume has none (FAT, Drive's virtual drive), `MoveFileExW` without replace, which fails atomically. The same applies to the exporter's own finalize.
- **Cache-friendly IO:** the copy reads with `FILE_FLAG_SEQUENTIAL_SCAN` and writes with `FILE_FLAG_WRITE_THROUGH` on a background-IO thread, so it doesn't crowd the game's assets out of the cache.
- **Overlap checks** (after resolving links) refuse a Drive folder inside JondisClip's own folder (buffer, cache, share) or overlapping the clips folder, in both directions. Changing the clips folder later is checked too.
- **Small fixes:** network drive letters are skipped when looking for Drive folders. Show-in-Explorer handles UNC paths and runs off the UI thread. The editor ignores stray events from older jobs.

## Consequences
- Zero network code, credentials or background work. Uploads, retries and conflict handling stay Drive's job.
- If Drive is set to "stream" files, `G:` is a virtual drive, and the free space it reports is the Drive quota as Windows sees it. A full quota shows up as a write error, which is reported.
- **Not verified on this PC:** Google Drive for desktop isn't installed here. Everything was tested against ordinary folders and a missing drive letter.

## Update 2026-11-05: Google Drive settings section and verified copies
- Settings has its own **Google Drive** section:
  - Choose, change or turn off the folder.
  - A check, run when Settings opens and on demand: the folder exists, doesn't overlap the clips folder or JondisClip's own folders, and a small `.jondisclip-write-test-<pid>.tmp` can be written, read back and removed.
  - Open in Explorer.
  - A "Keep my folders" option (on by default). It names the Drive subfolder after the clip's library folder (ADR-0015); off, it uses the game.
- **Whole-clip copies are verified before they get their real name.** The size and the content fingerprint (read back from disk) must match the source. Otherwise nothing is saved and an error is shown. Trims are new encodes and aren't compared with the source.
- **The wording never claims an upload.** It says "Saved in your Google Drive folder · uploads when Google Drive for desktop syncs".
- **Verified on 2026-11-05** in a sandbox profile, with a plain folder standing in for Drive:
  - check ok / missing / overlapping clips / inside JondisClip's folders
  - no test file left behind
  - copy byte-identical (SHA-1)
  - "already there" on repeat
  - Keep my folders on (Edited) and off (DayZ)
- **Still not verified:** real Google Drive for desktop and the cloud upload. The write test does briefly create a file in the synced folder; Drive may see it before it's removed.
