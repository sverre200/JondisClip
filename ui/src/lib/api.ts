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
  /** Google Drive for desktop folder for Save to Drive ("" = off) */
  drive_dir: string;
  /** Save to Drive keeps the clip's library folder name (else its game) */
  drive_keep_folders: boolean;
  /** Window theme id (see lib/theme.ts) */
  theme: string;
}

export interface DriveCheck {
  dir: string;
  state: "off" | "ok" | "problem";
  message: string | null;
}

export interface DriveStatus {
  dir: string;
  available: boolean;
  suggestions: string[];
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
  /** Content fingerprint: identity for titles and folders (ADR-0014) */
  key: string;
  /** Custom title (metadata; the file name never changes for it) */
  title?: string;
  /** The user folder it's in on disk (null: "Not in a folder") */
  folder?: string | null;
  /** Thumbnail cache key (content fingerprint: survives moves) */
  thumb_key: string;
  /** Its copy in the Google Drive folder (Save to Drive), if still there. Not proof of upload. */
  in_drive?: string | null;
}

export type FolderColor = "red" | "orange" | "yellow" | "green" | "blue" | "purple" | "gray";
export const FOLDER_COLORS: { value: FolderColor; label: string }[] = [
  { value: "red", label: "Red" },
  { value: "orange", label: "Orange" },
  { value: "yellow", label: "Yellow" },
  { value: "green", label: "Green" },
  { value: "blue", label: "Blue" },
  { value: "purple", label: "Purple" },
  { value: "gray", label: "Gray" },
];

/** A user folder: a real directory under the clips root (ADR-0015). */
export interface Folder {
  /** = name: its path under the clips root, `\`-separated ("Edited\DayZ") */
  id: string;
  /** what the sidebar shows: the last part of the path */
  leaf: string;
  /** the folder it's inside, or null at the top level */
  parent: string | null;
  /** 1 = top level; folders go up to MAX_DEPTH */
  depth: number;
  name: string;
  color: FolderColor;
  /** clip keys in custom order (complete for the folder's clips) */
  clips: string[];
  count: number;
}

/** v1 folders whose clips haven't been moved into the real folder yet. */
export interface PendingFolder {
  folder: string;
  color: FolderColor;
  ids: string[];
}

export interface LibraryLoad {
  view: LibraryView;
  folders: Folder[];
  pending: PendingFolder[];
  rootOk: boolean;
}

/** Folders go up to this many levels deep (as the backend). */
export const MAX_DEPTH = 3;

/** `a` is inside `b` (not equal), ignoring case like Windows. */
export const isWithin = (a: string, b: string) => a.length > b.length + 1 && a.slice(0, b.length).toLowerCase() === b.toLowerCase() && a[b.length] === "\\";

export interface MoveResult {
  moved: number;
  failed: string[];
}

export interface Item {
  id: string;
  key: string;
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
  /** The page a later launch (tray "Settings…" / left-click) asked for, once. */
  takePageRequest: () => call<"library" | "settings" | null>("take_page_request"),
  /** Scans the clips root, reconciles folders with the disk, returns all. */
  async libraryLoad(): Promise<LibraryLoad> {
    const r = await call<{
      root: string;
      root_ok: boolean;
      clips: Clip[];
      folders: { name: string; color: FolderColor; count: number; order: string[] }[];
      pending: PendingFolder[];
    }>("library_load");
    return {
      view: { root: r.root, clips: r.clips.map((c) => ({ ...c, title: c.title ?? undefined })) },
      folders: r.folders.map((f) => {
        const cut = f.name.lastIndexOf("\\");
        return {
          id: f.name,
          name: f.name,
          leaf: cut < 0 ? f.name : f.name.slice(cut + 1),
          parent: cut < 0 ? null : f.name.slice(0, cut),
          depth: f.name.split("\\").length,
          color: f.color,
          clips: f.order,
          count: f.count,
        };
      }),
      pending: r.pending,
      rootOk: r.root_ok,
    };
  },
  /** `parent`: make it inside that folder (null: top level). */
  folderCreate: (name: string, color: FolderColor, parent: string | null = null) => call<void>("folder_create", { name, color, parent }),
  /** Moves a folder (and what's in it) into `parent`, or to the top level. */
  folderMove: (name: string, parent: string | null) => call<void>("folder_move", { name, parent }),
  /** Opens the folder's directory in File Explorer. */
  folderReveal: (name: string) => call<void>("folder_reveal", { name }),
  folderRename: (name: string, to: string) => call<void>("folder_rename", { name, to }),
  folderColor: (name: string, color: FolderColor) => call<void>("folder_color", { name, color }),
  folderDelete: (name: string) =>
    call<{ moved: number; failed: string[]; kept_dir: boolean; kept_reason: "in-use" | "files" | null; kept_files: string[] }>("folder_delete", { name }),
  foldersReorder: (names: string[]) => call<void>("folders_reorder", { names }),
  /** Moves the files into folder `to` (null: back to their game folders). */
  clipsMove: (items: Item[], to: string | null) => call<MoveResult>("clips_move", { items, to }),
  /** Custom order inside a folder: metadata only, no file moves. */
  clipsArrange: (folder: string, present: string[], keys: string[], before: string | null) =>
    call<void>("clips_arrange", { folder, present, keys, before }),
  clipTitle: (id: string, key: string, title: string) => call<void>("clip_title", { id, key, title }),
  /** Records a Save to Drive copy, so the library marks the clip. */
  clipDriveMark: (key: string, path: string) => call<void>("clip_drive_mark", { key, path }),
  pendingResolve: (folder: string, items: Item[], moveThem: boolean) => call<MoveResult>("pending_resolve", { folder, items, moveThem }),
  clipsDelete: (ids: string[]) => call<number>("clips_delete", { ids }),
  clipCopyFile: (id: string) => call<void>("clip_copy_file", { id }),
  showInFolder: (id: string) => call<void>("show_in_folder", { id }),
  clipKeyframes: (id: string) => call<number[]>("clip_keyframes", { id }),
  exportStart: (id: string, start: number, end: number, exact: boolean) =>
    call<{ job: number; output: string }>("export_start", { id, start, end, exact }),
  exportCancel: () => call<void>("export_cancel"),
  sharePlan: (id: string, start: number, end: number) => call<SharePlan>("share_plan", { id, start, end }),
  shareStart: (id: string, start: number, end: number, title: string | null) =>
    call<{ job: number; output: string }>("share_start", { id, start, end, title }),
  shareCopyAgain: (path: string) => call<void>("share_copy_again", { path }),
  shareReveal: (path: string) => call<void>("share_reveal", { path }),
  driveStatus: () => call<DriveStatus>("drive_status"),
  /** `title`: the clip's custom title, used as the file name in Drive. */
  driveSave: (id: string, start: number, end: number, whole: boolean, exact: boolean, title: string | null) =>
    call<{ job: number | null; output: string; already: boolean }>("drive_save", { id, start, end, whole, exact, title }),
  driveReveal: (path: string) => call<void>("drive_reveal", { path }),
  /** Exists, isn't inside the clips folder, and a test file can be written and removed. */
  driveCheck: (dir?: string) => call<DriveCheck>("drive_check", { dir: dir ?? null }),
  driveOpen: () => call<void>("drive_open"),
  async pickDriveFolder(current: string): Promise<string | null> {
    if (!isDesktop) return "G:\\My Drive";
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ directory: true, defaultPath: current || undefined, title: "Choose your Google Drive folder" });
    return typeof picked === "string" ? picked : null;
  },
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
const mockFolders: { name: string; color: FolderColor; order: string[] }[] = [
  { name: "DayZ Raids", color: "red", order: [] },
  { name: "Funny Moments", color: "green", order: [] },
  { name: "Friends", color: "blue", order: [] },
];
const mockTitles: Record<string, string> = {
  "000000000000a000": "Our Base Got Raided",
  "000000000000a001": "The Worst Grenade Ever, Thrown Straight Into Our Own Car",
};
const mockDrive: Record<string, string> = {
  "000000000000a000": "G:\\My Drive\\JondisClip\\DayZ\\Our Base Got Raided.mp4",
};
let mockLib: Clip[] | null = null;
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
      key: (0xa000 + i).toString(16).padStart(16, "0"),
      thumb_key: (0xa000 + i).toString(16).padStart(16, "0"),
      folder: i === 0 || i === 4 ? "DayZ Raids" : i === 1 ? "Funny Moments" : null,
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
    drive_dir: "",
    drive_keep_folders: true,
    theme: "crimson",
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
    case "take_page_request":
      return null;
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
    case "drive_status":
      return { dir: mockState.settings.drive_dir, available: !!mockState.settings.drive_dir, suggestions: ["G:\\My Drive"] };
    case "drive_save": {
      const job = Date.now();
      let p = 0;
      clearInterval(mockExport);
      mockExport = setInterval(() => {
        p = Math.min(1, p + 0.2);
        mockExportListeners.forEach((f) => f({ kind: "progress", job, progress: p }));
        if (p >= 1) {
          clearInterval(mockExport);
          mockExportListeners.forEach((f) =>
            f({ kind: "done", job, path: "G:\\My Drive\\JondisClip\\DayZ\\DayZ edit.mp4", size: 58e6, start: 0, duration: 20, video: "copy", seconds: 2, share: null, clipboard: null }),
          );
        }
      }, 200);
      return { job, output: "G:\\My Drive\\JondisClip\\DayZ\\DayZ edit.mp4", already: false };
    }
    case "drive_check": {
      await new Promise((r) => setTimeout(r, 400));
      const dir = (args?.dir as string | null) ?? mockState.settings.drive_dir;
      if (!dir) return { dir, state: "off", message: null };
      if (new URLSearchParams(location.search).has("driveoff"))
        return { dir, state: "problem", message: `Google Drive isn't available (${dir} not found). Is Google Drive for desktop running and signed in?` };
      return { dir, state: "ok", message: null };
    }
    case "drive_open":
      return undefined;
    case "drive_reveal":
    case "share_copy_again":
    case "share_reveal":
      return undefined;
    case "export_cancel":
      clearInterval(mockExport);
      mockExportListeners.forEach((f) => f({ kind: "cancelled", job: 0 }));
      return undefined;
    case "library_load": {
      await new Promise((r) => setTimeout(r, 200));
      if (new URLSearchParams(location.search).has("empty")) return { root: mockState.default_save_dir, root_ok: true, clips: [], folders: [], pending: [] };
      mockLib ??= mockClips();
      const clips = mockLib.map((c) => ({ ...c, title: mockTitles[c.key] ?? null, in_drive: mockDrive[c.key] ?? null }));
      return {
        root: mockState.default_save_dir,
        root_ok: true,
        clips,
        folders: mockFolders.map((f) => {
          const keys = clips.filter((c) => c.folder === f.name).map((c) => c.key);
          const order = [...f.order.filter((k) => keys.includes(k)), ...keys.filter((k) => !f.order.includes(k))];
          return { name: f.name, color: f.color, count: keys.length, order };
        }),
        pending: [],
      };
    }
    case "folder_create": {
      const leaf = String(args?.name ?? "").trim();
      if (!leaf) throw "Give the folder a name.";
      if (/[<>:"/\\|?*]/.test(leaf)) throw `Folder names can't contain ${leaf.match(/[<>:"/\\|?*]/)![0]}`;
      const parent = (args?.parent as string | null) ?? null;
      if (parent && parent.split("\\").length >= MAX_DEPTH) throw `Folders go up to ${MAX_DEPTH} levels deep.`;
      const name = parent ? `${parent}\\${leaf}` : leaf;
      if (mockFolders.some((f) => f.name.toLowerCase() === name.toLowerCase())) throw `There's already something called "${leaf}".`;
      mockFolders.push({ name, color: args?.color as FolderColor, order: [] });
      return undefined;
    }
    case "folder_rename":
    case "folder_move": {
      const from = String(args?.name ?? "");
      const f = mockFolders.find((x) => x.name === from);
      if (!f) throw "That folder no longer exists.";
      const leafOf = (n: string) => n.slice(n.lastIndexOf("\\") + 1);
      const parentOf = (n: string) => (n.includes("\\") ? n.slice(0, n.lastIndexOf("\\")) : null);
      let to: string;
      if (cmd === "folder_rename") {
        const leaf = String(args?.to ?? "").trim();
        if (!leaf) throw "Give the folder a name.";
        const p = parentOf(from);
        to = p ? `${p}\\${leaf}` : leaf;
      } else {
        const p = (args?.parent as string | null) ?? null;
        if (p && (p === from || isWithin(p, from))) throw "A folder can't go inside itself.";
        to = p ? `${p}\\${leafOf(from)}` : leafOf(from);
        const deepest = Math.max(...mockFolders.filter((x) => x.name === from || isWithin(x.name, from)).map((x) => x.name.split("\\").length - from.split("\\").length));
        if (to.split("\\").length + deepest > MAX_DEPTH) throw `Folders go up to ${MAX_DEPTH} levels deep.`;
      }
      if (to !== from && mockFolders.some((x) => x.name.toLowerCase() === to.toLowerCase())) throw `There's already something called "${leafOf(to)}" there.`;
      const re = (n: string) => (n === from ? to : isWithin(n, from) ? to + n.slice(from.length) : n);
      for (const x of mockFolders) x.name = re(x.name);
      for (const c of mockLib ?? []) if (c.folder) c.folder = re(c.folder);
      return undefined;
    }
    case "folder_reveal":
      return undefined;
    case "folder_color": {
      const f = mockFolders.find((x) => x.name === args?.name);
      if (f) f.color = args?.color as FolderColor;
      return undefined;
    }
    case "folder_delete": {
      const name = String(args?.name ?? "");
      const gone = (n: string) => n === name || isWithin(n, name);
      let moved = 0;
      for (const c of mockLib ?? []) if (c.folder && gone(c.folder)) ((c.folder = null), moved++);
      for (let k = mockFolders.length - 1; k >= 0; k--) if (gone(mockFolders[k]!.name)) mockFolders.splice(k, 1);
      return { moved, failed: [], kept_dir: false, kept_reason: null, kept_files: [] };
    }
    case "folders_reorder": {
      const names = args?.names as string[];
      mockFolders.sort((a, b) => names.indexOf(a.name) - names.indexOf(b.name));
      return undefined;
    }
    case "clips_move": {
      const items = args?.items as Item[];
      for (const it of items) {
        const c = mockLib?.find((x) => x.id === it.id);
        if (c) c.folder = (args?.to as string | null) ?? null;
      }
      const f = mockFolders.find((x) => x.name === args?.to);
      if (f) f.order = [...f.order.filter((k) => !items.some((i) => i.key === k)), ...items.map((i) => i.key)];
      return { moved: items.length, failed: [] };
    }
    case "clips_arrange": {
      const f = mockFolders.find((x) => x.name === args?.folder);
      if (!f) return undefined;
      const present = args?.present as string[];
      const keys = args?.keys as string[];
      let list = [...f.order.filter((k) => present.includes(k)), ...present.filter((k) => !f.order.includes(k))];
      let before = args?.before as string | null;
      if (before && keys.includes(before)) before = list.slice(list.indexOf(before)).find((k) => !keys.includes(k)) ?? null;
      list = list.filter((k) => !keys.includes(k));
      const at = before ? list.indexOf(before) : list.length;
      list.splice(at < 0 ? list.length : at, 0, ...keys);
      f.order = list;
      return undefined;
    }
    case "clip_title": {
      const t = String(args?.title ?? "").trim();
      if (t) mockTitles[String(args?.key)] = t;
      else delete mockTitles[String(args?.key)];
      return undefined;
    }
    case "clip_drive_mark":
      mockDrive[String(args?.key)] = String(args?.path);
      return undefined;
    case "pending_resolve":
      return { moved: 0, failed: [] };
    case "clips_delete":
      return (args?.ids as string[]).length;
    case "clip_copy_file":
      return undefined;

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
