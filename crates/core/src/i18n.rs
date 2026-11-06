//! Translations for messages the user sees from the Rust side: the tray
//! (menu, tooltip, notifications) and the errors and notes the window shows
//! (ADR-0018). Logs stay in English.
//!
//! Messages are written in English at the call site, `tr!("Moved {name}",
//! name = x)`; the English text is the key into [`TABLE`] (bokmål, nynorsk).
//! Placeholders are `{name}`. A message missing from the table shows in
//! English; a test lists every `tr!` in the workspace and fails on gaps.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[path = "i18n_table.rs"]
mod table;
pub use table::TABLE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Nb,
    Nn,
}

static LANG: AtomicU8 = AtomicU8::new(0);

/// Sets the language for this process ("en", "nb" or "nn"; anything else is English).
pub fn set_language(code: &str) {
    let v = match code {
        "nb" => 1,
        "nn" => 2,
        _ => 0,
    };
    LANG.store(v, Ordering::Relaxed);
}

pub fn language() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Nb,
        2 => Lang::Nn,
        _ => Lang::En,
    }
}

fn index() -> &'static HashMap<&'static str, (&'static str, &'static str)> {
    static I: OnceLock<HashMap<&'static str, (&'static str, &'static str)>> = OnceLock::new();
    I.get_or_init(|| TABLE.iter().map(|(en, nb, nn)| (*en, (*nb, *nn))).collect())
}

/// The message in the current language (the English text if it isn't in the table).
pub fn tr_str(en: &str) -> &str {
    let lang = language();
    if lang == Lang::En {
        return en;
    }
    match index().get(en) {
        Some((nb, _)) if lang == Lang::Nb => nb,
        Some((_, nn)) => nn,
        None => en,
    }
}

/// Translates `$en` and fills `{name}` placeholders: `tr!("Moved {n} clips", n = 3)`.
#[macro_export]
macro_rules! tr {
    ($en:literal) => {
        $crate::i18n::tr_str($en).to_string()
    };
    ($en:literal, $($k:ident = $v:expr),+ $(,)?) => {{
        let mut s = $crate::i18n::tr_str($en).to_string();
        $( s = s.replace(concat!("{", stringify!($k), "}"), &($v).to_string()); )+
        s
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn holes(s: &str) -> Vec<String> {
        let mut v: Vec<String> = s
            .split('{')
            .skip(1)
            .filter_map(|p| p.split_once('}').map(|(k, _)| k.to_string()))
            .filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .collect();
        v.sort();
        v
    }

    fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !matches!(
                    name.as_str(),
                    "target" | "node_modules" | "third_party" | "dist" | ".git"
                ) {
                    rs_files(&p, out);
                }
            } else if name.ends_with(".rs") && name != "i18n_table.rs" && name != "i18n.rs" {
                out.push(p);
            }
        }
    }

    /// Every `tr!("…")` in the workspace, unescaped.
    fn keys() -> Vec<(String, String)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        rs_files(&root.join("crates"), &mut files);
        rs_files(&root.join("ui").join("src-tauri"), &mut files);
        let mut out = Vec::new();
        for f in files {
            let src = std::fs::read_to_string(&f).unwrap_or_default();
            let mut rest = src.as_str();
            while let Some(i) = rest.find("tr!(") {
                rest = &rest[i + 4..];
                let r = rest.trim_start();
                let Some(body) = r.strip_prefix('"') else {
                    continue;
                };
                let mut s = String::new();
                let mut chars = body.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(o) => s.push(o),
                            None => break,
                        },
                        '"' => break,
                        c => s.push(c),
                    }
                }
                out.push((s, f.display().to_string()));
            }
        }
        out
    }

    #[test]
    fn every_message_has_both_translations() {
        let keys = keys();
        assert!(keys.len() > 50, "found only {} tr! calls", keys.len());
        let idx = index();
        let mut missing = Vec::new();
        for (k, f) in &keys {
            match idx.get(k.as_str()) {
                None => missing.push(format!("missing: {k:?} ({f})")),
                Some((nb, nn)) => {
                    if holes(k) != holes(nb) || holes(k) != holes(nn) {
                        missing.push(format!("placeholders differ: {k:?}"));
                    }
                }
            }
        }
        let used: std::collections::HashSet<&str> = keys.iter().map(|(k, _)| k.as_str()).collect();
        for (en, _, _) in TABLE {
            if !used.contains(en) {
                missing.push(format!("unused: {en:?}"));
            }
        }
        assert!(missing.is_empty(), "{}", missing.join("\n"));
    }

    #[test]
    fn falls_back_to_english_and_fills_placeholders() {
        // English only: the language is process-wide and other tests check
        // English messages in parallel.
        assert_eq!(
            crate::tr!("Not in the table {x}", x = 2),
            "Not in the table 2"
        );
        assert_eq!(tr_str("Not in the table either"), "Not in the table either");
    }
}
