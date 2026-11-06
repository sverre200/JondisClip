//! Messages follow the language setting (ADR-0018). Its own test binary: the
//! language is process-wide, and the unit tests check English messages.

use jondisclip_core::i18n::set_language;
use jondisclip_core::organize::folder_name;
use jondisclip_core::settings::{Settings, apply_language};
use jondisclip_core::tr;

#[test]
fn messages_follow_the_language_setting() {
    let (s, warnings) = Settings::from_toml("language = \"nb\"\ndate_format = \"dmy\"\n");
    assert!(warnings.is_empty(), "{warnings:?}");
    apply_language(&s);
    assert_eq!(
        tr!(
            "Clip saved · {game} · {length}",
            game = "DayZ",
            length = "5 min"
        ),
        "Klipp lagret · DayZ · 5 min"
    );
    assert_eq!(
        folder_name("").unwrap_err().to_string(),
        "Gi mappen et navn."
    );

    set_language("nn");
    assert_eq!(tr!("Quit JondisClip"), "Avslutt JondisClip");
    assert_eq!(
        folder_name("a?").unwrap_err().to_string(),
        "Mappenamn kan ikkje innehalde ?"
    );

    // Unknown codes fall back to English.
    let (s, warnings) = Settings::from_toml("language = \"de\"\n");
    assert_eq!(s.language, "en");
    assert_eq!(warnings.len(), 1);
    apply_language(&s);
    assert_eq!(tr!("Quit JondisClip"), "Quit JondisClip");
}
