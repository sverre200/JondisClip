// Bridge to the Rust side (ui/src-tauri/src/main.rs). In a plain browser
// (design preview with `npm run dev`) it falls back to an in-memory mock, so the
// screen can be built and checked without the desktop app.

export type Resolution = "source" | "720p" | "1080p" | "1440p";
export type Quality = "performance" | "balanced" | "high";
export type Codec = "auto" | "h264" | "hevc" | "av1";

export interface Settings {
  replay_seconds: number;
  hotkey: string;
  save_dir: string;
  fps: number;
  resolution: Resolution;
  quality: Quality;
  codec: Codec;
  system_audio: boolean;
  microphone: boolean;
  /** Windows endpoint ID; empty = Windows default */
  microphone_device: string;
  /** dB, -12..+24, applied live */
  microphone_gain_db: number;
  /** 80 Hz low-cut, live */
  microphone_low_cut: boolean;
  save_sound: boolean;
  start_with_windows: boolean;
  /** "Share to Discord" size limit, MB (1 MB = 1,000,000 bytes) */
  discord_limit_mb: number;
}

export interface SharePlan {
  limit_mb: number;
  plan: { width: number; height: number; fps: number; video_bps: number; audio_bps: number; bpp: number; rough: boolean } | null;
  /** longest selection that fits, when this one doesn't */
  max_seconds: number | null;
}

export interface ShareInfo {
  width: number;
  height: number;
  fps: number;
  copied: boolean;
  limit: number;
  attempts: number;
}

export interface Probe {
  duration: number;
  width: number;
  height: number;
  /** "avc1" (H.264), "hvc1"/"hev1" (HEVC), "av01" (AV1) */
  codec: string;
  audio_tracks: number;
}

export interface Clip {
  /** Stable while the file is unchanged; also the thumbnail key */
  id: string;
  path: string;
  file_name: string;
  game: string;
  /** Local "YYYY-MM-DD HH:MM:SS" */
  recorded: string;
  size: number;
  probe: Probe | null;
  error: string | null;
  thumb: boolean;
  /** Made by the editor (`… edit.mp4`) */
  edit: boolean;
}

export type ExportEvent =
  | { kind: "progress"; job: number; progress: number }
  | {
      kind: "done";
      job: number;
      path: string;
      size: number;
      start: number;
      duration: number;
      video: string;
      seconds: number;
      /** set for a Discord share */
      share: ShareInfo | null;
      /** for a share: on the clipboard, or why not (Rust Result) */
      clipboard: { Ok: null } | { Err: string } | null;
    }
  | { kind: "error"; job: number; message: string }
  | { kind: "cancelled"; job: number };

export interface LibraryView {
  root: string;
  clips: Clip[];
}

export interface TrayStatus {
  state: "recording" | "paused" | "sleeping" | "low_disk" | "restarting" | string;
  summary: string;
  hotkey: string;
  hotkey_ok: boolean;
  encoder: string | null;
  width: number | null;
  height: number | null;
  fps: number | null;
  bitrate: number | null;
  audio_tracks: string[];
  audio_error: string | null;
  /** false while the chosen mic is unplugged; null when the mic is off */
  mic_connected: boolean | null;
}

export interface InputDevice {
  id: string;
  name: string;
  is_default: boolean;
  is_virtual: boolean;
}

export interface MicLevel {
  peak_db: number;
  rms_db: number;
  error: string | null;
}

export interface MicReport {
  seconds: number;
  mode: "mono" | "two_track" | "stereo";
  raw_peak_db: number;
  adc_clipped: number;
  raw_speech_db: number;
  speech_db: number;
  peak_db: number;
  noise_floor_db: number;
  limited_ms: number;
  low_freq_db: number;
  speech_detected: boolean;
  verdict: "good" | "quiet" | "hot" | "clipping" | "no_speech";
  tips: string[];
  suggested_gain_db: number;
}

export interface MicCheck {
  report: MicReport;
  audio: string;
}

export interface TrayView {
  running: boolean;
  status: TrayStatus | null;
}

export interface Snapshot {
  settings: Settings;
  default_save_dir: string;
  replay_presets: number[];
  fps_presets: number[];
  warnings: string[];
  tray: TrayView;
}

export const isDesktop = "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isDesktop) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  }
  return mock(cmd, args) as Promise<T>;
}

/** Base of the `clip:` protocol (Windows WebView2 form). */
const CLIP_BASE = "http://clip.localhost";

export const clipUrl = (id: string) => (isDesktop ? `${CLIP_BASE}/v/${id}` : "/dev-media/sample.mp4");
export const thumbUrl = (id: string) => (isDesktop ? `${CLIP_BASE}/t/${id}.jpg` : (mockThumbs.get(id) ?? ""));

export const api = {
  initialPage: () => call<"library" | "settings">("initial_page"),
  libraryScan: () => call<LibraryView>("library_scan"),
  showInFolder: (id: string) => call<void>("show_in_folder", { id }),
  clipKeyframes: (id: string) => call<number[]>("clip_keyframes", { id }),
  exportStart: (id: string, start: number, end: number, exact: boolean) =>
    call<{ job: number; output: string }>("export_start", { id, start, end, exact }),
  exportCancel: () => call<void>("export_cancel"),
  sharePlan: (id: string, start: number, end: number) => call<SharePlan>("share_plan", { id, start, end }),
  shareStart: (id: string, start: number, end: number) => call<{ job: number; output: string }>("share_start", { id, start, end }),
  shareCopyAgain: (path: string) => call<void>("share_copy_again", { path }),
  shareReveal: (path: string) => call<void>("share_reveal", { path }),
  /** Subscribes to export progress; returns the unsubscribe function. */
  async onExport(fn: (e: ExportEvent) => void): Promise<() => void> {
    if (!isDesktop) {
      mockExportListeners.add(fn);
      return () => mockExportListeners.delete(fn);
    }
    const { listen } = await import("@tauri-apps/api/event");
    return listen<ExportEvent>("export", (e) => fn(e.payload));
  },
  openExternal: (id: string) => call<void>("open_external", { id }),
  async thumbSave(id: string, jpeg: Uint8Array): Promise<void> {
    if (!isDesktop) {
      mockThumbs.set(id, URL.createObjectURL(new Blob([jpeg as BlobPart], { type: "image/jpeg" })));
      return;
    }
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("thumb_save", jpeg, { headers: { "x-clip-id": id } });
  },
  load: () => call<Snapshot>("load"),
  trayStatus: () => call<TrayView>("tray_status"),
  update: (patch: Partial<Settings>) => call<Settings>("update", { patch }),
  checkHotkey: (text: string) => call<string>("check_hotkey", { text }),
  openFolder: (path: string) => call<void>("open_folder", { path }),
  startTray: () => call<void>("start_tray"),
  listMicrophones: () => call<InputDevice[]>("list_microphones"),
  micLevel: (device: string, gainDb: number) => call<MicLevel>("mic_level", { device, gainDb }),
  micMeterStop: () => call<void>("mic_meter_stop"),
  micCheck: (device: string, gainDb: number, lowCut: boolean, seconds: number) =>
    call<MicCheck>("mic_check", { device, gainDb, lowCut, seconds }),

  async pickFolder(current: string): Promise<string | null> {
    if (!isDesktop) return null;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ directory: true, defaultPath: current, title: "Choose where clips are saved" });
    return typeof picked === "string" ? picked : null;
  },

  /** Shows the window once the first frame is ready (no blank flash). */
  async reveal(): Promise<void> {
    if (!isDesktop) return;
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const w = getCurrentWindow();
    await w.show();
    await w.setFocus();
  },
};

// ---------- browser-preview mock (never used in the desktop app) ----------

const mockThumbs = new Map<string, string>();
const mockExportListeners = new Set<(e: ExportEvent) => void>();
let mockExport: ReturnType<typeof setInterval> | undefined;

function mockClips(): Clip[] {
  const games = ["DayZ", "DayZ", "DayZ", "Escape From Tarkov", "DayZ", "Counter-Strike 2", "Escape From Tarkov", "DayZ"];
  const now = new Date();
  const out: Clip[] = [];
  for (let i = 0; i < 23; i++) {
    const d = new Date(now.getTime() - i * 5.3 * 3600_000);
    const pad = (n: number) => String(n).padStart(2, "0");
    const stamp = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
    const game = games[i % games.length] ?? "DayZ";
    const broken = i === 6;
    const duration = i % 4 === 1 ? 42.7 : i % 5 === 2 ? 18.2 : 300.4;
    out.push({
      id: (0x1000 + i).toString(16).padStart(16, "0"),
      path: `C:\Users\you\Videos\JondisClip\${game}\${game} ${stamp.replaceAll(":", "-")}.mp4`,
      file_name: `${game} ${stamp.replaceAll(":", "-")}.mp4`,
      game,
      recorded: stamp,
      size: Math.round(duration * 2.2e6),
      probe: broken ? null : { duration, width: 1920, height: 1080, codec: "avc1", audio_tracks: 3 },
      error: broken ? "the file is cut off (incomplete recording?)" : null,
      thumb: false,
      edit: i === 3,
    });
  }
  return out;
}

const mockState: Snapshot = {
  settings: {
    replay_seconds: 300,
    hotkey: "Alt+F10",
    save_dir: "",
    fps: 60,
    resolution: "source",
    quality: "balanced",
    codec: "auto",
    system_audio: true,
    microphone: false,
    microphone_device: "",
    microphone_gain_db: 0,
    microphone_low_cut: false,
    save_sound: true,
    start_with_windows: false,
    discord_limit_mb: 19,
  },
  default_save_dir: "C:\\Users\\you\\Videos\\JondisClip",
  replay_presets: [30, 60, 120, 180, 300, 600, 900],
  fps_presets: [30, 60, 120, 144],
  warnings: [],
  tray: {
    running: true,
    status: {
      state: "recording",
      summary: "Buffering last 5 min · H.264 1920×1080 60 fps",
      hotkey: "Alt+F10",
      hotkey_ok: true,
      encoder: "h264_amf",
      width: 1920,
      height: 1080,
      fps: 60,
      bitrate: 21_150_000,
      audio_tracks: ["Game"],
      audio_error: null,
      mic_connected: null,
    },
  },
};

async function mock(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  await new Promise((r) => setTimeout(r, 120));
  switch (cmd) {
    case "load":
      return structuredClone(mockState);
    case "initial_page":
      return new URLSearchParams(location.search).get("page") === "settings" ? "settings" : "library";
    case "clip_keyframes": {
      const n = 301;
      return Array.from({ length: n }, (_, i) => i);
    }
    case "export_start": {
      const job = Date.now();
      const exact = Boolean(args?.exact);
      const start = Number(args?.start ?? 0);
      const end = Number(args?.end ?? 1);
      let p = 0;
      clearInterval(mockExport);
      mockExport = setInterval(() => {
        p = Math.min(1, p + (exact ? 0.07 : 0.4));
        mockExportListeners.forEach((f) => f({ kind: "progress", job, progress: p }));
        if (p >= 1) {
          clearInterval(mockExport);
          const s = exact ? start : Math.floor(start);
          mockExportListeners.forEach((f) =>
            f({ kind: "done", job, path: "C:\Users\you\Videos\JondisClip\DayZ\DayZ 2026-11-04 17-47-11 edit.mp4", size: (end - s) * 2.9e6, start: s, duration: end - s, video: exact ? "h264_amf" : "copy", seconds: 1, share: null, clipboard: null }),
          );
        }
      }, 200);
      return { job, output: "C:\…\DayZ 2026-11-04 17-47-11 edit.mp4" };
    }
    case "share_plan": {
      const len = Number(args?.end ?? 0) - Number(args?.start ?? 0);
      const mb = mockState.settings.discord_limit_mb;
      const total = (mb * 1e6 * 0.92 * 8) / Math.max(len, 0.1);
      if (total < 214_000) return { limit_mb: mb, plan: null, max_seconds: Math.floor((mb * 1e6 * 0.92 * 8) / 214_000) };
      const rungs = [[1920, 1080, 60], [1280, 720, 60], [1280, 720, 30], [960, 540, 30], [852, 480, 30], [640, 360, 30]] as const;
      let pick = rungs[rungs.length - 1]!;
      for (const r of rungs) if ((total - 160_000) / (r[0] * r[1] * r[2]) >= 0.06) { pick = r; break; }
      return { limit_mb: mb, plan: { width: pick[0], height: pick[1], fps: pick[2], video_bps: total - 160_000, audio_bps: 0, bpp: 0.07, rough: false }, max_seconds: null };
    }
    case "share_start": {
      const job = Date.now();
      let p = 0;
      clearInterval(mockExport);
      mockExport = setInterval(() => {
        p = Math.min(1, p + 0.12);
        mockExportListeners.forEach((f) => f({ kind: "progress", job, progress: p }));
        if (p >= 1) {
          clearInterval(mockExport);
          mockExportListeners.forEach((f) =>
            f({ kind: "done", job, path: "C:\\\\Users\\\\you\\\\AppData\\\\Local\\\\JondisClip\\\\share\\\\DayZ 2026-11-04 17-47-11.mp4", size: 18.1e6, start: 0, duration: 20, video: "h264_amf", seconds: 5, share: { width: 1280, height: 720, fps: 60, copied: false, limit: 19e6, attempts: 1 }, clipboard: { Ok: null } }),
          );
        }
      }, 200);
      return { job, output: "share.mp4" };
    }
    case "share_copy_again":
    case "share_reveal":
      return undefined;
    case "export_cancel":
      clearInterval(mockExport);
      mockExportListeners.forEach((f) => f({ kind: "cancelled", job: 0 }));
      return undefined;
    case "library_scan":
      await new Promise((r) => setTimeout(r, 300));
      if (new URLSearchParams(location.search).has("empty")) return { root: mockState.default_save_dir, clips: [] };
      return { root: mockState.default_save_dir, clips: mockClips() };
    case "tray_status":
      return structuredClone(mockState.tray);
    case "update":
      Object.assign(mockState.settings, args?.patch as Partial<Settings>);
      return structuredClone(mockState.settings);
    case "list_microphones":
      return [
        { id: "{mock-1}", name: "CABLE Output (VB-Audio Virtual Cable)", is_default: true, is_virtual: true },
        { id: "{mock-2}", name: "Headset Microphone (Arctis GameBuds)", is_default: false, is_virtual: false },
      ];
    case "mic_level": {
      // Speech-like: bursts around -22 dBFS with pauses.
      const t = Date.now() / 1000;
      const talking = Math.sin(t * 1.3) > -0.2;
      const base = talking ? -24 + 6 * Math.sin(t * 7.1) : -62;
      const gain = Number(args?.gainDb ?? 0);
      return { peak_db: base + 9 + gain, rms_db: base + gain, error: null };
    }
    case "mic_meter_stop":
      return undefined;
    case "mic_check":
      await new Promise((r) => setTimeout(r, Number(args?.seconds ?? 8) * 1000 - 120));
      return {
        report: {
          seconds: 8, mode: "mono", raw_peak_db: -14.2, adc_clipped: 0, raw_speech_db: -33.5, speech_db: -15.5,
          peak_db: -0.9, noise_floor_db: -64.8, limited_ms: 0, low_freq_db: -27.1, speech_detected: true,
          verdict: "good", suggested_gain_db: 14,
          tips: [
            "Your voice reaches the computer at -34 dBFS. Turn the G-Track's MIC GAIN knob up (aim for about 14 dB more) and set Microphone gain here back to 0 dB.",
          ],
        },
        audio: "",
      };
    case "check_hotkey": {
      const text = String(args?.text ?? "");
      if (/^(F\d{1,2})$/.test(text) || text.includes("+")) return text;
      throw `"${text}" needs Ctrl, Alt, Shift or Win`;
    }
    default:
      return undefined;
  }
}
