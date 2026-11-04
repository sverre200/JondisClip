//! Shared between the tray app (`jondisclip.exe`) and the settings window
//! (`jondisclip-ui.exe`): settings model and validation, hotkey parsing,
//! well-known paths, and the tray status file the UI reads.

pub mod devices;
pub mod hotkey;
pub mod paths;
pub mod settings;
pub mod status;
