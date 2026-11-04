//! Copies the FFmpeg runtime DLLs next to built binaries and test executables
//! so `cargo run` / `cargo test` work without touching the global PATH.
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    let Ok(ffmpeg_dir) = env::var("FFMPEG_DIR") else {
        return;
    };
    let bin = PathBuf::from(ffmpeg_dir).join("bin");
    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let Some(profile_dir) = out.ancestors().nth(3) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&bin) else {
        println!("cargo:warning=FFmpeg bin dir not found: {}", bin.display());
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
        {
            for dest in [
                profile_dir.to_path_buf(),
                profile_dir.join("deps"),
                profile_dir.join("examples"),
            ] {
                let _ = fs::create_dir_all(&dest);
                let target = dest.join(entry.file_name());
                if !target.exists() {
                    let _ = fs::copy(&path, &target);
                }
            }
        }
    }
}
