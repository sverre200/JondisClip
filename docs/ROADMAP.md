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
- [ ] App crate: tray icon, single-instance check, settings TOML, global hotkey → `save_clip`.
- [x] **Audio** (2026-11-04): WASAPI loopback of the default output (game/system audio) plus an optional mic (off by default), 48 kHz stereo, on the same QPC clock as video, mixed 100 ms behind real time into native AAC (160 kb/s per track). Track layout: Mix (default) / Game / Microphone, or a single track when only one source is on. The writer routes audio to segments by timestamp, so clips have no holes at either end. Verified: flash/beep A/V offset median +4 ms (−39 to +14 ms, 11 pairs, after the drift fix), 3-track mic run correct, CPU 0.2–0.7 %, devices reopened automatically.
- [ ] Process loopback for the detected game only (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`), so Discord voice and music stay out of clips. This needs game detection. For now, "Game" means all system audio.
- [ ] Supervisor: automatically restart the recorder after `DisplayChanged`, `SourceClosed`, device removal and `PBT_APMSUSPEND`/resume.
- [ ] Low disk space: warn, and stop buffering at the threshold.
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
PresentMon baseline vs buffering on 3 titles. Tune the queue sizes, thread priorities, GOP and segment length, and the conversion path. Only move on when the targets in `PERFORMANCE.md` are met or the owner accepts the gaps.

## Phase 4: UI and library
Tauri window with Library (grid, thumbnails, player, rename, delete to the Recycle Bin, open folder, copy path), Recording status and Settings. Trim with stream copy, and a share export.

## Phase 5: Sharing and cloud
Copy to clipboard, Discord size-targeted export, a hint for the Drive sync folder. Native Drive upload only if it's still wanted.

## Phase 6: Hardening
Real-game matrix (fullscreen, borderless, windowed, multi-monitor, HDR, 144 Hz), 8 h soak test, NVIDIA and Intel testers, installer (MSI/NSIS via Tauri), start with Windows, auto-update (optional, signed).

## Explicitly not in v1
Overlays, webcam, live streaming, social feed, accounts, AI highlights, a multi-track timeline editor, game hooking.
