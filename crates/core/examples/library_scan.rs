//! Lists the library the way the window sees it (headers only, read-only).
//! `cargo run -p jondisclip-core --example library_scan [dir]`
use jondisclip_core::library::{Cache, probe_file, scan};

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(Into::into)
        .unwrap_or_else(jondisclip_core::paths::default_save_dir);
    let t = std::time::Instant::now();
    let cache = Cache::new(std::env::temp_dir().join("jc-library-scan"));
    let s = scan(&root, &cache, &|secs| format!("@{secs}"));
    println!(
        "{} clips in {:?} ({} probed)",
        s.clips.len(),
        t.elapsed(),
        s.probed
    );
    for c in &s.clips {
        println!(
            "{:<10} {}  {:>6.1} MB  {:?}  {:?}",
            c.game,
            c.recorded,
            c.size as f64 / 1e6,
            c.probe,
            c.error
        );
    }
    if let Some(c) = s.clips.first() {
        let t = std::time::Instant::now();
        let _ = probe_file(&c.path);
        println!("one header read: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let k = jondisclip_core::library::keyframes_file(&c.path);
        println!(
            "keyframes in {:?}: {:?}",
            t.elapsed(),
            k.map(|k| (
                k.len(),
                k.iter().take(4).copied().collect::<Vec<_>>(),
                k.last().copied()
            ))
        );
    }
}
