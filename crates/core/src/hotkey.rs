//! Global hotkey parsing ("Alt+F10") for `RegisterHotKey`.
//!
//! `RegisterHotKey` swallows the key system-wide, so a bare letter or digit
//! would stop you typing it in games and chat. Those need a modifier. Only
//! F-keys and a few navigation keys may be used alone.

use std::fmt;

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;
pub const MOD_NOREPEAT: u32 = 0x4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    /// MOD_* flags (without MOD_NOREPEAT).
    pub modifiers: u32,
    pub vk: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    Empty,
    UnknownKey(String),
    NoKey,
    TwoKeys,
    NeedsModifier(String),
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str(&crate::tr!("hotkey is empty")),
            Self::UnknownKey(k) => f.write_str(&crate::tr!("unknown key \"{key}\"", key = k)),
            Self::NoKey => f.write_str(&crate::tr!("hotkey has modifiers but no key")),
            Self::TwoKeys => f.write_str(&crate::tr!("hotkey has more than one key")),
            Self::NeedsModifier(k) => f.write_str(&crate::tr!(
                "\"{key}\" needs Ctrl, Alt, Shift or Win",
                key = k
            )),
        }
    }
}

impl std::error::Error for HotkeyError {}

/// (name, virtual-key code, allowed without a modifier)
fn key_table() -> Vec<(String, u16, bool)> {
    let mut t: Vec<(String, u16, bool)> = Vec::new();
    for i in 1..=24u16 {
        t.push((format!("F{i}"), 0x6F + i, true)); // VK_F1 = 0x70
    }
    for c in b'A'..=b'Z' {
        t.push(((c as char).to_string(), u16::from(c), false));
    }
    for c in b'0'..=b'9' {
        t.push(((c as char).to_string(), u16::from(c), false));
    }
    for i in 0..=9u16 {
        t.push((format!("Numpad{i}"), 0x60 + i, false));
    }
    for (n, vk, alone) in [
        ("PrintScreen", 0x2C, true),
        ("Pause", 0x13, true),
        ("ScrollLock", 0x91, true),
        ("Insert", 0x2D, true),
        ("Home", 0x24, false),
        ("End", 0x23, false),
        ("PageUp", 0x21, false),
        ("PageDown", 0x22, false),
        ("Space", 0x20, false),
    ] {
        t.push((n.to_string(), vk, alone));
    }
    t
}

fn alias(name: &str) -> &str {
    match name {
        "prtsc" | "prtscn" | "printscr" => "printscreen",
        "ins" => "insert",
        "pgup" => "pageup",
        "pgdn" => "pagedown",
        _ => name,
    }
}

impl Hotkey {
    pub fn parse(s: &str) -> Result<Self, HotkeyError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(HotkeyError::Empty);
        }
        let table = key_table();
        let mut modifiers = 0;
        let mut key: Option<(u16, bool, String)> = None;
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            let lower = part.to_ascii_lowercase();
            let m = match lower.as_str() {
                "ctrl" | "control" => MOD_CONTROL,
                "alt" => MOD_ALT,
                "shift" => MOD_SHIFT,
                "win" | "windows" | "super" => MOD_WIN,
                _ => 0,
            };
            if m != 0 {
                modifiers |= m;
                continue;
            }
            let name = alias(&lower);
            let Some((canon, vk, alone)) =
                table.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(name))
            else {
                return Err(HotkeyError::UnknownKey(part.to_string()));
            };
            if key.is_some() {
                return Err(HotkeyError::TwoKeys);
            }
            key = Some((*vk, *alone, canon.clone()));
        }
        let (vk, alone, canon) = key.ok_or(HotkeyError::NoKey)?;
        if modifiers == 0 && !alone {
            return Err(HotkeyError::NeedsModifier(canon));
        }
        Ok(Self { modifiers, vk })
    }

    fn key_name(&self) -> String {
        key_table()
            .into_iter()
            .find(|(_, vk, _)| *vk == self.vk)
            .map(|(n, _, _)| n)
            .unwrap_or_else(|| format!("VK{:#04x}", self.vk))
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (flag, name) in [
            (MOD_CONTROL, "Ctrl"),
            (MOD_ALT, "Alt"),
            (MOD_SHIFT, "Shift"),
            (MOD_WIN, "Win"),
        ] {
            if self.modifiers & flag != 0 {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_combos() {
        assert_eq!(
            Hotkey::parse("Alt+F10"),
            Ok(Hotkey {
                modifiers: MOD_ALT,
                vk: 0x79
            })
        );
        assert_eq!(
            Hotkey::parse(" ctrl + shift + s "),
            Ok(Hotkey {
                modifiers: MOD_CONTROL | MOD_SHIFT,
                vk: b'S' as u16
            })
        );
        assert_eq!(
            Hotkey::parse("F8"),
            Ok(Hotkey {
                modifiers: 0,
                vk: 0x77
            })
        );
        assert_eq!(
            Hotkey::parse("PrtSc"),
            Ok(Hotkey {
                modifiers: 0,
                vk: 0x2C
            })
        );
        assert_eq!(Hotkey::parse("Win+Numpad5").map(|h| h.vk), Ok(0x65));
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(Hotkey::parse(""), Err(HotkeyError::Empty));
        assert_eq!(Hotkey::parse("Ctrl+Alt"), Err(HotkeyError::NoKey));
        assert_eq!(Hotkey::parse("Alt+F1+F2"), Err(HotkeyError::TwoKeys));
        assert_eq!(
            Hotkey::parse("Alt+Banana"),
            Err(HotkeyError::UnknownKey("Banana".into()))
        );
        assert_eq!(
            Hotkey::parse("S"),
            Err(HotkeyError::NeedsModifier("S".into()))
        );
        assert_eq!(
            Hotkey::parse("Space"),
            Err(HotkeyError::NeedsModifier("Space".into()))
        );
    }

    #[test]
    fn display_is_canonical_and_round_trips() {
        for s in ["Alt+F10", "Ctrl+Shift+S", "F8", "Ctrl+Alt+Win+PrintScreen"] {
            let h = Hotkey::parse(s).unwrap();
            assert_eq!(h.to_string(), s);
            assert_eq!(Hotkey::parse(&h.to_string()), Ok(h));
        }
        assert_eq!(
            Hotkey::parse("shift+alt+f10").unwrap().to_string(),
            "Alt+Shift+F10"
        );
    }
}
