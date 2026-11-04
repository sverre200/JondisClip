//! The always-running tray app (ADR-0006): a hidden top-level window on the
//! main thread owns the tray icon, the global hotkey and the recorder, and
//! supervises the recorder. It restarts it after errors, display changes and
//! sleep, and pauses it on low disk space.
//!
//! Nothing here is on the capture path: the window only wakes for input, a
//! 1 s supervisor timer and save completions.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use jondisclip_engine::encoder::Codec;
use jondisclip_engine::recorder::{Recorder, default_buffer_dir};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_NOSOUND,
    NIIF_RESPECT_QUIET_TIME, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIN_SELECT, NINF_KEY, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, HICON, HMENU,
    KillTimer, MB_ICONHAND, MB_OK, MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SW_SHOWNORMAL,
    SetForegroundWindow, SetMenuDefaultItem, SetTimer, TPM_BOTTOMALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CONTEXTMENU,
    WM_DESTROY, WM_DISPLAYCHANGE, WM_HOTKEY, WM_NULL, WM_POWERBROADCAST, WM_TIMER, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::autostart;
use crate::icon::{self, IconState};
use crate::log::{error, info, warning};
use crate::naming;
use jondisclip_core::hotkey::MOD_NOREPEAT;
use jondisclip_core::paths;
use jondisclip_core::settings::{REPLAY_PRESETS, Settings, replay_label};
use jondisclip_core::status::TrayStatus;

pub const WINDOW_CLASS: PCWSTR = w!("JondisClipTray");
const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_SAVED: u32 = WM_APP + 2;
pub const WM_APP_SECOND_INSTANCE: u32 = WM_APP + 3;
const HOTKEY_ID: i32 = 1;
const TIMER_ID: usize = 1;
const TRAY_UID: u32 = 1;

const ID_SAVE: u32 = 100;
const ID_PAUSE: u32 = 101;
const ID_MIC: u32 = 102;
const ID_OPEN_FOLDER: u32 = 103;
const ID_EDIT_SETTINGS: u32 = 104;
const ID_QUIT: u32 = 105;
const ID_REPLAY_BASE: u32 = 200;

/// Free space below which buffering pauses, and above which it resumes.
const LOW_DISK_BYTES: u64 = 2 << 30;
const RESUME_DISK_BYTES: u64 = 3 << 30;
const RESTART_BACKOFF_S: [u64; 5] = [2, 5, 10, 30, 60];

#[derive(Debug, Clone, PartialEq)]
enum Status {
    Recording,
    Paused,
    Suspended,
    LowDisk,
    Restarting { at: Instant, reason: String },
}

struct SaveOutcome {
    result: Result<Vec<PathBuf>, String>,
    app: String,
    seconds: u32,
}

struct App {
    hwnd: HWND,
    settings: Settings,
    settings_path: PathBuf,
    settings_mtime: Option<SystemTime>,
    default_save_dir: PathBuf,
    recorder: Option<Recorder>,
    status: Status,
    hotkey_ok: bool,
    icons: [HICON; 4],
    shown: Option<(IconState, String)>,
    /// Last status published to status.json (written only on change).
    published: Option<TrayStatus>,
    saved_flash_until: Option<Instant>,
    last_save_at: Option<Instant>,
    saves: Arc<Mutex<Vec<SaveOutcome>>>,
    failures: usize,
    tick: u64,
    taskbar_created: u32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app state. It returns None if the state is unavailable,
/// e.g. on a nested message while it's already borrowed.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn wide_into(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16()) {
        *d = c;
        n += 1;
    }
    let end = n.min(dst.len() - 1);
    dst[end] = 0;
}

fn file_mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn free_bytes(dir: &Path) -> Option<u64> {
    let mut probe = dir.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let mut free = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(
            &HSTRING::from(probe.as_os_str()),
            Some(&mut free),
            None,
            None,
        )
        .ok()?
    };
    Some(free)
}

fn shell_open(file: &str, params: Option<&str>) {
    let params = params.map(HSTRING::from);
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(file),
            params
                .as_ref()
                .map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Loads settings, creating the file with defaults on first run.
pub fn load_settings(path: &Path) -> Settings {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (s, warnings) = Settings::from_toml(&text);
            for w in warnings {
                warning!("{w}");
            }
            s
        }
        Err(_) => {
            let s = Settings::default();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(path, s.to_toml()) {
                warning!("could not create {}: {e}", path.display());
            }
            s
        }
    }
}

pub fn run(settings_path: PathBuf, default_save_dir: PathBuf) -> Result<(), String> {
    let settings = load_settings(&settings_path);
    let hinstance = unsafe { GetModuleHandleW(None) }.map_err(|e| e.to_string())?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance.into(),
        lpszClassName: WINDOW_CLASS,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err("RegisterClassW failed".into());
    }
    // A hidden top-level window (not message-only), so it receives
    // WM_POWERBROADCAST and WM_DISPLAYCHANGE.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0),
            WINDOW_CLASS,
            w!("JondisClip Tray"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
    }
    .map_err(|e| e.to_string())?;

    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let icons = [
        IconState::Recording,
        IconState::Paused,
        IconState::Saved,
        IconState::Warning,
    ]
    .map(|s| icon::make(s, size).unwrap_or_default());
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };

    let app = App {
        hwnd,
        settings_mtime: file_mtime(&settings_path),
        settings,
        settings_path,
        default_save_dir,
        recorder: None,
        status: Status::Paused,
        hotkey_ok: false,
        icons,
        shown: None,
        published: None,
        saved_flash_until: None,
        last_save_at: None,
        saves: Arc::new(Mutex::new(Vec::new())),
        failures: 0,
        tick: 0,
        taskbar_created,
    };
    APP.with(|a| *a.borrow_mut() = Some(app));
    with_app(|a| {
        a.add_tray_icon();
        a.register_hotkey(true);
        if let Err(e) = autostart::apply(a.settings.start_with_windows) {
            warning!("start with Windows: {e}");
        }
        a.start_recorder();
        a.refresh();
    });
    unsafe { SetTimer(Some(hwnd), TIMER_ID, 1000, None) };

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    APP.with(|a| a.borrow_mut().take());
    info!("JondisClip exited");
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_TRAY => {
            let event = (lparam.0 & 0xFFFF) as u32;
            if event == WM_CONTEXTMENU {
                show_menu(hwnd);
            } else if event == NIN_SELECT || event == (NIN_SELECT | NINF_KEY) {
                // Left click / Enter opens the app window.
                with_app(App::open_settings);
            }
            LRESULT(0)
        }
        WM_HOTKEY if wparam.0 as i32 == HOTKEY_ID => {
            with_app(App::save_clip);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_ID => {
            with_app(App::on_tick);
            LRESULT(0)
        }
        WM_APP_SAVED => {
            with_app(App::on_saved);
            LRESULT(0)
        }
        WM_APP_SECOND_INSTANCE => {
            with_app(|a| {
                a.balloon(
                    "JondisClip is already running",
                    "Right-click the tray icon for options.",
                    false,
                )
            });
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            match wparam.0 as u32 {
                PBT_APMSUSPEND => {
                    with_app(|a| {
                        info!("system suspending: stopping recorder");
                        a.stop_recorder();
                        a.status = Status::Suspended;
                        a.refresh();
                    });
                }
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                    with_app(|a| {
                        if a.status == Status::Suspended {
                            info!("system resumed: restarting recorder shortly");
                            a.schedule_restart(3, "resuming from sleep".into());
                        }
                    });
                }
                _ => {}
            }
            LRESULT(1)
        }
        WM_DISPLAYCHANGE => {
            with_app(|a| {
                if a.recorder.is_some() {
                    info!("display mode changed: restarting recorder");
                    a.stop_recorder();
                    a.schedule_restart(2, "display changed".into());
                }
            });
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_DESTROY => {
            with_app(App::shutdown);
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => {
            let recreated = with_app(|a| {
                if msg == a.taskbar_created && msg != 0 {
                    // Explorer restarted: the icon must be added again.
                    a.shown = None;
                    a.add_tray_icon();
                    a.refresh();
                    true
                } else {
                    false
                }
            });
            if recreated == Some(true) {
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

/// Builds the menu with the state borrowed, then runs its modal loop without
/// the borrow, so timer ticks keep working while the menu is open.
fn show_menu(hwnd: HWND) {
    let Some(menu) = with_app(App::build_menu) else {
        return;
    };
    let mut pt = POINT::default();
    let cmd = unsafe {
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0,
            pt.x,
            pt.y,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0 as u32
    };
    if cmd != 0 {
        with_app(|a| a.on_command(cmd));
    }
}

fn menu_text(s: &str) -> HSTRING {
    HSTRING::from(s)
}

impl App {
    // ---------- tray icon ----------

    fn base_nid(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_UID,
            ..Default::default()
        }
    }

    fn add_tray_icon(&mut self) {
        let mut nid = self.base_nid();
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        nid.uCallbackMessage = WM_APP_TRAY;
        nid.hIcon = self.icons[1];
        wide_into(&mut nid.szTip, "JondisClip");
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
    }

    fn icon_state(&self) -> IconState {
        if self.saved_flash_until.is_some_and(|t| Instant::now() < t) {
            return IconState::Saved;
        }
        match self.status {
            Status::Recording => IconState::Recording,
            Status::Paused | Status::Suspended => IconState::Paused,
            Status::LowDisk | Status::Restarting { .. } => IconState::Warning,
        }
    }

    fn status_text(&self) -> String {
        match &self.status {
            Status::Recording => {
                let detail = self.recorder.as_ref().map(|r| {
                    let i = r.info();
                    let codec = match i.encoder.codec {
                        Codec::Hevc => "HEVC",
                        Codec::H264 => "H.264",
                        Codec::Av1 => "AV1",
                    };
                    format!(" · {codec} {}×{} {} fps", i.width, i.height, i.fps)
                });
                format!(
                    "Buffering last {}{}",
                    replay_label(self.settings.replay_seconds),
                    detail.unwrap_or_default()
                )
            }
            Status::Paused => "Paused".into(),
            Status::Suspended => "Sleeping".into(),
            Status::LowDisk => "Paused: low disk space".into(),
            Status::Restarting { reason, .. } => format!("Restarting ({reason})"),
        }
    }

    /// Updates icon and tooltip if they changed.
    fn refresh(&mut self) {
        let state = self.icon_state();
        let mut tip = format!("JondisClip · {}", self.status_text());
        if !self.hotkey_ok {
            tip.push_str(" · hotkey unavailable");
        }
        if self
            .shown
            .as_ref()
            .is_some_and(|(s, t)| *s == state && *t == tip)
        {
            self.publish_status();
            return;
        }
        let mut nid = self.base_nid();
        nid.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        nid.hIcon = self.icons[match state {
            IconState::Recording => 0,
            IconState::Paused => 1,
            IconState::Saved => 2,
            IconState::Warning => 3,
        }];
        wide_into(&mut nid.szTip, &tip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
        self.shown = Some((state, tip));
        self.publish_status();
    }

    /// A quiet notification. Windows holds it back during Focus/game mode.
    fn balloon(&self, title: &str, text: &str, warning: bool) {
        let mut nid = self.base_nid();
        nid.uFlags = NIF_INFO;
        wide_into(&mut nid.szInfoTitle, title);
        wide_into(&mut nid.szInfo, text);
        nid.dwInfoFlags = (if warning { NIIF_WARNING } else { NIIF_INFO })
            | NIIF_NOSOUND
            | NIIF_RESPECT_QUIET_TIME;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    // ---------- hotkey ----------

    fn register_hotkey(&mut self, notify: bool) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
        }
        let hk = self.settings.hotkey();
        let ok = unsafe {
            RegisterHotKey(
                Some(self.hwnd),
                HOTKEY_ID,
                HOT_KEY_MODIFIERS(hk.modifiers | MOD_NOREPEAT),
                u32::from(hk.vk),
            )
        };
        self.hotkey_ok = ok.is_ok();
        if self.hotkey_ok {
            info!("hotkey {hk} registered");
        } else {
            warning!("hotkey {hk} is already in use by another program");
            if notify {
                self.balloon(
                    "Save hotkey unavailable",
                    &format!("{hk} is used by another program. Choose another hotkey in settings."),
                    true,
                );
            }
        }
    }

    // ---------- recorder lifecycle ----------

    fn start_recorder(&mut self) {
        if self.recorder.is_some() {
            return;
        }
        let cfg = crate::config::recorder_config(&self.settings);
        match Recorder::start(&cfg) {
            Ok(r) => {
                let i = r.info();
                info!(
                    "recording {}x{}@{} {} {:.1} Mb/s on {}; audio {:?}",
                    i.width,
                    i.height,
                    i.fps,
                    i.encoder.ffmpeg_name(),
                    i.bitrate as f64 / 1e6,
                    i.adapter,
                    i.audio_tracks
                );
                if let Some(e) = &i.audio_error {
                    warning!("audio unavailable: {e}");
                    self.balloon(
                        "Recording without audio",
                        "Audio capture couldn't start. Video is still recorded.",
                        true,
                    );
                }
                self.recorder = Some(r);
                self.status = Status::Recording;
                self.failures = 0;
            }
            Err(e) => {
                let delay = RESTART_BACKOFF_S[self.failures.min(RESTART_BACKOFF_S.len() - 1)];
                self.failures += 1;
                error!("recorder failed to start: {e} (retry in {delay} s)");
                if self.failures == 3 {
                    self.balloon(
                        "JondisClip can't record",
                        &format!("{e}. Retrying automatically."),
                        true,
                    );
                }
                self.schedule_restart(delay, short_reason(&e.to_string()));
            }
        }
        self.refresh();
    }

    fn stop_recorder(&mut self) {
        if let Some(r) = self.recorder.take() {
            match r.stop() {
                Ok(s) => info!(
                    "recorder stopped: encoded {} late {} dropped {} audio dropped {}",
                    s.frames_encoded, s.frames_late, s.packets_dropped, s.audio_dropped
                ),
                Err(e) => warning!("recorder stopped with error: {e}"),
            }
        }
    }

    fn schedule_restart(&mut self, secs: u64, reason: String) {
        self.status = Status::Restarting {
            at: Instant::now() + std::time::Duration::from_secs(secs),
            reason,
        };
        self.refresh();
    }

    fn restart_recorder(&mut self, reason: &str) {
        info!("restarting recorder: {reason}");
        self.stop_recorder();
        self.start_recorder();
    }

    // ---------- periodic supervision ----------

    fn on_tick(&mut self) {
        self.tick += 1;
        // Recorder died on its own (device lost, display change, writer error).
        if self.recorder.as_ref().is_some_and(|r| !r.is_running())
            && let Some(r) = self.recorder.take()
        {
            let reason = match r.stop() {
                Err(e) => e.to_string(),
                Ok(_) => "stopped".into(),
            };
            warning!("recorder stopped unexpectedly: {reason}");
            self.schedule_restart(2, short_reason(&reason));
        }
        if let Status::Restarting { at, .. } = &self.status
            && Instant::now() >= *at
        {
            self.start_recorder();
        }
        if self.tick.is_multiple_of(2) {
            self.check_settings_file();
        }
        if self.tick.is_multiple_of(30) || self.tick == 1 {
            self.check_disk();
        }
        if self.saved_flash_until.is_some_and(|t| Instant::now() >= t) {
            self.saved_flash_until = None;
        }
        self.refresh();
    }

    fn check_disk(&mut self) {
        let save_dir = self.settings.save_dir_or(&self.default_save_dir);
        let free = [free_bytes(&default_buffer_dir()), free_bytes(&save_dir)]
            .into_iter()
            .flatten()
            .min();
        let Some(free) = free else { return };
        match self.status {
            Status::Recording if free < LOW_DISK_BYTES => {
                warning!("low disk space ({} MB free): pausing buffer", free >> 20);
                self.stop_recorder();
                self.status = Status::LowDisk;
                self.balloon(
                    "Buffering paused: low disk space",
                    "Free up some space and JondisClip will resume automatically.",
                    true,
                );
            }
            Status::LowDisk if free > RESUME_DISK_BYTES => {
                info!("disk space recovered: resuming");
                self.start_recorder();
            }
            _ => {}
        }
    }

    fn check_settings_file(&mut self) {
        let mtime = file_mtime(&self.settings_path);
        if mtime == self.settings_mtime {
            return;
        }
        self.settings_mtime = mtime;
        let new = load_settings(&self.settings_path);
        if new != self.settings {
            info!("settings file changed");
            self.apply_settings(new);
        }
    }

    fn apply_settings(&mut self, new: Settings) {
        let old = std::mem::replace(&mut self.settings, new);
        if old.hotkey != self.settings.hotkey {
            self.register_hotkey(true);
        }
        if old.start_with_windows != self.settings.start_with_windows
            && let Err(e) = autostart::apply(self.settings.start_with_windows)
        {
            warning!("start with Windows: {e}");
        }
        if old.needs_restart(&self.settings) {
            if self.recorder.is_some() {
                self.restart_recorder("settings changed");
            }
        } else if old.replay_seconds != self.settings.replay_seconds
            && let Some(r) = self.recorder.as_mut()
            && let Err(e) = r.set_replay(self.settings.replay())
        {
            warning!("replay change: {e}");
        }
        self.refresh();
    }

    fn save_settings(&mut self) {
        if let Err(e) = std::fs::write(&self.settings_path, self.settings.to_toml()) {
            warning!("could not save settings: {e}");
        }
        self.settings_mtime = file_mtime(&self.settings_path);
    }

    // ---------- saving clips ----------

    fn save_clip(&mut self) {
        if self
            .last_save_at
            .is_some_and(|t| t.elapsed().as_millis() < 1000)
        {
            return; // debounce accidental double presses
        }
        self.last_save_at = Some(Instant::now());
        let Some(rec) = self.recorder.as_ref() else {
            if self.settings.save_sound {
                unsafe {
                    let _ = MessageBeep(MB_ICONHAND);
                }
            }
            self.balloon(
                "Clip not saved",
                &format!("JondisClip isn't buffering ({}).", self.status_text()),
                true,
            );
            return;
        };
        let app = naming::foreground_app().unwrap_or_else(|| "Desktop".into());
        let dir = self.settings.save_dir_or(&self.default_save_dir);
        let path = naming::clip_path(&dir, &app, &naming::timestamp());
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            error!("cannot create {}: {e}", parent.display());
            self.balloon(
                "Clip not saved",
                &format!("Can't write to {}: {e}", parent.display()),
                true,
            );
            return;
        }
        info!(
            "saving last {} s from {app} to {}",
            self.settings.replay_seconds,
            path.display()
        );
        let handle = rec.save_clip(self.settings.replay(), path);
        // Instant feedback: the hotkey was heard. The notification follows on completion.
        if self.settings.save_sound {
            unsafe {
                let _ = MessageBeep(MB_OK);
            }
        }
        self.saved_flash_until = Some(Instant::now() + std::time::Duration::from_millis(1500));
        self.refresh();

        let saves = self.saves.clone();
        let hwnd_raw = self.hwnd.0 as isize;
        let seconds = self.settings.replay_seconds;
        let spawned = std::thread::Builder::new()
            .name("jc-save-wait".into())
            .spawn(move || {
                let result = match handle.join() {
                    Ok(Ok(files)) => Ok(files),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("save thread panicked".into()),
                };
                if let Ok(mut q) = saves.lock() {
                    q.push(SaveOutcome {
                        result,
                        app,
                        seconds,
                    });
                }
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd_raw as *mut _)),
                        WM_APP_SAVED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            });
        if let Err(e) = spawned {
            error!("could not start save waiter: {e}");
        }
    }

    fn on_saved(&mut self) {
        let outcomes: Vec<SaveOutcome> = self
            .saves
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default();
        for o in outcomes {
            match o.result {
                Ok(files) => {
                    let name = files
                        .first()
                        .and_then(|f| f.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    info!("saved {:?}", files);
                    self.balloon(
                        &format!("Clip saved · {} · {}", o.app, replay_label(o.seconds)),
                        &name,
                        false,
                    );
                }
                Err(e) => {
                    error!("save failed: {e}");
                    if self.settings.save_sound {
                        unsafe {
                            let _ = MessageBeep(MB_ICONHAND);
                        }
                    }
                    self.saved_flash_until = None;
                    self.balloon("Couldn't save clip", &e, true);
                }
            }
        }
        self.refresh();
    }

    // ---------- menu ----------

    fn build_menu(&mut self) -> HMENU {
        unsafe {
            let menu = CreatePopupMenu().unwrap_or_default();
            let replay = CreatePopupMenu().unwrap_or_default();
            for (i, secs) in REPLAY_PRESETS.iter().enumerate() {
                let flags = if *secs == self.settings.replay_seconds {
                    MF_STRING | MF_CHECKED
                } else {
                    MF_STRING
                };
                let _ = AppendMenuW(
                    replay,
                    flags,
                    (ID_REPLAY_BASE + i as u32) as usize,
                    &menu_text(&replay_label(*secs)),
                );
            }
            let header = format!("JondisClip · {}", self.status_text());
            let _ = AppendMenuW(
                menu,
                MF_STRING | MF_DISABLED | MF_GRAYED,
                0,
                &menu_text(&header),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let save_label = format!(
                "Save last {}\t{}",
                replay_label(self.settings.replay_seconds),
                self.settings.hotkey()
            );
            let save_flags = if self.recorder.is_some() {
                MF_STRING
            } else {
                MF_STRING | MF_GRAYED
            };
            let _ = AppendMenuW(menu, save_flags, ID_SAVE as usize, &menu_text(&save_label));
            let pause_label = if matches!(self.status, Status::Paused) {
                "Resume buffering"
            } else {
                "Pause buffering"
            };
            let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE as usize, &menu_text(pause_label));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(
                menu,
                MF_STRING | MF_POPUP,
                replay.0 as usize,
                &menu_text("Replay length"),
            );
            let mic_flags = if self.settings.microphone {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            let _ = AppendMenuW(
                menu,
                mic_flags,
                ID_MIC as usize,
                &menu_text("Record microphone"),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_OPEN_FOLDER as usize,
                &menu_text("Open clips folder"),
            );
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_EDIT_SETTINGS as usize,
                &menu_text("Settings…"),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_QUIT as usize,
                &menu_text("Quit JondisClip"),
            );
            if self.recorder.is_some() {
                let _ = SetMenuDefaultItem(menu, ID_SAVE, 0);
            }
            menu
        }
    }

    fn on_command(&mut self, cmd: u32) {
        match cmd {
            ID_SAVE => self.save_clip(),
            ID_PAUSE => {
                if matches!(self.status, Status::Paused) {
                    info!("buffering resumed by user");
                    self.start_recorder();
                } else {
                    info!("buffering paused by user");
                    self.stop_recorder();
                    self.status = Status::Paused;
                }
            }
            ID_MIC => {
                self.settings.microphone = !self.settings.microphone;
                info!(
                    "microphone {}",
                    if self.settings.microphone {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.save_settings();
                if self.recorder.is_some() {
                    self.restart_recorder("microphone setting changed");
                }
            }
            ID_OPEN_FOLDER => {
                let dir = self.settings.save_dir_or(&self.default_save_dir);
                let _ = std::fs::create_dir_all(&dir);
                shell_open(&dir.to_string_lossy(), None);
            }
            ID_EDIT_SETTINGS => self.open_settings(),
            ID_QUIT => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            c if (ID_REPLAY_BASE..ID_REPLAY_BASE + REPLAY_PRESETS.len() as u32).contains(&c) => {
                let secs = REPLAY_PRESETS[(c - ID_REPLAY_BASE) as usize];
                let mut new = self.settings.clone();
                new.replay_seconds = secs;
                info!("replay length set to {secs} s");
                self.apply_settings(new);
                self.save_settings();
            }
            _ => {}
        }
        self.refresh();
    }

    /// Writes status.json for the settings window when anything changed.
    fn publish_status(&mut self) {
        let info = self.recorder.as_ref().map(|r| r.info());
        let status = TrayStatus {
            state: match self.status {
                Status::Recording => "recording",
                Status::Paused => "paused",
                Status::Suspended => "sleeping",
                Status::LowDisk => "low_disk",
                Status::Restarting { .. } => "restarting",
            }
            .into(),
            summary: self.status_text(),
            hotkey: self.settings.hotkey().to_string(),
            hotkey_ok: self.hotkey_ok,
            encoder: info.map(|i| i.encoder.ffmpeg_name().to_string()),
            width: info.map(|i| i.width),
            height: info.map(|i| i.height),
            fps: info.map(|i| i.fps),
            bitrate: info.map(|i| i.bitrate),
            audio_tracks: info
                .map(|i| i.audio_tracks.iter().map(|t| t.to_string()).collect())
                .unwrap_or_default(),
            audio_error: info.and_then(|i| i.audio_error.clone()),
        };
        if self.published.as_ref() == Some(&status) {
            return;
        }
        if let Err(e) = status.write(&paths::status_file()) {
            warning!("could not write status: {e}");
        }
        self.published = Some(status);
    }

    /// Starts the settings window (`jondisclip-ui.exe` next to this exe). The
    /// window is its own process: it costs nothing until opened and exits on
    /// close. A second launch just focuses the open window.
    fn open_settings(&mut self) {
        let ui = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("jondisclip-ui.exe")));
        match ui {
            Some(ui) if ui.exists() => {
                // We got foreground rights from the tray click. Pass them on so the
                // window opens in front (or the running one can focus itself).
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
                        windows::Win32::UI::WindowsAndMessaging::ASFW_ANY,
                    );
                }
                if let Err(e) = std::process::Command::new(&ui).spawn() {
                    error!("could not start {}: {e}", ui.display());
                    self.balloon("Couldn't open settings", &e.to_string(), true);
                }
            }
            _ => {
                warning!("jondisclip-ui.exe not found next to the tray app");
                self.balloon(
                    "Settings window missing",
                    "jondisclip-ui.exe wasn't found next to jondisclip.exe.",
                    true,
                );
            }
        }
    }

    fn shutdown(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_ID);
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
            let nid = self.base_nid();
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
        self.stop_recorder();
        let _ = std::fs::remove_file(paths::status_file());
        for i in self.icons {
            if !i.is_invalid() {
                unsafe {
                    let _ = DestroyIcon(i);
                }
            }
        }
    }
}

/// First clause of an error, short enough for a tooltip.
fn short_reason(e: &str) -> String {
    let s = e.split(['\n', ';']).next().unwrap_or(e).trim();
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(59).collect::<String>())
    } else {
        s.to_string()
    }
}
