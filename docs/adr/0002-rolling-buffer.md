# ADR-0002: Disk-backed segmented replay buffer

Status: **Proposed** (2026-11-04)

## Context
The buffer has to hold up to 15 minutes, saving must never interrupt it, RAM use must stay small, and saving should avoid re-encoding.

## Options
- **Raw frames in RAM:** 1440p60 BGRA is about 900 MB/s. Not possible.
- **Encoded packets in a RAM ring** (OBS-style): 5 min at 40 Mb/s ≈ 1.5 GB RAM, and 15 min ≈ 4.5 GB. Saving is instant, but RAM cost rises with duration, and a crash loses the buffer.
- **Encoded segments on disk (chosen):** RAM stays roughly constant (a few MB), saving is a concat with stream copy, and the files survive a crash. The cost is continuous disk writes at the recording bitrate.

## Decision
- Closed 1 s GOPs and 2 s MPEG-TS segments in `%LOCALAPPDATA%\JondisClip\buffer`. Retention keeps `duration + 1 segment`.
- To save, we pin the segment range, then a background job does the concat and remux to MP4 (faststart). We unpin when it finishes.
- Clip boundaries are accurate to ±1 s. That trade-off is accepted for instant replay.

## Consequences and mitigations
- **SSD wear:** 40 Mb/s for 4 h/day ≈ 72 GB/day ≈ 26 TB/year, small next to a typical 600 TBW drive, but users should know. Mitigations:
  - Buffer only while a game is detected (optional auto-start).
  - Let the user choose the buffer location.
  - A possible "RAM buffer" for durations up to 2 min (later, only if measured to matter).
- A 1 s GOP costs a little compression efficiency compared to longer GOPs. That's acceptable at gameplay bitrates.
