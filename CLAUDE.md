# JondisClip

Lightweight, always-on game clipping for Windows. It keeps a rolling replay buffer and saves the last N minutes when you press a hotkey. **Gaming performance comes first.** Every design decision answers to that, then to recording quality, then to simplicity.

Status: **Phase 1 → 2.** Stack accepted (ADR-0001, 2026-11-04). ADR-0005 (HEVC record, H.264 share) accepted. ADR-0002 to 0004 are still *Proposed*. Code so far: `crates/engine` (replay ring index + encoder probe).

## Read first
- `docs/ARCHITECTURE.md`: components, processes, threads and data flow
- `docs/adr/`: architecture decisions. Their status is *Proposed* until the owner approves them. Never contradict an *Accepted* ADR silently; write a new ADR that supersedes it.
- `docs/PERFORMANCE.md`: targets and benchmark method. Don't claim a perf result without a run recorded there.
- `docs/ROADMAP.md`: phases and current scope. Challenge anything that is outside the current phase.

## Non-negotiables (enforce in review)
1. Nothing on the capture/encode hot path allocates per frame, blocks on IO, takes a contended lock, or copies a frame GPU→CPU.
2. Saving a clip never stops or stalls the buffer.
3. Originals are never modified or deleted automatically. Edits and exports write new files.
4. No DLL injection or hooking into game processes (anti-cheat risk). Capture uses OS APIs only.
5. No telemetry, no required account, no automatic uploads unless the user opts in.
6. When the window is closed the UI is destroyed, not hidden. The resident footprint is the engine plus the tray only.
7. Secrets (OAuth tokens) live in Windows Credential Manager, never in config files or logs.

## Dev machine facts (2026-11-04)
- Ryzen 7 5800X (no iGPU), **AMD RX 6800 XT (RDNA2)**, 32 GB RAM, Windows 11.
- AMF H.264 and HEVC work. **AV1 encoding is not available** on RDNA2 (verified). NVENC and QSV can't be tested locally, so treat those paths as unverified until they run on real hardware.
- `ffmpeg`/`ffprobe` (gyan essentials build) are on PATH and only suitable for experiments. The shipped app will bundle its own LGPL build.
- Installed and verified (a smoke build created a D3D11 video device):
  - Rust 1.99.1, `stable-x86_64-pc-windows-msvc`
  - VS 2022 Build Tools with the VC tools and Windows SDK 10.0.26100
  - LLVM 23 (provides `libclang` for `ffmpeg-sys-next` bindgen)
  - PresentMon Console 2.6 (`presentmon` on PATH via the WinGet Links folder)
- Rust is in `~/.cargo/bin`. Shells opened before the install need it prepended to `PATH`.
- FFmpeg dev build: put it in `third_party/ffmpeg/9.0/` (BtbN `ffmpeg-n9.0-latest-win64-lgpl-shared-9.0.zip`, gitignored), then set `FFMPEG_DIR` to that path. It matches the `ffmpeg-next` 9.x crate.

## Project skills and agents
- `/media-pipeline`: rules for capture, encode, buffer and mux code
- `/clip-ops`: FFmpeg recipes for concat, trim, export and thumbnails, plus how to verify output with ffprobe
- `/perf-benchmark`: how to measure FPS impact and record results
- Agent `perf-reviewer`: read-only review of hot-path diffs
- Global agents `software-architect`, `ui-ux-designer` and `qa-engineer` cover the rest.

## Conventions
- Rust: `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` must pass. Hot-path crates use no `unwrap()` and return typed errors (`thiserror`).
- Frontend: TypeScript strict, verified visually in the browser pane against the Vite dev server.
- Record important decisions as a new ADR (`docs/adr/NNNN-title.md`).
