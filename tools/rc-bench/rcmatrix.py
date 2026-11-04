"""Rate-control matrix for h264_amf: size vs VMAF on lossless test scenes.

usage: python -I rcmatrix.py <dir with *.mkv scenes> <ffmpeg.exe> [config names...]
Writes results.csv in the scene dir and prints a table.
"""
import json
import os
import subprocess
import sys
import time

d, ff = sys.argv[1], sys.argv[2]
only = set(sys.argv[3:])
B = 21_150_720          # app Balanced target at 1080p60 H.264
PEAK = B * 3 // 2       # app max bitrate
COMMON = ["-usage", "transcoding", "-g", "60", "-bf", "0", "-pix_fmt", "nv12", "-an"]

configs = {
    "current":        ["-quality", "balanced", "-rc", "vbr_peak", "-b:v", str(B), "-maxrate", str(PEAK), "-bufsize", str(B)],
    "cur+quality":    ["-quality", "quality", "-rc", "vbr_peak", "-b:v", str(B), "-maxrate", str(PEAK), "-bufsize", str(B)],
    "cur+q+vbaq":     ["-quality", "quality", "-rc", "vbr_peak", "-b:v", str(B), "-maxrate", str(PEAK), "-bufsize", str(B), "-vbaq", "1"],
    "cur+q+pa":       ["-quality", "quality", "-rc", "vbr_peak", "-b:v", str(B), "-maxrate", str(PEAK), "-bufsize", str(B), "-preanalysis", "1"],
}
for lvl in (20, 23, 26, 29):
    configs[f"qvbr{lvl}"] = ["-quality", "quality", "-rc", "qvbr", "-qvbr_quality_level", str(lvl),
                             "-b:v", str(B), "-maxrate", str(PEAK), "-bufsize", str(PEAK)]
for qp in (20, 24, 28):
    configs[f"cqp{qp}"] = ["-quality", "quality", "-rc", "cqp", "-qp_i", str(qp), "-qp_p", str(qp)]

FRAME_CAP = PEAK // 60  # bits per frame at the app's peak rate
for qp in (22, 24, 26):
    configs[f"cqp{qp}cap"] = ["-quality", "balanced", "-rc", "cqp", "-qp_i", str(qp), "-qp_p", str(qp),
                              "-max_au_size", str(FRAME_CAP)]
    configs[f"cqp{qp}hrd"] = ["-quality", "balanced", "-rc", "cqp", "-qp_i", str(qp), "-qp_p", str(qp),
                              "-maxrate", str(PEAK), "-bufsize", str(PEAK), "-enforce_hrd", "1"]

for qmin in (18, 20, 22, 24):
    configs[f"cur+min{qmin}"] = ["-quality", "balanced", "-rc", "vbr_peak", "-b:v", str(B), "-maxrate", str(PEAK),
                                 "-bufsize", str(B), "-qmin", str(qmin)]

for label, scale, qmin in (("perf", 0.7, None), ("perf+min21", 0.7, 21), ("high", 1.5, None), ("high+min16", 1.5, 16)):
    t = int(B * scale)
    args = ["-quality", "balanced", "-rc", "vbr_peak", "-b:v", str(t), "-maxrate", str(t * 3 // 2), "-bufsize", str(t)]
    if qmin:
        args += ["-qmin", str(qmin)]
    configs[label] = args

scenes = sorted(f for f in os.listdir(d) if f.endswith(".mkv"))
rows = []
for name, args in configs.items():
    if only and name not in only:
        continue
    for sc in scenes:
        src = os.path.join(d, sc)
        out = os.path.join(d, f"enc_{name}_{sc[:-4]}.mp4")
        t0 = time.time()
        r = subprocess.run([ff, "-v", "error", "-y", "-i", src, "-c:v", "h264_amf", *COMMON, *args, out],
                           capture_output=True, text=True)
        enc_s = time.time() - t0
        if r.returncode != 0:
            print(f"{name:12} {sc:12} ENCODE FAILED: {r.stderr.strip()[:200]}")
            continue
        size = os.path.getsize(out)
        dur = 10.0
        log = os.path.join(d, "vmaf.json")
        lavfi = ("[0:v]setpts=N/60/TB[a];[1:v]setpts=N/60/TB[b];"
                 "[a][b]libvmaf=n_threads=16:log_fmt=json:log_path=vmaf.json")
        if os.path.exists(log):
            os.remove(log)
        subprocess.run([ff, "-v", "error", "-i", out, "-i", src, "-lavfi", lavfi, "-f", "null", "-"],
                       capture_output=True, text=True, cwd=d)
        try:
            j = json.load(open(log))
            per = sorted(f["metrics"]["vmaf"] for f in j["frames"])
            mean = j["pooled_metrics"]["vmaf"]["mean"]
            p5 = per[max(0, int(len(per) * 0.05) - 1)]
        except Exception as e:  # noqa: BLE001
            mean = p5 = float("nan")
            print("vmaf parse failed:", e)
        mbps = size * 8 / dur / 1e6
        rows.append((name, sc[:-4], mbps, mean, p5, 600 / enc_s))
        print(f"{name:12} {sc[:-4]:8} {mbps:6.1f} Mb/s  VMAF {mean:5.1f}  p5 {p5:5.1f}  enc {600 / enc_s:5.0f} fps", flush=True)
        os.remove(out)

with open(os.path.join(d, "results.csv"), "a") as f:
    for r in rows:
        f.write(",".join(str(x) for x in r) + "\n")
