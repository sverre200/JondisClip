//! Runs the real update steps against GitHub without touching an install:
//! check, download, unpack, and swap into a scratch copy.
//! Run: `cargo run -p jondisclip-core --example update_dry_run -- [current-version]`
//! (default 0.0.0, so any published release counts as newer).
use jondisclip_core::update;

fn main() {
    let current = std::env::args().nth(1).unwrap_or_else(|| "0.0.0".into());
    let root = std::env::temp_dir().join(format!("jc-update-dry-run-{}", std::process::id()));
    let (work, install) = (root.join("work"), root.join("install"));
    println!("current version: {current}");
    let release = match update::check(&current) {
        Ok(Some(r)) => r,
        Ok(None) => return println!("CHECK OK: up to date"),
        Err(e) => return println!("CHECK FAILED: {e}"),
    };
    println!("CHECK OK: {} at {}", release.version, release.zip_url);
    let staged = match update::download(&release, &work) {
        Ok(d) => d,
        Err(e) => return println!("DOWNLOAD FAILED: {e}"),
    };
    println!("DOWNLOAD OK: {}", staged.display());
    // An "old install" with one file that must be replaced and one that must stay.
    std::fs::create_dir_all(&install).expect("install dir");
    std::fs::write(install.join("jondisclip.exe"), b"old").expect("old exe");
    std::fs::write(install.join("mine.txt"), b"keep").expect("own file");
    if let Err(e) = update::apply(&staged, &install) {
        return println!("APPLY FAILED: {e}");
    }
    let mut names: Vec<String> = std::fs::read_dir(&install)
        .expect("read install")
        .flatten()
        .map(|e| {
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            format!("{} ({len} bytes)", e.file_name().to_string_lossy())
        })
        .collect();
    names.sort();
    println!("APPLY OK:\n  {}", names.join("\n  "));
    update::cleanup(&install, &work);
    let left = std::fs::read_dir(&install)
        .expect("read install")
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "old"))
        .count();
    println!(
        "CLEANUP OK: {left} .old files left, download folder gone: {}",
        !work.exists()
    );
    let _ = std::fs::remove_dir_all(root);
}
