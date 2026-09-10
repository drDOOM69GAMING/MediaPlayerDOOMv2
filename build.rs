use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let tools = Path::new(&manifest).join("tools");
    let mut out = String::new();
    for (name, const_name) in [
        ("ffmpeg.exe", "EMBEDDED_FFMPEG"),
        ("ffprobe.exe", "EMBEDDED_FFPROBE"),
        ("yt-dlp.exe", "EMBEDDED_YTDLP"),
    ] {
        let p = tools.join(name);
        println!("cargo:rerun-if-changed={}", p.display());
        if p.is_file() {
            let lit = p.to_string_lossy().replace('\\', "/");
            out.push_str(&format!(
                "pub const {}: Option<&'static [u8]> = Some(include_bytes!(\"{}\"));\n",
                const_name, lit
            ));
        } else {
            out.push_str(&format!(
                "pub const {}: Option<&'static [u8]> = None;\n",
                const_name
            ));
        }
    }
    let dst = Path::new(&std::env::var("OUT_DIR").unwrap()).join("embedded_tools.rs");
    std::fs::write(dst, out).unwrap();
}