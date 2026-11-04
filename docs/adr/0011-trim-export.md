# ADR-0011: Trimming: lossless when the cut allows it, exact by re-encoding video only, in a separate export process

Status: **Proposed** (2026-11-04). Implemented and measured: [docs/perf/2026-11-04-trim-export.md](../perf/2026-11-04-trim-export.md).

## Context
The editor has to turn a 5-minute recording into a 15–30 s highlight, quickly and without touching the original (non-negotiable 3). Stream copy is lossless and instant, but it can only start on a keyframe. Our recordings have:
- a closed GOP of exactly **1 s** (`gop_seconds = 1`, checked with ffprobe on real clips)
- **no B-frames** (`has_b_frames=0`), so packets are in display order
- a 1/90000 time base, slightly VFR (WGC delivers about 59.8 fps)
- 3 AAC tracks (Mix, Game, Mic)

## Decision
1. **The output is a new file** next to the original: `<stem> edit.mp4`, then `<stem> edit 2.mp4` and so on. It's never overwritten, it's written as `.partial` and renamed when complete, and it keeps **all audio tracks** with their titles plus faststart. The library lists edits under the same game and recording time, with an "Edit" badge.
2. **Two cut modes:**
   - **Lossless (stream copy).** Video starts at the keyframe at or before the chosen start, so up to 1 s earlier, and the UI shows exactly how much. Because there are no B-frames, the **end is exact** (the last frame shown is the one before the end handle). It's chosen automatically when the start already sits on a keyframe (within half a frame), because then it's both exact and lossless.
   - **Exact (default when the start is between keyframes).**
     - Video: decode from the preceding keyframe, drop frames before the start, re-encode `[start, end)`.
     - Encoder: the first available hardware H.264 encoder (AMF, NVENC, QSV, then Media Foundation) at **1.3× the source's average bitrate**, so generation loss stays invisible.
     - Audio is stream-copied for both modes, so audio quality never changes. Packets from the start onwards are kept, so the A/V offset is at most one AAC frame (21 ms).
   
   We don't promise frame accuracy for stream copy. Smart-cut (re-encoding only the first GOP and copying the rest) was rejected: it means splicing two encoders' SPS/PPS in one track, which is fragile across AMF, NVENC, QSV and MF.
3. **A separate process, `jondisclip-export.exe`** (crate `crates/export`, linking the engine's FFmpeg 9 LGPL build). The UI starts it:
   - one job at a time
   - at `BELOW_NORMAL_PRIORITY_CLASS`, with the decoder limited to 4 threads
   - inside a kill-on-close job object, so it can't outlive the window
   
   It reports progress as JSON lines on stdout. **Cancel** kills it and deletes the `.partial`. The recorder process is never involved, so an export can't stall the buffer (non-negotiable 2), and the UI process still doesn't link FFmpeg (ADR-0008).
4. **Keyframe times come from the MP4 sample tables** (`stss`, `stts`, `mdhd`), read in `jondisclip-core::library`. The editor can then show whether a cut is lossless without decoding anything.

## Review outcomes (perf-reviewer, 2026-11-04)
- **Priority follows attention.** While the library window is in front, the exporter runs at below-normal priority at full speed. When anything else is in front (the game), it switches itself to `PROCESS_MODE_BACKGROUND` (lowest CPU, IO and memory priority) and back. Background mode makes an exact 20 s cut about 3× slower (5.2 s → 14.6 s, measured), so it isn't always on.
- Encoding is paced to at most **4× real time**, so it never monopolises the video engine.
- **Robustness:**
  - The encoder's `EAGAIN` (input queue full, which is likely when the recorder shares the GPU) is handled by draining and resending.
  - Real decode and encode errors fail the export instead of producing a truncated file.
  - Non-increasing video timestamps are an error rather than a silent drop.
  - Copy mode refuses sources with B-frames.
- **Never overwrite:** the finished `.partial` is hard-linked to its name, which fails if the name exists. On volumes without hard links it falls back to a check plus rename.
- **Disk:** an export is refused unless the clip fits with **2 GB left free** for the recorder.
- **Process lifecycle:** no export starts without the kill-on-close job object. A finished file wins over a cancel that arrives at the same moment. Leftover `* edit*.mp4.partial` files (never the recorder's own) are removed on the next scan.
- **Editor:** you can't leave the player while it's saving (Back and previous/next are disabled until you cancel or it finishes). There's one frame-callback chain per video element.
- Checked: our recordings have no edit-list offset (video `start_time` is 0 and keyframes are at 0, 1, 2 s), so keyframe times from the sample tables match the player.
- **Still open:**
  - Exact mode passes the decoder's pixel format straight to the encoder. QSV needs NV12, so Intel PCs fall back to `h264_mf`. A 10-bit source can't be cut exactly. Our recorder only writes 8-bit yuv420p.

## Consequences
- A lossless trim of a 20 s moment takes well under a second and is bit-identical to the source. An exact trim costs one software decode plus a hardware encode of just the selection.
- The exporter uses the GPU encoder alongside the recorder's encoder. The measured effect is recorded in the stage 2 report. It runs only when the user saves.
- Discord export (stage 3) reuses this process and pipeline with a size target and Mix-only audio.
