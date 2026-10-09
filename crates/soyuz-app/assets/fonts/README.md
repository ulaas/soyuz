# Embedded fonts

These fonts are compiled into the binary (`include_bytes!` in `src/theme/fonts.rs`, table
`EMBEDDED`). Each one can be picked for any font role in `[fonts]` / Settings → Fonts; the
defaults are **Inter** (interface, numbers), **Inter SemiBold** (headings) and
**Cascadia Mono** (terminal, code editor, hex view). They are screen fonts that egui's
grayscale anti-aliasing draws well at 100 % scaling, and they cover Latin, Latin Extended
and Turkish (`ı İ ş ğ ç ö ü`; checked by the `embedded_fonts_cover_turkish_and_parse` test).

All files are the unmodified TTFs (static, plus the one variable Inter) from the official upstream releases (no subsetting:
several of these licenses reserve the font name for modified versions). All are licensed
under the SIL Open Font License 1.1; the license file sits next to each font.

| File | Family, version | Source (official release) | SHA-256 | License |
|---|---|---|---|---|
| `inter/Inter-Regular.ttf` | Inter 4.001 | <https://github.com/rsms/inter/releases/tag/v4.1> (`Inter-4.1.zip`, `extras/ttf/`) | `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82` | `inter/LICENSE.txt` |
| `inter/Inter-SemiBold.ttf` | Inter SemiBold 4.001 | same | `78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3` | `inter/LICENSE.txt` |
| `inter/InterVariable.ttf` | Inter Variable 4.001 (`opsz` 14–32, `wght` 100–900) | same zip, top level | `4989b125924991b90d05b2d16e0e388c48f7d5bb8b30539bbf9c755278d0ccaf` | `inter/LICENSE.txt` (identical to the zip's `LICENSE.txt`) |
| `ibm-plex-sans/IBMPlexSans-Regular.ttf` | IBM Plex Sans 3.005 | <https://github.com/IBM/plex/releases/tag/%40ibm%2Fplex-sans%401.1.0> (`ibm-plex-sans.zip`, `fonts/complete/ttf/`) | `975dcda37d80f038dcd143c22e33ca2d97a0cc5a929aace1c749153b0fe1afa5` | `ibm-plex-sans/LICENSE.txt` |
| `ibm-plex-sans/IBMPlexSans-SemiBold.ttf` | IBM Plex Sans SemiBold 3.005 | same | `a20caf8286023a6a7a85e40b1d2a4ae9fc3e3b1f9eda8f4c542dd4986af67bb1` | `ibm-plex-sans/LICENSE.txt` |
| `noto-sans/NotoSans-Regular.ttf` | Noto Sans 2.015 | <https://github.com/notofonts/latin-greek-cyrillic/releases/tag/NotoSans-v2.015> (`NotoSans-v2.015.zip`, `NotoSans/unhinted/ttf/`) | `f3961a9cde016d41a4879aecda1474d3a36d6bf54fa0e4643de029cc2248b0e8` | `noto-sans/OFL.txt` |
| `noto-sans/NotoSans-SemiBold.ttf` | Noto Sans SemiBold 2.015 | same | `a7af8d4a11e8e49fea9a19786a6469210b512cd4061413e8036bdd2425722795` | `noto-sans/OFL.txt` |
| `cascadia-mono/CascadiaMono-Regular.ttf` | Cascadia Mono 2407.024 | <https://github.com/microsoft/cascadia-code/releases/tag/v2407.24> (`CascadiaCode-2407.24.zip`, `ttf/static/`) | `06520d032ec274fa5040b22c6f4a1d829081b24ba40b2da56dae89bf10c7b481` | `cascadia-mono/LICENSE` (from the repository at tag `v2407.24`) |
| `jetbrains-mono/JetBrainsMono-Regular.ttf` | JetBrains Mono 2.304 | <https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304> | `a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f` | `jetbrains-mono/OFL.txt` |
| `ibm-plex-mono/IBMPlexMono-Regular.ttf` | IBM Plex Mono 2.005 | <https://github.com/IBM/plex/releases/tag/%40ibm%2Fplex-mono%402.5.0> (`ibm-plex-mono.zip`, `fonts/complete/ttf/`) | `7c6fbddca4b700be918f5f6183d9bd4464fa427fe435f0b480d77fe2bb8c5a43` | `ibm-plex-mono/LICENSE.txt` |
| `source-code-pro/SourceCodePro-Regular.ttf` | Source Code Pro 2.042 | <https://github.com/adobe-fonts/source-code-pro/releases/tag/2.042R-u%2F1.062R-i%2F1.026R-vf> (`TTF-source-code-pro-2.042R-u_1.062R-i.zip`) | `74bd80d3e42a08517cd7e1108ba3d86f2da29ac0f3065be95e0357956ab9db37` | `source-code-pro/LICENSE.md` (from the repository at that tag) |

Total: 4.02 MiB (3.18 MiB before the variable Inter; the first set, Inter Regular / SemiBold +
JetBrains Mono, was 1.05 MiB).
Only the fonts a role actually uses are handed to egui, so unused ones cost binary size,
not memory or startup time.

The Phosphor icon fonts come from the `egui-phosphor` crate (MIT).

## Why these, and what was left out

The comparison sheets in `docs/font-comparison/` (made with `scripts/font-comparison.ps1`)
show every candidate in the real app at 13 and 14 px. In short:

- **Inter** has the most even spacing and the crispest stems with `hinting_strength = "crisp"`;
  its large x-height reads like Segoe UI at 14 px while staying at 13 px.
- **Cascadia Mono** has no ligatures (the terminal shows `!=` and `->` as typed, unlike
  JetBrains Mono), a compact line height like Consolas, unbroken box drawing and strokes
  heavy enough for grayscale anti-aliasing.
- **Source Sans 3** was dropped: with hinting on, egui's hinter draws the capital `İ` without
  its dot (`docs/font-comparison/rejected-source-sans-3*.png`), which breaks Turkish text.
- **Inter Medium / IBM Plex Sans Medium** were dropped: SemiBold headings separate better
  from the regular text at small sizes (`rejected-inter-medium.png`).
- **Noto Sans** spaces letters unevenly with crisp hinting (`rendering-noto.png`); it looks
  right with `hinting_strength = "normal"`.
- No bold weights: egui draws no bold text (the terminal's bold is drawn by overstriking).

## Inter Variable (optical sizes)

`InterVariable.ttf` backs the **Inter Variable (optical sizes)** choice for the interface font
(`[fonts.ui] family = "Inter Variable"`). egui cannot vary optical size with the font size,
so Soyuz pins named instances (`FontTweak::coords`, one egui font per instance) and gives
small text and large headings egui families of their own (`fonts::CAPTION`,
`fonts::DISPLAY`):

| Use | Text styles | `opsz` | `wght` |
|---|---|---|---|
| Body | Body, Button, file names (13 pt) | 14 | 400 |
| Captions | Small, Caption (11–11.5 pt) | 14 (axis minimum) | 440 |
| Titles | the headings role: tabs, dialog titles (13–15 pt) | 15 | 600 |
| Large headings | Heading, Display, Settings titles (17–22 pt) | 20 | 600 |

Inter's `opsz` axis starts at 14, which *is* the static "Inter" text design, so body text
and captions cannot get a smaller optical size; captions get a little more weight instead.
The static Inter stays in every family as the fallback, and a variable instance whose axes
cannot be read falls back to the static face.

**Not the default.** `docs/font-comparison/inter-variable.png`
(`scripts/font-inter-variable.ps1`) compares both in the real app at 100 %. The static TTFs
carry TrueType hinting; the variable font has none, so egui's autohinter fits it: with the
default crisp hinting the body text gets uneven letter gaps ("Variabl e") and lighter stems,
and with light hinting it is even but softer and wider than the static face. The display
optical size makes 17–22 pt headings slightly tighter, which is the only visible gain. So the
static Inter stays the default, and the variable font is an option.

## System fonts are not bundled

Segoe UI / Segoe UI Semibold, Consolas, Cascadia Code and the other Microsoft fonts (the
previous defaults) can still be chosen per role. Their license does not allow
redistribution, so Soyuz reads them at run time from `%WINDIR%\Fonts` (or the per-user font
folder) and never copies them. They are hinted for ClearType, which egui does not do, so
they look thinner and softer than the embedded fonts here. Cascadia Mono is the exception:
Microsoft releases it under the OFL, so it is embedded. When a configured system font is
missing the role falls back to its embedded default.
