# ADR-0009: Audio clock correction by resampling; minimal, transparent mic processing

Status: **Accepted** (2026-11-04). Evidence: [docs/perf/2026-11-04-mic-audio.md](../perf/2026-11-04-mic-audio.md).

## Context
Each audio device runs on its own clock, and JondisClip's timeline runs on QPC. The original placement corrected the difference by nudging ±1 sample per packet and filled forward nudges with a zero sample. Measured on real timing profiles, that produced 2.4–2.9 clicks or slips per second. Separately: the voice was about 7 dB under the game with no gain control, the Mix had no headroom (game peaks at −0.2 dBFS) and was hard-clamped, and the mic was stored as duplicated stereo.

## Decision
1. **Asynchronous resampling instead of sample nudging.**
   - Each capture source runs through `audio_dsp::Asrc`, a 16-tap Blackman-Harris windowed-sinc with a 1024-phase table, interpolated between phases. **This includes game audio:** it's transparent (exact at unity rate), with a response of −0.55 dB at 18 kHz in the worst fractional phase.
   - The rate is steered (≤ ±1000 ppm, 2 s time constant) by a smoothed (EMA) error between QPC stamps and the produced timeline, so jitter is ignored and real drift is followed with no discontinuity.
   - At unity rate it's an exact passthrough.
   - A hard resync happens on `DATA_DISCONTINUITY`, at once on a jump over 100 ms (for example, loopback resuming after silence), or after 3 consecutive packets more than 20 ms off. Real discontinuities are joined with 64-frame (1.3 ms) fades in `SourceBuf`, never hard cuts or zero samples.
2. **Mic processing kept minimal and transparent.** There's deliberately no noise suppression, compressor, EQ, AGC or ducking.
   - **Mono fold:** (L+R)/2 for the usual duplicated mono, at exactly the same level. If one side is silent (>30 dB lower for about 1 s), the other side is used alone, crossfaded. Hysteresis: it switches back below 20 dB, and the detector resets on non-finite samples.
   - **User gain:** −12 to +24 dB, default 0, **applied live** with a per-block ramp and no recorder restart.
   - **Lookahead peak limiter** at −1 dBFS (64-frame lookahead, 150 ms release), only on the Mix and Mic tracks. It's bit-transparent below the ceiling, so it only acts when a peak would otherwise clip. Its 63-frame delay is compensated in those tracks' pts, and the mixer starts 63 frames later so no pts is negative. The Game track has no limiter or gain.
3. **Track format:** Mix stereo 160 kb/s (default track), Game stereo 160 kb/s, **Microphone mono 128 kb/s** (more bits per channel than the previous 80 kb/s per channel).
4. **Device visibility:**
   - The settings window flags virtual devices (bus ROOT/SWD), shows a live level meter, and shows a banner when the recorder reports the mic as not connected (`status.json` → `mic_connected`).
   - The meter is a separate capture in the UI process that runs only while the window polls, so the recorder is never touched.

## Consequences
- **No clicks or slips from clock handling:** 0 in the 60 s simulation for all measured profiles, down from up to 2.9 per second.
- About 3 M multiply-adds per second for two sources. No measurable CPU or RAM change (0.48 % vs 0.66 % before, within noise).
- The Mix and Mic tracks have 1.3 ms of internal latency, compensated in pts.
- **Windows mic enhancements (APOs, AGC) still apply** in shared mode. Raw mode (`AUDCLNT_STREAMOPTIONS_RAW`) was not adopted, because it would also remove processing some headsets rely on. Revisit it only if listening tests point to driver processing.
