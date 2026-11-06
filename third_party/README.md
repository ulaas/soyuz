# Vendored crates

## egui-phosphor 0.14.0 (MIT OR Apache-2.0, https://github.com/amPerl/egui-phosphor)

The crates.io source, trimmed to what Soyuz uses, with its `egui` dependency moved from 0.36 to
0.37 (no egui-phosphor release supports egui 0.37 yet). Kept: `src/lib.rs`, the `regular` and
`fill` variants (`src/variants/`, `res/Phosphor.ttf`, `res/Phosphor-Fill.ttf`) and the licenses.
Dropped: the thin / light / bold fonts, the `subset!` macro, examples and tests. The code is
otherwise unchanged.

Wired in through `[patch.crates-io]` in the workspace `Cargo.toml`. Delete this directory and
the patch entry once a crates.io egui-phosphor depends on egui 0.37.

## epaint 0.37.0 and egui-wgpu 0.37.0 (MIT OR Apache-2.0, https://github.com/emilk/egui)

The crates.io sources with LCD subpixel text added, because egui draws grayscale text only.
Wired in through `[patch.crates-io]`. Every change is marked `Soyuz:`:

- `epaint`: `HintingTarget::SmoothLcd` and `LcdOrder`, `rasterize_lcd` in `text/font_face.rs` (three times
  the width, FreeType's FIR5 filter), and the coverage curve per channel in `image.rs`. LCD
  texels are `(r, g, b, 0)` in the ordinary atlas: premultiplied colours never have colour
  without alpha.
- `egui-wgpu`: `renderer.rs` builds its one pipeline from `egui_lcd.wgsl` with dual-source
  blending when the device has the feature, and the unchanged one otherwise. Non-LCD texels
  blend exactly as before.

Dropped from `epaint`: the benchmarks. Soyuz only uses `HintingTarget::SmoothLcd` when the device has
`DUAL_SOURCE_BLENDING` (`theme::fonts::set_lcd_supported`), since other renderers would draw
those texels wrong.

Updating egui means reapplying these changes to the new sources: `grep -rn "Soyuz:"` finds them.
