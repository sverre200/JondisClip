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
- [ ] **Stage 2: editor.** Trim handles on the player's timeline, trimmed duration and size estimate, and non-destructive export. Stream copy when the cut falls on keyframes, and a smart re-encode of the edges otherwise (to be decided in a new ADR). Runs as an FFmpeg child process at below-normal priority, one job at a time.
- [ ] **Stage 3: Discord.** A size-targeted H.264 export (default about 19 MB, configurable) with Mix audio only. It checks the final size and copies the file to the clipboard (`CF_HDROP`, ADR-0004).
- [ ] **Stage 4: Google Drive.** An optional synced-folder destination and "Save to Drive" (copy or export, with checks for free space and an unavailable destination). It never claims the upload finished.
- [ ] Later: rename, delete to the Recycle Bin, copy path. If the window is already open, "Settings…" should switch its page (today it only focuses the window).
Tauri window with Library (grid, thumbnails, player, rename, delete to the Recycle Bin, open folder, copy path), Recording status and Settings. Trim with stream copy, and a share export.

## Phase 5: Sharing and cloud
Copy to clipboard, Discord size-targeted export, a hint for the Drive sync folder. Native Drive upload only if it's still wanted.

## Phase 6: Hardening
Real-game matrix (fullscreen, borderless, windowed, multi-monitor, HDR, 144 Hz), 8 h soak test, NVIDIA and Intel testers, installer (MSI/NSIS via Tauri), start with Windows, auto-update (optional, signed).

## Explicitly not in v1
Overlays, webcam, live streaming, social feed, accounts, AI highlights, a multi-track timeline editor, game hooking.
