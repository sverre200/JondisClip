# ADR-0022: First-run welcome screen and "Check now"

Status: **Proposed** (2026-11-12). Extends ADR-0020 (updates) and ADR-0008 (the window is a separate process).

## Context
A new user who double-clicked `jondisclip.exe` saw nothing but a tray icon, often hidden behind the `^` arrow. Recording started with defaults they never saw: no microphone, clips in `Videos\JondisClip`, no autostart. And the only way to find out about an update was to wait for the automatic check.

## Decision
- **First run opens the window.** When the tray starts and `settings.toml` doesn't exist yet, it creates the file and opens the library window. Later starts stay silent in the tray, as before.
- **Welcome screen.** While the new `setup_done` setting is false, the window shows a welcome card over the library (`ui/src/Welcome.svelte`). It says what the app does and where it lives, and offers the choices a new user should make once:
  - language, save hotkey, save location, record my microphone (and which one), start with Windows.
  - Every row saves at once, exactly like Settings. **Start clipping** only sets `setup_done = true`.
  - Recording has already started with the defaults; the card doesn't block it (non-negotiable 2 isn't affected, and nothing waits on the user).
  - Closing the window without finishing shows the card again the next time the window opens.
- **Check now.** Settings → General has a row with the running version and a **Check now** button. It posts `WM_APP + 7` to the tray, which checks GitHub at once, even when the automatic check is off (it's an explicit request). The tray reports `update_check` in `status.json`: "checking", "current" or "failed"; a found update appears as the existing Install row.
- A developer build (run from `target`) may check but refuses to install over itself.

## Consequences
- Existing installs have no `setup_done` key, so their owner sees the welcome card once, with their current values, and dismisses it.
- Non-negotiable 6 holds: the window is still a normal on-demand process, opened once on first run and gone when closed.
- The window still can't check for updates while the tray isn't running; the button is disabled then and the hint says why.

## Verification (2026-11-12)
- Browser preview with the mock API (`?welcome`): the card shows over the library; turning the microphone on reveals the device picker; switching to bokmål retranslates the card and the app at once; Start clipping closes it. In Settings, Check now shows "Checking…" then "You have the latest version."
- `npm --prefix ui run check`: 0 errors; 373 strings translated in bokmål and nynorsk.
- **Not verified:** the real first-run path (tray opening the window on a PC with no settings file) and the real Check now round trip through the tray. Both need the packaged app started by the owner; an agent-launched tray is sandboxed and single-instance.
