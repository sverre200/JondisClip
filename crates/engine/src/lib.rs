//! JondisClip recording engine: capture → GPU conversion → hardware encode →
//! segmented replay buffer → clip finalisation. See `docs/ARCHITECTURE.md`.

pub mod encoder;
pub mod ring;
