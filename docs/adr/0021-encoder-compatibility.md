# ADR-0021: Encoder compatibility and fallbacks

Status: **Proposed** (2026-11-12). Refines ADR-0005/0007 (encoder choice) and ADR-0011/0012 (exports).

## Context
JondisClip was only ever run on the owner's AMD desktop. The first Intel PC (Arc 130V laptop, 3440×1440) couldn't record (fixed in `hwenc.rs`, commit "Fix recording on Intel graphics") and then couldn't export: "no hardware H.264 encoder could be opened". Friends will run it on hardware nobody can test in advance, so every encoder path needs a fallback that doesn't depend on the GPU vendor.

## Decision
- **Recorder: open the real encoder, not a probe.** `Recorder::start` used to pick an encoder with a 720p software-frame probe and then open it once at the real size; a probe that passed said nothing about the real resolution or GPU surfaces. It now opens each candidate for real and takes the first that works: the capture adapter's own encoder, then Media Foundation (`*_mf`, fed the same D3D11 surfaces, so still zero-copy). If none opens, the error names the adapter, the size and every encoder's reason.
- **Recorder: HEVC only when H.264 can't do the size.** Hardware H.264 stops at 4096 pixels a side and 36 864 macroblocks (`encoder::fits_h264`). For a larger output (5120×1440, 5K) with the H.264 preference, HEVC is tried after H.264. At sizes H.264 can do, a failure is reported rather than silently switching codec (ADR-0007's reason for H.264, playing everywhere, still holds).
- **Exports: NV12 where the encoder needs it.** The exporter passed the decoder's planar 4:2:0 frames straight to the encoder. AMF and NVENC accept that; QSV only takes NV12, and Intel's Media Foundation encoder refuses planar input too. Frames are now converted to NV12 (swscale, same size) for every encoder except AMF/NVENC with 8-bit 4:2:0 input, which keep the measured direct path.
- **Exports: a software last resort.** After the vendor encoders and Media Foundation's hardware encoder, exports try Media Foundation's software H.264 encoder (`h264_mf` with `hw_encoding=0`). It is part of Windows, so trim and Share to Discord work on any PC, only slower. The recorder gets no software fallback: it would break non-negotiable 1.
- **Intel surface cache.** `HwEncoder` holds its own reference to each D3D11 texture behind a QSV pool surface and refuses to track more than 32, so a cached entry can't dangle and the per-frame lookup never allocates.
- **Share-folder overlap check** resolves the existing part of each path, so a short-name alias (`C:\Users\SVERRE~1.RUL`, which `%TEMP%` uses for long user names) no longer hides an overlap.

## Consequences
- A PC whose screen is too large for H.264 records HEVC, which needs the HEVC extension to play in some apps. The tray tooltip and `status.json` show the codec in use.
- Starting the recorder can take a few hundred ms longer when the first candidate fails (up to four opens). It's off the frame loop.
- Older Intel GPUs (before the oneVPL runtime, roughly pre-11th gen) may refuse the dynamic QSV frame pool. They then fall to Media Foundation, which uses Intel's hardware encoder through Windows. A fixed QSV pool was tried and dropped: Intel's driver refuses NV12 render-target texture arrays.

## Verification (2026-11-12, Intel Arc 130V, driver 32.0.101.8826)
- `cargo run -p jondisclip-engine --example hwopen -- <args>` opens the encoder like the recorder and encodes 2 s through the real converter, 0 frames dropped in every case:
  - 3440×1440@60, 3840×2160@60, 2560×1440@120, 1920×1080@144: `h264_qsv`.
  - 5120×1440@60: `h264_qsv` and `h264_mf` refused, `hevc_qsv` opened.
  - `mf` (vendor encoder skipped): `h264_mf` at 3440×1440, `hevc_mf` at 5120×1440.
- `jondisclip-export` on a real 3440×1440 recording: exact trim (720 frames, 12.0 s) and Share to Discord (1290×540, 74 s, 4.5 MB under a 19 MB limit) both encoded with `h264_qsv`. Before the change both failed with the error above.
- `ffmpeg` on this PC: `h264_mf` with `hw_encoding=0` encodes 3440×1440 and 5120×1440 NV12.
- Unit tests: `fits_h264`/`codecs_for`; the overlap test now passes with a short-name temp path.
- **Not verified:** NVIDIA (NVENC), AMD after these changes (the AMD paths are unchanged in code), pre-oneVPL Intel, hybrid laptops where the game runs on a second GPU, and the FPS cost of the Media Foundation path (no `/perf-benchmark` run: no game on the test PC).
