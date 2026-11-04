# 2026-11-04: Samson G-Track Pro tuning

Device-specific review of the mic path for the owner's Samson G-Track Pro (USB). Builds on [2026-11-04-mic-audio.md](2026-11-04-mic-audio.md). Measured with `cargo run --release -p jondisclip-engine --example audio_probe -- device|miccheck Samson <s>` (statistics only, no audio stored).

## Measured
| Item | Result | Consequence |
|---|---|---|
| Device format | 48 kHz, 2 ch, 24-bit (valid 24), mask 0x3 | Same rate as the pipeline: no Windows resampling. Nothing to change. |
| Windows input level | 100 % = 0 dB, range 0..0 dB, in hardware | Windows can't attenuate it. Level is set only by the MIC GAIN knob, plus JondisClip's digital gain. |
| Mode | L/R correlation +1.000 (Mono mode) | Mono fold is (L+R)/2 at the same level. No one-sided routing or downmix loss. The mic check warns if 2-Track is ever detected (one side > 20 dB lower). |
| DC offset | +0.000001 | Negligible. |
| Quiet-room floor | about −122 dBFS raw (10th percentile of 50 ms windows) | The knob is low. The owner runs +18 dB digital gain to compensate. |
| Clock drift | +28 ppm vs QPC | Handled by the ASRC (ADR-0009). |
| Settings file | was capturing the virtual cable, not the Samson | `microphone_device` set to the Samson's endpoint ID. **This was the main real problem.** |

## Changes
- **Mic check** (Settings → Audio): records 8 s in the UI process, runs it through the recorder's exact mic chain (fold, optional low-cut, gain, limiter) and reports:
  - voice level, peaks, noise floor and mode
  - ADC clipping, which happens before any digital gain
  - knob-vs-digital advice, plus playback of the processed audio
  
  Statistics (no audio) are written to `%LOCALAPPDATA%\JondisClip\mic-check.json`. Start-up samples (0.4 s) and digitally silent windows are excluded from the noise estimate. A capture with under 1 s of audio is an error, not a result.
- **Optional 80 Hz low-cut** (2nd-order Butterworth, off by default, live): about 5 multiply-adds per sample on `jc-audio-mix`, with no allocation. The mic check suggests it only when the measured energy under 60 Hz during speech exceeds −20 dB relative. Turning it off crossfades across one block (no DC click). Its state is flushed to zero on silence, so it never runs on subnormals (perf-review finding).
- No compressor, noise suppression, EQ or AGC was added. The G-Track's digital path already measures clean.

## Not verified
- A mic check with real speech, and a listening test in a DayZ clip. Both need the owner.
- The CPU cost of the low-cut wasn't benchmarked separately. It's about 0.25 M multiply-adds per second, well below the noise of the existing 0.2–0.7 % measurement.
