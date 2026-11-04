# ADR-0008: The app window is a separate on-demand process (Tauri 2 + Svelte 5)

Status: **Accepted** (2026-11-04). This refines ADR-0001, which had the UI in the same process.

## Context
The settings (and later library) window must cost nothing while gaming. Tauri's event loop can't be restarted within a process, so "create the UI on demand, destroy it on close" inside the tray process would keep the Tauri runtime resident all the time. The tray is raw Win32 (ADR-0006).

## Decision
- **The window is `jondisclip-ui.exe`** (crate `ui/src-tauri`, frontend in `ui/`). The tray starts it on left-click, or via "Settings…", passing on foreground rights with `AllowSetForegroundWindow`. It exits when the window closes.
- **No IPC channel; the two processes share files:**
  - The UI writes `%APPDATA%\JondisClip\settings.toml`. The tray already applies changes within 2 s. The UI merges each change into the file as it is now, so changes made from the tray menu meanwhile aren't overwritten.
  - The tray publishes `%LOCALAPPDATA%\JondisClip\status.json` (atomic write, only on change): recording state, hotkey conflicts, encoder and audio errors. The UI polls it every 1.5 s.
  - The UI detects a running tray through its named mutex and can start it.
- **`jondisclip-core` crate:** shared settings model and validation, hotkey parsing, paths and status. The UI and tray validate identically, and the UI never links the engine or FFmpeg.
- **Frontend: Svelte 5 + TypeScript + Vite.** It has a tiny runtime, and the components are reused for the library later.
  - `src/tokens.css` is the only source of colours, spacing and type.
  - `src/lib/api.ts` falls back to a mock in a plain browser, so design work happens in the browser preview (`npm run dev`).
- The `custom-protocol` feature is on by default, so `cargo build` embeds `ui/dist`. Run `npm run build` in `ui/` before building the UI crate.

## Measured (2026-11-04, RX 6800 XT PC)
- Window visible about 0.3 s after launch. UI process about 34 MB plus 6 WebView2 helper processes while open.
- **After closing: 0 processes left** (WebView2 count back to baseline). The tray keeps recording throughout.
- Single instance: a second launch focuses the existing window.

## Consequences
- Two executables to ship side by side (the installer comes in Phase 6).
- Status in the UI can lag by up to about 1.5 s, and settings apply after up to about 2 s. That's fine for settings, though library actions such as "save now" may need a real channel later (named pipe). We'll decide that in Phase 4.
- Hotkey capture can't record a combination already registered globally (including the current save hotkey, which saves a clip instead). That's acceptable, and it effectively signals the key is taken.
