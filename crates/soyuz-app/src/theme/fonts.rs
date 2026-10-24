//! Font roles (`[fonts]`): which font draws the interface, headings, terminal, code
//! editor, hex view and numbers, plus the Phosphor icon fonts.
//!
//! Each role resolves to a *primary* font — one of the [`EMBEDDED`] fonts (screen fonts
//! under the SIL OFL compiled into the binary, `assets/fonts/<family>/`) or a system font
//! read from the OS font folder (Segoe UI, Consolas, …; never bundled: their licenses
//! forbid redistribution) — followed by the role's embedded default, egui's default fonts
//! and the system CJK / emoji fallbacks (`font_fallback`). A family name that matches an
//! embedded font always uses the embedded copy (no disk access).
//!
//! egui families per role: `Proportional` (ui), [`SEMIBOLD`] (ui_semibold),
//! `Monospace` (code; also every other monospace text), [`TERMINAL`], [`HEX`] and
//! [`NUMBERS`]. Icons always use [`ICONS`] / [`PHOSPHOR_FILL`], whose first font is
//! Phosphor, so a text font with Private Use Area glyphs can never shadow an icon.
//!
//! Rendering: egui rasterizes glyphs with grayscale anti-aliasing (no ClearType), so
//! fonts hinted for ClearType (Segoe UI, Consolas) look thin and soft at 100 % scaling.
//! The defaults are therefore embedded fonts drawn well by this renderer, with
//! `[fonts] hinting_strength` (skrifa hinting target) and `text_gamma_*` (coverage →
//! opacity curve, see `apply.rs`) tuning the result. System font files are mapped
//! read-only once (`soyuz_fs::mapped`; read whole where mapping isn't safe) and kept for
//! the process lifetime.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::epaint::text::{FontTweak, HintingTarget, SmoothHinting, VariationCoords};
use egui::{Context, FontData, FontDefinitions, FontFamily, FontId};
use parking_lot::Mutex;
use soyuz_core::config::{FontRoleKind, Fonts, HintingStrength};

use super::sysfonts;

/// Font family used for headings / titles / tab labels (`[fonts.ui_semibold]`).
pub const SEMIBOLD: &str = "semibold";
/// The terminal grid (`[fonts.terminal]`).
pub const TERMINAL: &str = "role-terminal";
/// The hex editor / preview (`[fonts.hex]`).
pub const HEX: &str = "role-hex";
/// Sizes, dates, counters, progress figures (`[fonts.numbers]`).
pub const NUMBERS: &str = "role-numbers";
/// Phosphor Regular as a dedicated family (icon font first). Always draw icons with
/// this family (or [`PHOSPHOR_FILL`]): text fonts may define glyphs in the Private Use
/// Area that would otherwise shadow Phosphor's codepoints in plain proportional text.
pub const ICONS: &str = "icons";
/// Phosphor Fill variant, used for the active sidebar item.
pub const PHOSPHOR_FILL: &str = "phosphor-fill";
/// Small text (`TextStyle::Small`, captions): the ui font, or its caption optical size.
pub const CAPTION: &str = "caption";
/// Large headings (`TextStyle::Heading`, display): the heading font, or its display
/// optical size.
pub const DISPLAY: &str = "display";

/// One font compiled into the binary.
#[derive(Debug)]
pub struct EmbeddedFont {
    /// Face name a `family` field matches (`IBM Plex Sans SemiBold`).
    pub name: &'static str,
    /// Name shown in Settings (usually [`Self::name`]).
    pub label: &'static str,
    /// The family it belongs to (`IBM Plex Sans`).
    pub family: &'static str,
    /// Monospace face.
    pub mono: bool,
    /// The face headings use when this face draws the interface (`ui_semibold` empty).
    heading: Option<Embedded>,
    bytes: &'static [u8],
    /// Fixed variation coordinates (a named instance of a variable font; empty = static).
    coords: &'static [Coord],
    /// Extra optical-size instances used for small text and large headings while this
    /// face draws the interface (see [`CAPTION`] / [`DISPLAY`]).
    optical: Option<&'static Optical>,
}

/// One variation coordinate (`opsz`, `wght`, …).
pub type Coord = (&'static [u8; 4], f32);

/// Optical-size instances of a variable interface font beyond its regular / heading
/// faces: [`CAPTION`] draws small text (`TextStyle::Small`, captions), [`DISPLAY`] large
/// headings (`TextStyle::Heading`, display).
#[derive(Debug)]
pub struct Optical {
    pub caption: &'static [Coord],
    pub display: &'static [Coord],
}

macro_rules! font {
    ($path:literal) => {
        include_bytes!(concat!("../../assets/fonts/", $path))
    };
}

const fn face(
    name: &'static str,
    family: &'static str,
    mono: bool,
    heading: Option<Embedded>,
    bytes: &'static [u8],
) -> EmbeddedFont {
    EmbeddedFont { name, label: name, family, mono, heading, bytes, coords: &[], optical: None }
}

/// A named instance of a variable font.
const fn instance(
    name: &'static str,
    label: &'static str,
    family: &'static str,
    heading: Option<Embedded>,
    bytes: &'static [u8],
    coords: &'static [Coord],
    optical: Option<&'static Optical>,
) -> EmbeddedFont {
    EmbeddedFont { name, label, family, mono: false, heading, bytes, coords, optical }
}

/// Inter 4's variable font: `opsz` 14–32 (14 = the static "Inter" text design, 32 =
/// "Inter Display"), `wght` 100–900.
static INTER_VARIABLE: &[u8] = font!("inter/InterVariable.ttf");
/// Body text: the axis minimum (the static Inter Regular design).
const INTER_VAR_BODY: &[Coord] = &[(b"opsz", 14.0), (b"wght", 400.0)];
/// Titles, tab labels, dialog headings (13–15 pt).
const INTER_VAR_SEMIBOLD: &[Coord] = &[(b"opsz", 15.0), (b"wght", 600.0)];
/// Small text (11–11.5 pt): `opsz` cannot go below 14, so a touch more weight instead.
const INTER_VAR_CAPTION: &[Coord] = &[(b"opsz", 14.0), (b"wght", 440.0)];
/// Large headings (18–22 pt): tighter spacing and finer details of the display design.
const INTER_VAR_DISPLAY: &[Coord] = &[(b"opsz", 20.0), (b"wght", 600.0)];
static INTER_VAR_OPTICAL: Optical = Optical { caption: INTER_VAR_CAPTION, display: INTER_VAR_DISPLAY };

/// Every embedded font, in Settings order (index = [`Embedded`]). See
/// `assets/fonts/README.md` for versions, sources and checksums.
pub static EMBEDDED: [EmbeddedFont; 12] = [
    face("Inter", "Inter", false, Some(Embedded::INTER_SEMIBOLD), font!("inter/Inter-Regular.ttf")),
    face("Inter SemiBold", "Inter", false, None, font!("inter/Inter-SemiBold.ttf")),
    instance(
        "Inter Variable",
        "Inter Variable (optical sizes)",
        "Inter Variable",
        Some(Embedded(3)),
        INTER_VARIABLE,
        INTER_VAR_BODY,
        Some(&INTER_VAR_OPTICAL),
    ),
    instance(
        "Inter Variable SemiBold",
        "Inter Variable SemiBold (optical sizes)",
        "Inter Variable",
        None,
        INTER_VARIABLE,
        INTER_VAR_SEMIBOLD,
        None,
    ),
    face("IBM Plex Sans", "IBM Plex Sans", false, Some(Embedded(5)), font!("ibm-plex-sans/IBMPlexSans-Regular.ttf")),
    face("IBM Plex Sans SemiBold", "IBM Plex Sans", false, None, font!("ibm-plex-sans/IBMPlexSans-SemiBold.ttf")),
    face("Noto Sans", "Noto Sans", false, Some(Embedded(7)), font!("noto-sans/NotoSans-Regular.ttf")),
    face("Noto Sans SemiBold", "Noto Sans", false, None, font!("noto-sans/NotoSans-SemiBold.ttf")),
    face("Cascadia Mono", "Cascadia Mono", true, None, font!("cascadia-mono/CascadiaMono-Regular.ttf")),
    face("JetBrains Mono", "JetBrains Mono", true, None, font!("jetbrains-mono/JetBrainsMono-Regular.ttf")),
    face("IBM Plex Mono", "IBM Plex Mono", true, None, font!("ibm-plex-mono/IBMPlexMono-Regular.ttf")),
    face("Source Code Pro", "Source Code Pro", true, None, font!("source-code-pro/SourceCodePro-Regular.ttf")),
];

/// Handle of one [`EMBEDDED`] font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Embedded(usize);

impl Embedded {
    pub const INTER: Embedded = Embedded(0);
    pub const INTER_SEMIBOLD: Embedded = Embedded(1);
    pub const INTER_VARIABLE: Embedded = Embedded(2);
    pub const INTER_VARIABLE_SEMIBOLD: Embedded = Embedded(3);
    pub const CASCADIA_MONO: Embedded = Embedded(8);
    pub const JETBRAINS_MONO: Embedded = Embedded(9);

    /// Every embedded font.
    pub fn all() -> impl Iterator<Item = Embedded> {
        (0..EMBEDDED.len()).map(Embedded)
    }

    pub fn font(self) -> &'static EmbeddedFont {
        &EMBEDDED[self.0]
    }

    /// The embedded face called `name` (`Regular` suffix and case / spacing ignored).
    pub fn find(name: &str) -> Option<Embedded> {
        let want = sysfonts::norm(name);
        let want = want.strip_suffix("regular").filter(|w| !w.is_empty()).unwrap_or(&want);
        Embedded::all().find(|e| sysfonts::norm(e.font().name) == want)
    }

    /// Key in the egui font data map.
    pub fn key(self) -> &'static str {
        self.font().name
    }

    pub fn label(self) -> &'static str {
        self.font().label
    }

    fn bytes(self) -> &'static [u8] {
        self.font().bytes
    }

    /// Whether egui can use this face: always for static faces; a variable instance
    /// needs every axis it pins (a damaged or truncated file falls back, see [`build`]).
    fn usable(self) -> bool {
        let f = self.font();
        if f.coords.is_empty() {
            return true;
        }
        let axes = FontData::from_static(f.bytes).variation_axes();
        f.coords.iter().all(|(tag, _)| axes.iter().any(|a| a.tag.to_be_bytes() == **tag))
    }

    /// The static face used instead when [`Self::usable`] is false.
    fn static_fallback(self) -> Embedded {
        match self {
            Embedded::INTER_VARIABLE => Embedded::INTER,
            Embedded::INTER_VARIABLE_SEMIBOLD => Embedded::INTER_SEMIBOLD,
            e => e,
        }
    }

    /// The face for headings when this one draws the interface.
    pub fn heading(self) -> Embedded {
        self.font().heading.unwrap_or(self)
    }

    /// The embedded font a role falls back to: the default family of the role when it is
    /// embedded (it is by default), else Inter / Inter SemiBold / Cascadia Mono.
    pub fn for_role(role: FontRoleKind) -> Embedded {
        let (default, builtin) = if role.is_mono() {
            (Fonts::DEFAULT_MONO, Embedded::CASCADIA_MONO)
        } else {
            (Fonts::DEFAULT_UI, Embedded::INTER)
        };
        let e = Embedded::find(default).unwrap_or(builtin);
        if role == FontRoleKind::UiSemibold { e.heading() } else { e }
    }
}

/// Where a role's primary font comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A font file of the system (or a user-given path).
    System {
        /// The family as configured / derived (`Segoe UI Semibold`).
        family: String,
        path: PathBuf,
        index: u32,
        /// `wght` coordinate for a variable font used at another weight.
        weight: Option<f32>,
    },
    Embedded(Embedded),
}

impl Source {
    /// Short description for Settings (`Segoe UI`, `Inter (embedded)`).
    pub fn describe(&self) -> String {
        match self {
            Source::System { family, weight: Some(w), .. } => {
                tr!("settings-font-source-weight", family = family, weight = format!("{w:.0}"))
            }
            Source::System { family, .. } => family.clone(),
            Source::Embedded(e) => tr!("settings-font-source-embedded", font = e.label()),
        }
    }

    fn key(&self) -> String {
        match self {
            Source::System { path, index, weight, .. } => {
                format!("sys:{}#{index}@{}", path.display(), weight.unwrap_or(0.0))
            }
            Source::Embedded(e) => e.key().to_string(),
        }
    }
}

/// One role after resolution.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub role: FontRoleKind,
    pub source: Source,
    /// Why the configured font is not the one in use (shown in Settings).
    pub notice: Option<String>,
}

/// File lookup used by [`resolve`] (family name or path → file + face index).
pub type Locate<'a> = &'a dyn Fn(&str) -> Option<(PathBuf, u32)>;
/// Whether a font file is variable with a `wght` axis.
pub type IsVariable<'a> = &'a dyn Fn(&Path, u32) -> bool;

/// Families tried for a role, in order (before its embedded fallback).
pub fn candidates(fonts: &Fonts, role: FontRoleKind) -> Vec<String> {
    let configured = fonts.role(role).family.trim().to_string();
    let first = |default: &str| if configured.is_empty() { default.to_string() } else { configured.clone() };
    let mut v = match role {
        FontRoleKind::Ui => vec![first(Fonts::DEFAULT_UI)],
        // A missing monospace font falls back to the embedded Cascadia Mono.
        FontRoleKind::Terminal | FontRoleKind::Code | FontRoleKind::Hex => {
            vec![first(Fonts::DEFAULT_MONO), "Cascadia Mono".into()]
        }
        // Derived from the ui font when empty (see `resolve`).
        FontRoleKind::UiSemibold | FontRoleKind::Numbers => {
            if configured.is_empty() {
                vec![]
            } else {
                vec![configured.clone()]
            }
        }
    };
    v.dedup_by(|a, b| sysfonts::norm(a) == sysfonts::norm(b));
    v
}

/// Resolve every role (pure apart from the two lookups; unit-tested with fakes).
pub fn resolve(fonts: &Fonts, locate: Locate<'_>, is_variable: IsVariable<'_>) -> Vec<Resolved> {
    let mut out: Vec<Resolved> = Vec::new();
    let find = |role: FontRoleKind| -> Resolved {
        let fallback = Embedded::for_role(role);
        if fonts.role(role).embedded {
            // Only embedded fonts: the named one when it is embedded, else the default.
            let e = Embedded::find(&fonts.role(role).family).unwrap_or(fallback);
            return Resolved { role, source: Source::Embedded(e), notice: None };
        }
        let names = candidates(fonts, role);
        for name in &names {
            let notice =
                (name != &names[0]).then(|| tr!("settings-font-not-installed", missing = &names[0], used = name));
            if let Some(e) = Embedded::find(name) {
                return Resolved { role, source: Source::Embedded(e), notice };
            }
            if let Some((path, index)) = locate(name) {
                return Resolved {
                    role,
                    source: Source::System { family: name.clone(), path, index, weight: None },
                    notice,
                };
            }
        }
        let embedded = Source::Embedded(fallback);
        let notice = names.first().map(|n| tr!("settings-font-not-installed", missing = n, used = embedded.describe()));
        Resolved { role, source: embedded, notice }
    };
    let ui = find(FontRoleKind::Ui);
    for role in FontRoleKind::ALL {
        let r = match role {
            FontRoleKind::Ui => ui.clone(),
            FontRoleKind::UiSemibold if fonts.ui_semibold.family.trim().is_empty() && !fonts.ui_semibold.embedded => {
                semibold_of(&ui, locate, is_variable)
            }
            FontRoleKind::Numbers if fonts.numbers.family.trim().is_empty() && !fonts.numbers.embedded => {
                Resolved { role, source: ui.source.clone(), notice: None }
            }
            _ => find(role),
        };
        out.push(r);
    }
    out
}

/// The semibold face that goes with the resolved ui font: the embedded family's heading
/// face, `<family> Semibold` when installed, else the same variable font at weight 600,
/// else the regular face.
fn semibold_of(ui: &Resolved, locate: Locate<'_>, is_variable: IsVariable<'_>) -> Resolved {
    let role = FontRoleKind::UiSemibold;
    match &ui.source {
        Source::Embedded(e) => Resolved { role, source: Source::Embedded(e.heading()), notice: None },
        Source::System { family, path, index, .. } => {
            for suffix in ["Semibold", "SemiBold", "Demibold", "Medium"] {
                let name = format!("{family} {suffix}");
                if let Some((p, i)) = locate(&name) {
                    return Resolved {
                        role,
                        source: Source::System { family: name, path: p, index: i, weight: None },
                        notice: None,
                    };
                }
            }
            if is_variable(path, *index) {
                let source =
                    Source::System { family: family.clone(), path: path.clone(), index: *index, weight: Some(600.0) };
                return Resolved { role, source, notice: None };
            }
            Resolved {
                role,
                source: ui.source.clone(),
                notice: Some(tr!("settings-font-no-semibold", family = family)),
            }
        }
    }
}

/// Font file bytes shared with egui (`FontData::from_blob`).
pub type Blob = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// Font bytes of system files, mapped (or read) once per process.
static LOADED: Mutex<Option<HashMap<PathBuf, Blob>>> = Mutex::new(None);

/// The file's bytes (once per distinct file: fonts live as long as the app). Mapped
/// read-only where safe, so a system font costs file-backed pages, not private memory.
fn load_file(path: &Path) -> Option<Blob> {
    if let Some(b) = LOADED.lock().get_or_insert_with(HashMap::new).get(path) {
        return Some(b.clone());
    }
    let bytes: Blob = Arc::new(soyuz_fs::mapped::map_or_read(path).ok()?);
    LOADED.lock().get_or_insert_with(HashMap::new).insert(path.to_path_buf(), bytes.clone());
    Some(bytes)
}

fn is_variable_file(path: &Path, index: u32) -> bool {
    let Some(bytes) = load_file(path) else { return false };
    let data = FontData::from_blob(bytes, index);
    data.variation_axes().iter().any(|a| a.tag.to_be_bytes() == *b"wght")
}

/// The hinting target for `[fonts] hinting_strength`.
pub fn hinting_target(strength: HintingStrength) -> HintingTarget {
    let normal = SmoothHinting::default();
    HintingTarget::Smooth(match strength {
        // Horizontal grid-fitting too, and asymmetric rendering for fonts with their own
        // TrueType instructions: the sharpest stems with grayscale AA at 100 % scaling.
        HintingStrength::Crisp => {
            SmoothHinting { light: false, symmetric_rendering: false, preserve_linear_metrics: false }
        }
        HintingStrength::Normal => normal,
        HintingStrength::Light => SmoothHinting { light: true, ..normal },
    })
}

fn tweak(fonts: &Fonts) -> FontTweak {
    FontTweak {
        hinting: Some(fonts.hinting),
        hinting_target: hinting_target(fonts.hinting_strength),
        subpixel_binning: Some(fonts.subpixel_positioning),
        ..FontTweak::default()
    }
}

/// The egui family of a role.
pub fn family(role: FontRoleKind) -> FontFamily {
    match role {
        FontRoleKind::Ui => FontFamily::Proportional,
        FontRoleKind::UiSemibold => FontFamily::Name(SEMIBOLD.into()),
        FontRoleKind::Code => FontFamily::Monospace,
        FontRoleKind::Terminal => FontFamily::Name(TERMINAL.into()),
        FontRoleKind::Hex => FontFamily::Name(HEX.into()),
        FontRoleKind::Numbers => FontFamily::Name(NUMBERS.into()),
    }
}

/// Build the font definitions for resolved roles. `load` returns a file's bytes
/// (`None`: the role silently uses its embedded fallback).
pub fn build(fonts: &Fonts, roles: &[Resolved], load: &dyn Fn(&Path) -> Option<Blob>) -> FontDefinitions {
    let defaults = FontDefinitions::default();
    let prop_defaults = defaults.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mono_defaults = defaults.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let mut defs = FontDefinitions { font_data: defaults.font_data.clone(), families: Default::default() };
    let tw = tweak(fonts);
    // Only the embedded fonts a role uses (primary or fallback) are handed to egui.
    let instance = |bytes: &'static [u8], coords: &[Coord]| {
        let mut t = tw.clone();
        if !coords.is_empty() {
            t.coords = VariationCoords::new(coords.iter().copied());
        }
        FontData::from_static(bytes).tweak(t)
    };
    // A variable instance that cannot be used (damaged file) becomes its static face.
    let roles: Vec<Resolved> = roles
        .iter()
        .map(|r| match r.source {
            Source::Embedded(e) if !e.usable() => {
                log::warn!("fonts: {} is not usable, using {}", e.label(), e.static_fallback().label());
                Resolved { source: Source::Embedded(e.static_fallback()), ..r.clone() }
            }
            _ => r.clone(),
        })
        .collect();
    let roles = &roles[..];
    let add_embedded = |defs: &mut FontDefinitions, e: Embedded| {
        defs.font_data.entry(e.key().into()).or_insert_with(|| instance(e.bytes(), e.font().coords).into());
    };
    for r in roles {
        if let Source::Embedded(e) = r.source {
            add_embedded(&mut defs, e);
        }
        add_embedded(&mut defs, Embedded::for_role(r.role));
    }
    egui_phosphor::add_to_fonts(&mut defs, egui_phosphor::Variant::Regular);
    egui_phosphor::add_font_bytes_as_family(&mut defs, PHOSPHOR_FILL, egui_phosphor::Variant::Fill.font_bytes());
    for r in roles {
        let mut list: Vec<String> = Vec::new();
        if let Source::Embedded(e) = r.source {
            list.push(e.key().to_string());
        } else if let Source::System { path, index, weight, .. } = &r.source
            && let Some(bytes) = load(path)
        {
            let key = r.source.key();
            if !defs.font_data.contains_key(&key) {
                let mut t = tw.clone();
                if let Some(w) = weight {
                    t.coords = VariationCoords::new([(b"wght", *w)]);
                }
                let data = FontData::from_blob(bytes, *index).tweak(t);
                defs.font_data.insert(key.clone(), data.into());
            }
            list.push(key);
        }
        let fallback = Embedded::for_role(r.role).key().to_string();
        if !list.contains(&fallback) {
            list.push(fallback);
        }
        if r.role.is_mono() {
            list.extend(mono_defaults.iter().cloned());
        } else {
            list.push("phosphor".into());
            list.extend(prop_defaults.iter().cloned());
        }
        defs.families.insert(family(r.role), list);
    }
    // Small text and large headings: the ui / heading lists, led by the optical-size
    // instances when the ui font has them.
    let optical = roles.iter().find(|r| r.role == FontRoleKind::Ui).and_then(|r| match r.source {
        Source::Embedded(e) => e.font().optical.map(|o| (e, o)),
        Source::System { .. } => None,
    });
    let derived = |defs: &mut FontDefinitions, name: &str, base: FontFamily, coords: Option<(Embedded, &[Coord])>| {
        let mut list = defs.families.get(&base).cloned().unwrap_or_default();
        if let Some((e, coords)) = coords {
            let key = format!("{} {name}", e.key());
            defs.font_data.entry(key.clone()).or_insert_with(|| instance(e.bytes(), coords).into());
            list.insert(0, key);
        }
        defs.families.insert(FontFamily::Name(name.into()), list);
    };
    derived(&mut defs, CAPTION, FontFamily::Proportional, optical.map(|(e, o)| (e, o.caption)));
    // The display size only while headings use the same family's heading face.
    let display = optical.filter(|(e, _)| {
        roles.iter().any(|r| r.role == FontRoleKind::UiSemibold && r.source == Source::Embedded(e.heading()))
    });
    derived(&mut defs, DISPLAY, family(FontRoleKind::UiSemibold), display.map(|(e, o)| (e, o.display)));
    // Icon families: the icon font *first*, with egui's fallbacks after it.
    let with_fallbacks =
        |first: &str| std::iter::once(first.to_string()).chain(prop_defaults.iter().cloned()).collect();
    defs.families.insert(FontFamily::Name(ICONS.into()), with_fallbacks("phosphor"));
    defs.families.insert(FontFamily::Name(PHOSPHOR_FILL.into()), with_fallbacks(PHOSPHOR_FILL));
    // Font pickers (Settings → Fonts) ask for one family per usable embedded face, so
    // each name is drawn in its own font; only then (startup stays lean).
    let previews = PREVIEWS_WANTED.load(std::sync::atomic::Ordering::Relaxed);
    for e in Embedded::all().filter(|e| previews && e.usable()) {
        add_embedded(&mut defs, e);
        let mut list = vec![e.key().to_string()];
        list.extend(if e.font().mono { mono_defaults.iter() } else { prop_defaults.iter() }.cloned());
        defs.families.insert(preview_family_of(e), list);
    }
    // System CJK / Hangul / emoji fonts loaded so far (see `font_fallback`).
    super::font_fallback::append(&mut defs);
    defs
}

fn preview_family_of(e: Embedded) -> FontFamily {
    FontFamily::Name(format!("preview:{}", e.key()).into())
}

/// Preview families requested ([`enable_previews`]).
static PREVIEWS_WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The pass in which fonts with the preview families were first handed to egui (they
/// apply from the next pass on); `u64::MAX` = not yet.
static PREVIEWS_FROM: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
/// The `[fonts]` section last installed (to re-install with previews).
static LAST: Mutex<Option<Fonts>> = Mutex::new(None);

/// Add the preview families (one per embedded face) on the next frame, re-installing the
/// current fonts once. Called by font pickers; a no-op after the first call.
pub fn enable_previews(ctx: &Context) {
    if PREVIEWS_WANTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let last = LAST.lock().clone();
    if let Some(f) = last {
        install(ctx, &f);
        ctx.request_repaint();
    }
}

/// A family that draws `name` in its own font when that is cheap: embedded faces (each
/// has a preview family) and system fonts a role currently uses. `None` otherwise (the
/// caller draws the name in the interface font).
pub fn preview_family(ctx: &Context, name: &str) -> Option<FontFamily> {
    let from = PREVIEWS_FROM.load(std::sync::atomic::Ordering::Relaxed);
    if from == u64::MAX || ctx.cumulative_pass_nr() <= from {
        return None;
    }
    if let Some(e) = Embedded::find(name).filter(|e| e.usable()) {
        return Some(preview_family_of(e));
    }
    let want = sysfonts::norm(name);
    STATUS.lock().iter().find_map(|r| match &r.source {
        Source::System { family: f, weight: None, .. } if sysfonts::norm(f) == want => Some(family(r.role)),
        _ => None,
    })
}

/// Definitions with only the embedded fonts (tests, previews of the embedded look).
pub fn definitions() -> FontDefinitions {
    let mut fonts = Fonts::default();
    for role in FontRoleKind::ALL {
        fonts.role_mut(role).embedded = true;
    }
    let roles = resolve(&fonts, &|_| None, &|_, _| false);
    build(&fonts, &roles, &|_| None)
}

/// What the last [`install`] resolved (Settings → Fonts shows it).
static STATUS: Mutex<Vec<Resolved>> = Mutex::new(Vec::new());

/// The roles as last installed.
pub fn status() -> Vec<Resolved> {
    STATUS.lock().clone()
}

fn offsets_id() -> egui::Id {
    egui::Id::unique("soyuz.fonts.offsets")
}

/// Resolve `[fonts]`, install the fonts and remember the per-role size offsets.
/// Returns notices for fonts that are not installed.
pub fn install(ctx: &Context, fonts: &Fonts) -> Vec<String> {
    let t = std::time::Instant::now();
    // Recorded first: a re-install scheduled below must see this `[fonts]`, even when
    // the catalog becomes ready while this install is still running.
    *LAST.lock() = Some(fonts.clone());
    sysfonts::take_missed();
    let roles = resolve(fonts, &sysfonts::locate, &is_variable_file);
    if sysfonts::take_missed() {
        // Linux: a system font was asked for while the font catalog is still being built
        // in the background; install again (with the latest `[fonts]`) once it is, on
        // the UI thread at the start of the next frame ([`begin_frame`]). The hook only
        // flags it: it may run right away (catalog just finished), inside this install.
        let ctx = ctx.clone();
        sysfonts::when_catalog_ready(move || {
            REINSTALL_DUE.store(true, std::sync::atomic::Ordering::SeqCst);
            ctx.request_repaint();
        });
    }
    let defs = build(fonts, &roles, &load_file);
    ctx.set_fonts(defs);
    if PREVIEWS_WANTED.load(std::sync::atomic::Ordering::Relaxed)
        && PREVIEWS_FROM.load(std::sync::atomic::Ordering::Relaxed) == u64::MAX
    {
        PREVIEWS_FROM.store(ctx.cumulative_pass_nr(), std::sync::atomic::Ordering::Relaxed);
    }
    let offsets: [f32; 6] = std::array::from_fn(|i| fonts.role(FontRoleKind::ALL[i]).offset());
    ctx.data_mut(|d| d.insert_temp(offsets_id(), offsets));
    log::debug!(
        "fonts: {} in {:.1} ms",
        roles.iter().map(|r| format!("{}={}", r.role.key(), r.source.describe())).collect::<Vec<_>>().join(", "),
        t.elapsed().as_secs_f64() * 1000.0
    );
    let notices =
        roles.iter().filter_map(|r| r.notice.as_ref().map(|n| format!("[fonts.{}] {n}", r.role.key()))).collect();
    *STATUS.lock() = roles;
    notices
}

/// The system font catalog became ready after an install that needed it.
static REINSTALL_DUE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Re-install the last `[fonts]` when the system font catalog became ready since (call
/// once per frame, on the UI thread, before anything is drawn).
pub fn begin_frame(ctx: &Context) {
    if !REINSTALL_DUE.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let last = LAST.lock().clone();
    if let Some(fonts) = last {
        let _ = install(ctx, &fonts);
        ctx.request_repaint();
    }
}

/// `[fonts.<role>] size_offset` (points) as installed.
pub fn offset(ctx: &Context, role: FontRoleKind) -> f32 {
    let i = FontRoleKind::ALL.iter().position(|r| *r == role).unwrap_or(0);
    ctx.data(|d| d.get_temp::<[f32; 6]>(offsets_id())).map_or(0.0, |o| o[i])
}

/// The font of `role` at `size` points plus the role's size offset.
pub fn role_font(ctx: &Context, role: FontRoleKind, size: f32) -> FontId {
    FontId::new((size + offset(ctx, role)).max(4.0), family(role))
}

/// A semibold heading at `size` points: large sizes (17 pt and up) use the [`DISPLAY`]
/// family, so a variable ui font draws them at its display optical size.
pub fn heading(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(if size >= 17.0 { DISPLAY } else { SEMIBOLD }.into()))
}

/// Terminal grid font.
pub fn terminal(ctx: &Context, size: f32) -> FontId {
    role_font(ctx, FontRoleKind::Terminal, size)
}

/// Text editor / code font.
pub fn code(ctx: &Context, size: f32) -> FontId {
    role_font(ctx, FontRoleKind::Code, size)
}

/// Hex view font.
pub fn hex(ctx: &Context, size: f32) -> FontId {
    role_font(ctx, FontRoleKind::Hex, size)
}

/// Numbers (sizes, dates, counters, progress).
pub fn numbers(ctx: &Context, size: f32) -> FontId {
    role_font(ctx, FontRoleKind::Numbers, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_locate(have: &'static [&'static str]) -> impl Fn(&str) -> Option<(PathBuf, u32)> {
        move |name: &str| {
            have.iter()
                .find(|h| sysfonts::norm(h) == sysfonts::norm(name))
                .map(|h| (PathBuf::from(format!("C:/F/{}.ttf", sysfonts::norm(h))), 0))
        }
    }

    #[test]
    fn catalog_ready_reinstalls_on_the_next_frame() {
        let ctx = Context::default();
        let fonts = Fonts::default();
        let _ = install(&ctx, &fonts);
        assert!(LAST.lock().is_some(), "recorded by install");
        REINSTALL_DUE.store(true, std::sync::atomic::Ordering::SeqCst);
        begin_frame(&ctx);
        assert!(!REINSTALL_DUE.load(std::sync::atomic::Ordering::SeqCst), "handled once, on the frame");
    }

    fn src(r: &[Resolved], role: FontRoleKind) -> Source {
        r.iter().find(|x| x.role == role).unwrap().source.clone()
    }

    fn name(s: &Source) -> &str {
        match s {
            Source::System { family, .. } => family,
            Source::Embedded(e) => e.label(),
        }
    }

    fn system_fonts() -> Fonts {
        let mut f = Fonts::default();
        f.ui.family = "Segoe UI".into();
        for role in [FontRoleKind::Terminal, FontRoleKind::Code, FontRoleKind::Hex] {
            f.role_mut(role).family = "Consolas".into();
        }
        f
    }

    #[test]
    fn defaults_are_embedded_and_need_no_disk() {
        let windows = fake_locate(&["Segoe UI", "Segoe UI Semibold", "Consolas", "Cascadia Mono", "Inter"]);
        let nothing = |_: &str| None;
        for locate in [&windows as Locate<'_>, &nothing] {
            let r = resolve(&Fonts::default(), locate, &|_, _| false);
            assert_eq!(r.len(), 6);
            assert_eq!(src(&r, FontRoleKind::Ui), Source::Embedded(Embedded::INTER), "embedded wins over installed");
            assert_eq!(src(&r, FontRoleKind::UiSemibold), Source::Embedded(Embedded::INTER_SEMIBOLD));
            for role in [FontRoleKind::Terminal, FontRoleKind::Code, FontRoleKind::Hex] {
                assert_eq!(src(&r, role), Source::Embedded(Embedded::CASCADIA_MONO));
            }
            assert_eq!(src(&r, FontRoleKind::Numbers), src(&r, FontRoleKind::Ui), "numbers follow the ui font");
            assert!(r.iter().all(|x| x.notice.is_none()));
        }
        assert_eq!(Embedded::for_role(FontRoleKind::Ui), Embedded::INTER);
        assert_eq!(Embedded::for_role(FontRoleKind::UiSemibold), Embedded::INTER_SEMIBOLD);
        assert_eq!(Embedded::for_role(FontRoleKind::Hex), Embedded::CASCADIA_MONO);
    }

    #[test]
    fn system_fonts_resolve_and_fall_back_in_order() {
        let loc = fake_locate(&["Segoe UI", "Segoe UI Semibold", "Consolas"]);
        let r = resolve(&system_fonts(), &loc, &|_, _| false);
        assert_eq!(name(&src(&r, FontRoleKind::Ui)), "Segoe UI");
        assert!(matches!(src(&r, FontRoleKind::Ui), Source::System { .. }));
        assert_eq!(name(&src(&r, FontRoleKind::UiSemibold)), "Segoe UI Semibold");
        assert_eq!(name(&src(&r, FontRoleKind::Terminal)), "Consolas");
        // No Consolas: the embedded Cascadia Mono, with a notice.
        let loc = fake_locate(&["Segoe UI"]);
        let r = resolve(&system_fonts(), &loc, &|_, _| false);
        assert_eq!(src(&r, FontRoleKind::Code), Source::Embedded(Embedded::CASCADIA_MONO));
        let t = r.iter().find(|x| x.role == FontRoleKind::Terminal).unwrap();
        assert!(t.notice.as_deref().unwrap().contains("Consolas is not installed"));
        // Semibold missing: no variable font, so the regular face, with a notice.
        let s = r.iter().find(|x| x.role == FontRoleKind::UiSemibold).unwrap();
        assert_eq!(name(&s.source), "Segoe UI");
        assert!(s.notice.is_some());
        // An unknown ui font: the embedded Inter.
        let mut f = Fonts::default();
        f.ui.family = "No Such Sans".into();
        let r = resolve(&f, &|_| None, &|_, _| false);
        assert_eq!(src(&r, FontRoleKind::Ui), Source::Embedded(Embedded::INTER));
        assert_eq!(src(&r, FontRoleKind::UiSemibold), Source::Embedded(Embedded::INTER_SEMIBOLD));
    }

    #[test]
    fn every_embedded_family_is_selectable() {
        let mut f = Fonts::default();
        f.ui.family = "ibm plex sans".into();
        f.terminal.family = "JetBrains Mono Regular".into();
        f.code.family = "Source Code Pro".into();
        f.hex.family = "IBM Plex Mono".into();
        f.numbers.family = "Noto Sans".into();
        let r = resolve(&f, &|_| None, &|_, _| false);
        assert_eq!(name(&src(&r, FontRoleKind::Ui)), "IBM Plex Sans");
        assert_eq!(name(&src(&r, FontRoleKind::UiSemibold)), "IBM Plex Sans SemiBold", "the family's heading face");
        assert_eq!(name(&src(&r, FontRoleKind::Terminal)), "JetBrains Mono");
        assert_eq!(name(&src(&r, FontRoleKind::Code)), "Source Code Pro");
        assert_eq!(name(&src(&r, FontRoleKind::Hex)), "IBM Plex Mono");
        assert_eq!(name(&src(&r, FontRoleKind::Numbers)), "Noto Sans");
        assert!(r.iter().all(|x| matches!(x.source, Source::Embedded(_)) && x.notice.is_none()));
        for e in Embedded::all() {
            assert_eq!(Embedded::find(e.font().name), Some(e));
            assert!(e.font().heading.is_none_or(|h| h.font().family == e.font().family && !h.font().mono));
        }
    }

    #[test]
    fn variable_ui_font_gets_a_semibold_weight() {
        let loc = fake_locate(&["Segoe UI Variable"]);
        let mut f = Fonts::default();
        f.ui.family = "Segoe UI Variable".into();
        let r = resolve(&f, &loc, &|_, _| true);
        match src(&r, FontRoleKind::UiSemibold) {
            Source::System { family, weight, .. } => {
                assert_eq!(family, "Segoe UI Variable");
                assert_eq!(weight, Some(600.0));
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn embedded_switch_and_explicit_roles() {
        let loc = fake_locate(&["Segoe UI", "Segoe UI Semibold", "Consolas", "Fira Code"]);
        let mut f = system_fonts();
        f.ui.embedded = true;
        f.code.family = "Fira Code".into();
        f.numbers.family = "Consolas".into();
        f.hex.family = "IBM Plex Mono".into();
        f.hex.embedded = true;
        f.terminal.family = "Fira Code".into();
        f.terminal.embedded = true;
        let r = resolve(&f, &loc, &|_, _| false);
        assert_eq!(src(&r, FontRoleKind::Ui), Source::Embedded(Embedded::INTER), "Segoe UI is not embedded");
        assert_eq!(src(&r, FontRoleKind::UiSemibold), Source::Embedded(Embedded::INTER_SEMIBOLD), "follows ui");
        assert_eq!(name(&src(&r, FontRoleKind::Code)), "Fira Code");
        assert_eq!(name(&src(&r, FontRoleKind::Numbers)), "Consolas");
        assert_eq!(name(&src(&r, FontRoleKind::Hex)), "IBM Plex Mono", "a named embedded font");
        assert_eq!(src(&r, FontRoleKind::Terminal), Source::Embedded(Embedded::CASCADIA_MONO));
        assert_eq!(candidates(&f, FontRoleKind::Code), ["Fira Code", "Cascadia Mono"]);
        f.code.family = "cascadia mono".into();
        assert_eq!(candidates(&f, FontRoleKind::Code), ["cascadia mono"], "deduplicated");
    }

    #[test]
    fn families_keep_icons_first_and_fallbacks_after() {
        let f = definitions();
        let prop = &f.families[&FontFamily::Proportional];
        assert_eq!(prop[0], "Inter");
        assert_eq!(prop[1], "phosphor");
        assert_eq!(f.families[&FontFamily::Monospace][0], "Cascadia Mono");
        assert_eq!(f.families[&FontFamily::Name(TERMINAL.into())][0], "Cascadia Mono");
        assert_eq!(f.families[&FontFamily::Name(HEX.into())][0], "Cascadia Mono");
        assert_eq!(f.families[&FontFamily::Name(NUMBERS.into())][0], "Inter");
        assert_eq!(f.families[&FontFamily::Name(SEMIBOLD.into())][0], "Inter SemiBold");
        assert_eq!(f.families[&FontFamily::Name(ICONS.into())][0], "phosphor");
        assert_eq!(f.families[&FontFamily::Name(PHOSPHOR_FILL.into())][0], PHOSPHOR_FILL);
        assert_eq!(f.families[&FontFamily::Name(CAPTION.into())][0], "Inter", "static fonts: same as the ui list");
        assert_eq!(f.families[&FontFamily::Name(DISPLAY.into())][0], "Inter SemiBold");
        // Only the fonts in use are handed to egui.
        assert!(!f.font_data.contains_key("Noto Sans") && !f.font_data.contains_key("JetBrains Mono"));
        assert!(!f.font_data.contains_key("Inter Variable"));
    }

    #[test]
    fn inter_variable_has_optical_size_instances() {
        use skrifa::MetadataProvider as _;
        let font = skrifa::FontRef::new(Embedded::INTER_VARIABLE.bytes()).unwrap();
        let axis = |tag: &[u8; 4]| {
            font.axes().iter().find(|a| a.tag().to_be_bytes() == *tag).map(|a| (a.min_value(), a.max_value()))
        };
        assert_eq!(axis(b"opsz"), Some((14.0, 32.0)), "Inter 4.1: opsz 14 (text) to 32 (display)");
        assert_eq!(axis(b"wght"), Some((100.0, 900.0)));
        assert!(Embedded::INTER_VARIABLE.usable() && Embedded::INTER_VARIABLE_SEMIBOLD.usable());
        assert!(Embedded::INTER.usable(), "static faces need no axes");
        assert_eq!(Embedded::find("inter variable"), Some(Embedded::INTER_VARIABLE));
        assert_eq!(Embedded::INTER_VARIABLE.heading(), Embedded::INTER_VARIABLE_SEMIBOLD);
        assert_eq!(Embedded::INTER_VARIABLE.label(), "Inter Variable (optical sizes)");

        let mut fonts = Fonts::default();
        fonts.ui.family = "Inter Variable".into();
        let roles = resolve(&fonts, &|_| None, &|_, _| false);
        assert_eq!(src(&roles, FontRoleKind::UiSemibold), Source::Embedded(Embedded::INTER_VARIABLE_SEMIBOLD));
        let defs = build(&fonts, &roles, &|_| None);
        let fam = |n: &str| defs.families[&FontFamily::Name(n.into())].clone();
        assert_eq!(defs.families[&FontFamily::Proportional][..2], ["Inter Variable", "Inter"], "static Inter after it");
        assert_eq!(fam(SEMIBOLD)[0], "Inter Variable SemiBold");
        assert_eq!(fam(CAPTION)[..2], ["Inter Variable caption", "Inter Variable"]);
        assert_eq!(fam(DISPLAY)[..2], ["Inter Variable display", "Inter Variable SemiBold"]);
        let coords = |k: &str| defs.font_data[k].tweak.coords.as_ref().to_vec();
        let tag = |t: &[u8; 4]| skrifa::raw::types::Tag::new(t);
        assert_eq!(coords("Inter Variable"), [(tag(b"opsz"), 14.0), (tag(b"wght"), 400.0)]);
        assert_eq!(coords("Inter Variable display"), [(tag(b"opsz"), 20.0), (tag(b"wght"), 600.0)]);
        // Headings set to another font: no display instance.
        fonts.ui_semibold.family = "IBM Plex Sans SemiBold".into();
        let roles = resolve(&fonts, &|_| None, &|_, _| false);
        let defs = build(&fonts, &roles, &|_| None);
        assert_eq!(defs.families[&FontFamily::Name(DISPLAY.into())][0], "IBM Plex Sans SemiBold");
        assert_eq!(defs.families[&FontFamily::Name(CAPTION.into())][0], "Inter Variable caption");
    }

    #[test]
    fn system_fonts_go_first_and_are_shared() {
        static BYTES: &[u8] = b"not parsed here";
        let loc = fake_locate(&["Segoe UI", "Segoe UI Semibold", "Consolas"]);
        let fonts = system_fonts();
        let roles = resolve(&fonts, &loc, &|_, _| false);
        let defs = build(&fonts, &roles, &|_| Some(Arc::new(BYTES) as Blob));
        let prop = &defs.families[&FontFamily::Proportional];
        assert!(prop[0].starts_with("sys:") && prop[0].contains("segoeui"));
        assert_eq!(prop[1], "Inter");
        assert_eq!(prop[2], "phosphor");
        let mono = &defs.families[&FontFamily::Monospace];
        assert_eq!(&defs.families[&FontFamily::Name(TERMINAL.into())][0], &mono[0], "one Consolas for every mono role");
        assert_eq!(mono[1], "Cascadia Mono", "then the embedded fallback");
        assert_eq!(defs.families[&FontFamily::Name(ICONS.into())][0], "phosphor");
        assert_eq!(defs.font_data.keys().filter(|k| k.starts_with("sys:")).count(), 3);
        // Unreadable file: the embedded fallback takes the first slot.
        let defs = build(&fonts, &roles, &|_| None);
        assert_eq!(defs.families[&FontFamily::Proportional][0], "Inter");
    }

    #[test]
    fn hinting_strengths_map_to_targets() {
        let t = |s| match hinting_target(s) {
            HintingTarget::Smooth(h) => (h.light, h.symmetric_rendering, h.preserve_linear_metrics),
            HintingTarget::Mono => panic!("mono"),
        };
        assert_eq!(t(HintingStrength::Crisp), (false, false, false));
        assert_eq!(hinting_target(HintingStrength::Normal), HintingTarget::default());
        assert!(t(HintingStrength::Light).0);
    }

    /// Every embedded font parses and has the glyphs of the Turkish UI (dotted and
    /// dotless i, ş, ğ, ç, ö, ü, typographic punctuation) plus, for monospace faces, the
    /// terminal's box drawing and one advance for every ASCII glyph.
    #[test]
    fn embedded_fonts_cover_turkish_and_parse() {
        use skrifa::MetadataProvider as _;
        for e in Embedded::all() {
            let f = e.font();
            let font = skrifa::FontRef::new(f.bytes).unwrap_or_else(|err| panic!("{}: {err}", f.name));
            let cmap = font.charmap();
            let mut need: Vec<char> = "ıİşŞğĞçÇöÖüÜâîû€·…—–“”‘’".chars().collect();
            need.extend('!'..='~');
            if f.mono {
                need.extend("─│┌┐└┘├┤┬┴┼═║▀▄█░▒▓".chars());
            }
            for c in need {
                assert!(cmap.map(c).is_some(), "{} has no glyph for {c:?} (U+{:04X})", f.name, c as u32);
            }
            if f.mono {
                let size = skrifa::instance::Size::unscaled();
                let metrics = font.glyph_metrics(size, skrifa::instance::LocationRef::default());
                let adv = |c| metrics.advance_width(cmap.map(c).unwrap());
                assert!(('!'..='~').all(|c| adv(c) == adv('M')), "{} is not monospaced", f.name);
            }
        }
    }

    /// Segoe UI must not define glyphs at Phosphor's codepoints (Inter did, in the PUA);
    /// icons use the `icons` family anyway, this guards plain-text icon fallbacks.
    #[cfg(windows)]
    #[test]
    fn segoe_ui_does_not_shadow_phosphor() {
        use crate::theme::icons::ph;
        use skrifa::MetadataProvider as _;
        let Some((path, idx)) = sysfonts::locate("Segoe UI") else { return };
        let bytes = std::fs::read(path).unwrap();
        let font = skrifa::FontRef::from_index(&bytes, idx).unwrap();
        let cmap = font.charmap();
        for icon in [ph::FOLDER, ph::FILE, ph::GEAR_SIX, ph::TERMINAL_WINDOW, ph::MAGNIFYING_GLASS, ph::X] {
            let c = icon.chars().next().unwrap();
            assert!(cmap.map(c).is_none(), "Segoe UI has a glyph at U+{:04X}", c as u32);
        }
    }
}
