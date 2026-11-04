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
  save_sound: boolean;
  start_with_windows: boolean;
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

export const api = {
  load: () => call<Snapshot>("load"),
  trayStatus: () => call<TrayView>("tray_status"),
  update: (patch: Partial<Settings>) => call<Settings>("update", { patch }),
  checkHotkey: (text: string) => call<string>("check_hotkey", { text }),
  openFolder: (path: string) => call<void>("open_folder", { path }),
  startTray: () => call<void>("start_tray"),
  listMicrophones: () => call<InputDevice[]>("list_microphones"),
  micLevel: (device: string, gainDb: number) => call<MicLevel>("mic_level", { device, gainDb }),
  micMeterStop: () => call<void>("mic_meter_stop"),

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
    save_sound: true,
    start_with_windows: false,
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
    case "check_hotkey": {
      const text = String(args?.text ?? "");
      if (/^(F\d{1,2})$/.test(text) || text.includes("+")) return text;
      throw `"${text}" needs Ctrl, Alt, Shift or Win`;
    }
    default:
      return undefined;
  }
}
