# Rate-control bench

Reproduces [docs/perf/2026-11-04-rate-control.md](../../docs/perf/2026-11-04-rate-control.md): encodes lossless test scenes with `h264_amf` under different rate-control settings and scores them with VMAF (frame-index aligned).

```bash
tools/rc-bench/make_scenes.sh "$TEMP/rc-scenes"
python -I tools/rc-bench/rcmatrix.py "$TEMP/rc-scenes" third_party/ffmpeg/9.0/bin/ffmpeg.exe current cur+min18
```

Run it with no config names to run the whole matrix (about 10 minutes). The scenes take about 3 GB, so delete them afterwards. Synthetic grain makes scenes A and B harsher than real games, so confirm changes on real gameplay too.
