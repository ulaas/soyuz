//! Generated reference docs stay in sync with the code.
//!
//! `docs/KEYBOARD.md` + `docs/keyboard.html` come from soyuz-core's default key maps and
//! the English palette strings; `docs/CONFIG.md` from the config schema, its doc comments,
//! `Config::default()` and the comments in `docs/config.example.toml`. When a test here
//! fails, regenerate the files and commit them:
//!
//! ```sh
//! SOYUZ_BLESS_DOCS=1 cargo test -p soyuz-app --test docs_sync
//! ```

mod docs_gen;

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().and_then(|p| p.parent()).unwrap().to_path_buf()
}

/// Compare (or, with `SOYUZ_BLESS_DOCS=1`, write) one generated file. Line endings are
/// ignored so a CRLF checkout does not count as stale.
fn check(rel: &str, fresh: &str) {
    let path = repo_root().join(rel);
    if std::env::var("SOYUZ_BLESS_DOCS").is_ok_and(|v| v == "1") {
        std::fs::write(&path, fresh).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    if committed != fresh {
        let line = committed
            .lines()
            .zip(fresh.lines())
            .position(|(a, b)| a != b)
            .map_or_else(|| committed.lines().count().min(fresh.lines().count()) + 1, |i| i + 1);
        panic!(
            "{rel} is stale (first difference at line {line}). Regenerate it with\n  \
             SOYUZ_BLESS_DOCS=1 cargo test -p soyuz-app --test docs_sync\nand commit the result."
        );
    }
}

#[test]
fn keyboard_reference_is_current() {
    check("docs/KEYBOARD.md", &docs_gen::keyboard::markdown());
    check("docs/keyboard.html", &docs_gen::keyboard::html());
}

#[test]
fn config_reference_is_current() {
    check("docs/CONFIG.md", &docs_gen::config::markdown(&repo_root()));
}
