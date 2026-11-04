# 2026-11-04: Trim export (editor stage 2)

RX 6800 XT, Ryzen 7 5800X. The source is a real 5-minute DayZ recording (1080p60 H.264, about 17 Mb/s, 3 AAC tracks). The cut is 20 s from 2:00.4. ADR-0011 has the design.

## Speed
| Mode | Time for a 20 s cut | Output |
|---|---|---|
| Lossless (stream copy) | **0.12 s** | Starts at the keyframe at 2:00.22 (0.2 s earlier). Bit-identical frames. |
| Exact, AMF `quality` preset | 10.5 s | 1200 frames, starts exactly at 2:00.40 |
| Exact, AMF `balanced` preset (chosen) | **5.2–5.5 s** | 1200 frames, 58 MB (estimate shown before saving: 56 MB) |

Decode alone (4 threads) takes 2.6 s. `quality` limits AMF to about 115 fps. At 1.3× the source bitrate, `balanced` shows no visible difference, so that's what we use.

## Priority (after review)
Exact 20 s cut, run from the CLI with the window's PID set:

| Window in front | Window not in front (background mode) |
|---|---|
| 5.2 s | 14.6 s |

The real window, saved with the window not in front (driven over DevTools): 15 s exact in 14.7 s (900/900 frames), and 10 s lossless in 0.6 s (600/600 frames). The original's hash was unchanged.

## Effect on the recorder
The tray was recording 1080p60 AMF the whole time. Counters are from the `recorder stopped` log line:

| 60 s window | Frames encoded | Late | Dropped | Audio dropped |
|---|---|---|---|---|
| Idle | 4218 | 0 | 0 | 0 |
| 12 back-to-back exact exports | 4489 | 0 | 0 | 0 |

**Not measured:** the same run with a game rendering at full load. The VCN encode engine and CPU are then shared three ways. An export only runs when the user saves, and the player pauses when the window loses focus. A `/perf-benchmark` run with a game is still due.

## Correctness
These checks are automated in `crates/export/tests/trim.rs`, using a synthetic source with a flash and a beep both at exactly 5.000 s:
- **Lossless on a keyframe:** exact start and end (180/180 frames). The flash lands within half a frame, and the beep within 25 ms of it.
- **Lossless between keyframes:** the start snaps to the previous keyframe, and the end stays frame-exact.
- **Exact:** 150/150 frames. Video starts at 0, audio within 25 ms, and the flash/beep offset is under 25 ms.
- **Exact to the end of the clip:** the decoder and encoder are fully flushed.
- **Errors:** an empty or reversed range, a start past the end, a missing or broken input, or an existing output are all refused. Nothing is left behind, and no file is overwritten.
- **Cancel:** killing the process never produces a finished file.
- **The original stays byte-identical** (checked in the tests and on the real clip).

A bug was found and fixed along the way: encoder packets had no duration, so the MP4's last sample got duration 0 and decoders dropped that frame (149/150).

## In the real window
- Opened a clip, used I/O to select 20 s, then Ctrl+S. Progress ran from 2% to 92% and finished with "Saved 0:20.0 · 58 MB".
- "Play it" opens the edit. The library lists it under DayZ 15:27 with an "Edit" badge.
- Cancelling at 9% left no file.
- Closing the window mid-export: the job object killed the exporter at once, and the leftover `.partial` was removed when the window reopened.
