# Embedded fonts

These files are compiled into the binary (`include_bytes!` in `src/theme/fonts.rs`) and
are the fallbacks of the font roles in `[fonts]`.

| File | Family / version | Role | Source | License |
|---|---|---|---|---|
| `Inter-Regular.ttf` | Inter 4.001 | ui, numbers | <https://github.com/rsms/inter> (release 4.0) | SIL OFL 1.1 — `Inter-LICENSE.txt` |
| `Inter-SemiBold.ttf` | Inter SemiBold 4.001 | ui_semibold | same | SIL OFL 1.1 — `Inter-LICENSE.txt` |
| `JetBrainsMono-Regular.ttf` | JetBrains Mono 2.304 | terminal, code, hex | <https://github.com/JetBrains/JetBrainsMono> (release 2.304) | SIL OFL 1.1 — `JetBrainsMono-OFL.txt` |

The Phosphor icon fonts come from the `egui-phosphor` crate (MIT).

## System fonts are not bundled

The defaults Segoe UI / Segoe UI Semibold (interface) and Consolas (terminal, code, hex),
with Cascadia Mono / Cascadia Code as second choice, are Microsoft fonts whose license does
not allow redistribution. Soyuz reads them at run time from `%WINDIR%\Fonts` (or the
per-user font folder) of the machine it runs on and never copies them. When one is
missing the role falls back to the embedded font above.

Not added: bold weights (egui draws no bold text; the terminal's bold is drawn by
overstriking) and Selawik (Microsoft's OFL Segoe UI metric-compatible font) — Segoe UI
ships with every supported Windows version, and Inter is the better-looking fallback.
