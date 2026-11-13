# ADR-0023: Settings pages, live update feedback, and Windows toasts

Status: **Proposed** (2026-11-12).

## Context
The owner asked for three things:
- Settings is sectioned, but everything is on one long page.
- Checking for updates gives too little visual feedback.
- The popups outside the app (notifications and the tray menu) should look better.

## Decision

### Settings: one page per group
- Settings has a side menu with six pages: **Recording** (instant replay, video including the codec), **Audio**, **Sharing** (Discord, Google Drive), **Appearance** (theme), **General** (startup, language and dates) and **Updates**.
- Only the chosen page is rendered. The last page is remembered (`localStorage`, `jc.settings.tab`).
- The menu looks like the library's folder menu. Below 760 px it becomes a strip above the page.
- The microphone level meter only captures while the Audio page is showing. Hints that change while you watch (the level advice) keep room for two lines, so the page does not jump.
- No setting was added, removed or renamed; `settings.toml` is unchanged.

### Update feedback
- The Updates page has one status card. It shows, in turn: the version and **Check now**; a spinner while checking; "You have the latest version" with the time; the offer with **Install update**; a progress bar while downloading; "Installing…"; or a warning with **Try again**.
- The card reacts on the click, before the tray answers. A check is shown for at least 0.7 s, and an answer left over from an earlier check isn't taken as the new one.
- While a check or download runs, the window reads `status.json` four times a second (twice a second for the top-bar button) instead of every 1.5 s.
- **Download progress.** `Release` keeps the zip's `size` from GitHub's answer. While downloading, the tray publishes `update_progress` (0 to 99) in `status.json`: the size of `update.zip` so far against that size, read once a second on the supervisor tick. Without a size the bar is indeterminate.
- A dot marks the Updates page in the menu while a newer version is on offer. The top-bar button shows a spinner and the percentage.

### Notifications: Windows toasts
- The tray's notifications are now toasts made with `Windows.UI.Notifications`, shown under the name **JondisClip** with its icon, instead of `Shell_NotifyIcon` balloons under the exe's name.
- **Identity.** An unpackaged app needs an app id. The tray writes `HKCU\Software\Classes\AppUserModelId\SverreJ.JondisClip` (`DisplayName`, `IconUri`) at start, and the icon to `%LOCALAPPDATA%\JondisClip\notification-icon.png`. Per user, no admin rights.
- **Buttons.**
  - Clip saved: **Open library** and **Show in folder** (Explorer with the clip selected).
  - Update available: **Install update**. Clicking the toast itself opens Settings.
  - Clicking any other toast opens the library.
- Clicks arrive on a Windows thread-pool thread. They're queued and the tray window is woken with `WM_APP + 8`, so all work stays on the main thread.
- Toasts are silent (the save sound is the app's own) and Windows holds them back during Do Not Disturb and fullscreen games, as before.
- **Fallback.** If the identity can't be registered, or Windows reports notifications as off for the app, the old balloon is used. The reason is logged once.
- No clip thumbnail in the toast: making one in the tray would mean decoding video in the always-on process.

### Tray menu: follows dark mode
- The tray menu is still a Windows menu, but it now follows Windows' dark mode.
- This uses two unnamed `uxtheme.dll` exports (ordinals 135 `SetPreferredAppMode` and 136 `FlushMenuThemes`), the ones File Explorer uses. They're undocumented but stable since Windows 10 1809. They're only called on build 17763 or newer; otherwise the menu stays light.
- A fully custom-drawn menu in the app's theme was not done: it's a much larger change for a menu that's open a second at a time.

## Consequences
- Non-negotiable 1 holds: nothing here touches capture or encode. The progress read is one `metadata` call a second, only during a download.
- Non-negotiable 5 holds: toasts are local; no network call was added.
- The registry value and icon file are new traces on the PC. They're harmless if the app is deleted, but they aren't removed.
- If Microsoft changes the `uxtheme` ordinals, the menu could go back to light. The build gate keeps unknown old builds safe; a future build can't be ruled out, so this is logged at start when it isn't applied.
- New Windows crate features in the tray: `Foundation`, `Data_Xml_Dom`, `UI_Notifications`. No new crates.

## Verification (2026-11-12)
- `cargo test --workspace`, `cargo clippy --workspace -- -D warnings` and `npm --prefix ui run check` pass (388 window strings, complete in both languages).
- Unit tests: download progress follows the file size and is absent without a size; toast XML escapes text and lists buttons; the Windows build is read.
- Browser preview (mock tray): the six pages, the narrow layout, the check sequence (idle → checking → latest version with the time) and the download sequence (available → downloading with a moving percentage → installing), in the card and the top bar.
- **Not verified:** a real toast and the dark tray menu on screen. Programs started from the agent's shell have their registry writes redirected, so Windows didn't see the app id there (`Element not found`, which is the fallback path). They need a run from the owner's own shell.
- **Not verified:** real download progress against GitHub (needs a newer release than the installed one).
