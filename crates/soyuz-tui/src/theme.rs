//! Colour roles of the terminal UI.
//!
//! Views never name a colour: they ask the theme for the [`Style`] of a [`Role`]. The
//! colours come from the same design tokens as the GUI (`soyuz_appcore::theme`): the
//! colour theme, user themes in `<config dir>/themes`, `accent = "system"` and the
//! `[theme.colors]` overrides, in the dark or light variant `[appearance] theme` picks.
//! How they reach the terminal depends on its [`ColorMode`]:
//!
//! * truecolor: the token's RGB (translucent tokens composited over `bg.window` first);
//! * 256 colours: the nearest entry of the xterm 6×6×6 cube or grey ramp (after the same
//!   compositing); the 16 user-themed entries are never used;
//! * 16 colours: a fixed per-role ANSI table on the terminal's own background;
//! * mono (`NO_COLOR`): [`TuiTheme::plain`], bold / reversed / underlined only — the look
//!   of Phase T1.
//!
//! The theme is rebuilt only when something it depends on changes
//! ([`TuiTheme::cached`]: `[appearance]` / `[theme]` / `[tui]` edits flagged by
//! `TuiHost::take_palette_dirty`, the session's dark / light choice, or — while
//! following the OS — the OS appearance, re-read every few seconds).

use std::cell::RefCell;
use std::time::{Duration, Instant};

use ratatui::style::{Color, Modifier, Style};
use soyuz_appcore::color::Rgba8;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::file_class::FileClass;
use soyuz_appcore::host::ThemePref;
use soyuz_appcore::models::toasts::ToastKind;
use soyuz_appcore::theme::tokens::blend_over;
use soyuz_appcore::theme::{AccentChoice, Tokens};
use soyuz_core::config::{TuiBorders, TuiColorMode, TuiSection};

use crate::icons::IconSet;

/// What a piece of text or a cell is, for styling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Ordinary text.
    Text,
    /// Secondary text (sizes, dates, counts, "this folder is empty").
    Muted,
    /// Barely there (separators, placeholders).
    Faint,
    /// Accent (links, matches, the active tab marker).
    Accent,
    /// The location line / titles.
    Title,
    /// Key hints (dialog footers, status hints).
    KeyHint,
    /// A selected row.
    Selected,
    /// The cursor row of the focused pane.
    Cursor,
    /// The cursor row of a pane without focus.
    CursorInactive,
    /// A group header in the list.
    GroupHeader,
    Border,
    BorderFocused,
    Success,
    Warning,
    Danger,
    Info,
    Folder,
    Image,
    Video,
    Audio,
    Archive,
    Code,
    /// Word-processor documents.
    Doc,
    Exe,
    /// PDF files.
    Pdf,
    /// Spreadsheets.
    Sheet,
    /// Presentations.
    Slides,
    /// Links (symlinks, shortcuts).
    Link,
    /// File names of other classes (text, fonts, unknown).
    File,
    /// A tag colour (`0..` in `[tags]` order).
    Tag(u8),
    /// The status line.
    StatusBar,
    /// The active tab.
    TabActive,
    /// The other tabs.
    TabInactive,
    /// An input field (address, filter, palette).
    Input,
    /// The palette's selected row.
    PaletteSelected,
}

impl Role {
    /// The role of a file name of class `c` (the same token colours as the GUI's
    /// file-type glyphs, `ui::views::icons::class_color`).
    pub fn of_class(c: FileClass) -> Role {
        match c {
            FileClass::Folder => Role::Folder,
            FileClass::Image => Role::Image,
            FileClass::Video => Role::Video,
            FileClass::Audio => Role::Audio,
            FileClass::Archive => Role::Archive,
            FileClass::Code => Role::Code,
            FileClass::Pdf => Role::Pdf,
            FileClass::Doc => Role::Doc,
            FileClass::Sheet => Role::Sheet,
            FileClass::Slides => Role::Slides,
            FileClass::Exe => Role::Exe,
            FileClass::Link => Role::Link,
            FileClass::Text | FileClass::Font | FileClass::Other => Role::File,
        }
    }

    /// The role of a toast of kind `k`.
    pub fn of_toast(k: ToastKind) -> Role {
        match k {
            ToastKind::Info => Role::Info,
            ToastKind::Success => Role::Success,
            ToastKind::Warning => Role::Warning,
            ToastKind::Error => Role::Danger,
        }
    }
}

/// `[tui] color_mode`: how many colours to use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorSetting {
    /// Detect from the environment ([`ColorMode::detect`]).
    #[default]
    Auto,
    Truecolor,
    Ansi256,
    Ansi16,
    Mono,
}

impl ColorSetting {
    /// The configured colour setting (`[tui] color_mode`).
    pub fn from_config(tui: &TuiSection) -> ColorSetting {
        match tui.color_mode {
            TuiColorMode::Auto => ColorSetting::Auto,
            TuiColorMode::Truecolor => ColorSetting::Truecolor,
            TuiColorMode::Ansi256 => ColorSetting::Ansi256,
            TuiColorMode::Ansi16 => ColorSetting::Ansi16,
            TuiColorMode::Mono => ColorSetting::Mono,
        }
    }
}

/// The colour depth a theme is built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorMode {
    /// 24-bit RGB.
    Truecolor,
    /// The xterm 256-colour palette (cube and grey ramp).
    Ansi256,
    /// The 16 ANSI colours of the terminal's own scheme.
    Ansi16,
    /// No colours: modifiers only.
    Mono,
}

impl ColorMode {
    /// The mode for `setting`; [`ColorSetting::Auto`] asks [`Self::detect`] with `env`
    /// (an environment lookup, `std::env::var` in the app).
    pub fn resolve(setting: ColorSetting, env: &dyn Fn(&str) -> Option<String>) -> ColorMode {
        match setting {
            ColorSetting::Auto => ColorMode::detect(env),
            ColorSetting::Truecolor => ColorMode::Truecolor,
            ColorSetting::Ansi256 => ColorMode::Ansi256,
            ColorSetting::Ansi16 => ColorMode::Ansi16,
            ColorSetting::Mono => ColorMode::Mono,
        }
    }

    /// From the environment: `NO_COLOR` (non-empty) → mono; `COLORTERM` = `truecolor` /
    /// `24bit` or Windows Terminal (`WT_SESSION`) → truecolor; a `TERM` with `256color`
    /// → 256; anything else → 16.
    pub fn detect(env: &dyn Fn(&str) -> Option<String>) -> ColorMode {
        let set = |k: &str| env(k).filter(|v| !v.is_empty());
        if set("NO_COLOR").is_some() {
            return ColorMode::Mono;
        }
        let colorterm = set("COLORTERM").unwrap_or_default().to_ascii_lowercase();
        if colorterm == "truecolor" || colorterm == "24bit" || set("WT_SESSION").is_some() {
            return ColorMode::Truecolor;
        }
        if set("TERM").is_some_and(|t| t.contains("256color")) {
            return ColorMode::Ansi256;
        }
        ColorMode::Ansi16
    }
}

/// The token colours a theme draws with, already converted for its [`ColorMode`].
#[derive(Clone, Debug, PartialEq, Eq)]
struct Colors {
    window: Color,
    surface1: Color,
    surface3: Color,
    sunken: Color,
    border: Color,
    text: Color,
    text2: Color,
    text3: Color,
    accent: Color,
    on_accent: Color,
    /// `accent.subtle` composited over `bg.window`: the selection fill.
    selection: Color,
    success: Color,
    warning: Color,
    danger: Color,
    info: Color,
    folder: Color,
    image: Color,
    video: Color,
    audio: Color,
    archive: Color,
    code: Color,
    doc: Color,
    exe: Color,
    tags: [Color; 8],
}

/// The styles one frame draws with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiTheme {
    mode: ColorMode,
    dark: bool,
    icons: IconSet,
    /// The box lines (`[tui] borders`).
    borders: TuiBorders,
    /// `None` for the 16-colour and mono modes (fixed tables).
    colors: Option<Box<Colors>>,
}

impl Default for TuiTheme {
    fn default() -> Self {
        TuiTheme::plain()
    }
}

impl TuiTheme {
    /// Monochrome: the look of Phase T1 (bold and reversed only, terminal colours).
    pub fn plain() -> TuiTheme {
        TuiTheme {
            mode: ColorMode::Mono,
            dark: true,
            icons: IconSet::default(),
            borders: TuiBorders::default(),
            colors: None,
        }
    }

    /// The theme of the palette `t` in `mode`, with the icon set `icons`.
    pub fn from_tokens(t: &Tokens, mode: ColorMode, icons: IconSet) -> TuiTheme {
        let colors = match mode {
            ColorMode::Truecolor | ColorMode::Ansi256 => Some(Box::new(Colors::of(t, mode))),
            ColorMode::Ansi16 | ColorMode::Mono => None,
        };
        TuiTheme { mode, dark: t.dark, icons, borders: TuiBorders::default(), colors }
    }

    /// The theme for the core's current settings, the colour depth from `[tui]` or the
    /// environment. Builds from scratch (theme files, OS appearance): call through
    /// [`Self::cached`] once per frame.
    pub fn for_core(core: &AppCore, tui: &TuiSection) -> TuiTheme {
        let mode = ColorMode::resolve(ColorSetting::from_config(tui), &|k| std::env::var(k).ok());
        TuiTheme::for_core_in(core, tui, mode)
    }

    /// [`Self::for_core`] with the colour mode given.
    pub fn for_core_in(core: &AppCore, tui: &TuiSection, mode: ColorMode) -> TuiTheme {
        let icons = IconSet::from_config(tui);
        if mode == ColorMode::Mono {
            return TuiTheme { icons, ..TuiTheme::plain() }.with_borders(tui.borders);
        }
        let os = OsLook::read(core);
        let a = &core.config.appearance;
        let (dark, light, warnings) =
            soyuz_appcore::theme::resolve(a, &core.config.theme.colors, &soyuz_appcore::theme::themes_dir(), os.accent);
        for w in warnings {
            log::debug!("tui theme: {w}");
        }
        let is_dark = match core.ui.theme {
            ThemePref::Dark => true,
            ThemePref::Light => false,
            ThemePref::System => os.light != Some(true),
        };
        TuiTheme::from_tokens(if is_dark { &dark } else { &light }, mode, icons).with_borders(tui.borders)
    }

    /// This theme with the box lines `borders`.
    pub fn with_borders(self, borders: TuiBorders) -> TuiTheme {
        TuiTheme { borders, ..self }
    }

    /// This theme with the icon set `icons`.
    pub fn with_icons(self, icons: IconSet) -> TuiTheme {
        TuiTheme { icons, ..self }
    }

    /// The theme for this frame, rebuilt only when needed: `dirty` (the host saw an
    /// `[appearance]` / `[theme]` / `[tui]` change), a new dark / light choice, or a
    /// change of the OS appearance the settings follow (polled every few seconds).
    pub fn cached(core: &AppCore, dirty: bool) -> TuiTheme {
        thread_local! {
            static CACHE: RefCell<Option<Cached>> = const { RefCell::new(None) };
        }
        CACHE.with_borrow_mut(|c| {
            let pref = core.ui.theme;
            let stale = match c.as_mut() {
                None => true,
                Some(c) => {
                    let mut stale = dirty || c.pref != pref;
                    if !stale && follows_os(core) && c.polled.elapsed() >= OS_POLL {
                        c.polled = Instant::now();
                        stale = OsLook::read(core) != c.os;
                    }
                    stale
                }
            };
            if stale {
                let theme = TuiTheme::for_core(core, &core.config.tui);
                *c = Some(Cached { pref, os: OsLook::read(core), polled: Instant::now(), theme });
            }
            c.as_ref().map_or_else(TuiTheme::plain, |c| c.theme.clone())
        })
    }

    /// The colour depth this theme draws with.
    pub fn mode(&self) -> ColorMode {
        self.mode
    }

    /// Whether the dark variant is shown.
    pub fn is_dark(&self) -> bool {
        self.dark
    }

    /// The icon set (`[tui] icons`).
    pub fn icons(&self) -> IconSet {
        self.icons
    }

    /// The box lines (`[tui] borders`).
    pub fn borders(&self) -> TuiBorders {
        self.borders
    }

    /// Only ASCII is drawn: `[tui] icons = "ascii"` with `borders = "ascii"` or `"none"`.
    /// `view::draw` then also replaces the typographic punctuation of labels and
    /// translations (`…`, `·`, arrows, quotes) with ASCII look-alikes
    /// ([`crate::borders::asciify`]); other characters (names in other scripts) stay.
    pub fn ascii_only(&self) -> bool {
        self.icons == IconSet::Ascii && matches!(self.borders, TuiBorders::Ascii | TuiBorders::None)
    }

    /// The whole screen's base style: `bg.window` behind `text.primary` with colours, the
    /// terminal's own colours (`Color::Reset`) in the 16-colour and mono modes. Fill the
    /// frame with it before drawing.
    pub fn base(&self) -> Style {
        match &self.colors {
            Some(c) => Style::new().fg(c.text).bg(c.window),
            None => Style::new().fg(Color::Reset).bg(Color::Reset),
        }
    }

    /// The style of `role`.
    pub fn style(&self, role: Role) -> Style {
        match (&self.colors, self.mode) {
            (Some(c), _) => c.style(role),
            (None, ColorMode::Ansi16) => ansi16(role),
            (None, _) => plain(role),
        }
    }
}

const OS_POLL: Duration = Duration::from_secs(3);

struct Cached {
    pref: ThemePref,
    os: OsLook,
    polled: Instant,
    theme: TuiTheme,
}

/// Whether the current settings read the OS appearance.
fn follows_os(core: &AppCore) -> bool {
    core.ui.theme == ThemePref::System || AccentChoice::parse(&core.config.appearance.accent).0 == AccentChoice::System
}

/// The parts of the OS appearance a theme depends on (only read when the settings
/// follow the OS).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct OsLook {
    light: Option<bool>,
    accent: Option<Rgba8>,
}

impl OsLook {
    fn read(core: &AppCore) -> OsLook {
        let light = if core.ui.theme == ThemePref::System { soyuz_fs::os_theme::apps_use_light_theme() } else { None };
        let accent = if AccentChoice::parse(&core.config.appearance.accent).0 == AccentChoice::System {
            soyuz_appcore::theme::os_accent()
        } else {
            None
        };
        OsLook { light, accent }
    }
}

impl Colors {
    fn of(t: &Tokens, mode: ColorMode) -> Colors {
        let window = t.bg_window.opaque();
        let c = |x: Rgba8| to_color(blend_over(x, window), mode);
        let lum = 0.2126 * f32::from(t.accent.r) + 0.7152 * f32::from(t.accent.g) + 0.0722 * f32::from(t.accent.b);
        let on_accent = if lum > 165.0 { Rgba8::rgb(0x10, 0x13, 0x1A) } else { Rgba8::rgb(255, 255, 255) };
        Colors {
            window: c(window),
            surface1: c(t.bg_surface1),
            surface3: c(t.bg_surface3),
            sunken: c(t.bg_sunken),
            border: c(t.border_strong),
            text: c(t.text_primary),
            text2: c(t.text_secondary),
            text3: c(t.text_tertiary),
            accent: c(t.accent),
            on_accent: c(on_accent),
            selection: c(t.accent_subtle),
            success: c(t.success),
            warning: c(t.warning),
            danger: c(t.danger),
            info: c(t.info),
            folder: c(t.glyph_folder),
            image: c(t.glyph_image),
            video: c(t.glyph_video),
            audio: c(t.glyph_audio),
            archive: c(t.glyph_archive),
            code: c(t.glyph_code),
            doc: c(t.glyph_doc),
            exe: c(t.glyph_exe),
            tags: t.tags.map(c),
        }
    }

    fn style(&self, role: Role) -> Style {
        let fg = |c: Color| Style::new().fg(c);
        let bold = Modifier::BOLD;
        match role {
            Role::Text => fg(self.text),
            Role::Muted => fg(self.text2),
            Role::Faint => fg(self.text3),
            Role::Accent => fg(self.accent),
            Role::Title => fg(self.text).add_modifier(bold),
            Role::KeyHint => fg(self.accent).add_modifier(bold),
            Role::Selected | Role::PaletteSelected => fg(self.text).bg(self.selection),
            Role::Cursor => fg(self.on_accent).bg(self.accent),
            Role::CursorInactive => fg(self.text).bg(self.surface3),
            Role::GroupHeader => fg(self.text2).add_modifier(bold),
            Role::Border => fg(self.border),
            Role::BorderFocused => fg(self.accent),
            Role::Success => fg(self.success),
            Role::Warning => fg(self.warning).add_modifier(bold),
            Role::Danger => fg(self.danger).add_modifier(bold),
            Role::Info => fg(self.info),
            Role::Folder => fg(self.folder),
            Role::Image => fg(self.image),
            Role::Video => fg(self.video),
            Role::Audio => fg(self.audio),
            Role::Archive => fg(self.archive),
            Role::Code => fg(self.code),
            // The GUI's class colours (`class_color`): documents in the accent, PDFs in
            // danger, sheets in success, slides in warning, links in info.
            Role::Doc => fg(self.accent),
            Role::Pdf => fg(self.danger),
            Role::Sheet => fg(self.success),
            Role::Slides => fg(self.warning),
            Role::Link => fg(self.info),
            Role::Exe => fg(self.exe),
            Role::File => fg(self.doc),
            Role::Tag(i) => fg(self.tags[usize::from(i) % 8]),
            Role::StatusBar => fg(self.text2).bg(self.surface1),
            Role::TabActive => fg(self.text).bg(self.surface3).add_modifier(bold),
            Role::TabInactive => fg(self.text2),
            Role::Input => fg(self.text).bg(self.sunken),
        }
    }
}

/// A token colour for `mode` (opaque; composite translucent tokens first).
pub fn to_color(c: Rgba8, mode: ColorMode) -> Color {
    match mode {
        ColorMode::Truecolor => Color::Rgb(c.r, c.g, c.b),
        ColorMode::Ansi256 => Color::Indexed(nearest_256(c)),
        ColorMode::Ansi16 | ColorMode::Mono => Color::Reset,
    }
}

/// The xterm cube's channel levels.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The nearest xterm-256 entry (16..=255: the 6×6×6 cube or the 24-step grey ramp; the
/// 16 base colours follow the terminal's scheme and are never picked).
pub fn nearest_256(c: Rgba8) -> u8 {
    let d2 = |a: [u8; 3]| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a[0], c.r) + d(a[1], c.g) + d(a[2], c.b)
    };
    let level = |v: u8| (0..6).min_by_key(|&i| (i32::from(CUBE[i]) - i32::from(v)).abs()).unwrap_or(0);
    let (r, g, b) = (level(c.r), level(c.g), level(c.b));
    let cube = (16 + 36 * r + 6 * g + b) as u8;
    let cube_d = d2([CUBE[r], CUBE[g], CUBE[b]]);
    // Grey ramp 232..=255: 8, 18, …, 238.
    let avg = (u32::from(c.r) + u32::from(c.g) + u32::from(c.b)) / 3;
    let gi = ((avg.saturating_sub(3)) / 10).min(23) as u8;
    let (grey, grey_d) = [gi.saturating_sub(1), gi, (gi + 1).min(23)]
        .into_iter()
        .map(|i| {
            let v = 8 + 10 * i;
            (232 + i, d2([v, v, v]))
        })
        .min_by_key(|&(_, d)| d)
        .unwrap_or((232, i32::MAX));
    if grey_d < cube_d { grey } else { cube }
}

/// The 16-colour table: fixed ANSI colours per role on the terminal's own background
/// (`ls`-like file colours, reversed bars).
fn ansi16(role: Role) -> Style {
    let fg = |c: Color| Style::new().fg(c);
    let bold = Modifier::BOLD;
    match role {
        Role::Text | Role::Doc | Role::File | Role::TabInactive => Style::new(),
        Role::Muted | Role::Faint | Role::Border => fg(Color::DarkGray),
        Role::Accent | Role::BorderFocused => fg(Color::Blue),
        Role::Title | Role::GroupHeader => Style::new().add_modifier(bold),
        Role::KeyHint => fg(Color::Cyan).add_modifier(bold),
        Role::Selected => fg(Color::Yellow).add_modifier(bold),
        Role::Cursor | Role::StatusBar | Role::PaletteSelected => Style::new().add_modifier(Modifier::REVERSED),
        Role::TabActive => Style::new().add_modifier(Modifier::REVERSED | bold),
        Role::CursorInactive | Role::Input => Style::new().add_modifier(Modifier::UNDERLINED),
        Role::Success | Role::Exe | Role::Sheet => fg(Color::Green),
        Role::Warning => fg(Color::Yellow).add_modifier(bold),
        Role::Danger => fg(Color::Red).add_modifier(bold),
        Role::Info | Role::Audio | Role::Link => fg(Color::Cyan),
        Role::Folder => fg(Color::Blue).add_modifier(bold),
        Role::Image | Role::Video => fg(Color::Magenta),
        Role::Archive | Role::Pdf => fg(Color::Red),
        Role::Code | Role::Slides => fg(Color::Yellow),
        Role::Tag(i) => fg(match i % 8 {
            0 => Color::Red,
            1 => Color::Yellow,
            2 => Color::LightYellow,
            3 => Color::Green,
            4 => Color::Cyan,
            5 => Color::Blue,
            6 => Color::Magenta,
            _ => Color::DarkGray,
        }),
    }
}

/// [`TuiTheme::plain`]'s styles.
fn plain(role: Role) -> Style {
    let s = Style::new();
    match role {
        Role::Title | Role::KeyHint | Role::Selected | Role::Warning | Role::Danger | Role::GroupHeader => {
            s.add_modifier(Modifier::BOLD)
        }
        Role::Cursor | Role::StatusBar | Role::TabActive | Role::PaletteSelected => s.add_modifier(Modifier::REVERSED),
        Role::CursorInactive => s.add_modifier(Modifier::UNDERLINED),
        Role::Text
        | Role::Muted
        | Role::Faint
        | Role::Accent
        | Role::Border
        | Role::BorderFocused
        | Role::Success
        | Role::Info
        | Role::Folder
        | Role::Image
        | Role::Video
        | Role::Audio
        | Role::Archive
        | Role::Code
        | Role::Doc
        | Role::Exe
        | Role::Pdf
        | Role::Sheet
        | Role::Slides
        | Role::Link
        | Role::File
        | Role::Tag(_)
        | Role::TabInactive
        | Role::Input => s,
    }
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::theme::tokens::{ORBIT_DARK, ORBIT_LIGHT};

    use super::*;

    const ALL_ROLES: [Role; 35] = [
        Role::Text,
        Role::Muted,
        Role::Faint,
        Role::Accent,
        Role::Title,
        Role::KeyHint,
        Role::Selected,
        Role::Cursor,
        Role::CursorInactive,
        Role::GroupHeader,
        Role::Border,
        Role::BorderFocused,
        Role::Success,
        Role::Warning,
        Role::Danger,
        Role::Info,
        Role::Folder,
        Role::Image,
        Role::Video,
        Role::Audio,
        Role::Archive,
        Role::Code,
        Role::Doc,
        Role::Exe,
        Role::Pdf,
        Role::Sheet,
        Role::Slides,
        Role::Link,
        Role::File,
        Role::Tag(0),
        Role::Tag(7),
        Role::StatusBar,
        Role::TabActive,
        Role::TabInactive,
        Role::Input,
    ];

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn plain_is_monochrome() {
        let t = TuiTheme::plain();
        for r in ALL_ROLES {
            let s = t.style(r);
            assert_eq!((s.fg, s.bg), (None, None), "{r:?}");
        }
        assert!(t.style(Role::Cursor).add_modifier.contains(Modifier::REVERSED));
        assert!(t.style(Role::Selected).add_modifier.contains(Modifier::BOLD));
        assert_eq!(Role::of_class(FileClass::Pdf), Role::Pdf);
        assert_eq!(Role::of_class(FileClass::Text), Role::File);
        assert_eq!(Role::of_toast(ToastKind::Error), Role::Danger);
    }

    #[test]
    fn settings_from_the_config() {
        let tui = |color_mode| TuiSection { color_mode, ..TuiSection::default() };
        assert_eq!(ColorSetting::from_config(&TuiSection::default()), ColorSetting::Auto);
        assert_eq!(ColorSetting::from_config(&tui(TuiColorMode::Truecolor)), ColorSetting::Truecolor);
        assert_eq!(ColorSetting::from_config(&tui(TuiColorMode::Ansi256)), ColorSetting::Ansi256);
        assert_eq!(ColorSetting::from_config(&tui(TuiColorMode::Ansi16)), ColorSetting::Ansi16);
        assert_eq!(ColorSetting::from_config(&tui(TuiColorMode::Mono)), ColorSetting::Mono);
        // Pure ASCII only with ASCII icons and ASCII (or no) lines.
        let t = TuiTheme::plain();
        assert_eq!((t.icons(), t.borders()), (IconSet::Ascii, TuiBorders::Rounded));
        assert!(!t.ascii_only());
        assert!(t.clone().with_borders(TuiBorders::Ascii).ascii_only());
        assert!(t.clone().with_borders(TuiBorders::None).ascii_only());
        assert!(!t.with_borders(TuiBorders::Ascii).with_icons(IconSet::Unicode).ascii_only());
    }

    #[test]
    fn detection() {
        let d = |v: &[(&str, &str)]| ColorMode::detect(&env(v));
        assert_eq!(d(&[]), ColorMode::Ansi16);
        assert_eq!(d(&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")]), ColorMode::Mono);
        assert_eq!(d(&[("NO_COLOR", ""), ("COLORTERM", "truecolor")]), ColorMode::Truecolor, "empty NO_COLOR");
        assert_eq!(d(&[("COLORTERM", "24bit")]), ColorMode::Truecolor);
        assert_eq!(d(&[("WT_SESSION", "abc"), ("TERM", "xterm")]), ColorMode::Truecolor);
        assert_eq!(d(&[("TERM", "xterm-256color")]), ColorMode::Ansi256);
        assert_eq!(d(&[("TERM", "screen")]), ColorMode::Ansi16);
        let explicit = ColorMode::resolve(ColorSetting::Ansi256, &env(&[("NO_COLOR", "1")]));
        assert_eq!(explicit, ColorMode::Ansi256, "an explicit setting wins over the environment");
        assert_eq!(ColorMode::resolve(ColorSetting::Auto, &env(&[("NO_COLOR", "1")])), ColorMode::Mono);
    }

    #[test]
    fn nearest_256_known_colours() {
        assert_eq!(nearest_256(Rgba8::rgb(0, 0, 0)), 16);
        assert_eq!(nearest_256(Rgba8::rgb(255, 255, 255)), 231);
        assert_eq!(nearest_256(Rgba8::rgb(255, 0, 0)), 196);
        assert_eq!(nearest_256(Rgba8::rgb(0, 255, 0)), 46);
        assert_eq!(nearest_256(Rgba8::rgb(0, 0, 255)), 21);
        assert_eq!(nearest_256(Rgba8::rgb(95, 135, 175)), 67);
        assert_eq!(nearest_256(Rgba8::rgb(128, 128, 128)), 244, "grey ramp: 8 + 10 * 12");
        assert_eq!(nearest_256(Rgba8::rgb(8, 8, 8)), 232);
        assert_eq!(nearest_256(Rgba8::rgb(238, 238, 238)), 255);
        // Orbit's accent #5B8CFF → (95, 135, 255).
        assert_eq!(nearest_256(Rgba8::rgb(0x5B, 0x8C, 0xFF)), 69);
        // Orbit Dark's window #0F1115 is a near-black grey.
        assert_eq!(nearest_256(Rgba8::rgb(0x0F, 0x11, 0x15)), 233);
        for v in 0..=255u8 {
            assert!(nearest_256(Rgba8::rgb(v, v / 2, 255 - v)) >= 16);
        }
    }

    #[test]
    fn truecolor_uses_the_tokens_and_blends_alpha() {
        let t = TuiTheme::from_tokens(&ORBIT_DARK, ColorMode::Truecolor, IconSet::Unicode);
        let rgb = |c: Rgba8| Color::Rgb(c.r, c.g, c.b);
        assert_eq!(t.style(Role::Text).fg, Some(rgb(ORBIT_DARK.text_primary)));
        assert_eq!(t.base().bg, Some(rgb(ORBIT_DARK.bg_window)));
        assert_eq!(t.style(Role::Folder).fg, Some(rgb(ORBIT_DARK.glyph_folder)));
        assert_eq!(t.style(Role::Pdf).fg, Some(rgb(ORBIT_DARK.danger)));
        assert_eq!(t.style(Role::Doc).fg, Some(rgb(ORBIT_DARK.accent)));
        assert_eq!(t.style(Role::Tag(3)).fg, Some(rgb(ORBIT_DARK.tags[3])));
        // accent.subtle (#5B8CFF at alpha 46) over #0F1115.
        let a = f32::from(ORBIT_DARK.accent_subtle.a) / 255.0;
        let m = |x: u8, y: u8| (f32::from(x) * a + f32::from(y) * (1.0 - a)).round() as u8;
        let (s, w) = (ORBIT_DARK.accent_subtle, ORBIT_DARK.bg_window);
        let sel = Color::Rgb(m(s.r, w.r), m(s.g, w.g), m(s.b, w.b));
        assert_eq!(t.style(Role::Selected).bg, Some(sel));
        assert_ne!(sel, rgb(w), "the selection shows");
        assert_eq!(t.style(Role::Cursor).bg, Some(rgb(ORBIT_DARK.accent)));
        assert_eq!(t.style(Role::Cursor).fg, Some(Color::Rgb(255, 255, 255)));
        assert_eq!(t.icons(), IconSet::Unicode);
        let l = TuiTheme::from_tokens(&ORBIT_LIGHT, ColorMode::Truecolor, IconSet::Ascii);
        assert!(!l.is_dark());
        assert_eq!(l.base().bg, Some(rgb(ORBIT_LIGHT.bg_window)));
    }

    #[test]
    fn ansi256_quantizes_after_blending() {
        let t = TuiTheme::from_tokens(&ORBIT_DARK, ColorMode::Ansi256, IconSet::Ascii);
        for r in ALL_ROLES {
            for c in [t.style(r).fg, t.style(r).bg].into_iter().flatten() {
                assert!(matches!(c, Color::Indexed(16..=255)), "{r:?}: {c:?}");
            }
        }
        let sel = blend_over(ORBIT_DARK.accent_subtle, ORBIT_DARK.bg_window);
        assert_eq!(t.style(Role::Selected).bg, Some(Color::Indexed(nearest_256(sel))));
        assert_eq!(t.base().bg, Some(Color::Indexed(233)));
    }

    #[test]
    fn ansi16_table_keeps_the_terminal_background() {
        let t = TuiTheme::from_tokens(&ORBIT_DARK, ColorMode::Ansi16, IconSet::Ascii);
        assert_eq!(t.base(), Style::new().fg(Color::Reset).bg(Color::Reset));
        let expect = [
            (Role::Folder, Some(Color::Blue)),
            (Role::Exe, Some(Color::Green)),
            (Role::Archive, Some(Color::Red)),
            (Role::Image, Some(Color::Magenta)),
            (Role::Danger, Some(Color::Red)),
            (Role::Warning, Some(Color::Yellow)),
            (Role::Muted, Some(Color::DarkGray)),
            (Role::Tag(0), Some(Color::Red)),
            (Role::Tag(7), Some(Color::DarkGray)),
            (Role::Text, None),
        ];
        for (r, fg) in expect {
            assert_eq!(t.style(r).fg, fg, "{r:?}");
        }
        for r in ALL_ROLES {
            assert_eq!(t.style(r).bg, None, "{r:?}: no background colours in 16-colour mode");
        }
        assert!(t.style(Role::Cursor).add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn mono_is_plain() {
        let t = TuiTheme::from_tokens(&ORBIT_DARK, ColorMode::Mono, IconSet::Nerd);
        for r in ALL_ROLES {
            assert_eq!(t.style(r), plain(r), "{r:?}");
        }
        assert_eq!(t.base().bg, Some(Color::Reset));
    }
}
