# JondisClip

Lightweight, always-on game clipping for Windows. It keeps a rolling replay buffer and saves the last N minutes when you press a hotkey. **Gaming performance comes first.** Every design decision answers to that, then to recording quality, then to simplicity.

Status: **Phase 1 → 2.** Stack accepted (ADR-0001, 2026-11-04). ADR-0007 (H.264 by default, which supersedes ADR-0005's HEVC default) accepted. ADR-0002 to 0004 are still *Proposed*. Code so far:
- `crates/engine`: video and audio replay pipeline.
- `crates/app`: `jondisclip.exe`, the tray app with hotkey and supervisor.
- `crates/core`: shared settings, hotkey, paths and status.
- `ui/` + `ui/src-tauri`: `jondisclip-ui.exe`, the clip library and settings window (Tauri 2 + Svelte 5, separate process, ADR-0008/0010). Library stage 1 (browse, thumbnails, player), stage 2 (trim editor, `crates/export` = `jondisclip-export.exe`, ADR-0011) stage 3 (Share to Discord: size-targeted file on the clipboard, ADR-0012) stage 4 (Save to Drive into the Drive for desktop folder, ADR-0013) and organisation (colours, titles, drag-and-drop order, ADR-0014) are done. The overhaul on 2026-11-05 added: real Windows folders (ADR-0015), a Google Drive Settings section with verified copies (ADR-0013 update), six accent themes (ADR-0016), and a Name sort with per-view sort memory, reorder under filters and Alt+←/→. Next: a DayZ perf run with the library open, then Phase 3 perf benchmarks and Phase 6 hardening.
- To drive the real window in tests, launch it with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` and use the DevTools protocol. Synthetic mouse input doesn't reach it from the agent shell.
- **Agent-launched processes are MSIX-virtualised.** Anything started from the Claude desktop app's shell (the tray, the window, scripts) has its `%APPDATA%`/`%LOCALAPPDATA%` writes and HKCU registry writes redirected to `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\…` and a private hive. The owner's normally started app never sees them. This happened on 2026-11-04: the "Start with Windows" Run key, the mic choice and the library folders all landed there. So:
  - Never use an agent-launched app to change the owner's real settings or library.
  - Treat real-window tests as writing to a sandbox copy.
  - Hand the owner a command to run in their own PowerShell when real data must change.
  - D:\ and C:\dev are not redirected.
  - **Safe real-window tests:** every path comes from `%APPDATA%`/`%LOCALAPPDATA%` (`crates/core/src/paths.rs`), so set both to folders under `D:\JondisClip-Test` with a `settings.toml` whose `save_dir` is a test folder. The window then has its own library, cache and settings, and never sees the owner's. Make real recorder clips for it with `--example record` under the same variables (its buffer moves too, so it can run beside the owner's tray). The tray itself can't be tested this way while the owner's is running: it's single-instance and owns Alt+F10.
- The project lives in `D:\Claude-Projects\JondisClip` (moved from `C:\dev\JondisClip` on 2026-11-05). Build output (`target\`) wasn't copied, so the first build there is a full one.

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
- Run the app with `cargo run --release -p jondisclip`. Logs go to `%LOCALAPPDATA%\JondisClip\jondisclip.log` and settings to `%APPDATA%\JondisClip\settings.toml`. Only one instance can run. Quit it from the tray before starting another build. Never run `--example record` while the tray app is running: they share the buffer root.
- UI:
  - Design in the browser with the mock API: `npm --prefix ui run dev` (`.claude/launch.json` → `ui-dev`). Use only tokens from `ui/src/tokens.css`.
  - Before `cargo build -p jondisclip-ui`, run `npm --prefix ui run build`. The exe embeds `ui/dist`.
  - Build `jondisclip-export` as well (`cargo build --release -p jondisclip -p jondisclip-ui -p jondisclip-export`). The window looks for the exporter next to its own exe.
  - `npm --prefix ui run check` must pass. It also checks the Norwegian dictionaries (ADR-0018).
  - User-visible text goes through `t()`/`tn()` in the UI and `jondisclip_core::tr!` in Rust, with bokmål and nynorsk added in `ui/src/lib/i18n/{nb,nn}.ts` and `crates/core/src/i18n_table.rs`. Logs stay English.
  - Quit the tray before rebuilding, because Windows locks the running exe.
- Rust: `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace` must pass. Hot-path crates use no `unwrap()` and return typed errors (`thiserror`).
- Frontend: TypeScript strict, verified visually in the browser pane against the Vite dev server.
- Record important decisions as a new ADR (`docs/adr/NNNN-title.md`).
- Shareable build: `powershell -ExecutionPolicy Bypass -File tools\package.ps1` → `dist\JondisClip-<version>-win64.zip` (portable, static C runtime, only the FFmpeg DLLs in use, README and licence notices from `tools\package\`). The script loads the Visual C++ environment itself, so a plain PowerShell works.
- Release to friends (ADR-0020): commit, then `powershell -ExecutionPolicy Bypass -File tools\release.ps1 <version>`. It bumps the version, packages, tags `v<version>`, pushes and creates the GitHub release. Running apps offer it as "Install update" within 6 hours. The repo is public (since 2026-11-12): never commit secrets.
- Intel (QSV) recording works since 2026-11-12 (verified on an Arc 130V; `hwenc.rs` feeds the encoder QSV surfaces derived from the D3D11 device). NVENC is still unverified. `cargo run -p jondisclip-engine --example hwopen` tests the encoder path on any PC. Fallbacks and what is verified per vendor: ADR-0021. Exports convert to NV12 for Intel and Media Foundation and end with Windows' software H.264 encoder, so they work on any PC. The export tests need an `ffmpeg` with libx264 on PATH; the bundled LGPL build has none, so they skip without one.
