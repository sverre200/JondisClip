# JondisClip architecture (proposed)

Status: **Proposed.** It depends on ADR-0001 to ADR-0004 being accepted.

## Processes

```
┌──────────────────────────── jondisclip.exe (always running, Rust) ────────────────────────────┐
│ Tray + hotkeys (Win32 msg loop)   Engine (capture → encode → ring)   Jobs (save/export/upload) │
│        │  RegisterHotKey / tray menu        ▲ commands / status            ▲ low-prio IO        │
│        └────────────── control channel ─────┴────────────────────────────────┘                  │
│                                   ▲                                                             │
│                     Tauri IPC (only while a window exists)                                      │
│                                   ▼                                                             │
│                WebView2 window (Library / Recording / Settings), created on demand,             │
│                destroyed on close → ~0 UI cost while gaming                                     │
└─────────────────────────────────────────────────────────────────────────────────────────────────┘
```

The tray and recorder run in **`jondisclip.exe`** (raw Win32, ADR-0006). The app window is a **separate `jondisclip-ui.exe`** (Tauri 2), started on demand and gone when closed. The two share files only: `settings.toml` and `status.json` (ADR-0008). The diagram above shows the original single-process idea.

## Engine pipeline (hot path)

```
Capture thread                GPU                         Encoder thread            Writer thread
WGC FrameArrived ──► D3D11 texture (BGRA) ──► VideoProcessor BGRA→NV12 (+scale) ──► HW encoder (AMF/NVENC/QSV
                         pooled, no CPU copy                                         via FFmpeg d3d11 hwframes) ──► packets ─► segment muxer
Audio thread: WASAPI process-loopback (game) + mic ──► resample/mix ──► AAC ──────────────────────────────────────► (same muxer)
```

- **One D3D11 device** is shared by capture, conversion and the encoder, so frames stay in VRAM from capture to bitstream.
- **Frame pacing:** the encoder runs at a fixed output rate (e.g. 60 fps) on its own clock. It duplicates the last frame when the game produces fewer frames and drops extras when it produces more. Timestamps come from QPC, and audio is the master clock for A/V sync.
- **Bounded queues** connect the stages (capacity 2–3 frames). If the encoder falls behind, we drop the frame and count it. We never block the capture callback.
- **Thread priorities:** the capture and encoder threads are above normal, but never time-critical. The writer and all jobs run at below-normal priority with background IO priority.

## Rolling buffer (ADR-0002)

- The encoder emits a **closed GOP every 1 s** (configurable). The muxer cuts a new segment file at every keyframe, 2 s per segment, in **MPEG-TS** (crash-tolerant: a partly written segment is still readable).
- Segments live in `%LOCALAPPDATA%\JondisClip\buffer\<session>\NNNNNN.ts`. An in-memory index holds `(seq, pts_start, duration, bytes)`.
- **Retention** keeps segments covering `replay_duration + 1 segment` and deletes older ones. Disk use is bounded at about `bitrate × duration` (5 min at 40 Mb/s ≈ 1.5 GB).
- **Save clip** (hotkey):
  1. Under a short lock, take the index range for the last N seconds and *pin* those files so retention skips them. This takes microseconds.
  2. Play a short confirmation (optional sound or tray flash). Recording never pauses.
  3. A background job concatenates the pinned segments with stream copy into `<SaveDir>\<Game>\<Game> 2026-11-04 21-03-11.mp4` (faststart), then unpins them.
  4. The clip boundary is accurate to ±1 GOP (1 s) without re-encoding, which is fine for "save the last 5 minutes".
- **Crash recovery:** on start, any leftover session directory is offered as "Recover last buffer" and then removed.
- **Low disk space:** if free space drops below `2 × buffer size + 2 GB`, we warn. At 1 GB free we stop buffering and show the reason in the tray. We never fill the disk.

## Capture (ADR-0003)

- **Primary: Windows Graphics Capture (WGC)**, targeting the game's monitor. On Windows 11 we turn off the capture border (`IsBorderRequired=false`) and set `MinUpdateInterval` to the target frame rate.
- **Fallback: DXGI Desktop Duplication** when WGC is unavailable or fails.
- **No game hooking or injection.** Exclusive-fullscreen games that bypass DWM are handled by monitor capture. Unsupported cases show a clear message.
- **Game detection:** we watch the foreground window (`SetWinEventHook` on `EVENT_SYSTEM_FOREGROUND`). A fullscreen or borderless window from a known game exe, or one in a user list, names the game and can *auto-start buffering* (optional, to reduce idle disk writes).
- **Display changes:** on resolution, HDR or adapter change, or device loss, the engine rebuilds the capture and encoder and starts a new segment run. Clips that span the change are saved as two files.
- **Sleep and resume:** we stop the engine on `PBT_APMSUSPEND` and restart it on resume.

## Audio

- **Game/system audio:** WASAPI loopback. When a game is detected, we use **process loopback** (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`, Windows 10 2004+). This captures only the game and its child processes, so Discord voice is excluded if the user wants that.
- **Microphone:** a separate WASAPI capture of the default input. It's **off by default**.
- **Clock (ADR-0009):** video frame `n` and audio sample `s` both count from one QPC epoch. Each capture source runs through a drift-correcting resampler (`audio_dsp::Asrc`, ≤ ±500 ppm, steered by smoothed QPC stamp error), so device clocks are followed without inserted or dropped samples. Real discontinuities get 1.3 ms fades.
- **Mixer:** encodes 1024-sample blocks 100 ms behind real time. The mic is folded to mono (one-sided input detected) and gets the user's gain (live, ramped). Mix = game + voice centred, through a transparent −1 dBFS lookahead limiter (also on the Mic track, pts-compensated). The Game track is untouched. Tracks: Mix stereo, Game stereo, Microphone **mono** 128 kb/s.
- **Muxing:** audio lags video by about 100 ms, so after a keyframe cut the old segment stays open until every audio stream passes the cut timestamp (or 500 ms passes). Only then does it enter the ring and serve saves. A save therefore takes about 0.4–0.5 s.
- **Tracks:** track 1 is the mix (the only track players and Discord use), track 2 is game audio and track 3 is the mic. Keeping them separate lets the user remove the mic later without re-recording. Each extra track costs about 160 kb/s of AAC.

## Library

- The library is the save folder itself, so files stay plain and portable. A small **SQLite** index (`rusqlite`) caches metadata: duration, game, created time, thumbnail path and upload state. It is rebuilt from disk if missing.
- Thumbnails are generated once per clip with a single keyframe decode and stored in `%LOCALAPPDATA%\JondisClip\thumbs`.
- Playback uses an HTML `<video>` element in WebView2. Hardware decoding handles H.264, and HEVC works when the OS HEVC extension is present.

## Edit and export (non-destructive)

- **Trim (keep quality):** stream copy. The start snaps to the previous keyframe (≤1 s). An optional "exact" mode re-encodes only the first GOP and copies the rest ("smart cut"), which we'd add in a later phase.
- **Share-friendly export:** a hardware H.264 + AAC re-encode at a target size (preset: Discord free 20 MB, editable). We calculate bitrate from duration and size, then scale to 720p or 1080p automatically.
- Outputs always go to new files (`… (trim).mp4`, `… (share).mp4`).

## Sharing and cloud (ADR-0004)

- **Discord:** "Copy to clipboard" puts the file on the clipboard as a file (`CF_HDROP`), so you can paste it straight into Discord. "Export for Discord" re-encodes to the target size. There are no tokens, bots or unofficial automation.
- **Google Drive v1:** a save folder inside a Google Drive for desktop folder, with zero code and zero overhead. A native upload with `drive.file` scope, OAuth PKCE loopback, resumable uploads and tokens in Credential Manager is **deferred to Phase 5** and only if it's still wanted.

## Settings

Settings are stored in `%APPDATA%\JondisClip\settings.toml`, which is versioned and validated on load. Invalid values fall back to defaults with a warning. They're grouped as Simple (duration, quality preset, hotkey, save folder, audio) and Advanced (encoder, codec, bitrate, GOP, resolution, frame rate, buffer location).
