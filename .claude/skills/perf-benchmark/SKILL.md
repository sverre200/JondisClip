---
name: perf-benchmark
description: Use when measuring JondisClip's impact on game performance, checking a hot-path change for regressions, or when anyone claims a performance number. Runs a baseline vs buffering comparison with PresentMon and PDH counters and records the result in docs/perf/.
---

# Performance benchmark

Never state a performance result without a recorded run. See `docs/PERFORMANCE.md` for the targets.

## Prerequisites
- PresentMon CLI on PATH (Intel, MIT licence, from github.com/GameTechDev/PresentMon releases; install only with the owner's approval).
- A repeatable scene, chosen with the owner. The owner runs the game. You start and stop the capture.

## Quick mode (synthetic, no game, for checking a hot-path change)
1. Start the engine on monitor capture with a GPU load running (any windowed 3D test that the owner approves).
2. Sample for 120 s:
   `typeperf "\Process(jondisclip)\% Processor Time" "\Process(jondisclip)\Working Set - Private" "\Process(jondisclip)\IO Write Bytes/sec" "\GPU Engine(*engtype_VideoEncode)\Utilization Percentage" -si 1 -sc 120 -f CSV -o <scratch>/pdh.csv`
3. Collect the engine's self-metrics (dropped and duplicated frames, encode latency) from the log.
4. Compare against the last recorded run. Flag a regression above 10 %.

## Full mode (real game)
1. For each condition, A (off), B (buffering) and C (buffering plus a save at t=30 s), run 3 times in alternating order:
   `PresentMon --process_name <game>.exe --timed 60 --delay 10 --output_file <scratch>/<cond>_<n>.csv --terminate_after_timed`
2. Run typeperf in parallel for the same window.
3. Compute avg FPS = 1000 / mean(frame time). The column is `MsBetweenPresents` or `FrameTime` depending on the PresentMon version. Compute the 1 % low as 1000 / p99(frame time), plus p99 frame time. Report the median across runs and the spread.
4. Write `docs/perf/YYYY-MM-DD-<game>.md` with the hardware, driver, game settings, recording settings, the results table, and a pass/fail against each target.
5. If a result is within the run-to-run spread, report "not measurable", not "0 %".
