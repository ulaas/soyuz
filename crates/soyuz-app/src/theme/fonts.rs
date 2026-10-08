//! Font roles (`[fonts]`): which font draws the interface, headings, terminal, code
//! editor, hex view and numbers, plus the Phosphor icon fonts.
//!
//! Each role resolves to a *primary* font — a system font read from the OS font folder
//! (Segoe UI, Consolas, Cascadia, …; never bundled: their licenses forbid
//! redistribution) or an embedded font — followed by the role's embedded fallback
//! (Inter / Inter SemiBold / JetBrains Mono, OFL, in `assets/fonts/`), egui's default
//! fonts and the system CJK / emoji fallbacks (`font_fallback`).
//!
//! egui families per role: `Proportional` (ui), [`SEMIBOLD`] (ui_semibold),
//! `Monospace` (code; also every other monospace text), [`TERMINAL`], [`HEX`] and
//! [`NUMBERS`]. Icons always use [`ICONS`] / [`PHOSPHOR_FILL`], whose first font is
//! Phosphor, so a text font with Private Use Area glyphs can never shadow an icon.
//!
//! The default system fonts (~2.4 MB: segoeui, seguisb, consola) are read synchronously
//! at startup — about a millisecond from the OS file cache — so the first frame already
//! uses them (no font flash). Bytes are cached for the process lifetime, so a later
//! re-install (settings change, fallback font loaded) does not touch the disk again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::epaint::text::{FontTweak, VariationCoords};
use egui::{Context, FontData, FontDefinitions, FontFamily, FontId};
use parking_lot::Mutex;
use soyuz_core::config::{FontRoleKind, Fonts};

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

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");

/// The fonts compiled into the binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Embedded {
    Inter,
    InterSemiBold,
    JetBrainsMono,
}

impl Embedded {
    pub fn key(self) -> &'static str {
        match self {
            Embedded::Inter => "Inter",
            Embedded::InterSemiBold => "Inter-SemiBold",
            Embedded::JetBrainsMono => "JetBrainsMono",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Embedded::Inter => "Inter",
            Embedded::InterSemiBold => "Inter SemiBold",
            Embedded::JetBrainsMono => "JetBrains Mono",
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Embedded::Inter => INTER_REGULAR,
            Embedded::InterSemiBold => INTER_SEMIBOLD,
            Embedded::JetBrainsMono => JETBRAINS_MONO,
        }
    }

    /// The embedded fallback of a role.
    pub fn for_role(role: FontRoleKind) -> Embedded {
        match role {
            FontRoleKind::UiSemibold => Embedded::InterSemiBold,
            r if r.is_mono() => Embedded::JetBrainsMono,
            _ => Embedded::Inter,
        }
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
        FontRoleKind::Terminal => vec![first(Fonts::DEFAULT_MONO), "Cascadia Mono".into()],
        FontRoleKind::Code => vec![first(Fonts::DEFAULT_MONO), "Cascadia Code".into()],
        FontRoleKind::Hex => vec![first(Fonts::DEFAULT_MONO), "Cascadia Mono".into()],
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
        let embedded = Source::Embedded(Embedded::for_role(role));
        if fonts.role(role).embedded {
            return Resolved { role, source: embedded, notice: None };
        }
        let names = candidates(fonts, role);
        for name in &names {
            if let Some((path, index)) = locate(name) {
                let notice =
                    (name != &names[0]).then(|| tr!("settings-font-not-installed", missing = &names[0], used = name));
                return Resolved {
                    role,
                    source: Source::System { family: name.clone(), path, index, weight: None },
                    notice,
                };
            }
        }
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

/// The semibold face that goes with the resolved ui font: `<family> Semibold` when
/// installed, else the same variable font at weight 600, else the regular face.
fn semibold_of(ui: &Resolved, locate: Locate<'_>, is_variable: IsVariable<'_>) -> Resolved {
    let role = FontRoleKind::UiSemibold;
    match &ui.source {
        Source::Embedded(_) => Resolved { role, source: Source::Embedded(Embedded::InterSemiBold), notice: None },
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

/// Font bytes of system files, read once per process.
static LOADED: Mutex<Option<HashMap<PathBuf, &'static [u8]>>> = Mutex::new(None);

/// The file's bytes (leaked once per distinct file: fonts live as long as the app).
fn load_static(path: &Path) -> Option<&'static [u8]> {
    if let Some(b) = LOADED.lock().get_or_insert_with(HashMap::new).get(path) {
        return Some(b);
    }
    let bytes = std::fs::read(path).ok()?;
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    LOADED.lock().get_or_insert_with(HashMap::new).insert(path.to_path_buf(), leaked);
    Some(leaked)
}

fn is_variable_file(path: &Path, index: u32) -> bool {
    let Some(bytes) = load_static(path) else { return false };
    let mut data = FontData::from_static(bytes);
    data.index = index;
    data.variation_axes().iter().any(|a| a.tag.to_be_bytes() == *b"wght")
}

fn tweak(fonts: &Fonts) -> FontTweak {
    FontTweak {
        hinting: Some(fonts.hinting),
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
pub fn build(fonts: &Fonts, roles: &[Resolved], load: &dyn Fn(&Path) -> Option<&'static [u8]>) -> FontDefinitions {
    let defaults = FontDefinitions::default();
    let prop_defaults = defaults.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mono_defaults = defaults.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let mut defs = FontDefinitions { font_data: defaults.font_data.clone(), families: Default::default() };
    let tw = tweak(fonts);
    for e in [Embedded::Inter, Embedded::InterSemiBold, Embedded::JetBrainsMono] {
        defs.font_data.insert(e.key().into(), FontData::from_static(e.bytes()).tweak(tw.clone()).into());
    }
    egui_phosphor::add_to_fonts(&mut defs, egui_phosphor::Variant::Regular);
    egui_phosphor::add_font_bytes_as_family(&mut defs, PHOSPHOR_FILL, egui_phosphor::Variant::Fill.font_bytes());
    for r in roles {
        let mut list: Vec<String> = Vec::new();
        if let Source::System { path, index, weight, .. } = &r.source
            && let Some(bytes) = load(path)
        {
            let key = r.source.key();
            if !defs.font_data.contains_key(&key) {
                let mut t = tw.clone();
                if let Some(w) = weight {
                    t.coords = VariationCoords::new([(b"wght", *w)]);
                }
                let mut data = FontData::from_static(bytes).tweak(t);
                data.index = *index;
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
    // Icon families: the icon font *first*, with egui's fallbacks after it.
    let with_fallbacks =
        |first: &str| std::iter::once(first.to_string()).chain(prop_defaults.iter().cloned()).collect();
    defs.families.insert(FontFamily::Name(ICONS.into()), with_fallbacks("phosphor"));
    defs.families.insert(FontFamily::Name(PHOSPHOR_FILL.into()), with_fallbacks(PHOSPHOR_FILL));
    // System CJK / Hangul / emoji fonts loaded so far (see `font_fallback`).
    super::font_fallback::append(&mut defs);
    defs
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
    egui::Id::new("soyuz.fonts.offsets")
}

/// Resolve `[fonts]`, install the fonts and remember the per-role size offsets.
/// Returns notices for fonts that are not installed.
pub fn install(ctx: &Context, fonts: &Fonts) -> Vec<String> {
    let t = std::time::Instant::now();
    let roles = resolve(fonts, &sysfonts::locate, &is_variable_file);
    let defs = build(fonts, &roles, &load_static);
    ctx.set_fonts(defs);
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

/// `[fonts.<role>] size_offset` (points) as installed.
pub fn offset(ctx: &Context, role: FontRoleKind) -> f32 {
    let i = FontRoleKind::ALL.iter().position(|r| *r == role).unwrap_or(0);
    ctx.data(|d| d.get_temp::<[f32; 6]>(offsets_id())).map_or(0.0, |o| o[i])
}

/// The font of `role` at `size` points plus the role's size offset.
pub fn role_font(ctx: &Context, role: FontRoleKind, size: f32) -> FontId {
    FontId::new((size + offset(ctx, role)).max(4.0), family(role))
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

    fn src(r: &[Resolved], role: FontRoleKind) -> Source {
        r.iter().find(|x| x.role == role).unwrap().source.clone()
    }

    fn sys_family(s: &Source) -> &str {
        match s {
            Source::System { family, .. } => family,
            Source::Embedded(e) => e.label(),
        }
    }

    #[test]
    fn windows_defaults_resolve_to_system_fonts() {
        let loc = fake_locate(&["Segoe UI", "Segoe UI Semibold", "Consolas", "Cascadia Mono", "Cascadia Code"]);
        let r = resolve(&Fonts::default(), &loc, &|_, _| false);
        assert_eq!(r.len(), 6);
        assert_eq!(sys_family(&src(&r, FontRoleKind::Ui)), "Segoe UI");
        assert_eq!(sys_family(&src(&r, FontRoleKind::UiSemibold)), "Segoe UI Semibold");
        assert_eq!(sys_family(&src(&r, FontRoleKind::Terminal)), "Consolas");
        assert_eq!(sys_family(&src(&r, FontRoleKind::Code)), "Consolas");
        assert_eq!(sys_family(&src(&r, FontRoleKind::Hex)), "Consolas");
        assert_eq!(src(&r, FontRoleKind::Numbers), src(&r, FontRoleKind::Ui), "numbers follow the ui font");
        assert!(r.iter().all(|x| x.notice.is_none()));
    }

    #[test]
    fn missing_fonts_fall_back_in_order() {
        // No Consolas: terminal → Cascadia Mono, code → Cascadia Code, hex → embedded.
        let loc = fake_locate(&["Segoe UI", "Cascadia Mono", "Cascadia Code"]);
        let mut f = Fonts::default();
        f.hex.family = "Consolas".into();
        let r = resolve(&f, &loc, &|_, _| false);
        assert_eq!(sys_family(&src(&r, FontRoleKind::Terminal)), "Cascadia Mono");
        assert_eq!(sys_family(&src(&r, FontRoleKind::Code)), "Cascadia Code");
        assert_eq!(sys_family(&src(&r, FontRoleKind::Hex)), "Cascadia Mono");
        let t = r.iter().find(|x| x.role == FontRoleKind::Terminal).unwrap();
        assert!(t.notice.as_deref().unwrap().contains("Consolas is not installed"));
        // Semibold missing: no variable font → the regular face, with a notice.
        let s = r.iter().find(|x| x.role == FontRoleKind::UiSemibold).unwrap();
        assert_eq!(sys_family(&s.source), "Segoe UI");
        assert!(s.notice.is_some());
        // Nothing installed (non-Windows): embedded fonts everywhere.
        let none = resolve(&Fonts::default(), &|_| None, &|_, _| false);
        assert_eq!(src(&none, FontRoleKind::Ui), Source::Embedded(Embedded::Inter));
        assert_eq!(src(&none, FontRoleKind::UiSemibold), Source::Embedded(Embedded::InterSemiBold));
        assert_eq!(src(&none, FontRoleKind::Terminal), Source::Embedded(Embedded::JetBrainsMono));
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
        let mut f = Fonts::default();
        f.ui.embedded = true;
        f.code.family = "Fira Code".into();
        f.numbers.family = "Consolas".into();
        let r = resolve(&f, &loc, &|_, _| false);
        assert_eq!(src(&r, FontRoleKind::Ui), Source::Embedded(Embedded::Inter));
        assert_eq!(src(&r, FontRoleKind::UiSemibold), Source::Embedded(Embedded::InterSemiBold), "follows ui");
        assert_eq!(sys_family(&src(&r, FontRoleKind::Code)), "Fira Code");
        assert_eq!(sys_family(&src(&r, FontRoleKind::Numbers)), "Consolas");
        assert_eq!(candidates(&f, FontRoleKind::Terminal), ["Consolas", "Cascadia Mono"]);
        f.terminal.family = "cascadia mono".into();
        assert_eq!(candidates(&f, FontRoleKind::Terminal), ["cascadia mono"], "deduplicated");
    }

    #[test]
    fn families_keep_icons_first_and_fallbacks_after() {
        let f = definitions();
        let prop = &f.families[&FontFamily::Proportional];
        assert_eq!(prop[0], "Inter");
        assert_eq!(prop[1], "phosphor");
        assert_eq!(f.families[&FontFamily::Monospace][0], "JetBrainsMono");
        assert_eq!(f.families[&FontFamily::Name(TERMINAL.into())][0], "JetBrainsMono");
        assert_eq!(f.families[&FontFamily::Name(HEX.into())][0], "JetBrainsMono");
        assert_eq!(f.families[&FontFamily::Name(NUMBERS.into())][0], "Inter");
        assert_eq!(f.families[&FontFamily::Name(SEMIBOLD.into())][0], "Inter-SemiBold");
        assert_eq!(f.families[&FontFamily::Name(ICONS.into())][0], "phosphor");
        assert_eq!(f.families[&FontFamily::Name(PHOSPHOR_FILL.into())][0], PHOSPHOR_FILL);
    }

    #[test]
    fn system_fonts_go_first_and_are_shared() {
        static BYTES: &[u8] = b"not parsed here";
        let loc = fake_locate(&["Segoe UI", "Segoe UI Semibold", "Consolas"]);
        let fonts = Fonts::default();
        let roles = resolve(&fonts, &loc, &|_, _| false);
        let defs = build(&fonts, &roles, &|_| Some(BYTES));
        let prop = &defs.families[&FontFamily::Proportional];
        assert!(prop[0].starts_with("sys:") && prop[0].contains("segoeui"));
        assert_eq!(prop[1], "Inter");
        assert_eq!(prop[2], "phosphor");
        let mono = &defs.families[&FontFamily::Monospace][0];
        assert_eq!(&defs.families[&FontFamily::Name(TERMINAL.into())][0], mono, "one Consolas for every mono role");
        assert_eq!(defs.families[&FontFamily::Name(ICONS.into())][0], "phosphor");
        assert_eq!(defs.font_data.keys().filter(|k| k.starts_with("sys:")).count(), 3);
        // Unreadable file: the embedded fallback takes the first slot.
        let defs = build(&fonts, &roles, &|_| None);
        assert_eq!(defs.families[&FontFamily::Proportional][0], "Inter");
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
