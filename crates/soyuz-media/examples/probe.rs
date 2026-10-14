//! Prints where the FFmpeg libraries are looked for, loads them, and dumps the metadata
//! of every file given (a manual test tool; `soyuz-cli media-info` later):
//!
//! ```text
//! SOYUZ_FFMPEG_DIR=<folder> cargo run -p soyuz-media --example probe -- [--json] clip.mp4 song.flac
//! ```
//!
//! `--json` prints one JSON document per file instead of the Rust debug form. Exit code
//! 2 when no runtime loads, 1 when a file fails.

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
    println!(
        "loaded FFmpeg {} ({}) from {}{}",
        v.majors().release().unwrap_or("?"),
        v.majors().describe(),
        if rt.dir().as_os_str().is_empty() { "the loader's path".to_string() } else { rt.dir().display().to_string() },
        if v.gpl { " [GPL build]" } else { "" }
    );
    let mut json = false;
    let mut failed = false;
    for arg in std::env::args().skip(1) {
        if arg == "--json" {
            json = true;
            continue;
        }
        match probe(&rt, &Source::File(PathBuf::from(&arg))) {
            Ok(info) if json => match serde_json::to_string_pretty(&info) {
                Ok(text) => println!("{text}"),
                Err(e) => eprintln!("{arg}: {e}"),
            },
            Ok(info) => println!("{arg}: {info:#?}"),
            Err(e) => {
                eprintln!("{arg}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}
