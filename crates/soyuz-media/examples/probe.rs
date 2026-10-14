//! Prints where the FFmpeg libraries are looked for, loads them, and dumps the metadata
//! of every file given (a manual test tool; `soyuz-cli media-info` later):
//!
//! ```text
//! SOYUZ_FFMPEG_DIR=<folder> cargo run -p soyuz-media --example probe -- clip.mp4 song.flac
//! ```
//!
//! Exit code 2 when no runtime loads (the V0 skeleton always: the loader lands in V1a).

use std::path::PathBuf;

use soyuz_media::{LOADER_PATH, Majors, Runtime, Source, lib_file_names, probe, search_dirs};

fn main() {
    let config_dir = std::env::var_os("SOYUZ_CONFIG_DIR").map(PathBuf::from).unwrap_or_default();
    let dirs = search_dirs("", &config_dir);
    for d in &dirs {
        let shown = if d.as_os_str() == LOADER_PATH {
            "(the loader's search path)".to_string()
        } else {
            d.display().to_string()
        };
        println!("search: {shown}");
    }
    println!("looking for: {}", lib_file_names(Majors::BUNDLED).join(", "));
    let rt = match Runtime::get(&dirs) {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let v = rt.versions();
    println!("loaded {} from {}", v.majors().describe(), rt.dir().display());
    for arg in std::env::args().skip(1) {
        match probe(&rt, &Source::File(PathBuf::from(&arg))) {
            Ok(info) => println!("{arg}: {info:#?}"),
            Err(e) => eprintln!("{arg}: {e}"),
        }
    }
}
