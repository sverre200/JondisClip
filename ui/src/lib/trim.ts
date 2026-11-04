// How a selection will be cut (ADR-0011). Lossless stream copy can only start
// on a keyframe; our recordings have one every second and no B-frames, so the
// end is always exact.

export interface CutPlan {
  /** The start already sits on a keyframe: lossless *and* exact. */
  onKeyframe: boolean;
  /** Stream copy (no re-encode). */
  lossless: boolean;
  /** Where the saved clip really starts. */
  start: number;
  /** How much earlier a lossless cut would start (s). */
  snapBy: number;
}

/** Half a frame at 60 fps: closer than this counts as "on" a keyframe. */
const ON_KEYFRAME = 1 / 120;

/** Last keyframe at or before `t` (0 if none is known). */
export function keyframeBefore(t: number, keyframes: number[]): number {
  let best = 0;
  for (const k of keyframes) {
    if (k <= t + ON_KEYFRAME) best = k;
    else break;
  }
  return best;
}

export function cutPlan(start: number, end: number, keyframes: number[], preferLossless: boolean): CutPlan {
  const kf = keyframeBefore(start, keyframes);
  const onKeyframe = keyframes.length > 0 && Math.abs(start - kf) <= ON_KEYFRAME;
  const lossless = onKeyframe || preferLossless;
  void end;
  return { onKeyframe, lossless, start: lossless ? kf : start, snapBy: Math.max(0, start - kf) };
}

/** Expected file size: same bitrate as the source for a copy; an exact cut
 * targets 1.3× the video bitrate (audio is copied, a small share). */
export function estimateSize(size: number, duration: number, plan: CutPlan, end?: number): number {
  if (!(duration > 0)) return 0;
  const length = (end ?? duration) - plan.start;
  const rate = size / duration;
  return Math.max(0, rate * length * (plan.lossless ? 1 : 1.28));
}
