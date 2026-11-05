// Lazy thumbnails (ADR-0010): only for cards on screen, one decode at a time,
// drawn by WebView2's own decoder and cached by the Rust side as JPEG. Nothing
// here runs unless the library is open and a card without a cached thumbnail
// scrolls into view.

import { api, clipUrl, type Clip } from "./api";

const W = 384;
const H = 216;
const TIMEOUT_MS = 10_000;

type Job = { clip: Clip; resolve: (ok: boolean) => void; cancelled: boolean };
const queue: Job[] = [];
let busy = false;
/** Clips that failed this session: don't retry until the window reopens. */
const failed = new Set<string>();

/** Where in a clip the thumbnail frame comes from. Saves are "the last N
 * minutes", so the moment is near the end: 8 s before it (middle of short clips). */
export function thumbTime(duration: number): number {
  if (!Number.isFinite(duration) || duration <= 0) return 0;
  if (duration <= 16) return duration / 2;
  return duration - 8;
}

/** Asks for a thumbnail. Resolves true when one is cached; `cancel` drops it
 * from the queue if it hasn't started (the card scrolled away). */
export function requestThumb(clip: Clip): { done: Promise<boolean>; cancel: () => void } {
  if (failed.has(clip.id) || !clip.probe) return { done: Promise.resolve(false), cancel: () => {} };
  let job!: Job;
  const done = new Promise<boolean>((resolve) => {
    job = { clip, resolve, cancelled: false };
  });
  queue.push(job);
  void pump();
  return {
    done,
    cancel: () => {
      job.cancelled = true;
    },
  };
}

async function pump() {
  if (busy) return;
  busy = true;
  try {
    for (let job = queue.shift(); job; job = queue.shift()) {
      if (job.cancelled) {
        job.resolve(false);
        continue;
      }
      const ok = await render(job.clip).catch(() => false);
      if (!ok) failed.add(job.clip.id);
      job.resolve(ok);
    }
  } finally {
    busy = false;
  }
}

function waitFor(v: HTMLVideoElement, event: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => done(new Error("timeout")), TIMEOUT_MS);
    const ok = () => done();
    const bad = () => done(new Error("decode error"));
    function done(err?: Error) {
      clearTimeout(timer);
      v.removeEventListener(event, ok);
      v.removeEventListener("error", bad);
      if (err) reject(err);
      else resolve();
    }
    v.addEventListener(event, ok, { once: true });
    v.addEventListener("error", bad, { once: true });
  });
}

function draw(g: CanvasRenderingContext2D, v: HTMLVideoElement) {
  g.fillStyle = "#000";
  g.fillRect(0, 0, W, H);
  const scale = Math.min(W / v.videoWidth, H / v.videoHeight);
  const dw = v.videoWidth * scale;
  const dh = v.videoHeight * scale;
  g.drawImage(v, (W - dw) / 2, (H - dh) / 2, dw, dh);
}

/** Mean luma 0..1 of a coarse sample of the canvas. */
export function brightness(g: CanvasRenderingContext2D): number {
  const d = g.getImageData(0, 0, W, H).data;
  let sum = 0;
  let n = 0;
  for (let i = 0; i < d.length; i += 4 * 97) {
    sum += 0.2126 * (d[i] ?? 0) + 0.7152 * (d[i + 1] ?? 0) + 0.0722 * (d[i + 2] ?? 0);
    n++;
  }
  return n ? sum / n / 255 : 0;
}

async function render(clip: Clip): Promise<boolean> {
  const v = document.createElement("video");
  v.muted = true;
  v.preload = "metadata";
  v.crossOrigin = "anonymous"; // the clip: protocol sends CORS headers, so the canvas stays readable
  try {
    v.src = clipUrl(clip.id);
    await waitFor(v, "loadedmetadata");
    const duration = clip.probe?.duration ?? v.duration;
    const c = document.createElement("canvas");
    c.width = W;
    c.height = H;
    const g = c.getContext("2d", { willReadFrequently: true });
    if (!g) return false;
    // Near the end first; if that frame is (nearly) black, such as a loading
    // screen or a fade, try the middle, then a quarter in.
    for (const t of [thumbTime(duration), duration / 2, duration / 4]) {
      v.currentTime = t;
      await waitFor(v, "seeked");
      if (!v.videoWidth) return false;
      draw(g, v);
      if (brightness(g) > 0.06) break;
    }
    const blob = await new Promise<Blob | null>((r) => c.toBlob(r, "image/jpeg", 0.82));
    if (!blob) return false;
    await api.thumbSave(clip.thumb_key, new Uint8Array(await blob.arrayBuffer()));
    return true;
  } finally {
    // Release the decoder at once.
    v.removeAttribute("src");
    v.load();
  }
}
