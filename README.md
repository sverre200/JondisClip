# JondisClip

Always-on game clipping for Windows. It quietly keeps the last few minutes of your screen and sound, and saves them when you press a hotkey (Alt+F10). It's built to stay out of your game's way.

## Download

**[Download the latest version](https://github.com/sverre200/JondisClip/releases/latest)** (the `JondisClip-…-win64.zip` file).

1. Unzip the folder somewhere it can stay, for example `C:\Users\<you>\JondisClip`. Keep all the files together.
2. Double-click `jondisclip.exe`. Windows may say "Windows protected your PC", because the app isn't signed. Click **More info**, then **Run anyway**. You only need to do this once.
3. JondisClip is now in the system tray, by the clock. Press **Alt+F10** to save the last 5 minutes.

Requires Windows 10 or 11, 64-bit. The full guide is in `README.txt` inside the zip.

## Updates

JondisClip tells you when a newer version exists. Right-click the tray icon and choose **Install update**. It downloads, installs and restarts by itself, and keeps your clips and settings.

## Something not working?

Recording uses your graphics card's video encoder. It's tested on AMD Radeon and Intel Arc; NVIDIA should work but hasn't been tested. If the tray icon shows a warning or recording doesn't start, [open an issue](https://github.com/sverre200/JondisClip/issues/new) or message whoever gave you JondisClip, and attach the log file:

```
%LOCALAPPDATA%\JondisClip\jondisclip.log
```

Paste that path into File Explorer's address bar to find it.

## Privacy

No account, no telemetry, nothing is uploaded. The update check only asks GitHub for the latest version number, and can be turned off in Settings.

## For developers

Rust (tray, recorder, exporter) plus a Tauri 2 + Svelte 5 window. See `CLAUDE.md` for the build setup, `docs/ARCHITECTURE.md` for the design and `docs/adr/` for the decisions.
