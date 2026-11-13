# ADR-0007: Record in H.264 by default (supersedes ADR-0005)

Status: **Accepted** (2026-11-04, approved by owner)

## Context
ADR-0005 made HEVC the default recording codec, noting the risk that HEVC playback on Windows depends on the "HEVC Video Extensions". That risk happened on the first real use. Opening a saved clip in the Windows player showed a Microsoft Store prompt to buy "HEVC Video Extensions" (kr 10). It would hit:
- every viewer on another PC
- WebView2, which the Phase 4 library player runs on

## Decision
- **The default recording codec is H.264** (`codec = "auto"` now means H.264). Plain Windows, browsers, Discord and phones all play it without extra codecs.
- HEVC and AV1 stay as explicit opt-ins (`codec = "hevc"` / `"av1"`). The settings file notes that they need the extension or a player like VLC.
- The share/Discord export stays H.264 (unchanged).

## Consequences
- **Bigger files.** AMD's H.264 encoder needs about 40 % more bitrate than HEVC for similar quality. The Balanced default is 0.17 bits/pixel, so 1080p60 is about 21 Mb/s and 5 minutes about 800 MB. (*Lowered by ADR-0024:* 0.136 bits/pixel, about 17 Mb/s and 635 MB.) The buffer and disk writes grow by the same factor (about 35 GB/day at 4 h/day of buffering, still small next to SSD endurance).
- **Lower quality per bit on RDNA2.** Mitigation: the "High" quality preset (×1.5). Tune in Phase 3 with real games.
- Clips already saved in HEVC stay HEVC. Open them in VLC, or convert them.
