# ADR-0024: Smaller files: a lower Balanced bitrate and a higher quality floor

Status: **Proposed** (2026-11-12). Changes the numbers in ADR-0007's consequences; H.264 stays the default.

## Context
The owner finds the clips big and asked for smaller files at the same quality. A 5-minute 1080p60 WoW clip is 430–900 MB.

Measured on two cuts from the owner's real WoW clips (`docs/perf/2026-11-12-file-size.md`):
- The H.264 encoder on the RX 6800 XT has no unused efficiency. High profile changes nothing, B-frames make it worse, and the slower presets are within 0.3 VMAF.
- In busy scenes size and quality trade directly: 20 % fewer bits cost about 2 VMAF (about 6 is noticeable).
- Calm scenes have slack: a higher quality floor saves 18 % for under 1 VMAF.
- HEVC is only 10–20 % smaller here, which doesn't pay for its playback problems (ADR-0007).

## Decision
The owner chose both encoder options.
- **Balanced H.264 target: 0.17 → 0.136 bits/pixel** (`EncodeSettings::default_bitrate`). 1080p60 goes from 21.2 to 16.9 Mb/s, and the peak cap (1.5×) from 31.7 to 25.4 Mb/s.
  - Performance (×0.7) and High (×1.5) scale from it: 11.8 and 25.4 Mb/s at 1080p60.
  - HEVC and AV1 stay at 0.12: HEVC at that target scored the same as H.264 at the new one.
- **Quality floor (`qmin`) up by 2:** Performance 22, Balanced 20, High 18 (`Quality::qp_floor`). Still AMD only, where it was measured.
- The window's size estimate (`ui/src/lib/format.ts`) uses the same numbers.
- Nothing else changes: `vbr_peak`, 1 s GOP, no B-frames, Main profile.

## Consequences
- **About 20 % smaller in action, and about 20–25 % smaller in calm scenes**, on AMD. In action the saving comes from the target. In calm scenes it comes from the floor (18 %); the lower target adds only about 6 % there, and the two together weren't measured. A 5-minute busy 1080p60 clip is about 635 MB instead of 800 MB.
- **Quality:** about 2 VMAF lower in busy scenes (4 in the worst 5 % of frames), under 1 in calm ones. Most viewers won't see it. Someone who does can choose High, which is now about what Balanced was (25.4 against 21.2 Mb/s).
- **Performance is now lean:** at 11.8 Mb/s the worst frames in busy scenes scored about 10 VMAF under the old Balanced. It's the "smallest files" choice and says so.
- The replay buffer's disk writes drop by the same share.
- Applies to new recordings only. Existing clips are untouched (non-negotiable 3).
- **Unmeasured:** Intel (QSV) and NVIDIA get the lower target without a measurement of their own, and no quality floor. Their H.264 encoders are generally more efficient than RDNA2's, so the risk is on the safe side, but it isn't verified.
- This changes encoder parameters only, not the per-frame code, so no FPS benchmark was run. A lower bitrate is, if anything, less work.

## Verification (2026-11-12)
- `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `npm --prefix ui run check` pass.
- `--example record`, 20 s on the test profile: `h264_amf` 1920×1080@60 at 16.9 Mb/s, 1201 frames, 0 late, 0 dropped.
- **Not verified:** a real game session at the new settings, and how it looks to the owner.
