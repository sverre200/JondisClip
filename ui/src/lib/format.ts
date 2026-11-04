import type { Settings } from "./api";

export function replayLabel(seconds: number): string {
  return seconds < 60 ? `${seconds} s` : `${seconds / 60} min`;
}

export function replayShort(seconds: number): string {
  return seconds < 60 ? `${seconds}s` : `${seconds / 60}m`;
}

/** Same bitrate model as the engine (hwenc.rs default_bitrate × quality scale). */
export function estimateBitrate(s: Settings, srcW: number, srcH: number): number {
  const target = s.resolution === "source" ? srcH : Math.min(parseInt(s.resolution, 10), srcH);
  const h = target & ~1;
  const w = Math.floor((srcW * h) / srcH) & ~1;
  const bpp = s.codec === "hevc" || s.codec === "av1" ? 0.12 : 0.17;
  const scale = { performance: 0.7, balanced: 1.0, high: 1.5 }[s.quality];
  return w * h * s.fps * bpp * scale;
}

export function outputSize(s: Settings, srcW: number, srcH: number): { w: number; h: number } {
  const target = s.resolution === "source" ? srcH : Math.min(parseInt(s.resolution, 10), srcH);
  const h = target & ~1;
  return { w: Math.floor((srcW * h) / srcH) & ~1, h };
}

export function bytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(n >= 1e10 ? 0 : 1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  return `${Math.round(n / 1e3)} KB`;
}

/** "Alt+Shift+F10" → ["Alt", "Shift", "F10"] for keycap rendering. */
export function keys(hotkey: string): string[] {
  return hotkey.split("+").filter(Boolean);
}

/** Builds a hotkey string from a key event, or null for a lone modifier. */
export function hotkeyFromEvent(e: KeyboardEvent): string | null {
  const code = e.code;
  let key: string | null = null;
  if (/^F([1-9]|1\d|2[0-4])$/.test(code)) key = code;
  else if (/^Key[A-Z]$/.test(code)) key = code.slice(3);
  else if (/^Digit\d$/.test(code)) key = code.slice(5);
  else if (/^Numpad\d$/.test(code)) key = code;
  else if (["Insert", "Home", "End", "PageUp", "PageDown", "Pause", "ScrollLock", "PrintScreen", "Space"].includes(code))
    key = code;
  if (!key) return null;
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Win"].filter(Boolean);
  return [...mods, key].join("+");
}

export function heldModifiers(e: KeyboardEvent): string[] {
  return [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Win"].filter(
    (m): m is string => Boolean(m),
  );
}
