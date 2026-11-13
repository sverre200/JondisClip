//! Windows notifications (ADR-0023): toasts with the JondisClip name and
//! icon and action buttons, drawn by Windows. They respect Do Not Disturb
//! and cost nothing while a game runs. If Windows refuses them (an old
//! build, or notifications switched off for the app) the caller falls back
//! to the tray balloon.
//!
//! An unpackaged app needs an identity for this: a value under
//! `HKCU\Software\Classes\AppUserModelId\<id>` with the name and icon to
//! show. It's per user and needs no admin rights.
//!
//! Clicks arrive on a Windows thread-pool thread. They're queued and the tray
//! window is woken with a message, so all work stays on the main thread.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{
    NotificationSetting, ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
    ToastNotifier,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegSetValueExW,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{HSTRING, IInspectable, Interface};

/// The app's identity for notifications.
pub const APP_ID: &str = "SverreJ.JondisClip";
const ICON_PNG: &[u8] = include_bytes!("../../../ui/src-tauri/icons/128x128.png");
/// Shown toasts kept alive, so a click from the notification centre still
/// reaches this process.
const KEEP: usize = 24;

/// A button on a toast.
pub struct Action {
    pub label: String,
    /// Handed back by [`Toasts::take`] when it's clicked.
    pub arg: String,
}

pub struct Toast<'a> {
    pub title: &'a str,
    pub text: &'a str,
    /// Handed back when the toast itself is clicked.
    pub launch: &'a str,
    pub actions: Vec<Action>,
    /// A newer toast with the same tag replaces the older one.
    pub tag: Option<&'a str>,
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters aren't allowed in XML 1.0.
            c if c.is_control() && c != '\n' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The toast as Windows' notification XML. Silent: the save sound is the app's own.
pub fn xml(t: &Toast) -> String {
    let mut s = format!(
        r#"<toast launch="{}"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual>"#,
        escape(t.launch),
        escape(t.title),
        escape(t.text)
    );
    if !t.actions.is_empty() {
        s.push_str("<actions>");
        for a in &t.actions {
            s.push_str(&format!(
                r#"<action content="{}" arguments="{}"/>"#,
                escape(&a.label),
                escape(&a.arg)
            ));
        }
        s.push_str("</actions>");
    }
    s.push_str(r#"<audio silent="true"/></toast>"#);
    s
}

fn set_value(key: HKEY, name: &str, value: &str) -> windows::core::Result<()> {
    let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: REG_SZ data is the UTF-16 string including its terminator.
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
    unsafe { RegSetValueExW(key, &HSTRING::from(name), Some(0), REG_SZ, Some(bytes)).ok() }
}

/// Tells Windows the name and icon to show for [`APP_ID`].
fn register(icon: &Path) -> windows::core::Result<()> {
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(format!(r"Software\Classes\AppUserModelId\{APP_ID}")),
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()?;
    }
    let result = set_value(key, "DisplayName", "JondisClip")
        .and_then(|()| set_value(key, "IconUri", &icon.to_string_lossy()));
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

pub struct Toasts {
    notifier: ToastNotifier,
    clicks: Arc<Mutex<Vec<String>>>,
    hwnd: isize,
    message: u32,
    alive: RefCell<VecDeque<ToastNotification>>,
}

impl Toasts {
    /// Registers the app's notification identity (its icon is written to
    /// `data_dir`). Clicks post `message` to `hwnd`; read them with [`take`](Self::take).
    pub fn new(hwnd: HWND, message: u32, data_dir: &Path) -> Result<Self, String> {
        let icon = data_dir.join("notification-icon.png");
        if std::fs::metadata(&icon).map_or(true, |m| m.len() != ICON_PNG.len() as u64) {
            let _ = std::fs::create_dir_all(data_dir);
            std::fs::write(&icon, ICON_PNG).map_err(|e| format!("{}: {e}", icon.display()))?;
        }
        register(&icon).map_err(|e| format!("registering the app id: {e}"))?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
            .map_err(|e| format!("creating the notifier: {e}"))?;
        Ok(Self {
            notifier,
            clicks: Arc::new(Mutex::new(Vec::new())),
            hwnd: hwnd.0 as isize,
            message,
            alive: RefCell::new(VecDeque::new()),
        })
    }

    /// Shows a toast. An error means Windows won't show it (the caller
    /// falls back to a balloon).
    pub fn show(&self, t: &Toast) -> Result<(), String> {
        // Windows drops toasts silently in these cases; say so instead.
        match self.notifier.Setting() {
            Ok(NotificationSetting::Enabled) => {}
            Ok(other) => return Err(format!("notifications are off (setting {})", other.0)),
            Err(e) => return Err(format!("notification setting: {e}")),
        }
        self.build_and_show(t).map_err(|e| e.to_string())
    }

    fn build_and_show(&self, t: &Toast) -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml(t)))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        if let Some(tag) = t.tag {
            toast.SetTag(&HSTRING::from(tag))?;
        }
        let clicks = Arc::clone(&self.clicks);
        let (hwnd, message) = (self.hwnd, self.message);
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, args| {
                let arg = args
                    .as_ref()
                    .and_then(|a| a.cast::<ToastActivatedEventArgs>().ok())
                    .and_then(|a| a.Arguments().ok())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if let Ok(mut q) = clicks.lock() {
                    q.push(arg);
                }
                // SAFETY: posting to a window handle is thread-safe; a stale
                // handle only makes the call fail.
                unsafe {
                    let _ = PostMessageW(Some(HWND(hwnd as *mut _)), message, WPARAM(0), LPARAM(0));
                }
                Ok(())
            },
        ))?;
        self.notifier.Show(&toast)?;
        let mut alive = self.alive.borrow_mut();
        alive.push_back(toast);
        while alive.len() > KEEP {
            alive.pop_front();
        }
        Ok(())
    }

    /// The arguments of toasts and buttons clicked since the last call.
    pub fn take(&self) -> Vec<String> {
        self.clicks.lock().map_or_else(
            |e| std::mem::take(&mut *e.into_inner()),
            |mut q| std::mem::take(&mut *q),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escapes_text_and_lists_actions() {
        let t = Toast {
            title: "Clip saved · R&D <game>",
            text: "Tom's \"best\" clip.mp4",
            launch: "open",
            actions: vec![Action {
                label: "Show in folder".into(),
                arg: r"folder:D:\Clips\A & B\x.mp4".into(),
            }],
            tag: None,
        };
        let x = xml(&t);
        assert!(
            x.contains("<text>Clip saved · R&amp;D &lt;game&gt;</text>"),
            "{x}"
        );
        assert!(
            x.contains("<text>Tom&apos;s &quot;best&quot; clip.mp4</text>"),
            "{x}"
        );
        assert!(
            x.contains(
                r#"<action content="Show in folder" arguments="folder:D:\Clips\A &amp; B\x.mp4"/>"#
            ),
            "{x}"
        );
        assert!(x.starts_with(r#"<toast launch="open">"#) && x.ends_with("</toast>"));
        assert!(
            x.contains(r#"<audio silent="true"/>"#),
            "silent: the app has its own sound"
        );
    }

    /// Shows a real toast for a visual check:
    /// `cargo test -p jondisclip show_a_real_toast -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn show_a_real_toast() {
        let dir = std::env::temp_dir().join("jc-toast-test");
        let toasts = Toasts::new(HWND::default(), 0, &dir).unwrap();
        let shown = toasts.show(&Toast {
            title: "Clip saved · DayZ · 5 min",
            text: "DayZ 12.11.2026 21-03-11.mp4",
            launch: "open",
            actions: vec![
                Action {
                    label: "Open library".into(),
                    arg: "open".into(),
                },
                Action {
                    label: "Show in folder".into(),
                    arg: r"folder:C:\x.mp4".into(),
                },
            ],
            tag: None,
        });
        println!("toast: {shown:?}");
        std::thread::sleep(std::time::Duration::from_secs(6));
        shown.unwrap();
    }

    #[test]
    fn a_toast_without_buttons_has_no_actions_element() {
        let t = Toast {
            title: "a",
            text: "b\u{7}c",
            launch: "open",
            actions: vec![],
            tag: Some("x"),
        };
        let x = xml(&t);
        assert!(!x.contains("<actions>"));
        assert!(
            x.contains("<text>b c</text>"),
            "control characters become spaces: {x}"
        );
    }
}
