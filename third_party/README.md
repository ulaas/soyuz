# Vendored crates

## egui-phosphor 0.14.0 (MIT OR Apache-2.0, https://github.com/amPerl/egui-phosphor)

The crates.io source, trimmed to what Soyuz uses, with its `egui` dependency moved from 0.36 to
0.37 (no egui-phosphor release supports egui 0.37 yet). Kept: `src/lib.rs`, the `regular` and
`fill` variants (`src/variants/`, `res/Phosphor.ttf`, `res/Phosphor-Fill.ttf`) and the licenses.
Dropped: the thin / light / bold fonts, the `subset!` macro, examples and tests. The code is
otherwise unchanged.

Wired in through `[patch.crates-io]` in the workspace `Cargo.toml`. Delete this directory and
the patch entry once a crates.io egui-phosphor depends on egui 0.37.
