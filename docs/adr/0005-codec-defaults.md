# ADR-0005: Record in HEVC, share as H.264

Status: **Superseded by [ADR-0007](0007-h264-default.md)** (2026-11-04). HEVC playback needed a paid Windows extension.

## Context
The dev GPU (RX 6800 XT, RDNA2) has AMF H.264 and HEVC but no AV1. AMD's H.264 encoder on this generation is noticeably worse than HEVC at the same bitrate, especially in high-motion, high-detail scenes. Discord and browsers don't reliably play HEVC inline.

## Decision
- **Default recording codec: HEVC** (8-bit Main) with a hardware encoder. Encoder auto-selection is unchanged (ADR-0001 and the `/media-pipeline` skill).
- **The share/Discord export is always H.264 + AAC**, the same in every quality preset.
- When HEVC isn't available on the active encoder, recording falls back to H.264 automatically. The user sees the codec that's actually in use on the Recording page.
- Recording codec is a user setting (Advanced → Codec: Auto/HEVC/H.264, AV1 where the GPU supports it).

## Consequences
- Files are about 30–40 % smaller than H.264 at equal quality, which also means a smaller buffer, fewer disk writes and faster saves.
- **Risk to verify in Phase 4:** HEVC playback in the WebView2 library depends on Windows HEVC decoding (hardware support, sometimes the "HEVC Video Extensions" package). If it's unreliable, we either decode for preview natively or switch the default to H.264 with a new ADR.
- Originals shared directly (not exported) may not play inline on Discord. The UI's share action uses the H.264 export.
