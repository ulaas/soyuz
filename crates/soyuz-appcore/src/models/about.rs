//! The About dialog's facts (Phase T7, agent A): version, commit, build, platform,
//! renderer, FFmpeg libraries, config folder, cache memory and the process's memory, and
//! the "Copy info" text, for the GUI's `AboutDialog` and the terminal UI's
//! `TuiDialog::About`.

use std::time::{Duration, Instant};

use parking_lot::Mutex;
use soyuz_fs::process::ProcessMemory;

/// How long a reading of the process's memory is reused. The GUI dialog rebuilds its
/// facts every frame it draws, and repaints this often to keep the line current.
pub const PROCESS_REFRESH: Duration = Duration::from_secs(2);

/// `(label, value)` facts copied by "Copy info" (English, for bug reports). `renderer`:
/// the GUI's wgpu / glow backend, or the terminal UI's colour depth and icons.
pub fn facts(renderer: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Version", env!("CARGO_PKG_VERSION").to_string()),
        ("Commit", crate::GIT_SHA.to_string()),
        ("Build", if cfg!(debug_assertions) { "debug".into() } else { "release".into() }),
        ("Platform", format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH)),
        ("Renderer", renderer.to_string()),
        ("FFmpeg", crate::services::media::about_line()),
        ("Config", soyuz_fs::strip_verbatim(&soyuz_core::config::config_dir()).display().to_string()),
        ("Memory", memory_line()),
        ("Process", process_line()),
    ]
}

/// Translated label of a [`facts`] entry.
pub fn fact_label(key: &str) -> &'static str {
    match key {
        "Commit" => tr!("dialogs-about-commit"),
        "Build" => tr!("dialogs-about-build"),
        "Platform" => tr!("dialogs-about-platform"),
        "Renderer" => tr!("dialogs-about-renderer"),
        "FFmpeg" => tr!("dialogs-about-ffmpeg"),
        "Config" => tr!("dialogs-about-config"),
        "Memory" => tr!("dialogs-about-memory"),
        "Process" => tr!("dialogs-about-process"),
        _ => tr!("dialogs-about-version"),
    }
}

/// The FFmpeg attribution the LGPL asks for ("This software uses libraries from the
/// FFmpeg project under the LGPLv2.1"), shown under the license line.
pub fn ffmpeg_notice() -> &'static str {
    tr!("dialogs-about-ffmpeg-license")
}

/// Cache memory: thumbnails and previews (decoded bytes).
fn memory_line() -> String {
    let (tb, tn) = crate::services::thumbs::memory();
    let (pb, pn) = crate::services::preview_cache::memory();
    tr!("dialogs-about-memory-line", thumbs = mib(tb), thumb_count = tn, previews = mib(pb), preview_count = pn)
}

fn mib(b: u64) -> String {
    format!("{} MiB", soyuz_core::locale::format_decimal(b as f64 / (1024.0 * 1024.0), 1))
}

/// This process's memory: private bytes and working set on Windows, anonymous and
/// resident memory on Linux. One non-blocking query (`K32GetProcessMemoryInfo`, a
/// `/proc/self/status` read), made at most every [`PROCESS_REFRESH`].
fn process_line() -> String {
    static LAST: Mutex<Option<(Instant, Option<ProcessMemory>)>> = Mutex::new(None);
    let mut last = LAST.lock();
    let m = match *last {
        Some((at, m)) if at.elapsed() < PROCESS_REFRESH => m,
        _ => {
            let m = soyuz_fs::process::memory();
            *last = Some((Instant::now(), m));
            m
        }
    };
    match m {
        Some(m) if cfg!(windows) => {
            tr!("dialogs-about-process-windows", private = mib(m.private), working_set = mib(m.resident))
        }
        Some(m) => tr!("dialogs-about-process-unix", anonymous = mib(m.private), resident = mib(m.resident)),
        None => tr!("dialogs-about-process-unknown").to_string(),
    }
}

/// The text "Copy info" puts on the clipboard: `Soyuz` and one `Key: value` line per fact.
pub fn copy_text(facts: &[(&'static str, String)]) -> String {
    let text = facts.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n");
    format!("Soyuz\n{text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_cover_the_build_and_copy_as_lines() {
        let f = facts("tui");
        let keys: Vec<&str> = f.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            keys,
            ["Version", "Commit", "Build", "Platform", "Renderer", "FFmpeg", "Config", "Memory", "Process"]
        );
        if cfg!(any(windows, target_os = "linux")) {
            assert!(f[8].1.contains("MiB"), "{}", f[8].1);
        }
        assert_eq!(f[1].1, crate::GIT_SHA);
        assert_eq!(f[4].1, "tui");
        assert!(!f[5].1.is_empty(), "the FFmpeg line always says something");
        let text = copy_text(&f);
        assert!(text.starts_with("Soyuz\nVersion: "), "{text}");
        assert!(text.contains(&format!("Commit: {}", crate::GIT_SHA)));
        assert!(text.contains("\nFFmpeg: "));
        for k in keys {
            assert!(!fact_label(k).is_empty());
        }
        assert!(ffmpeg_notice().contains("FFmpeg") && ffmpeg_notice().contains("LGPL"));
    }
}
