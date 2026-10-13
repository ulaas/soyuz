//! The About dialog's facts (Phase T7, agent A): version, commit, build, platform,
//! renderer, config folder and memory, and the "Copy info" text, for the GUI's
//! `AboutDialog` and the terminal UI's `TuiDialog::About`.

/// `(label, value)` facts copied by "Copy info" (English, for bug reports). `renderer`:
/// the GUI's wgpu / glow backend, or the terminal UI's colour depth and icons.
pub fn facts(renderer: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Version", env!("CARGO_PKG_VERSION").to_string()),
        ("Commit", crate::GIT_SHA.to_string()),
        ("Build", if cfg!(debug_assertions) { "debug".into() } else { "release".into() }),
        ("Platform", format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH)),
        ("Renderer", renderer.to_string()),
        ("Config", soyuz_fs::strip_verbatim(&soyuz_core::config::config_dir()).display().to_string()),
        ("Memory", memory_line()),
    ]
}

/// Translated label of a [`facts`] entry.
pub fn fact_label(key: &str) -> &'static str {
    match key {
        "Commit" => tr!("dialogs-about-commit"),
        "Build" => tr!("dialogs-about-build"),
        "Platform" => tr!("dialogs-about-platform"),
        "Renderer" => tr!("dialogs-about-renderer"),
        "Config" => tr!("dialogs-about-config"),
        "Memory" => tr!("dialogs-about-memory"),
        _ => tr!("dialogs-about-version"),
    }
}

/// Cache memory: thumbnails and previews (decoded bytes).
fn memory_line() -> String {
    let mib = |b: u64| format!("{} MiB", soyuz_core::locale::format_decimal(b as f64 / (1024.0 * 1024.0), 1));
    let (tb, tn) = crate::services::thumbs::memory();
    let (pb, pn) = crate::services::preview_cache::memory();
    tr!("dialogs-about-memory-line", thumbs = mib(tb), thumb_count = tn, previews = mib(pb), preview_count = pn)
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
        assert_eq!(keys, ["Version", "Commit", "Build", "Platform", "Renderer", "Config", "Memory"]);
        assert_eq!(f[1].1, crate::GIT_SHA);
        assert_eq!(f[4].1, "tui");
        let text = copy_text(&f);
        assert!(text.starts_with("Soyuz\nVersion: "), "{text}");
        assert!(text.contains(&format!("Commit: {}", crate::GIT_SHA)));
        for k in keys {
            assert!(!fact_label(k).is_empty());
        }
    }
}
