# Performance targets and method

Every number here must come from a recorded run (`docs/perf/YYYY-MM-DD-<name>.md`). Until a run exists, a target is only a target.

## Targets (v1, on the dev machine: 5800X + RX 6800 XT, 1440p/60 recording)
| Metric | Target |
|---|---|
| Average FPS loss vs baseline | ≤ 3 % |
| 1 % low FPS loss | ≤ 5 % |
| 99th-percentile frame-time increase | ≤ 1 ms |
| JondisClip CPU (all threads, % of the whole machine) | ≤ 1.5 % average |
| Resident RAM, window closed, buffering | ≤ 120 MB working set |
| Recording frames dropped | 0 over 30 min at normal GPU load |
| Hotkey → feedback | < 100 ms |
| Hotkey → 5-minute clip finalised on NVMe | < 3 s |
| Disk writes | ≈ recording bitrate (no write amplification beyond 5 %) |

Encoder use is reported but has no target, because the video engine is separate from the 3D engine.

## Tools
- **PresentMon** (Intel, MIT, CLI) reports per-frame times for the game process, plus average FPS, 1 % low and percentiles. It's also the engine behind CapFrameX.
- **Windows PDH counters** (`typeperf`, or our own `bench` binary later):
  - `\Process(jondisclip)\% Processor Time`, `Working Set - Private`, `IO Write Bytes/sec`
  - `\GPU Engine(*engtype_3D)\Utilization Percentage` and `\GPU Engine(*engtype_VideoEncode)\Utilization Percentage`
  - `\GPU Process Memory(*)\Dedicated Usage`
- **Engine self-metrics:** dropped and duplicated frames, queue depth, encode latency, and per-save timings. These go to the log every 60 s and are shown on the Recording page.

## Method (see the `/perf-benchmark` skill)
1. Use a fixed, repeatable scene: a built-in benchmark, or a fixed spot and camera path in DayZ. Capture 60 s with a 10 s warm-up.
2. Run A (JondisClip off), B (buffering) and C (buffering plus one save during the run). Do 3 runs each and alternate the order (A B C A B C …).
3. Report the median of the runs and the spread. A difference smaller than the run-to-run spread counts as "no measurable impact", not "0 %".
4. Note the driver version, game version, resolution and graphics preset, and the recording settings.
