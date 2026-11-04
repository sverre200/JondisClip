---
name: media-pipeline
description: Use when writing or changing JondisClip capture, GPU conversion, encoder, audio, segment muxer or ring-buffer code (anything on the recording hot path). Enforces zero-copy, no per-frame allocation, bounded queues and graceful device-loss handling.
---

# Media pipeline rules

Read `docs/ARCHITECTURE.md` and ADR-0002/0003 before you change anything.

## Hot-path checklist (each frame and each audio packet)
- [ ] No heap allocation. Use pooled textures and packets, and a preallocated ring of NV12 surfaces.
- [ ] No GPU→CPU copy (`Map`, `CopyResource` to a staging texture) except in a debug-only path.
- [ ] No blocking IO, logging at info level or above, or `Mutex` held across an FFI call. Pass work through bounded `crossbeam` channels, and use `try_send` with a dropped-frame counter on a full queue.
- [ ] Timestamps come from QPC (`QueryPerformanceCounter`), converted once to the encoder time base. Never use wall-clock time.
- [ ] Every FFmpeg and COM call returns a typed error. No `unwrap()`.

## Device and encoder lifecycle
- One `ID3D11Device` (with video support and multithread protection on) is shared by WGC, the VideoProcessor and FFmpeg's `AVD3D11VADeviceContext`.
- Encoder selection order: the user's choice, then the vendor of the captured adapter, then `*_mf` (Media Foundation), then a clear error. Probe by opening the encoder. Don't guess from the GPU name.
- On `DXGI_ERROR_DEVICE_REMOVED`/`RESET`, a resolution or HDR change, or resume from sleep: close the current segment, tear down everything in reverse order, rebuild, and start a new segment run. Log one line with the reason.
- Keyframes: `g = fps * gop_seconds`, closed GOP, and no B-frames that cross a segment boundary. Check this per codec and encoder.

## Testing
- Unit tests: ring retention, pin/unpin and range selection, which are pure logic with no Windows dependency.
- Integration tests (`#[ignore]` by default, run with `cargo test -- --ignored` on a GPU machine): encode synthetic frames for 10 s, then check the segments with ffprobe (see `/clip-ops`).
- After any hot-path change, run `/perf-benchmark` (quick mode at least) and ask the `perf-reviewer` agent to review the diff.
