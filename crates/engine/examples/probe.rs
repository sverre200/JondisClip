//! Lists which hardware encoders open on this machine.
//! Run: `cargo run -p jondisclip-engine --example probe`
use jondisclip_engine::encoder::{self, Codec};

fn main() {
    for codec in [Codec::Hevc, Codec::H264, Codec::Av1] {
        for id in encoder::candidates(codec) {
            match encoder::probe(id) {
                Ok(()) => println!("OK    {}", id.ffmpeg_name()),
                Err(e) => println!("--    {e}"),
            }
        }
    }
    match encoder::select(Codec::Hevc) {
        Ok(id) => println!("\nDefault (HEVC preferred): {}", id.ffmpeg_name()),
        Err(_) => println!("\nNo hardware encoder available"),
    }
}
