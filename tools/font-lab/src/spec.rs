//! What one sample is: every axis, its code in the sample id, and the font list.

use std::path::PathBuf;

use egui::epaint::text::{HintingTarget, SmoothHinting};

/// One selectable font.
#[derive(Clone)]
pub struct FontEntry {
    pub code: &'static str,
    pub name: &'static str,
    pub bytes: &'static [u8],
    /// The file on disk (DirectWrite opens fonts by path).
    pub path: Option<PathBuf>,
    pub mono: bool,
    /// Fixed variation coordinates of this entry.
    pub coords: &'static [(&'static [u8; 4], f32)],
    pub has_wght: bool,
    pub system: bool,
}

macro_rules! asset {
    ($p:literal) => {
        (
            include_bytes!(concat!("../../../crates/soyuz-app/assets/fonts/", $p)).as_slice(),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/soyuz-app/assets/fonts/", $p),
        )
    };
}

type Coords = &'static [(&'static [u8; 4], f32)];
const NO_COORDS: Coords = &[];
const OPSZ14: Coords = &[(b"opsz", 14.0)];
const OPSZ32: Coords = &[(b"opsz", 32.0)];
const OPSZ36: Coords = &[(b"opsz", 36.0)];

/// The fonts Soyuz embeds, then whichever system fonts this machine has.
pub fn fonts() -> Vec<FontEntry> {
    let mut out = Vec::new();
    let mut emb = |code, name, (bytes, path): (&'static [u8], &str), mono, coords| {
        out.push(entry(code, name, bytes, Some(PathBuf::from(path)), mono, coords, false));
    };
    emb("IN", "Inter (Soyuz default)", asset!("inter/Inter-Regular.ttf"), false, NO_COORDS);
    emb("IS", "Inter SemiBold", asset!("inter/Inter-SemiBold.ttf"), false, NO_COORDS);
    emb("IV", "Inter Variable, text", asset!("inter/InterVariable.ttf"), false, OPSZ14);
    emb("IX", "Inter Variable, display", asset!("inter/InterVariable.ttf"), false, OPSZ32);
    emb("IT", "Inter Tabular", asset!("inter/InterTabular-Regular.ttf"), false, NO_COORDS);
    emb("PS", "IBM Plex Sans", asset!("ibm-plex-sans/IBMPlexSans-Regular.ttf"), false, NO_COORDS);
    emb("PB", "IBM Plex Sans SemiBold", asset!("ibm-plex-sans/IBMPlexSans-SemiBold.ttf"), false, NO_COORDS);
    emb("NS", "Noto Sans", asset!("noto-sans/NotoSans-Regular.ttf"), false, NO_COORDS);
    emb("NB", "Noto Sans SemiBold", asset!("noto-sans/NotoSans-SemiBold.ttf"), false, NO_COORDS);
    emb("CM", "Cascadia Mono", asset!("cascadia-mono/CascadiaMono-Regular.ttf"), true, NO_COORDS);
    emb("JB", "JetBrains Mono", asset!("jetbrains-mono/JetBrainsMono-Regular.ttf"), true, NO_COORDS);
    emb("PM", "IBM Plex Mono", asset!("ibm-plex-mono/IBMPlexMono-Regular.ttf"), true, NO_COORDS);
    emb("SC", "Source Code Pro", asset!("source-code-pro/SourceCodePro-Regular.ttf"), true, NO_COORDS);
    out.push(entry("HK", "Hack", epaint_default_fonts::HACK_REGULAR, None, true, NO_COORDS, false));
    out.push(entry(
        "UB",
        "Ubuntu Light (egui default)",
        epaint_default_fonts::UBUNTU_LIGHT,
        None,
        false,
        NO_COORDS,
        false,
    ));

    const SYSTEM: &[(&str, &str, &str, bool, Coords)] = &[
        ("SG", "Segoe UI", "segoeui.ttf", false, NO_COORDS),
        ("SM", "Segoe UI SemiBold", "seguisb.ttf", false, NO_COORDS),
        ("SV", "Segoe UI Variable, text", "SegUIVar.ttf", false, OPSZ14),
        ("SX", "Segoe UI Variable, display", "SegUIVar.ttf", false, OPSZ36),
        ("TA", "Tahoma", "tahoma.ttf", false, NO_COORDS),
        ("VE", "Verdana", "verdana.ttf", false, NO_COORDS),
        ("AR", "Arial", "arial.ttf", false, NO_COORDS),
        ("CA", "Calibri", "calibri.ttf", false, NO_COORDS),
        ("TR", "Trebuchet MS", "trebuc.ttf", false, NO_COORDS),
        ("CO", "Consolas", "consola.ttf", true, NO_COORDS),
        ("CU", "Courier New", "cour.ttf", true, NO_COORDS),
        ("DV", "DejaVu Sans", "DejaVuSans.ttf", false, NO_COORDS),
        ("LI", "Liberation Sans", "LiberationSans-Regular.ttf", false, NO_COORDS),
        ("CT", "Cantarell", "Cantarell-Regular.ttf", false, NO_COORDS),
        ("DM", "DejaVu Sans Mono", "DejaVuSansMono.ttf", true, NO_COORDS),
    ];
    let files = system_font_files();
    for &(code, name, file, mono, coords) in SYSTEM {
        let found =
            files.iter().find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(file)));
        let Some(path) = found else { continue };
        if let Ok(bytes) = std::fs::read(path) {
            out.push(entry(code, name, Box::leak(bytes.into_boxed_slice()), Some(path.clone()), mono, coords, true));
        }
    }
    out
}

fn entry(
    code: &'static str,
    name: &'static str,
    bytes: &'static [u8],
    path: Option<PathBuf>,
    mono: bool,
    coords: Coords,
    system: bool,
) -> FontEntry {
    let axes = egui::FontData::from_static(bytes).variation_axes();
    let has_wght = axes.iter().any(|a| a.tag.to_be_bytes() == *b"wght");
    FontEntry { code, name, bytes, path, mono, coords, has_wght, system }
}

fn system_font_files() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        let win = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        roots.push(win.join("Fonts"));
    } else {
        roots.push("/usr/share/fonts".into());
        roots.push("/usr/local/share/fonts".into());
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join(".local/share/fonts"));
        }
    }
    let mut out = Vec::new();
    while let Some(dir) = roots.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                roots.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

pub const SIZES: [f32; 16] =
    [9.0, 10.0, 11.0, 11.5, 12.0, 12.5, 13.0, 13.5, 14.0, 15.0, 16.0, 18.0, 20.0, 22.0, 24.0, 28.0];
pub const DEFAULT_SIZE: usize = 6;

pub struct Hint {
    pub code: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    /// `None`: hinting off.
    pub target: Option<HintingTarget>,
}

const fn smooth(light: bool, symmetric_rendering: bool, preserve_linear_metrics: bool) -> Option<HintingTarget> {
    Some(HintingTarget::Smooth(SmoothHinting { light, symmetric_rendering, preserve_linear_metrics }))
}

pub const HINTS: [Hint; 8] = [
    Hint {
        code: "CR",
        label: "Crisp (Soyuz)",
        help: "Normal mode, asymmetric, horizontal fit. Soyuz's hinting_strength = crisp.",
        target: smooth(false, false, false),
    },
    Hint {
        code: "NX",
        label: "Normal, H-fit",
        help: "Normal mode, symmetric, horizontal fit.",
        target: smooth(false, true, false),
    },
    Hint {
        code: "NA",
        label: "Normal, asym",
        help: "Normal mode, asymmetric, linear widths.",
        target: smooth(false, false, true),
    },
    Hint {
        code: "NM",
        label: "Normal (egui)",
        help: "egui's default; Soyuz's hinting_strength = normal.",
        target: smooth(false, true, true),
    },
    Hint {
        code: "LT",
        label: "Light",
        help: "Snaps heights only; Soyuz's hinting_strength = light.",
        target: smooth(true, true, true),
    },
    Hint {
        code: "LA",
        label: "Light, asym",
        help: "Snaps heights only, asymmetric rendering.",
        target: smooth(true, false, true),
    },
    Hint {
        code: "MO",
        label: "Mono target",
        help: "Strongest grid fit, meant for 1-bit rendering, still anti-aliased here.",
        target: Some(HintingTarget::Mono),
    },
    Hint { code: "H0", label: "Off", help: "No hinting: the designer's outlines.", target: None },
];
pub const DEFAULT_HINT: usize = 0;

/// `None` is egui's own dark-mode curve, `2c - c²`.
pub const GAMMAS: [(&str, Option<f32>); 14] = [
    ("GE", None),
    ("G30", Some(0.30)),
    ("G35", Some(0.35)),
    ("G40", Some(0.40)),
    ("G45", Some(0.45)),
    ("G50", Some(0.50)),
    ("G55", Some(0.55)),
    ("G60", Some(0.60)),
    ("G65", Some(0.65)),
    ("G70", Some(0.70)),
    ("G80", Some(0.80)),
    ("G90", Some(0.90)),
    ("G100", Some(1.00)),
    ("G120", Some(1.20)),
];
pub const G45: usize = 4;
pub const G80: usize = 10;

/// Coverage to opacity, the same math as `FontColorTransferFunction::to_atlas_color`.
pub fn transfer(gamma: Option<f32>, c: f32) -> f32 {
    match gamma {
        None => 2.0 * c - c * c,
        Some(g) => c.powf(g),
    }
}

pub const WEIGHTS: [u16; 9] = [0, 300, 350, 380, 420, 450, 500, 550, 600];
pub const SCALES: [u16; 6] = [100, 125, 150, 175, 200, 300];

pub struct Theme {
    pub code: &'static str,
    pub label: &'static str,
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    pub dark: bool,
}

/// Soyuz's Orbit palette (`soyuz_appcore::theme::tokens`).
pub const THEMES: [Theme; 6] = [
    Theme { code: "D", label: "Dark, primary", bg: [0x0F, 0x11, 0x15], fg: [0xE6, 0xE8, 0xEC], dark: true },
    Theme { code: "DS", label: "Dark, secondary", bg: [0x0F, 0x11, 0x15], fg: [0xA3, 0xA9, 0xB7], dark: true },
    Theme { code: "DP", label: "Dark, on panel", bg: [0x16, 0x19, 0x20], fg: [0xE6, 0xE8, 0xEC], dark: true },
    Theme { code: "L", label: "Light, primary", bg: [0xF6, 0xF7, 0xF9], fg: [0x17, 0x1A, 0x20], dark: false },
    Theme { code: "LS", label: "Light, secondary", bg: [0xF6, 0xF7, 0xF9], fg: [0x4B, 0x52, 0x60], dark: false },
    Theme { code: "LP", label: "Light, on panel", bg: [0xFF, 0xFF, 0xFF], fg: [0x17, 0x1A, 0x20], dark: false },
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// The real pipeline: epaint atlas, egui_wgpu renderer.
    Stock,
    /// The real pipeline with Soyuz's LCD patch: per-channel atlas and blending on the GPU.
    StockLcd,
    /// Our rasterizer, one coverage per pixel.
    Gray { linear: bool, embolden: f32, sharpen: f32 },
    /// Our rasterizer, one coverage per colour channel.
    Lcd { fir: [f32; 5], bgr: bool, linear: bool },
    /// Windows' own rasterizer.
    DirectWrite { mode: i32, measuring: i32, gray: bool, linear: bool },
}

pub struct Tech {
    pub code: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    /// What it would take for Soyuz to draw text this way.
    pub ship: &'static str,
    pub kind: Kind,
}

const FIR5: [f32; 5] = [8.0 / 256.0, 77.0 / 256.0, 86.0 / 256.0, 77.0 / 256.0, 8.0 / 256.0];
const FIR3: [f32; 5] = [0.0, 85.0 / 256.0, 86.0 / 256.0, 85.0 / 256.0, 0.0];
const FIRB: [f32; 5] = [1.0 / 9.0, 2.0 / 9.0, 3.0 / 9.0, 2.0 / 9.0, 1.0 / 9.0];

const GRAY: Kind = Kind::Gray { linear: false, embolden: 0.0, sharpen: 0.0 };
const fn bold(embolden: f32) -> Kind {
    Kind::Gray { linear: false, embolden, sharpen: 0.0 }
}
const fn sharp(sharpen: f32) -> Kind {
    Kind::Gray { linear: false, embolden: 0.0, sharpen }
}
const fn dw(mode: i32, measuring: i32, gray: bool, linear: bool) -> Kind {
    Kind::DirectWrite { mode, measuring, gray, linear }
}

const SHIP_NOW: &str = "Ships today: only a settings change, this is the pipeline Soyuz already uses.";
const SHIP_RASTER: &str =
    "Small: a custom glyph rasterizer behind egui's GlyphRasterizer hook; the atlas and shader stay.";
const SHIP_LINEAR: &str = "Medium: the shader would have to blend text in linear light, which egui does not do.";
const SHIP_LCD: &str = "Large: per-channel coverage needs a colour atlas and dual-source blending, so a patched egui_wgpu. Only correct over opaque backgrounds, and wrong on rotated or BGR panels unless detected.";
const SHIP_DW: &str = "Large and Windows-only: DirectWrite draws every glyph, Linux and macOS would need something else; LCD variants also need the shader work.";

pub const TECHS: [Tech; 25] = [
    Tech { code: "E", label: "egui atlas (Soyuz today)", group: "Real pipeline", ship: SHIP_NOW, kind: Kind::Stock },
    Tech {
        code: "CG",
        label: "Custom gray (control)",
        group: "Grayscale",
        ship: "A control: the same rasterizer as egui through this lab's path. Should look nearly identical to E.",
        kind: GRAY,
    },
    Tech {
        code: "GL",
        label: "Gray, linear blend",
        group: "Grayscale",
        ship: SHIP_LINEAR,
        kind: Kind::Gray { linear: true, embolden: 0.0, sharpen: 0.0 },
    },
    Tech { code: "EM05", label: "Embolden 0.05 px", group: "Stem darkening", ship: SHIP_RASTER, kind: bold(0.05) },
    Tech { code: "EM10", label: "Embolden 0.10 px", group: "Stem darkening", ship: SHIP_RASTER, kind: bold(0.10) },
    Tech { code: "EM15", label: "Embolden 0.15 px", group: "Stem darkening", ship: SHIP_RASTER, kind: bold(0.15) },
    Tech { code: "EM20", label: "Embolden 0.20 px", group: "Stem darkening", ship: SHIP_RASTER, kind: bold(0.20) },
    Tech { code: "EM30", label: "Embolden 0.30 px", group: "Stem darkening", ship: SHIP_RASTER, kind: bold(0.30) },
    Tech { code: "SH15", label: "Sharpen 0.15", group: "Sharpening", ship: SHIP_RASTER, kind: sharp(0.15) },
    Tech { code: "SH30", label: "Sharpen 0.30", group: "Sharpening", ship: SHIP_RASTER, kind: sharp(0.30) },
    Tech { code: "SH50", label: "Sharpen 0.50", group: "Sharpening", ship: SHIP_RASTER, kind: sharp(0.50) },
    Tech { code: "EL", label: "LCD on the GPU (Soyuz today)", group: "Real pipeline", ship: "Shipped: Soyuz draws LCD text this way. LCD5 is its CPU reference.", kind: Kind::StockLcd },
    Tech {
        code: "LCD5",
        label: "LCD RGB, FIR5 default",
        group: "LCD subpixel",
        ship: SHIP_LCD,
        kind: Kind::Lcd { fir: FIR5, bgr: false, linear: false },
    },
    Tech {
        code: "LCD3",
        label: "LCD RGB, FIR3 light",
        group: "LCD subpixel",
        ship: SHIP_LCD,
        kind: Kind::Lcd { fir: FIR3, bgr: false, linear: false },
    },
    Tech {
        code: "LCDB",
        label: "LCD RGB, FIR5 soft",
        group: "LCD subpixel",
        ship: SHIP_LCD,
        kind: Kind::Lcd { fir: FIRB, bgr: false, linear: false },
    },
    Tech {
        code: "BGR5",
        label: "LCD BGR panel, FIR5",
        group: "LCD subpixel",
        ship: SHIP_LCD,
        kind: Kind::Lcd { fir: FIR5, bgr: true, linear: false },
    },
    Tech {
        code: "LCD5L",
        label: "LCD RGB, FIR5, linear",
        group: "LCD subpixel",
        ship: SHIP_LCD,
        kind: Kind::Lcd { fir: FIR5, bgr: false, linear: true },
    },
    Tech {
        code: "LCD3L",
        label: "LCD RGB, FIR3, linear",
        group: "LCD subpixel",
        ship: SHIP_LCD,
        kind: Kind::Lcd { fir: FIR3, bgr: false, linear: true },
    },
    // Rendering modes 5 / 4 / 3 / 2 / 1 are DWRITE_RENDERING_MODE_*; measuring 0 natural, 1 GDI classic, 2 GDI natural.
    Tech {
        code: "DWS",
        label: "DirectWrite, symmetric",
        group: "DirectWrite",
        ship: SHIP_DW,
        kind: dw(5, 0, false, false),
    },
    Tech {
        code: "DWSL",
        label: "DirectWrite, symmetric, linear",
        group: "DirectWrite",
        ship: SHIP_DW,
        kind: dw(5, 0, false, true),
    },
    Tech {
        code: "DWN",
        label: "DirectWrite, natural",
        group: "DirectWrite",
        ship: SHIP_DW,
        kind: dw(4, 0, false, false),
    },
    Tech {
        code: "DWG",
        label: "DirectWrite, GDI natural",
        group: "DirectWrite",
        ship: SHIP_DW,
        kind: dw(3, 2, false, false),
    },
    Tech {
        code: "DWC",
        label: "DirectWrite, GDI classic",
        group: "DirectWrite",
        ship: SHIP_DW,
        kind: dw(2, 1, false, false),
    },
    Tech {
        code: "DWY",
        label: "DirectWrite, averaged to gray",
        group: "DirectWrite",
        ship: SHIP_DW,
        kind: dw(5, 0, true, false),
    },
    Tech {
        code: "DWA",
        label: "DirectWrite, aliased",
        group: "DirectWrite",
        ship: SHIP_DW,
        kind: dw(1, 1, true, false),
    },
];

/// One sample. Fields are indices into the axis tables.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Spec {
    pub font: usize,
    pub size: usize,
    pub tech: usize,
    pub hint: usize,
    pub bin: bool,
    pub gamma: usize,
    pub theme: usize,
    pub wght: u16,
    pub scale: u16,
}

impl Spec {
    /// The id shown on the card. Only axes that change the result appear, so equal looks
    /// get equal ids.
    pub fn id(&self, fonts: &[FontEntry]) -> String {
        let t = &TECHS[self.tech];
        let dw = matches!(t.kind, Kind::DirectWrite { .. });
        let size = SIZES[self.size];
        let mut s = format!("{}{}-{}", fonts[self.font].code, size, t.code);
        if !dw {
            s += &format!("-{}-B{}", HINTS[self.hint].code, u8::from(self.bin));
        }
        s += &format!("-{}-{}", GAMMAS[self.gamma].0, THEMES[self.theme].code);
        if self.wght != 0 {
            s += &format!("-W{}", self.wght);
        }
        if self.scale != 100 {
            s += &format!("-P{}", self.scale);
        }
        s
    }

    /// The sample an id names (the inverse of [`Self::id`]).
    pub fn parse(id: &str, fonts: &[FontEntry]) -> Option<Spec> {
        let mut parts = id.split('-');
        let head = parts.next()?;
        let font = fonts.iter().position(|f| head.starts_with(f.code))?;
        let size: f32 = head.get(fonts[font].code.len()..)?.parse().ok()?;
        let size = SIZES.iter().position(|s| *s == size)?;
        let code = parts.next()?;
        let tech = TECHS.iter().position(|t| t.code == code)?;
        let (mut hint, mut bin) = (DEFAULT_HINT, true);
        if !matches!(TECHS[tech].kind, Kind::DirectWrite { .. }) {
            let code = parts.next()?;
            hint = HINTS.iter().position(|h| h.code == code)?;
            bin = parts.next()? == "B1";
        }
        let code = parts.next()?;
        let gamma = GAMMAS.iter().position(|g| g.0 == code)?;
        let code = parts.next()?;
        let theme = THEMES.iter().position(|t| t.code == code)?;
        let (mut wght, mut scale) = (0, 100);
        for p in parts {
            if let Some(v) = p.strip_prefix('W') {
                wght = v.parse().ok()?;
            } else if let Some(v) = p.strip_prefix('P') {
                scale = v.parse().ok()?;
            }
        }
        let spec = Spec { font, size, tech, hint, bin, gamma, theme, wght, scale };
        spec.valid(fonts).then_some(spec)
    }

    /// Whether this combination can be drawn at all.
    pub fn valid(&self, fonts: &[FontEntry]) -> bool {
        let f = &fonts[self.font];
        if self.wght != 0 && !f.has_wght {
            return false;
        }
        if matches!(TECHS[self.tech].kind, Kind::DirectWrite { .. }) {
            // DirectWrite here reads the file's default instance only, and needs a file.
            return cfg!(windows) && f.path.is_some() && self.wght == 0 && f.coords.is_empty();
        }
        true
    }

    pub fn px(&self) -> f32 {
        SIZES[self.size] * f32::from(self.scale) / 100.0
    }

    /// The axes in words.
    pub fn describe(&self, fonts: &[FontEntry]) -> Vec<(&'static str, String)> {
        let t = &TECHS[self.tech];
        let mut v = vec![
            ("Font", fonts[self.font].name.to_string()),
            ("Size", format!("{} pt = {} px", SIZES[self.size], self.px())),
            ("Technique", t.label.to_string()),
        ];
        if !matches!(t.kind, Kind::DirectWrite { .. }) {
            v.push(("Hinting", format!("{}: {}", HINTS[self.hint].label, HINTS[self.hint].help)));
            v.push(("Subpixel positions", if self.bin { "on (4 bins)" } else { "off (whole pixels)" }.to_string()));
        }
        v.push((
            "Coverage curve",
            GAMMAS[self.gamma].1.map_or_else(|| "egui's 2c - c²".to_string(), |g| format!("gamma {g:.2}")),
        ));
        v.push(("Colours", THEMES[self.theme].label.to_string()));
        if self.wght != 0 {
            v.push(("Weight", self.wght.to_string()));
        }
        v.push(("Display scale", format!("{} %", self.scale)));
        v.push(("To ship", t.ship.to_string()));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Picks are stored as ids: every id must read back as the same look.
    #[test]
    fn ids_read_back() {
        let fonts = fonts();
        let mut seen = 0;
        for font in 0..fonts.len() {
            for size in 0..SIZES.len() {
                for tech in 0..TECHS.len() {
                    for hint in 0..HINTS.len() {
                        for bin in [false, true] {
                            for (gamma, theme) in [(0, 0), (G45, 1), (G80, 3), (13, 5)] {
                                for wght in [0, 450] {
                                    for scale in [100, 150] {
                                        let s = Spec { font, size, tech, hint, bin, gamma, theme, wght, scale };
                                        if !s.valid(&fonts) {
                                            continue;
                                        }
                                        let id = s.id(&fonts);
                                        let back = Spec::parse(&id, &fonts).unwrap_or_else(|| panic!("{id}"));
                                        assert_eq!(back.id(&fonts), id);
                                        seen += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(seen > 100_000, "{seen}");
    }

    #[test]
    fn soyuz_default_id() {
        let fonts = fonts();
        let s = Spec {
            font: 0,
            size: DEFAULT_SIZE,
            tech: 0,
            hint: DEFAULT_HINT,
            bin: true,
            gamma: G45,
            theme: 0,
            wght: 0,
            scale: 100,
        };
        assert_eq!(s.id(&fonts), "IN13-E-CR-B1-G45-D");
    }
}
