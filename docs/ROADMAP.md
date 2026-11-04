# Roadmap

## Phase 1: Research and architecture (current)
- [x] Environment inspection, encoder probe, zero-copy spike (AMF + Desktop Duplication)
- [x] Proposed ADRs 0001–0004, architecture, performance plan, project skills
- [x] Owner approval of the stack (2026-11-04)
- [ ] Owner decision: recording codec default, licence (personal vs released)
- [x] Toolchain install: Rust, VS Build Tools, LLVM, PresentMon (2026-11-04)
- [ ] LGPL FFmpeg 9.0 dev build in third_party/ffmpeg/9.0 (owner to download)

## Phase 2: Minimal working recorder (headless + tray)
1. Cargo workspace: `engine` (lib), `app` (bin). Tray icon, single-instance check, settings TOML.
2. WGC monitor capture → D3D11 NV12 conversion → FFmpeg `hevc_amf`/`h264_amf` (hardware frames) → TS segments.
3. Ring retention and pinning. Save-clip job (concat → MP4). Global hotkey.
4. WASAPI loopback and mic → AAC, A/V sync.
5. Robustness: device loss, display change, sleep and resume, low disk space, crash recovery.
6. Tests: unit tests for ring math and retention, integration tests for the encoder producing valid segments, and ffprobe checks of saved clips (duration, A/V drift < 40 ms).

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
