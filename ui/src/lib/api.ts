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
}

export interface InputDevice {
  id: string;
  name: string;
  is_default: boolean;
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
        { id: "{mock-1}", name: "Microphone (Arctis 7 Chat)", is_default: true },
        { id: "{mock-2}", name: "Microphone (Logitech BRIO)", is_default: false },
      ];
    case "check_hotkey": {
      const text = String(args?.text ?? "");
      if (/^(F\d{1,2})$/.test(text) || text.includes("+")) return text;
      throw `"${text}" needs Ctrl, Alt, Shift or Win`;
    }
    default:
      return undefined;
  }
}
