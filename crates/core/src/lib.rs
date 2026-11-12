//! Shared between the tray app (`jondisclip.exe`) and the settings window
//! (`jondisclip-ui.exe`): settings model and validation, hotkey parsing,
//! well-known paths, and the tray status file the UI reads.

pub mod audio_dsp;
pub mod devices;
pub mod discord;
pub mod drive;
pub mod hotkey;
pub mod i18n;
pub mod library;
pub mod mic_check;
pub mod organize;
pub mod paths;
pub mod settings;
pub mod status;
pub mod update;
