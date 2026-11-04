//! JondisClip recording engine: capture → GPU conversion → hardware encode →
//! segmented replay buffer → clip finalisation. See `docs/ARCHITECTURE.md`.

pub mod audio;
pub mod audio_buf;
/// Moved to `jondisclip-core` so the settings window's mic check runs the same DSP.
pub use jondisclip_core::audio_dsp;
pub mod capture;
pub mod clip;
pub mod convert;
pub mod encoder;
pub mod gpu;
pub mod hwenc;
pub mod recorder;
pub mod ring;
pub mod segment;
