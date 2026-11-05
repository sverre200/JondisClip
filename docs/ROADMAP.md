# Roadmap

## Phase 1: Research and architecture (current)
- [x] Environment inspection, encoder probe, zero-copy spike (AMF + Desktop Duplication)
- [x] Proposed ADRs 0001–0004, architecture, performance plan, project skills
- [x] Owner approval of the stack (2026-11-04)
- [ ] Owner decision: recording codec default, licence (personal vs released)
- [x] Toolchain install: Rust, VS Build Tools, LLVM, PresentMon (2026-11-04)
- [ ] LGPL FFmpeg 9.0 dev build in third_party/ffmpeg/9.0 (owner to download)

## Phase 2: Minimal working recorder (headless + tray)
- [x] **Video pipeline** (2026-11-04, `crates/engine`): WGC monitor capture → GPU copy → D3D11 video processor BGRA→NV12 (BT.709 limited) → FFmpeg D3D11VA hwframes → `hevc_amf` (H.264 fallback) → 2 s MPEG-TS segments → ring retention → save as a stream-copy MP4 (faststart, `hvc1`). Verified on the RX 6800 XT at 1080p60: 0 late or dropped frames, about 0.3 % CPU, about 68 MB working set, about 0.25 s to save 10 s. The save cut lands at the save moment (forced IDR). Run it with `cargo run --release -p jondisclip-engine --example record`.
- [x] Reviewed by `perf-reviewer`. Fixed: AMD `forced_idr`, the save/IDR ordering race, encoder EAGAIN under GPU saturation, the shutdown hang after a thread panic, a dead writer looping on IDRs, resolution change detection (ContentSize), monitor removal (`Closed`), leaked pins on save timeout, long stalls inflating segment duration (it now starts a new run), deletes under the ring lock, and the buffer location plus a stale-session sweep.
- [x] **Tray app** (2026-11-04, `crates/app`, ADR-0006):
  - Tray icon and menu: save, pause, replay length, mic, open folder, edit settings, quit.
  - Global hotkey (Alt+F10), settings TOML with live reload, clip naming by foreground app.
  - Single instance, start with Windows (off by default), local log.
  - Verified: hotkey save, second instance refused, live settings reload (replay length applied without restart, invalid hotkey rejected), clean quit with the buffer removed. Idle cost while buffering: 0.28 % CPU, 73 MB working set, one process.
- [x] Supervisor: automatic restart after errors and display changes, stop/restart around sleep, pause below 2 GB free disk. *Not yet exercised on real sleep/resume, a resolution change or low disk. Those code paths are untested.*
- [x] Fixed while testing:
  - A clip including the session's first segment failed to save (negative AAC priming timestamps). The clip writer now also skips any non-advancing packet.
  - The start-up sweep could delete another live session. Sessions are now locked.
- [x] **Audio** (2026-11-04): WASAPI loopback of the default output (game/system audio) plus an optional mic (off by default), 48 kHz stereo, on the same QPC clock as video, mixed 100 ms behind real time into native AAC (160 kb/s per track). Track layout: Mix (default) / Game / Microphone, or a single track when only one source is on. The writer routes audio to segments by timestamp, so clips have no holes at either end. Verified: flash/beep A/V offset median +4 ms (−39 to +14 ms, 11 pairs, after the drift fix), 3-track mic run correct, CPU 0.2–0.7 %, devices reopened automatically.
- [x] **Mic audio overhaul** (2026-11-04, [report](perf/2026-11-04-mic-audio.md), ADR-0009):
  - Clock correction by resampling (no more zero-sample clicks or slips; 0 in simulation vs 2.4–2.9/s).
  - Mono mic track, live mic gain, transparent −1 dBFS limiter on Mix and Mic, level meter, warnings for virtual and disconnected mics.
  - A/V re-checked unlocked: median −3 to −8 ms, and Mix and Game are sample-aligned.
  - G-Track Pro tuning ([report](perf/2026-11-04-gtrack-pro.md)): the right device selected, plus a mic check (level, clipping, noise, mode, playback) and an optional 80 Hz low-cut.
  - **To do:** listening test in DayZ.
- [ ] Process loopback for the detected game only (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`), so Discord voice and music stay out of clips. This needs game detection. For now, "Game" means all system audio.
- [ ] Crash recovery: offer "Recover last buffer" before the stale-session sweep deletes it.
- [ ] Known gaps from review, still to do:
  - Segment files at background IO priority (`FileIoPriorityHintInfo`).
  - Handle the 33-bit MPEG-TS timestamp wrap (sessions over about 26.5 h).
  - Fall back to the next encoder if opening with hwframes fails (e.g. MF).
  - Capture adapter selection on hybrid-GPU laptops.
  - `CoDecrementMTAUsage` on stop.
  - Audio review leftovers: on stop the last ~100 ms of audio isn't encoded (mixer latency), and a little audio is lost at a NewRun boundary. AAC packets aren't pooled, so there's one small allocation per packet, the same as video.
- [ ] Integration test (`#[ignore]`, GPU machine): record → save → ffprobe assertions, with A/V drift < 40 ms once audio exists.

**Exit criteria:** a 5-minute clip saved from DayZ plays correctly with in-sync audio, and the buffer runs 2 h without leaks or drops.

## Phase 3: Performance
- [x] **Rate-control tuning** (2026-11-04, [report](perf/2026-11-04-rate-control.md), bench in `tools/rc-bench/`):
  - Keep capped VBR and add a per-preset quality floor (qmin 21 / 18 / 16). Calm scenes are about 55–60 % smaller for about 1–2 VMAF, and busy scenes are unchanged.
  - Rejected: QVBR, which is broken on AMF H.264, and CQP, whose caps are ignored.
  - **To do:** confirm on a real DayZ sample, and tune NVENC/QSV on that hardware.
PresentMon baseline vs buffering on 3 titles. Tune the queue sizes, thread priorities, GOP and segment length, and the conversion path. Only move on when the targets in `PERFORMANCE.md` are met or the owner accepts the gaps.

## Phase 4: UI and library
- [x] **Settings window** (2026-11-04, ADR-0008): `jondisclip-ui.exe`, Tauri 2 + Svelte 5.
  - Settings: replay length, hotkey capture, save folder picker, sound, quality, resolution, frame rate, audio, start with Windows, codec (advanced).
  - Live tray status, live size estimates, autosave with toast.
  - Opened by left-clicking the tray icon or choosing Settings….
  - Verified: browser preview (interactions, validation, 640×520 minimum size) and the real window (live data, single instance). After closing, 0 processes remain.
  - Not yet clicked by hand in the real window: the folder picker and changing a setting there (the merge logic is unit-tested and the tray's file reload was verified earlier).
- [x] **Clip library, stage 1** (2026-11-04, ADR-0010): Library and Settings pages in `jondisclip-ui.exe`. The tray's left-click opens the library, "Settings…" opens settings, and there's a new "Clip library" menu item.
  - Grid with lazy thumbnails (only on-screen cards, one decode at a time, cached as JPEG, dark frames skipped). It's grouped by day and filtered by game, with search, four sorts, keyboard navigation and a selection bar (Open, Show in folder, Open in player).
  - The player has play/pause, a seek bar (coalesced seeks while dragging), J/K/L, arrow and frame keys, and previous/next clip. It pauses when the window loses focus.
  - Unreadable clips are listed with a reason. HEVC without the codec extension falls back to "Open in default player".
  - Verified:
    - 20 Rust tests: discovery and sorting, odd names and paths, broken files, the thumbnail cache, range serving, read-only originals.
    - Browser preview of all states.
    - The real window on 6 real clips: thumbnails, playback, seeks of 5–128 ms in a 661 MB clip, Esc back.
    - After closing, 0 UI processes remain and the tray keeps recording.
  - **Not verified:** a long library (1000+ clips) and the FPS impact of playing a clip while a game records. The player pauses on blur, so this needs a `/perf-benchmark` run.
- [x] **Stage 2: editor** (2026-11-04, ADR-0011, [report](perf/2026-11-04-trim-export.md)):
  - The player's timeline has red trim handles (drag them, or use I/O at the playhead, or arrow keys on a focused handle). It shows the selection, its length and an estimated size, and playback stops at the end of the selection. **Save clip** (Ctrl+S) writes `<name> edit.mp4` next to the original, which is never modified.
  - **Lossless · exact** is automatic when the start is on a keyframe. Otherwise you choose **Exact** (video re-encoded at 1.3× the source bitrate, audio copied) or **Lossless · N s earlier**.
  - `jondisclip-export.exe` runs as a below-normal-priority child in a kill-on-close job, with progress, cancel and a free-space check.
  - Measured: a lossless 20 s cut takes 0.12 s and an exact one 5.3 s. 0 late or dropped recorder frames over 12 back-to-back exports (desktop, no game).
  - **Not verified:** export while a game is running (needs `/perf-benchmark`), NVENC, QSV and MF on real hardware, and HEVC sources in Exact mode (output is H.264).
- [x] **Stage 3: Discord** (2026-11-04, ADR-0012, [report](perf/2026-11-04-discord-share.md)):
  - **Share to Discord** is the editor's main button. It works on the selection or the whole clip, and before you click it shows what you'll get ("Discord: 720p60 under 19 MB", or "Too long for 19 MB: trim it to under 10:53").
  - It makes an H.264 Main file with the Mix track only, in `%LOCALAPPDATA%\JondisClip\share` (pruned after 14 days), and puts **the file on the clipboard**, so you paste it into a chat with Ctrl+V.
  - It's a lossless copy when that fits. Otherwise it follows the quality ladder (1080p60 → 720p60 → 720p30 → … 360p30), encoded CBR and measured, with one smaller retry. A file over the limit is never handed over.
  - Settings → Sharing → Discord upload limit: 10 / 19 (default) / 50 / 500 MB.
  - **Not verified:** pasting into the Discord app itself (the clipboard was read back by Windows' own API), NVENC/QSV CBR behaviour, and a game running during the export.
- [x] **Stage 4: Google Drive** (2026-11-04, ADR-0013):
  - Settings → Sharing → **Google Drive folder**. It's optional, and it suggests `X:\My Drive` or `%USERPROFILE%\My Drive` when Google Drive for desktop is installed.
  - The editor's **Save to Drive** copies the whole clip byte for byte, or writes the selection as a trim, into `<Drive>\JondisClip\<Game>\`. Drive for desktop does the upload, and JondisClip never claims it happened.
  - Nothing is copied twice: the same name and size is reported as "Already in your Google Drive folder".
  - It refuses a missing folder (Drive not running), a folder that overlaps the clips folder or our own folders, and a destination without room for the file plus a reserve (2 GB on the recorder's drive, otherwise 256 MB).
  - The copy runs on a background-IO thread with progress and cancel, written as `.partial` and renamed without overwriting.
  - Verified in the real window against a stand-in folder: byte-identical copy (hash), "already there", trim into Drive, and the "isn't available" error. Plus 4 new Rust tests.
  - **Not verified:** a real Google Drive for desktop install (not on this PC), so there's no check yet that Drive picks up and uploads the file.
- [x] **Organisation** (2026-11-04, ADR-0014):
  - A collapsible **Folders** sidebar (All clips, Not in a folder, your folders) with muted colour icons and counts. You can create, rename, recolour (7-colour palette), delete (recordings are kept) and drag folders to reorder them (or Alt+↑/↓).
  - **Clip titles**: F2, right-click → Rename, or the Rename button in the selection bar. The title replaces the game name on the card, and the game moves to the metadata line. Search matches titles, and Share to Discord names the file after the title.
  - **Drag and drop**: drag one or several (Ctrl/Shift-click) clips onto a folder, or onto "Not in a folder". Inside a folder in **Custom order** (the default there), dragging rearranges clips with an insertion bar. With a search or game filter active, it explains why it can't reorder.
  - **Context menus**: Open, Rename, Move to folder ▸, Show in folder, Copy file, Open in default player, Delete… (Recycle Bin, with confirmation).
  - Organisation is metadata in `library\organize.json` (atomic writes and a `.bak`), keyed by a content fingerprint, so it survives renames and moves. Recordings never move.
  - Verified on the real clips: everything persisted across a reload, the Edit badges stayed on all 6 edits, playback worked, and the recordings were byte-identical afterwards. 15 new Rust tests.
- [x] **Folders, Drive, themes and ordering overhaul** (2026-11-05):
  - **Real Windows folders** (ADR-0015): the app and Explorer match. Explorer changes are picked up. Moves never overwrite. Old metadata folders ask before moving anything.
  - **Settings → Google Drive:** a folder check with a write test, Open in Explorer, "Keep my folders". Whole-clip copies are verified by size and fingerprint.
  - **Six accent themes** (ADR-0016), with the Windows 11 title bar tinted. Status colours are fixed.
  - **Name sort** and a sort remembered per view. Reordering works with a filter on (hidden clips keep their places). Alt+←/→ moves clips. Dragging under another sort is refused with a hint.
  - All verified in a sandbox profile on sample clips; real Google Drive was not tested.
- [ ] Later: export a folder as real files, and folder sharing. If the window is already open, "Settings…" should switch its page (today it only focuses the window).
Tauri window with Library (grid, thumbnails, player, rename, delete to the Recycle Bin, open folder, copy path), Recording status and Settings. Trim with stream copy, and a share export.

## Phase 5: Sharing and cloud
Copy to clipboard, Discord size-targeted export, a hint for the Drive sync folder. Native Drive upload only if it's still wanted.

## Phase 6: Hardening
Real-game matrix (fullscreen, borderless, windowed, multi-monitor, HDR, 144 Hz), 8 h soak test, NVIDIA and Intel testers, installer (MSI/NSIS via Tauri), start with Windows, auto-update (optional, signed).

## Explicitly not in v1
Overlays, webcam, live streaming, social feed, accounts, AI highlights, a multi-track timeline editor, game hooking.
