//! The frontend boundary: `soyuz-appcore` must build without any GUI toolkit, on every
//! target. Checks the resolved normal-dependency graph (`cargo tree`), so a toolkit
//! pulled in through another crate's feature (e.g. `soyuz-term/egui`) is caught too.

use std::process::Command;

/// Crates that belong to a GUI frontend, never to the core.
const BANNED: &[&str] =
    &["egui", "eframe", "epaint", "emath", "ecolor", "winit", "wgpu", "glow", "glutin", "accesskit"];

#[test]
fn core_has_no_gui_toolkit_dependency() {
    let out = Command::new(env!("CARGO"))
        .args(["tree", "--locked", "-p", "soyuz-appcore", "-e", "normal", "--target", "all", "--prefix", "none"])
        .args(["--format", "{p}"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree");
    assert!(out.status.success(), "cargo tree failed: {}", String::from_utf8_lossy(&out.stderr));
    let tree = String::from_utf8_lossy(&out.stdout);
    assert!(tree.lines().any(|l| l.starts_with("soyuz-appcore ")), "unexpected cargo tree output:\n{tree}");
    let offenders: Vec<&str> = tree
        .lines()
        .filter(|l| {
            let name = l.split_whitespace().next().unwrap_or_default();
            BANNED
                .iter()
                .any(|b| name == *b || name.starts_with(&format!("{b}-")) || name.starts_with(&format!("{b}_")))
        })
        .collect();
    assert!(offenders.is_empty(), "soyuz-appcore depends on GUI crates: {offenders:?}");
}
