# ADR-0020: Updates from GitHub Releases

Status: **Proposed** (2026-11-12).

## Context
The owner shares JondisClip with friends as a portable zip. Their PCs have hardware the owner can't test (the first Intel PC failed to record until `hwenc.rs` was fixed), so fixes need to reach them without sending zips around by hand. The repository became public on 2026-11-12 for this.

## Decision
- **Distribution.** Each version is a GitHub release `v<version>` on `sverre200/JondisClip` with one asset, `JondisClip-<version>-win64.zip` (the output of `tools\package.ps1`). Friends download from `releases/latest`.
- **Publishing.** `tools\release.ps1 <version>` sets the version (`Cargo.toml`, `tauri.conf.json`, `package.json`), builds the zip, commits, tags, pushes and uploads. One command per release.
- **Check.** The tray asks `api.github.com/repos/sverre200/JondisClip/releases/latest` 20 s after start and every 6 hours, and compares `tag_name` with its own version.
  - It's one anonymous GET. Nothing about the PC, the user or the clips is sent. GitHub sees the IP address, as with any download.
  - The **Check for updates** setting (`check_for_updates`, on by default) turns it off.
  - Developer builds (run from Cargo's `target` folder) never check.
  - A failed check (offline) is logged and stays silent.
- **Offer.** A newer version shows a quiet notification, an **Install update <version>** item in the tray menu, and an **Install update** button in Settings → General. The tray publishes `version`, `update_version` and `update_state` in `status.json`; the window's button posts `WM_APP + 6` to the tray, which does all the work.
- **Install, only on a click.**
  1. The zip downloads to `%LOCALAPPDATA%\JondisClip\update` and is unpacked there. Recording continues.
  2. The tray waits until no clip is being written, stops the recorder, and swaps the files: each file in the install folder is renamed to `<name>.old` and the new one is copied in. Windows allows renaming files that are in use, so this works while the exes run.
  3. It starts the new `jondisclip.exe --updated` and quits. The new one waits for the single-instance mutex, then deletes the `.old` files and the download folder.
  - If any step fails, every file is put back, recording restarts and a notification says why. The offer stays.
- **No new dependencies.** Windows' own `curl.exe` and `tar.exe` (System32, Windows 10 1803+) do HTTPS and unzip, called by full path with no window. The tray gains no HTTP or zip code.
- **Trust.** Only an asset whose URL starts with `https://github.com/sverre200/JondisClip/releases/download/` and ends with `-win64.zip` is accepted, over HTTPS only. The zip must contain all three exes before anything is replaced.

## Consequences
- Non-negotiable 5 (no telemetry, no automatic uploads) holds: the check sends no data and can be switched off. It is a new network call, which is why this ADR exists.
- Non-negotiable 2 holds: installing waits for clips being saved, and the buffer only stops for the restart the user asked for. The replay buffer is lost at that restart, as with any quit.
- The releases aren't code-signed. Whoever controls the GitHub account can ship code to everyone who clicks Install. Friends still see the SmartScreen warning on the first download; the in-app update doesn't trigger it.
- The settings and clips aren't touched by an update (they live in `%APPDATA%`, `%LOCALAPPDATA%` and the save folder).
- An open library window keeps running the old version until it's closed.
- Windows older than 10 1803 has no `curl.exe`; the check fails quietly there and updates are manual.

## Verification
- Unit tests (`crates/core/src/update.rs`): version parsing and comparison, refusing assets from other hosts or with odd names, the rename-and-copy swap, and the `.old` cleanup.
- End to end: see the note added below when the first update between two published versions has been installed from the tray.
