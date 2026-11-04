//! Concatenates TS segments into an MP4 with the engine's clip code (debugging aid).
//! `cargo run -p jondisclip-engine --example concat -- out.mp4 seg1.ts seg2.ts ...`
fn main() {
    let args: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();
    let Some((out, inputs)) = args.split_first() else {
        eprintln!("usage: concat <out.mp4> <segments...>");
        std::process::exit(2);
    };
    match jondisclip_engine::clip::concat_to_mp4(inputs, out, &["Game"]) {
        Ok(()) => println!("ok: {}", out.display()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
