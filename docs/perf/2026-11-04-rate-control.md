# Rate control: file size vs quality (2026-11-04)

**Question:** are clips bigger than they need to be, and can they shrink without visible quality loss?

## Setup
- **Encoder:** `h264_amf` from the bundled FFmpeg 9 (same encoder and options as the app). RX 6800 XT, driver 32.0.21045.16010. 1080p60, GOP 60, no B-frames.
- **Footage:** three lossless 10 s synthetic scenes (FFV1), built from a 4K fractal render with fine detail and smooth gradients:
  - **A, pan:** slow camera pan with temporal grain. A foliage-like worst case.
  - **B, fast:** fast camera swings plus grain. A whip-turn worst case.
  - **C, calm:** static scene with a small animated HUD element. Menus, looting, standing still.
- **Metric:** VMAF v0.6.1 (libvmaf in the bundled FFmpeg), frame-index aligned. "p5" is the score exceeded by 95 % of frames.
- **Caveat:** A and B contain per-pixel temporal grain, which is harsher than real games, so every capped mode sits at its cap there. **Real-game confirmation is still to do** (a DayZ sample).

## Results (video bitrate · VMAF mean / p5)

| Config | A pan | B fast | C calm |
|---|---|---|---|
| Current Balanced (vbr_peak 21.2 / max 31.7 Mb/s) | 23.3 · 91.2/90.2 | 23.5 · 97.8/93.0 | 12.9 · 96.7/95.8 |
| + `quality=quality` | same | same | same (slower) |
| + VBAQ | 22.9 · **87.8**/85.9 | 23.5 · **93.8**/87.3 | 13.8 · 96.6 |
| + pre-analysis | same as quality | same | same (half the encode speed) |
| QVBR 20 / 23 / 26 / 29 (cap 31.7) | 23 / **40 / 71 / 110** · 73–85 | 27–106 · 84–95 | 1.4–2.0 · **59–71** |
| CQP 20 / 24 / 28 (uncapped) | 328 / 188 / 71 · 99/98/94 | 335 / 195 / 79 | 4.5 / 3.6 / 2.8 · 95/93/89 |
| CQP 22–26 + `max_au_size` or HRD cap | caps **ignored** (126–262 Mb/s) | ignored | 3.2–4.3 |
| **Current + qmin 18** | 23.3 · 91.2/90.2 (identical) | 23.5 · 97.8/93.0 (identical) | **5.2 · 95.4/95.3** |
| Current + qmin 20 / 22 / 24 | identical | identical | 4.6 / 4.3 / 3.7 · 94.6 / 93.7 / 92.5 |
| Performance (×0.7) / + qmin 21 | 16.8 · 90.2 / identical | 16.9 · 96.5 / identical | 11.1 · 96.4 → **4.4 · 94.2** |
| High (×1.5) / + qmin 16 | 33.9 · 92.2 / identical | 35.0 · 98.3 / identical | 14.3 · 96.9 → **6.4 · 96.0** |

## Findings
1. **QVBR is unusable on AMF H.264 (RDNA2).** It exceeds the bitrate cap 3.5× and scores worse than capped VBR at the same or higher bitrate.
2. **AMF ignores bitrate caps in CQP mode** (`max_au_size`, `enforce_hrd` + maxrate). Constant quality would make disk use unbounded in busy scenes, which isn't acceptable for an always-on buffer.
3. The `quality` preset and pre-analysis give no VMAF gain here (pre-analysis halves encode speed). VBAQ *lowers* VMAF.
4. **The waste is calm content.** Capped VBR spends 11–14 Mb/s on near-static frames that look the same at 4–6 Mb/s.
5. **A minimum QP (generic `qmin`, which AMF honours) fixes exactly that.** It cuts 55–60 % on calm content for −0.9 to −2.2 VMAF (well under the ~6-point "just noticeable" difference), and busy scenes are bit-identical.

## Decision
Keep capped VBR and add a quality floor per preset: **Performance 21, Balanced 18, High 16** (`Quality::qp_floor`). It's applied to AMD encoders only, which is what was measured. NVENC and QSV stay unchanged until tested on that hardware.

Expected effect in real use: no change during heavy action, and noticeably smaller files and buffer writes for everything calmer. How much smaller depends on the game and needs the DayZ sample to quantify.
