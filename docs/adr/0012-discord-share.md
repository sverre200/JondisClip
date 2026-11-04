# ADR-0012: Share to Discord: size-targeted H.264 file on the clipboard

Status: **Proposed** (2026-11-04). This refines ADR-0004 (clipboard, no bots or tokens). Implemented and measured: [docs/perf/2026-11-04-discord-share.md](../perf/2026-11-04-discord-share.md). The budget is 92 % (AMF CBR overshoots by up to about 5 %), and short clips are capped at 1.3× the source bitrate.

## Context
The main workflow is: trim a moment, then post it in a Discord chat. Discord's upload limit depends on the account (free accounts are around 10–25 MB, Nitro is more, and it changes over time). There's no supported way to upload as the user, and we won't use bots, tokens, webhooks or UI automation. The user stays in charge of where the clip goes.

## Decision
1. **"Share to Discord"** makes one file that fits the configured limit and puts **the file itself** on the Windows clipboard, as `CF_HDROP` plus `Preferred DropEffect = copy`, exactly like Explorer's Copy. The user presses Ctrl+V in any Discord chat. Nothing is sent anywhere by JondisClip.
2. **The limit is a setting**, `discord_limit_mb`, defaulting to **19 MB**, a conservative margin under the common free limit. Presets are 10 / 19 / 50 / 500 MB, and the user can type any value from 5 to 1000.
3. **The output:**
   - H.264 (plays in Discord's inline player on every platform), yuv420p, faststart.
   - **The Mix audio track only**: the one track Discord plays.
   - The file is named after the clip and written to `%LOCALAPPDATA%\JondisClip\share\`. That keeps it out of the library, and it's pruned after 14 days, since the clipboard only needs it until Discord has uploaded it. The original and the trim are never touched.
4. **No re-encode if it already fits.** If the selection, copied losslessly (start on a keyframe) with the Mix track only, fits the limit, it's stream-copied. Otherwise it's re-encoded.
5. **Quality ladder** (`jondisclip-core::discord::plan`, a pure function shared by the UI and the exporter):
   - Budget: 92 % of the limit (container overhead, plus AMF CBR overshoots by up to about 5 %).
   - Audio comes off the top. The Mix is copied at its 160 kb/s when the budget is generous. Otherwise it's AAC at 128, 96 or 64 kb/s.
   - The rest goes to video. Pick the **highest rung** of 1080p60 → 720p60 → 720p30 → 540p30 → 480p30 → 360p30 (60 fps kept as long as possible, since motion matters more than detail in gameplay; a 30 fps source caps every rung at 30) that gets at least **0.06 bits per pixel per frame**. That's an empirical floor for game motion on hardware H.264. Never scale up past the source.
   - Below about 150 kb/s of video there is no watchable result. The export is refused, and the UI suggests a maximum length.
6. **Hit the size and check it:**
   - Encode with hardware H.264 in **CBR** mode with a 1-second buffer. Constant bitrate respects a size far better than VBR on AMF, NVENC, QSV and MF.
   - Then **measure the file**. If it's over the limit, re-encode with the bitrate scaled by `limit / actual × 0.95`, up to 3 encodes in total.
   - If it's still over, fail with an explicit message. We never hand over a file that's over the limit.
7. **Same process and rules as ADR-0011:** `jondisclip-export.exe discord …`, one job at a time, attention-based priority, paced to 4× real time, cancel by kill. Scaling uses FFmpeg's swscale (LGPL, already shipped as a DLL).

## Review outcomes (perf-reviewer, 2026-11-04)
- **The default limit was checked.** Discord's support FAQ gives 20 MB for free accounts (raised from 10 MB on 14 Aug 2026), so 19 decimal MB stays the default.
- **Bitrate is capped:** at most 1.3× the source's bitrate per output pixel, and never more than 50 Mb/s. Huge limits on tiny selections stay within the encoders' levels.
- **Lossless copy:** it's skipped for sources with B-frames, and any copy failure other than IO falls back to re-encoding.
- **Pacing follows attention:** 4× real time with the window in front, **1×** while the game is in front, applied frame by frame.
- **Scaling** goes to NV12, which every hardware encoder takes natively (QSV only takes NV12). It uses AREA for downscaling, and the scaled frame is made writable before reuse.
- **Audio transcode:** a native channel layout for the channel count, and real decode errors are surfaced.
- **Clipboard:** the window owns the clipboard (not NULL), and no global memory leaks on failure paths. Blocking commands run off the UI thread.
- **Share folder:** pruning is refused when the save folder overlaps the share folder. Besides the 14-day age limit, the folder is capped at 2 GB (oldest first).

## Consequences
- A 20–30 s highlight comes out around 720p60 at 19 MB. A 5-minute clip doesn't fit, and the UI says "trim it first".
- The clipboard holds a path into our share folder. Pruning after 14 days means a very old clipboard entry may point at a deleted file, which is acceptable.
- A Discord limit lower than the account's real one only costs quality. A higher one makes Discord reject the upload, so the default stays conservative.
