#!/usr/bin/env bash
# Builds the three lossless 1080p60 test scenes used in
# docs/perf/2026-11-04-rate-control.md (about 3 GB of FFV1).
# usage: tools/rc-bench/make_scenes.sh <out_dir>
set -euo pipefail
OUT="${1:?usage: make_scenes.sh <out_dir>}"
F="$(dirname "$0")/../../third_party/ffmpeg/9.0/bin/ffmpeg.exe"
mkdir -p "$OUT"
cd "$OUT"
"$F" -v error -f lavfi -i "mandelbrot=s=3840x2160:maxiter=1500:start_scale=0.012:start_x=-0.743643887:start_y=0.131825904" -frames:v 1 -y base.png
"$F" -v error -loop 1 -framerate 60 -i base.png -t 10 -vf "noise=alls=10:allf=t,crop=1920:1080:x='min(t*150\,1900)':y='min(t*50\,1000)',format=yuv420p" -c:v ffv1 -level 3 -threads 16 -y A_pan.mkv
"$F" -v error -loop 1 -framerate 60 -i base.png -t 10 -vf "noise=alls=10:allf=t,crop=1920:1080:x='960+900*sin(t*2.2)':y='540+480*sin(t*1.5)',format=yuv420p" -c:v ffv1 -level 3 -threads 16 -y B_fast.mkv
"$F" -v error -loop 1 -framerate 60 -i base.png -f lavfi -i "testsrc2=s=360x200:r=60" -t 10 -filter_complex "[0]crop=1920:1080:400:300,format=yuv420p[bg];[bg][1]overlay=x=1500:y=820:shortest=1,format=yuv420p" -c:v ffv1 -level 3 -threads 16 -y C_calm.mkv
echo "scenes in $OUT; score with: python -I tools/rc-bench/rcmatrix.py $OUT third_party/ffmpeg/9.0/bin/ffmpeg.exe [configs...]"
