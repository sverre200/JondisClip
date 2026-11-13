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

use jondisclip_core::tr;
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
use crate::toast::{self, Toast};
use jondisclip_core::hotkey::MOD_NOREPEAT;
use jondisclip_core::paths;
use jondisclip_core::settings::{REPLAY_PRESETS, Settings, replay_label};
use jondisclip_core::status::TrayStatus;
use jondisclip_core::update::{self, Release};

pub const WINDOW_CLASS: PCWSTR = w!("JondisClipTray");
const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_SAVED: u32 = WM_APP + 2;
pub const WM_APP_SECOND_INSTANCE: u32 = WM_APP + 3;
/// Posted to quit: the window must be destroyed outside `with_app`, or
/// WM_DESTROY can't borrow the app and `shutdown` is skipped.
const WM_APP_QUIT: u32 = WM_APP + 4;
/// An update worker thread (check or download) has a result in `update_events`.
const WM_APP_UPDATE: u32 = WM_APP + 5;
/// Posted by the settings window's "Install update" button
/// (`jondisclip_core::update::WM_INSTALL_UPDATE`).
const WM_APP_INSTALL_UPDATE: u32 = jondisclip_core::update::WM_INSTALL_UPDATE;
/// Posted by the settings window's "Check now" button.
const WM_APP_CHECK_UPDATE: u32 = jondisclip_core::update::WM_CHECK_UPDATE;
/// A notification or one of its buttons was clicked (see `toast.rs`).
const WM_APP_TOAST: u32 = WM_APP + 8;
const HOTKEY_ID: i32 = 1;
const TIMER_ID: usize = 1;
const TRAY_UID: u32 = 1;

const ID_SAVE: u32 = 100;
const ID_PAUSE: u32 = 101;
const ID_MIC: u32 = 102;
const ID_OPEN_FOLDER: u32 = 103;
const ID_EDIT_SETTINGS: u32 = 104;
const ID_QUIT: u32 = 105;
const ID_LIBRARY: u32 = 106;
const ID_UPDATE: u32 = 107;
const ID_REPLAY_BASE: u32 = 200;

/// Free space below which buffering pauses, and above which it resumes.
const LOW_DISK_BYTES: u64 = 2 << 30;
const RESUME_DISK_BYTES: u64 = 3 << 30;
const RESTART_BACKOFF_S: [u64; 5] = [2, 5, 10, 30, 60];
/// Longest Quit waits for clips still being written (a 5-minute clip took
/// up to 41 s while DayZ was loading the disk).
const QUIT_WAIT_MAX_S: u64 = 180;
/// First update check this long after start, then every `UPDATE_RECHECK_S`.
const UPDATE_FIRST_CHECK_S: u64 = 20;
const UPDATE_RECHECK_S: u64 = 6 * 3600;

/// Where the updater is (ADR-0020). Installing only ever starts from a click.
#[derive(Debug, Clone, PartialEq)]
enum UpdateState {
    Idle,
    Checking,
    Available(Release),
    Downloading(Release),
    /// Downloaded and unpacked: installs as soon as no clip is being written.
    Ready(Release, PathBuf),
}

enum UpdateEvent {
    Checked(Result<Option<Release>, String>),
    Downloaded(Result<PathBuf, String>),
}

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
    /// Saves still writing (their outcome hasn't arrived yet).
    pending_saves: usize,
    /// Quit was chosen while saves were writing: quit once they finish.
    quit_requested: Option<Instant>,
    failures: usize,
    tick: u64,
    taskbar_created: u32,
    update: UpdateState,
    update_events: Arc<Mutex<Vec<UpdateEvent>>>,
    last_update_check: Option<Instant>,
    /// Outcome of the last "Check now": "current" or "failed" (for the window).
    update_check_result: Option<&'static str>,
    /// Modern notifications; `None` when Windows can't provide them.
    toasts: Option<toast::Toasts>,
    /// A toast was refused once (logged a single time).
    toast_failed: std::cell::Cell<bool>,
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
    // No settings file yet: this is the first start on this PC.
    let first_run = !settings_path.exists();
    let settings = load_settings(&settings_path);
    jondisclip_core::settings::apply_language(&settings);
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
    if !crate::menu_theme::allow_dark_menus() {
        info!("dark tray menu not available on this Windows build");
    }

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
        pending_saves: 0,
        quit_requested: None,
        failures: 0,
        tick: 0,
        taskbar_created,
        update: UpdateState::Idle,
        update_events: Arc::new(Mutex::new(Vec::new())),
        last_update_check: None,
        update_check_result: None,
        toasts: match toast::Toasts::new(hwnd, WM_APP_TOAST, &paths::local_dir()) {
            Ok(t) => Some(t),
            Err(e) => {
                warning!("modern notifications unavailable, using balloons: {e}");
                None
            }
        },
        toast_failed: std::cell::Cell::new(false),
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
        if first_run {
            // A new user only sees a tray icon otherwise. The window shows
            // its welcome screen while `setup_done` is false (ADR-0022).
            info!("first run: opening the welcome screen");
            a.open_ui(false);
        }
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
                // Left click / Enter opens the clip library.
                with_app(|a| a.open_ui(false));
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
        WM_APP_UPDATE => {
            with_app(App::on_update_event);
            LRESULT(0)
        }
        WM_APP_INSTALL_UPDATE => {
            with_app(App::install_update);
            LRESULT(0)
        }
        WM_APP_CHECK_UPDATE => {
            with_app(App::check_update_now);
            LRESULT(0)
        }
        WM_APP_TOAST => {
            with_app(App::on_toast);
            LRESULT(0)
        }
        WM_APP_SECOND_INSTANCE => {
            with_app(|a| {
                a.balloon(
                    &tr!("JondisClip is already running"),
                    &tr!("Right-click the tray icon for options."),
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
        WM_APP_QUIT => {
            // Not inside with_app, so WM_DESTROY below can run shutdown.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
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
                    "{}{}",
                    tr!(
                        "Buffering last {length}",
                        length = replay_label(self.settings.replay_seconds)
                    ),
                    detail.unwrap_or_default()
                )
            }
            Status::Paused => tr!("Paused"),
            Status::Suspended => tr!("Sleeping"),
            Status::LowDisk => tr!("Paused: low disk space"),
            Status::Restarting { reason, .. } => {
                tr!("Restarting ({reason})", reason = reason_label(reason))
            }
        }
    }

    /// Updates icon and tooltip if they changed.
    fn refresh(&mut self) {
        let state = self.icon_state();
        let mut tip = format!("JondisClip · {}", self.status_text());
        if !self.hotkey_ok {
            tip.push_str(&format!(" · {}", tr!("hotkey unavailable")));
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

    /// A quiet notification; clicking it opens the library. Windows holds it
    /// back during Do Not Disturb and fullscreen games.
    fn balloon(&self, title: &str, text: &str, warning: bool) {
        self.notify(
            &Toast {
                title,
                text,
                launch: "open",
                actions: Vec::new(),
                tag: None,
            },
            warning,
        );
    }

    /// Shows a toast (ADR-0023), or the old tray balloon when Windows won't.
    fn notify(&self, t: &Toast, warning: bool) {
        if let Some(toasts) = &self.toasts {
            match toasts.show(t) {
                Ok(()) => return,
                Err(e) => {
                    // Once is enough in the log; it's usually a setting.
                    if !self.toast_failed.replace(true) {
                        warning!("toast not shown, using a balloon: {e}");
                    }
                }
            }
        }
        self.legacy_balloon(t.title, t.text, warning);
    }

    /// A notification or one of its buttons was clicked.
    fn on_toast(&mut self) {
        let Some(clicks) = self.toasts.as_ref().map(toast::Toasts::take) else {
            return;
        };
        for arg in clicks {
            info!("notification clicked: {arg}");
            if let Some(path) = arg.strip_prefix("folder:") {
                use std::os::windows::process::CommandExt;
                // Only a clip this app saved: the argument comes from our own toast.
                if let Err(e) = std::process::Command::new("explorer")
                    .raw_arg(format!("/select,\"{}\"", path.replace('"', "")))
                    .spawn()
                {
                    error!("could not open Explorer: {e}");
                }
            } else if arg == "update" {
                self.install_update();
            } else if arg == "settings" {
                self.open_ui(true);
            } else {
                self.open_ui(false);
            }
        }
    }

    fn legacy_balloon(&self, title: &str, text: &str, warning: bool) {
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
                    &tr!("Save hotkey unavailable"),
                    &tr!(
                        "{key} is used by another program. Choose another hotkey in settings.",
                        key = hk
                    ),
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
                        &tr!("Recording without audio"),
                        &tr!("Audio capture couldn't start. Video is still recorded."),
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
                        &tr!("JondisClip can't record"),
                        &tr!("{error}. Retrying automatically.", error = e),
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
        // A save that never reports back mustn't keep the app from quitting.
        if self
            .quit_requested
            .is_some_and(|t| t.elapsed().as_secs() >= QUIT_WAIT_MAX_S)
        {
            warning!("quitting with {} save(s) unfinished", self.pending_saves);
            self.post_quit();
            return;
        }
        // Quitting: no restarts or checks, only collect finished saves (in
        // case a WM_APP_SAVED went missing).
        if self.quit_requested.is_some() {
            self.on_saved();
            return;
        }
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
        self.check_for_update();
        if matches!(self.update, UpdateState::Ready(..)) {
            self.finish_update();
        }
        self.refresh();
    }

    // ---------- updates (ADR-0020) ----------

    fn install_dir() -> Option<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
    }

    /// A copy run from Cargo's build folder (or one we can't locate): it
    /// never replaces itself with a release.
    fn is_dev_build() -> bool {
        Self::install_dir().is_none_or(|d| update::is_dev_install(&d))
    }

    /// Runs `work` on its own thread and wakes the tray with its result.
    fn update_worker(&self, name: &str, work: impl FnOnce() -> UpdateEvent + Send + 'static) {
        let events = self.update_events.clone();
        let hwnd_raw = self.hwnd.0 as isize;
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let event = work();
                events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(event);
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd_raw as *mut _)),
                        WM_APP_UPDATE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            });
        if let Err(e) = spawned {
            error!("could not start {name}: {e}");
        }
    }

    /// Asks GitHub for a newer version shortly after start and then every few
    /// hours, unless the setting is off or this is a developer build.
    fn check_for_update(&mut self) {
        if self.update != UpdateState::Idle || !self.settings.check_for_updates {
            return;
        }
        let due = match self.last_update_check {
            None => self.tick >= UPDATE_FIRST_CHECK_S,
            Some(t) => t.elapsed().as_secs() >= UPDATE_RECHECK_S,
        };
        if !due {
            return;
        }
        self.last_update_check = Some(Instant::now());
        if Self::is_dev_build() {
            return;
        }
        self.start_update_check();
    }

    /// "Check now" in the settings window: checks at once, whatever the
    /// automatic-check setting says, and reports the outcome in status.json.
    fn check_update_now(&mut self) {
        if self.update != UpdateState::Idle {
            // Already checking, or an update is on offer: nothing new to learn.
            return;
        }
        info!("update check requested");
        self.last_update_check = Some(Instant::now());
        self.start_update_check();
        self.refresh();
    }

    fn start_update_check(&mut self) {
        self.update_check_result = None;
        self.update = UpdateState::Checking;
        self.update_worker("jc-update-check", || {
            UpdateEvent::Checked(update::check(env!("CARGO_PKG_VERSION")))
        });
    }

    fn on_update_event(&mut self) {
        let events: Vec<UpdateEvent> = self.update_events.lock().map_or_else(
            |e| std::mem::take(&mut *e.into_inner()),
            |mut q| std::mem::take(&mut *q),
        );
        for event in events {
            match (event, self.update.clone()) {
                (UpdateEvent::Checked(Ok(Some(release))), UpdateState::Checking) => {
                    info!("update available: {}", release.version);
                    let dev = Self::is_dev_build();
                    self.notify(
                        &Toast {
                            title: &tr!(
                                "Update available: JondisClip {version}",
                                version = release.version
                            ),
                            text: &if dev {
                                tr!(
                                    "This copy is a developer build, so it doesn't update itself. Rebuild it to get the new version."
                                )
                            } else {
                                tr!(
                                    "Install it here, from the tray menu or in Settings. Recording continues while it downloads."
                                )
                            },
                            launch: "settings",
                            actions: if dev {
                                Vec::new()
                            } else {
                                vec![toast::Action {
                                    label: tr!("Install update"),
                                    arg: "update".into(),
                                }]
                            },
                            tag: Some("update"),
                        },
                        false,
                    );
                    self.update = UpdateState::Available(release);
                }
                (UpdateEvent::Checked(Ok(None)), UpdateState::Checking) => {
                    self.update_check_result = Some("current");
                    self.update = UpdateState::Idle;
                }
                (UpdateEvent::Checked(Err(e)), UpdateState::Checking) => {
                    // Offline or GitHub unreachable: not worth a notification.
                    warning!("update check failed: {e}");
                    self.update_check_result = Some("failed");
                    self.update = UpdateState::Idle;
                }
                (UpdateEvent::Downloaded(Ok(staged)), UpdateState::Downloading(release)) => {
                    info!("update {} downloaded", release.version);
                    self.update = UpdateState::Ready(release, staged);
                    self.finish_update();
                }
                (UpdateEvent::Downloaded(Err(e)), UpdateState::Downloading(release)) => {
                    error!("update download failed: {e}");
                    self.balloon(&tr!("Couldn't update JondisClip"), &e, true);
                    self.update = UpdateState::Available(release);
                }
                _ => {}
            }
        }
        self.refresh();
    }

    /// The user chose "Install update" (tray menu or settings window).
    /// Recording continues while the new version downloads.
    fn install_update(&mut self) {
        let UpdateState::Available(release) = self.update.clone() else {
            return;
        };
        if Self::is_dev_build() {
            warning!("not installing an update over a developer build");
            return;
        }
        info!("downloading update {}", release.version);
        self.balloon(
            &tr!(
                "Downloading JondisClip {version}",
                version = release.version
            ),
            &tr!(
                "It installs and restarts by itself when it's ready. Recording continues meanwhile."
            ),
            false,
        );
        let wanted = release.clone();
        self.update = UpdateState::Downloading(release);
        self.update_worker("jc-update-download", move || {
            UpdateEvent::Downloaded(update::download(&wanted, &paths::update_dir()))
        });
        self.refresh();
    }

    /// Swaps in the downloaded files and restarts, once no clip is being
    /// written (non-negotiable 2). If the swap fails the old version carries on.
    fn finish_update(&mut self) {
        let UpdateState::Ready(release, staged) = self.update.clone() else {
            return;
        };
        if self.pending_saves > 0 || self.quit_requested.is_some() {
            return;
        }
        let Some(dir) = Self::install_dir() else {
            return;
        };
        let was_recording = self.recorder.is_some();
        self.stop_recorder();
        let started = update::apply(&staged, &dir).and_then(|()| {
            std::process::Command::new(dir.join("jondisclip.exe"))
                .arg("--updated")
                .spawn()
                .map_err(|e| format!("could not start the new version: {e}"))
        });
        match started {
            Ok(_) => {
                info!("update {} installed: restarting", release.version);
                self.status = Status::Paused;
                self.update = UpdateState::Idle;
                self.post_quit();
            }
            Err(e) => {
                error!("update {} failed: {e}", release.version);
                self.balloon(&tr!("Couldn't update JondisClip"), &e, true);
                self.update = UpdateState::Available(release);
                if was_recording {
                    self.start_recorder();
                }
            }
        }
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
                    &tr!("Buffering paused: low disk space"),
                    &tr!("Free up some space and JondisClip will resume automatically."),
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
        jondisclip_core::settings::apply_language(&new);
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
        } else {
            if old.replay_seconds != self.settings.replay_seconds
                && let Some(r) = self.recorder.as_mut()
                && let Err(e) = r.set_replay(self.settings.replay())
            {
                warning!("replay change: {e}");
            }
            if old.microphone_gain_db != self.settings.microphone_gain_db
                && let Some(r) = self.recorder.as_ref()
            {
                info!(
                    "microphone gain {:+.1} dB",
                    self.settings.microphone_gain_db
                );
                r.set_mic_gain_db(self.settings.microphone_gain_db);
            }
            if old.microphone_low_cut != self.settings.microphone_low_cut
                && let Some(r) = self.recorder.as_ref()
            {
                info!(
                    "microphone low-cut {}",
                    if self.settings.microphone_low_cut {
                        "on"
                    } else {
                        "off"
                    }
                );
                r.set_mic_low_cut(self.settings.microphone_low_cut);
            }
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
        if self.quit_requested.is_some() {
            self.balloon(
                &tr!("Clip not saved"),
                &tr!("JondisClip is quitting after the clip it's still saving."),
                true,
            );
            return;
        }
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
                &tr!("Clip not saved"),
                &tr!(
                    "JondisClip isn't buffering ({status}).",
                    status = self.status_text()
                ),
                true,
            );
            return;
        };
        let app = naming::foreground_app().unwrap_or_else(|| "Desktop".into());
        let dir = self.settings.save_dir_or(&self.default_save_dir);
        let path = naming::clip_path(
            &dir,
            &app,
            &naming::file_timestamp(&self.settings.date_format),
        );
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            error!("cannot create {}: {e}", parent.display());
            self.balloon(
                &tr!("Clip not saved"),
                &tr!(
                    "Can't write to {path}: {error}",
                    path = parent.display(),
                    error = e
                ),
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
                saves
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(SaveOutcome {
                        result,
                        app,
                        seconds,
                    });
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd_raw as *mut _)),
                        WM_APP_SAVED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            });
        match spawned {
            Ok(_) => self.pending_saves += 1,
            Err(e) => error!("could not start save waiter: {e}"),
        }
    }

    /// Quits now, or once the clips still being written are finished: the
    /// save threads die with the process, which would lose those clips.
    fn request_quit(&mut self) {
        // A second Quit while waiting means "now".
        if self.pending_saves == 0 || self.quit_requested.is_some() {
            self.post_quit();
            return;
        }
        info!(
            "quit requested: waiting for {} save(s) to finish",
            self.pending_saves
        );
        self.quit_requested = Some(Instant::now());
        // Stop recording now. Pinned segments stay until their save is done.
        self.stop_recorder();
        self.status = Status::Paused;
        self.refresh();
        self.balloon(
            &tr!("Finishing your clip"),
            &tr!("JondisClip quits as soon as the clip is saved."),
            false,
        );
    }

    fn on_saved(&mut self) {
        let outcomes: Vec<SaveOutcome> = self.saves.lock().map_or_else(
            |e| std::mem::take(&mut *e.into_inner()),
            |mut q| std::mem::take(&mut *q),
        );
        self.pending_saves = self.pending_saves.saturating_sub(outcomes.len());
        for o in outcomes {
            match o.result {
                Ok(files) => {
                    let name = files
                        .first()
                        .and_then(|f| f.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    info!("saved {:?}", files);
                    let mut actions = vec![toast::Action {
                        label: tr!("Open library"),
                        arg: "open".into(),
                    }];
                    if let Some(f) = files.first() {
                        actions.push(toast::Action {
                            label: tr!("Show in folder"),
                            arg: format!("folder:{}", f.display()),
                        });
                    }
                    self.notify(
                        &Toast {
                            title: &tr!(
                                "Clip saved · {game} · {length}",
                                game = o.app,
                                length = replay_label(o.seconds)
                            ),
                            text: &name,
                            launch: "open",
                            actions,
                            tag: None,
                        },
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
                    self.balloon(&tr!("Couldn't save clip"), &e, true);
                }
            }
        }
        if self.quit_requested.is_some() && self.pending_saves == 0 {
            self.post_quit();
            return;
        }
        self.refresh();
    }

    fn post_quit(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_APP_QUIT, WPARAM(0), LPARAM(0));
        }
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
            match &self.update {
                UpdateState::Available(r) if Self::is_dev_build() => {
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING | MF_DISABLED | MF_GRAYED,
                        0,
                        &menu_text(&tr!(
                            "Version {version} is out (developer build: rebuild to update)",
                            version = r.version
                        )),
                    );
                }
                UpdateState::Available(r) => {
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING,
                        ID_UPDATE as usize,
                        &menu_text(&tr!("Install update {version}", version = r.version)),
                    );
                }
                UpdateState::Downloading(_) | UpdateState::Ready(..) => {
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING | MF_DISABLED | MF_GRAYED,
                        0,
                        &menu_text(&tr!("Downloading update…")),
                    );
                }
                UpdateState::Idle | UpdateState::Checking => {}
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let save_label = format!(
                "{}\t{}",
                tr!(
                    "Save last {length}",
                    length = replay_label(self.settings.replay_seconds)
                ),
                self.settings.hotkey()
            );
            let save_flags = if self.recorder.is_some() {
                MF_STRING
            } else {
                MF_STRING | MF_GRAYED
            };
            let _ = AppendMenuW(menu, save_flags, ID_SAVE as usize, &menu_text(&save_label));
            let pause_label = if matches!(self.status, Status::Paused) {
                tr!("Resume buffering")
            } else {
                tr!("Pause buffering")
            };
            let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE as usize, &menu_text(&pause_label));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(
                menu,
                MF_STRING | MF_POPUP,
                replay.0 as usize,
                &menu_text(&tr!("Replay length")),
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
                &menu_text(&tr!("Record microphone")),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_LIBRARY as usize,
                &menu_text(&tr!("Clip library")),
            );
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_OPEN_FOLDER as usize,
                &menu_text(&tr!("Open clips folder")),
            );
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_EDIT_SETTINGS as usize,
                &menu_text(&tr!("Settings…")),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_QUIT as usize,
                &menu_text(&tr!("Quit JondisClip")),
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
            ID_EDIT_SETTINGS => self.open_ui(true),
            ID_LIBRARY => self.open_ui(false),
            ID_UPDATE => self.install_update(),
            ID_QUIT => {
                self.request_quit();
                return;
            }
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
            mic_connected: self.recorder.as_ref().and_then(|r| r.stats().mic_connected),
            version: env!("CARGO_PKG_VERSION").into(),
            update_version: match &self.update {
                UpdateState::Available(r)
                | UpdateState::Downloading(r)
                | UpdateState::Ready(r, _) => Some(r.version.clone()),
                UpdateState::Idle | UpdateState::Checking => None,
            },
            update_state: match &self.update {
                UpdateState::Downloading(_) => Some("downloading".into()),
                UpdateState::Ready(..) => Some("installing".into()),
                _ => None,
            },
            update_progress: match &self.update {
                UpdateState::Downloading(r) => update::download_progress(r, &paths::update_dir()),
                _ => None,
            },
            dev_build: Self::is_dev_build(),
            update_check: match &self.update {
                UpdateState::Checking => Some("checking".into()),
                UpdateState::Idle => self.update_check_result.map(Into::into),
                _ => None,
            },
        };
        if self.published.as_ref() == Some(&status) {
            return;
        }
        if let Err(e) = status.write(&paths::status_file()) {
            warning!("could not write status: {e}");
        }
        self.published = Some(status);
    }

    /// Starts the library/settings window (`jondisclip-ui.exe` next to this
    /// exe), on the settings page if `settings`. The window is its own process:
    /// it costs nothing until opened and exits on close. A second launch just
    /// focuses the open window.
    fn open_ui(&mut self, settings: bool) {
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
                let mut cmd = std::process::Command::new(&ui);
                if settings {
                    cmd.arg("--settings");
                }
                if let Err(e) = cmd.spawn() {
                    error!("could not start {}: {e}", ui.display());
                    self.balloon(&tr!("Couldn't open JondisClip"), &e.to_string(), true);
                }
            }
            _ => {
                warning!("jondisclip-ui.exe not found next to the tray app");
                self.balloon(
                    &tr!("JondisClip window missing"),
                    &tr!("jondisclip-ui.exe wasn't found next to jondisclip.exe."),
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
/// A restart reason for the tooltip and menu. The known ones are translated;
/// errors from the recorder stay as they are (they're also in the log).
fn reason_label(r: &str) -> String {
    match r {
        "resuming from sleep" => tr!("resuming from sleep"),
        "display changed" => tr!("display changed"),
        "settings changed" => tr!("settings changed"),
        "microphone setting changed" => tr!("microphone setting changed"),
        other => other.to_string(),
    }
}

fn short_reason(e: &str) -> String {
    let s = e.split(['\n', ';']).next().unwrap_or(e).trim();
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(59).collect::<String>())
    } else {
        s.to_string()
    }
}
