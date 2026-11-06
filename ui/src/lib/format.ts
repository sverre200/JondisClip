import type { Settings } from "./api";
import { numericDate, shortNumericDate, t, weekday } from "./i18n.svelte";

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

/** 65.4 → "1:05", 3725 → "1:02:05". */
export function clock(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const s = Math.floor(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** Like `clock` with tenths ("0:12.4"), for precise positions. */
export function clockPrecise(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const tenths = Math.floor((seconds % 1) * 10);
  return `${clock(seconds)}.${tenths}`;
}

/** "YYYY-MM-DD HH:MM:SS" (local) → Date. */
export function parseLocal(stamp: string): Date {
  const [d = "", t = "00:00:00"] = stamp.split(" ");
  const [y = 1970, mo, da] = d.split("-").map(Number);
  const [h, mi, s] = t.split(":").map(Number);
  return new Date(y, (mo || 1) - 1, da || 1, h || 0, mi || 0, s || 0);
}

const DAY = 86_400_000;
const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

/** Group heading for a clip's day: "Today", "Yesterday", or "Wednesday, 04.11.2026" in the chosen date format. */
export function dayLabel(stamp: string, now = new Date()): string {
  const d = parseLocal(stamp);
  const diff = Math.round((startOfDay(now) - startOfDay(d)) / DAY);
  if (diff === 0) return t("Today");
  if (diff === 1) return t("Yesterday");
  return `${weekday(d)}, ${numericDate(d)}`;
}

/** "17:47" (local clock time of a stamp). */
export function timeOfDay(stamp: string): string {
  return stamp.slice(11, 16);
}

/** A clip card's date, kept short so titles keep their room (no year when it's this year, except year-first). */
export function shortDate(stamp: string, now = new Date()): string {
  return shortNumericDate(parseLocal(stamp), now);
}

/** "Today 17:47", "Yesterday 09:02", "04.11. 17:47". */
export function whenShort(stamp: string, now = new Date()): string {
  const d = parseLocal(stamp);
  const diff = Math.round((startOfDay(now) - startOfDay(d)) / DAY);
  const day = diff === 0 ? t("Today") : diff === 1 ? t("Yesterday") : shortNumericDate(d, now);
  return `${day} ${timeOfDay(stamp)}`;
}

/** Codec fourcc → name. */
export function codecName(fourcc: string): string {
  if (fourcc === "avc1" || fourcc === "avc3") return "H.264";
  if (fourcc === "hvc1" || fourcc === "hev1") return "HEVC";
  if (fourcc === "av01") return "AV1";
  return fourcc;
}

/** True if a key event comes from inside an element matching `selector`. */
export function fromWithin(e: Event, selector: string): boolean {
  return e.target instanceof Element && e.target.closest(selector) !== null;
}
