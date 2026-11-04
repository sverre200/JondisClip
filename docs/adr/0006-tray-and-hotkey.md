# ADR-0006: Tray, hotkey and supervision in raw Win32, separate from the future UI window

Status: **Accepted** (2026-11-04, approved by owner when they asked for the tray and hotkey)

## Context
JondisClip has to sit in the tray all day with near-zero cost, answer a global hotkey instantly, and keep the recorder alive across display changes, sleep and errors. The Library/Settings UI (Phase 4) is a Tauri 2 / WebView2 window that only exists while it's open (ADR-0001).

## Options
- **Tauri tray and global-shortcut plugins:** the tray would live inside Tauri's event loop. That pulls in the tray-icon/muda/global-hotkey crates, and tray behaviour would be tied to the UI framework.
- **Raw Win32 on its own message loop (chosen):** `Shell_NotifyIconW`, `RegisterHotKey` and `TrackPopupMenuEx` on a hidden top-level window. It adds zero dependencies and is about 700 lines. A Tauri window added later can run in the same process with its own loop, and the tray code doesn't change.

## Decision
- `crates/app` (`jondisclip.exe`, no console in release builds) owns the tray, the hotkey, the recorder and a 1 s supervisor timer:
  - It restarts the recorder after it stops unexpectedly (backoff 2/5/10/30/60 s).
  - It stops on `PBT_APMSUSPEND` and restarts about 3 s after resume.
  - It restarts on `WM_DISPLAYCHANGE`.
  - It pauses below 2 GB free (buffer or save volume) and resumes above 3 GB.
  - It re-adds the icon on `TaskbarCreated` (Explorer restart).
- The default hotkey is **Alt+F10**. `RegisterHotKey` swallows the key system-wide, so letters and digits need a modifier. F-keys, PrintScreen, Pause, ScrollLock and Insert may be used alone.
- Save feedback works in exclusive fullscreen and has no overlay:
  - An immediate short system sound (setting: `save_sound`).
  - A 1.5 s green tray icon flash.
  - A quiet notification when the file is written (`NIIF_RESPECT_QUIET_TIME`, so Windows holds it during Focus/game mode).
- Settings live in `%APPDATA%\JondisClip\settings.toml`. Edits are picked up within 2 s. Invalid values fall back to defaults with a log warning. Replay length changes apply live (`Recorder::set_replay`), and pipeline settings restart the recorder.
- Clips are named `<save dir>\<App>\<App> YYYY-MM-DD HH-MM-SS.mp4`. App is the foreground program's executable, with friendly names for known games and apps.
- It's single-instance (named mutex). A second launch makes the running one show "already running", then exits.
- "Start with Windows" uses the per-user `HKCU\…\Run` value (`--background`). It's off by default.
- Each buffer session holds an exclusive `session.lock` until its last segment is gone, so the start-up stale-session sweep never deletes a live session or a save in progress.

## Consequences
- On Windows 11, new tray icons start in the overflow (**^**). Apps can't pin themselves, so the user pins JondisClip once. Mention this in onboarding (Phase 4).
- "Edit settings…" opens the file in Notepad until the Settings UI exists (Phase 4).
