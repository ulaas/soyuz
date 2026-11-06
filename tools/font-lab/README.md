# Soyuz font lab

Compare fonts and text rendering side by side, each sample with an id you can quote.

```sh
cargo run --release --manifest-path tools/font-lab/Cargo.toml
```

It uses the same stack as Soyuz: eframe and egui 0.37, wgpu (DX12 on Windows), the Orbit
colours and the fonts Soyuz embeds, plus the system fonts this machine has. It is a separate
Cargo workspace, so it is not part of Soyuz's build, tests or CI.

## Using it

Pick a preset or tick values on the left; every combination becomes a card. Hover a card to
read its settings, what it would take to ship and its pixels magnified. Click to pick it.
Picks are saved to `picks.txt` here; **Picks only** puts them side by side, whichever preset
they came from. Use **Copy ids** to pass them on.

## Reading an id

`IN13-E-CR-B1-G45-D` is Inter, 13 pt, technique E, hinting CR, subpixel positions on,
coverage curve gamma 0.45, dark primary colours. `-W450` adds a weight and `-P150` a display
scale. DirectWrite ids leave out hinting and positions, which it decides itself:
`IN13-DWS-G45-D`.

## Techniques

| Code | What it is | To ship in Soyuz |
|---|---|---|
| `E` | The real pipeline: epaint's atlas drawn by `egui_wgpu::Renderer`, offscreen | Settings only (font, hinting, curve, weight, positions) |
| `CG` | Control: the lab's own path, should match `E` | Nothing, it is a check |
| `EM..`, `SH..` | Outlines grown (stem darkening), or coverage sharpened | Small: egui's `GlyphRasterizer` hook |
| `GL` | Gray coverage blended in linear light | Medium: a shader change |
| `LCD..`, `BGR5` | Subpixel coverage, one per colour channel | Large: colour atlas and dual-source blending, a patched `egui_wgpu` |
| `DW..` | Windows' DirectWrite rasterizer, as a reference | Large and Windows-only |

Only `E` runs through the GPU pipeline. The others are composited on the CPU onto the card's
known background, with a shared layout and baseline. That shows how they would look, not that
Soyuz can draw them today.

## Limits

- DirectWrite cards read a font file's default instance, so they skip the variable-font
  entries and weights, and fonts with no file (Hack, Ubuntu Light).
- LCD cards are only right on an RGB-stripe panel at its native resolution; a screenshot or
  remote desktop session can change how they look. Judge them on the real monitor.
- Samples are single-style text. Icons and fallback fonts are not part of this.
