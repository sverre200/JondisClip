---
name: clip-ops
description: Use when implementing or debugging JondisClip clip finalisation (segment concat), trimming, share/Discord export, thumbnails or media verification. Gives FFmpeg-equivalent recipes and ffprobe checks so library code can be validated against known-good CLI output.
---

# Clip operations

The app uses libavformat/libavcodec directly. These CLI equivalents are the **reference behaviour**, and tests compare against them.

## Finalise a clip (stream copy, no re-encode)
`ffmpeg -f concat -safe 0 -i list.txt -c copy -map 0 -movflags +faststart out.mp4`
- Start at the first pinned segment (it begins with a keyframe). Rebase timestamps to 0 (`-avoid_negative_ts make_zero`).

## Trim, keeping quality
`ffmpeg -ss <start> -to <end> -i in.mp4 -c copy -map 0 -avoid_negative_ts make_zero out.mp4`
- With `-ss` before `-i`, the start snaps to the previous keyframe. Show the snapped start time in the UI.
- Exact cuts need a re-encode of the first GOP only (smart cut), which is deferred. Until then, offer "Exact (re-encode)" as a separate option.

## Share or Discord export (target size S MB, duration D s)
- `video_kbps = floor((S * 8192 * 0.95) / D) - audio_kbps`, with `audio_kbps = 128`.
- Pick the resolution from the bitrate: 1080p if it's ≥ 6000 kb/s, 720p if ≥ 2500, otherwise 480p. If `video_kbps` < 500, refuse and suggest trimming.
- Use the hardware encoder: `-c:v h264_amf -rc cbr -b:v {v}k -maxrate {v}k -bufsize {v}k -c:a aac -b:a 128k -ac 2 -map 0:v -map 0:a:0 -movflags +faststart`
- Check that the output is ≤ S MB. If it's over, retry once at 90 % bitrate.

## Thumbnail
`ffmpeg -ss <10% of duration> -i in.mp4 -frames:v 1 -vf scale=480:-2 thumb.jpg`

## Verification (use in tests)
- `ffprobe -v error -show_entries format=duration:stream=codec_name,width,height,avg_frame_rate,start_time -of json f.mp4`
- Duration within ±1 GOP of the request. Video and audio `start_time` differ by < 40 ms. The file decodes fully: `ffmpeg -v error -i f.mp4 -f null -` prints nothing.
