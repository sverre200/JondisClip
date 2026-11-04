//! Lists the library the way the window sees it (headers only, read-only).
//! `cargo run -p jondisclip-core --example library_scan [dir]`
use jondisclip_core::library::{Cache, probe_file, scan};

fn main() {
    if let Some(f) = std::env::args()
        .nth(2)
        .filter(|_| std::env::args().nth(1).as_deref() == Some("--kf"))
    {
        let k = jondisclip_core::library::keyframes_file(std::path::Path::new(&f)).unwrap();
        println!(
            "{} keyframes; 115..121: {:?}",
            k.len(),
            &k[115.min(k.len())..121.min(k.len())]
        );
        return;
    }
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
                k.iter().skip(115).take(6).copied().collect::<Vec<_>>(),
                k.last().copied()
            ))
        );
    }
}
