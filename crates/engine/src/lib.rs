//! JondisClip recording engine: capture → GPU conversion → hardware encode →
//! segmented replay buffer → clip finalisation. See `docs/ARCHITECTURE.md`.

pub mod audio;
pub mod audio_buf;
pub mod capture;
pub mod clip;
pub mod convert;
pub mod encoder;
pub mod gpu;
pub mod hwenc;
pub mod recorder;
pub mod ring;
pub mod segment;
