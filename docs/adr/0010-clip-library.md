# ADR-0010: The clip library lives in the on-demand UI process, with a lazy cache and WebView decoding

Status: **Proposed** (2026-11-04)

## Context
Phase 4 adds a library and an editor on top of the recorder. They must cost nothing while closed (non-negotiable 6), never touch the recorder or modify originals (non-negotiables 2 and 3), and handle hundreds of five-minute clips.

## Decision
1. **Same process as settings.** `jondisclip-ui.exe` gets two pages, Library and Settings. The tray opens the library on left-click and settings from "Settings…" (`--settings`). There's no new process, service, file watcher or background indexer: nothing runs while the window is closed.
2. **Discovery is a scan when the window opens** (or on refresh), done in `jondisclip-core::library`:
   - It reads `<save_dir>\*.mp4` and `<save_dir>\<App>\*.mp4` and skips `.mp4.partial`.
   - The game comes from the folder name. The time comes from the `<App> YYYY-MM-DD HH-MM-SS[ (n)].mp4` name, falling back to the file's modified time.
   - Metadata (duration, resolution, codec, audio track count) comes from a **minimal MP4 box reader**. It only descends `moov/trak/mdia/minf/stbl` and seeks past everything else, so it reads a few KB per file.
   - Results are cached in `%LOCALAPPDATA%\JondisClip\library\index.json`, keyed by path plus size plus modified time. Unchanged files are never reopened. A file that can't be parsed is listed as unreadable, not hidden.
3. **Playback and thumbnails use WebView2's own decoder.**
   - Clips are served through a custom `clip:` protocol with HTTP range support. It only serves IDs from the current scan, so there are no arbitrary paths and no traversal.
   - Thumbnails are drawn from a `<video>` into a canvas: **one at a time, only for cards on screen**, as 384×216 JPEG.
   - They're stored in `library\thumbs\<hash>.jpg`, keyed by path, size and modified time. Orphans are pruned on scan.
   - The thumbnail frame is 8 s before the end, because a save is "the last N minutes" and the moment is near the end. If that frame is nearly black, the middle is tried, then a quarter in.
   - This keeps FFmpeg out of the UI process for browsing (ADR-0008). WebView2 decodes with the GPU's video engine. On AMD (VCN) that engine also runs the recorder's encoder, hence the pause-on-blur rule below.
4. **Edits and exports (stages 2–4)** will run FFmpeg as a child process of the UI, at below-normal priority, one job at a time. Originals stay read-only, and outputs are new files. The exact export path gets its own ADR.

## Review outcomes (perf-reviewer, 2026-11-04)
- The protocol is served by **2 below-normal-priority workers** from a bounded queue (32, beyond that 503) in 1 MB ranges, not by a thread per request. While dragging, seeks are coalesced to one per frame (`fastSeek`), with an exact seek on release.
- Scans and thumbnail writes run in **background IO mode** (`THREAD_MODE_BACKGROUND_BEGIN`). Scans are serialised in Rust and coalesced in the page, with a unique temp index name.
- The **player pauses** when the window is hidden or loses focus, because the GPU's video engine is shared with the recorder's encoder.
- MP4 reader limits: 10,000 boxes and 64 tracks per file. Transient IO errors (a locked file) aren't cached as "broken". Leftover `*.jpg.tmp` files are pruned.
- Confirmed: Rust's `File::open` shares read, write and delete, so the library can't make the recorder's `.mp4.partial` rename fail, and `.partial` and `.ts` files are never opened.

## Consequences
- **Opening the library costs:**
  - one directory scan
  - a header read for each new clip
  - one frame decode for each thumbnail not yet cached
  
  None of this happens while the window is closed.
- HEVC/AV1 clips only preview if Windows has the codec extensions. Otherwise the card and player show "can't preview" and offer "Open in default player".
- WebView2 plays only the first audio track of a multi-track MP4, which is our Mix. That's the right track for previewing.
