# Vendored crates

## accesskit_unix 0.21.1 (MIT OR Apache-2.0, https://github.com/AccessKit/accesskit)

Unmodified crates.io source except one line in `src/context.rs`: it watches the AT-SPI
`IsEnabled` status flag instead of `ScreenReaderEnabled`. at-spi2-core 2.62 (Fedora 45 and
newer distros) removed `ScreenReaderEnabled`, so the stock 0.21.1 never activates and Orca
sees nothing (see docs/ACCESSIBILITY.md). Upstream fixed this in accesskit_unix 0.23, which
needs accesskit 0.25; egui-winit 0.36 pins accesskit_winit 0.32 / accesskit 0.24.

Wired in through `[patch.crates-io]` in the workspace `Cargo.toml`. Delete this directory and
the patch entry once egui moves to accesskit 0.25 or newer.
