# 2026-11-04: Share to Discord (stage 3)

RX 6800 XT with AMF. The source is a real 5-minute DayZ recording (1080p60, about 17 Mb/s average, with 26 Mb/s peaks in action). ADR-0012 has the design. The limit is decimal MB.

## Sizes and quality
| Selection | Limit | Result | Size | Attempts | Time* |
|---|---|---|---|---|---|
| 25 s (real window) | 19 MB | 720p60 H.264 Main + Mix AAC 160k (copied) | **17.90 MB** | 1 | 7.0 s |
| 20 s | 19.92 MB (old margin) | 720p60 | 19.00 MB | 1 | 20 s |
| 60 s | 19.92 MB (old margin) | 720p30, AAC 128k | 19.69 MB | 1 | 33 s |
| 300 s (whole clip) | 19.92 MB (old margin) | 360p30, AAC 64k | 19.62 MB | 1 | 132 s |
| 3 s (before the source cap) | 19.92 MB | 1080p60 at 48 Mb/s | 18.4 MB | **2** | 9 s |
| 3 s (with the 1.3× source cap) | 19 MB | 1080p60 at ≤ 22 Mb/s | 10.2 MB | 1 | 4.6 s |
| 4 s calm, starting on a keyframe | 19 MB | **stream copy** (lossless) | 9.3 MB | 0 | < 1 s |
| 6 s high-action, starting on a keyframe | 19 MB | copy came out at 19.6 MB, over the limit → re-encoded 1080p60 | 18.1 MB | 2 | 13 s |

\*Times were measured with the window in the background, so in background mode and paced to 4× real time.

## What we learned
- AMF CBR overshoots its target by up to about 5 %: 19.69 MB against a budget of 18.7 MB. That's why the budget is now **92 %** of the limit. The size is measured after every encode anyway.
- **Average bitrate isn't enough to decide on a copy.** High-action 6 s stretches run at about 26 Mb/s, 50 % over the clip's average. A copy is always measured before we trust it.
- For short clips we cap the bitrate at 1.3× the source. More can't improve a recording that's already compressed, and without the cap the first CBR attempt overshot.

## Clipboard
- The real-clipboard test (`cargo test -p jondisclip-ui -- --ignored clipboard`) reads the file back with `DragQueryFileW` and gets exactly one path.
- In the real window, after a share, `System.Windows.Forms.Clipboard.GetFileDropList()` returned the share file. That's the same thing Discord reads on Ctrl+V.
- **Not verified:** actually pasting into the Discord app. That needs the owner.

## Output format (ffprobe)
- H.264 Main, yuv420p, 1280×720, 60/1, 1500 frames
- AAC-LC with `handler_name=Mix`
- faststart
- decodes with no errors
