//! Localization: Project Fluent bundles for every UI string.
//!
//! * Translations live in `crates/soyuz-appcore/locales/<lang>/<area>.ftl` and are embedded
//!   at compile time (`include_str!`, see [`FILES`]). en-US is the source language and
//!   the fallback for any key a translation lacks.
//! * [`tr!`](crate::tr) is the API used by the UI:
//!   - `tr!("toolbar-back")` → `&'static str` (formatted once per language and interned,
//!     so it drops into APIs that want `&'static str`);
//!   - `tr!("toast-copied", count = n, name = file)` → `String` (Fluent arguments; numbers
//!     select plural variants and are formatted with the locale's digit grouping).
//! * Keys built at runtime (one per enum variant, e.g. `action-{name}`) go through
//!   [`t_dyn`] / [`t_dyn_args`]; mark them with an `// i18n-dynamic: <prefix>` comment
//!   so the key-coverage test knows the prefix is used.
//! * Missing keys fall back to en-US, then to the key itself; debug builds log a warning
//!   once per missing key.
//! * The current language is the process-wide [`soyuz_core::locale`] (which also drives
//!   number / size / date formatting and collation); [`apply_setting`] switches it live.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::types::FluentNumber;
use fluent_bundle::{FluentResource, FluentValue};
use intl_memoizer::concurrent::IntlLangMemoizer;
use parking_lot::{Mutex, RwLock};
use soyuz_core::Locale;
use unic_langid::LanguageIdentifier;

pub use fluent_bundle::FluentArgs;

macro_rules! ftl {
    ($area:literal) => {
        (
            $area,
            include_str!(concat!("../../locales/en-US/", $area, ".ftl")),
            include_str!(concat!("../../locales/tr-TR/", $area, ".ftl")),
        )
    };
}

/// `(area, en-US source, tr-TR source)` for every embedded `.ftl` file.
pub const FILES: &[(&str, &str, &str)] = &[
    ftl!("common"),
    ftl!("core"),
    ftl!("app"),
    ftl!("actions"),
    ftl!("views"),
    ftl!("dialogs"),
    ftl!("settings"),
    ftl!("panels"),
    ftl!("ops"),
    ftl!("tools"),
    ftl!("ai"),
    ftl!("automation"),
    ftl!("workspaces"),
    ftl!("editors"),
    ftl!("services"),
    ftl!("remote"),
    ftl!("tui"),
];

/// Translate a key: `tr!("key")` → `&'static str`; `tr!("key", name = value, …)` →
/// `String`. Argument names are Fluent variables (`{ $name }`); values may be strings,
/// `&String`, `Cow<str>` or numbers.
#[macro_export]
macro_rules! tr {
    ($key:literal $(,)?) => {
        $crate::i18n::t($key)
    };
    ($key:literal, $($name:ident = $val:expr),+ $(,)?) => {{
        let mut args = $crate::i18n::FluentArgs::new();
        $( args.set(stringify!($name), $crate::i18n::IntoArg::into_arg($val)); )+
        $crate::i18n::t_args($key, &args)
    }};
}

/// Values accepted as `tr!` arguments.
pub trait IntoArg {
    fn into_arg(self) -> FluentValue<'static>;
}

impl IntoArg for &str {
    fn into_arg(self) -> FluentValue<'static> {
        FluentValue::String(self.to_string().into())
    }
}
impl IntoArg for String {
    fn into_arg(self) -> FluentValue<'static> {
        FluentValue::String(self.into())
    }
}
impl IntoArg for &String {
    fn into_arg(self) -> FluentValue<'static> {
        FluentValue::String(self.clone().into())
    }
}
impl IntoArg for std::borrow::Cow<'_, str> {
    fn into_arg(self) -> FluentValue<'static> {
        FluentValue::String(self.into_owned().into())
    }
}
impl IntoArg for &std::borrow::Cow<'_, str> {
    fn into_arg(self) -> FluentValue<'static> {
        FluentValue::String(self.to_string().into())
    }
}
macro_rules! num_arg {
    ($($t:ty),*) => {$(
        impl IntoArg for $t {
            fn into_arg(self) -> FluentValue<'static> {
                FluentValue::from(self)
            }
        }
        impl IntoArg for &$t {
            fn into_arg(self) -> FluentValue<'static> {
                FluentValue::from(*self)
            }
        }
    )*};
}
num_arg!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, f32, f64);

type Bundle = FluentBundle<FluentResource>;

struct Catalog {
    en: Bundle,
    tr: Bundle,
}

fn catalog() -> &'static Catalog {
    static CAT: OnceLock<Catalog> = OnceLock::new();
    CAT.get_or_init(|| Catalog { en: build(Locale::EnUs), tr: build(Locale::TrTr) })
}

fn langid(l: Locale) -> LanguageIdentifier {
    l.tag().parse().unwrap_or_default()
}

/// Build the bundle for `l` from every embedded file (parse errors are logged; the
/// tests require the files to be clean).
fn build(l: Locale) -> Bundle {
    let mut b = FluentBundle::new_concurrent(vec![langid(l)]);
    // egui renders the Unicode bidi isolation marks Fluent adds around placeables as
    // boxes; layout direction is handled by the UI instead.
    b.set_use_isolating(false);
    b.set_formatter(Some(match l {
        Locale::EnUs => format_en,
        Locale::TrTr => format_tr,
    }));
    for (area, en, tr) in FILES {
        let src = match l {
            Locale::EnUs => *en,
            Locale::TrTr => *tr,
        };
        let res = match FluentResource::try_new(src.to_string()) {
            Ok(r) => r,
            Err((r, errs)) => {
                log::error!("i18n: {} parse errors in {}/{area}.ftl: {errs:?}", errs.len(), l.tag());
                r
            }
        };
        if let Err(errs) = b.add_resource(res) {
            log::error!("i18n: duplicate keys in {}/{area}.ftl: {errs:?}", l.tag());
        }
    }
    b
}

fn format_en(v: &FluentValue<'_>, _: &IntlLangMemoizer) -> Option<String> {
    format_number(v, Locale::EnUs)
}

fn format_tr(v: &FluentValue<'_>, _: &IntlLangMemoizer) -> Option<String> {
    format_number(v, Locale::TrTr)
}

/// Numbers in messages: locale digit grouping and decimal separator (`1.234,5` in
/// Turkish), unless the message asked for `NUMBER($n, useGrouping: "false")`.
fn format_number(v: &FluentValue<'_>, l: Locale) -> Option<String> {
    let FluentValue::Number(n) = v else { return None };
    Some(localize_number(n, l))
}

fn localize_number(n: &FluentNumber, l: Locale) -> String {
    let raw = n.as_string();
    let (sign, digits) = raw.strip_prefix('-').map_or(("", raw.as_ref()), |d| ("-", d));
    let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
    let mut out = String::from(sign);
    match int.parse::<u64>() {
        Ok(i) if n.options.use_grouping => out.push_str(&soyuz_core::locale::group_int_in(i, l)),
        _ => out.push_str(int),
    }
    if !frac.is_empty() {
        out.push(l.decimal_sep());
        out.push_str(frac);
    }
    out
}

/// Interned no-argument messages per locale.
type Interned = RwLock<[HashMap<String, &'static str>; 2]>;

fn interned() -> &'static Interned {
    static I: OnceLock<Interned> = OnceLock::new();
    I.get_or_init(|| RwLock::new([HashMap::new(), HashMap::new()]))
}

fn slot(l: Locale) -> usize {
    match l {
        Locale::EnUs => 0,
        Locale::TrTr => 1,
    }
}

/// The current UI language.
pub fn current() -> Locale {
    soyuz_core::locale::current()
}

/// Translate `key` (no arguments) in the current language.
pub fn t(key: &str) -> &'static str {
    t_in(key, current())
}

/// Translate a key computed at runtime (mark the call site with an
/// `// i18n-dynamic: <prefix>` comment).
pub fn t_dyn(key: &str) -> &'static str {
    t(key)
}

/// [`t_dyn`] with arguments.
pub fn t_dyn_args(key: &str, args: &FluentArgs<'_>) -> String {
    t_args(key, args)
}

/// [`t`] in an explicit language (tests, previews).
pub fn t_in(key: &str, l: Locale) -> &'static str {
    let i = slot(l);
    if let Some(s) = interned().read()[i].get(key) {
        return s;
    }
    let s: &'static str = Box::leak(format(key, None, l).into_boxed_str());
    interned().write()[i].insert(key.to_string(), s);
    s
}

/// Translate `key` with Fluent arguments in the current language.
pub fn t_args(key: &str, args: &FluentArgs<'_>) -> String {
    t_args_in(key, args, current())
}

/// [`t_args`] in an explicit language.
pub fn t_args_in(key: &str, args: &FluentArgs<'_>, l: Locale) -> String {
    format(key, Some(args), l)
}

/// Whether `key` exists in the bundle of `l` (no fallback).
pub fn has_in(key: &str, l: Locale) -> bool {
    bundle(l).has_message(key)
}

/// Whether `key` exists in the source (en-US) bundle.
pub fn has(key: &str) -> bool {
    has_in(key, Locale::EnUs)
}

fn bundle(l: Locale) -> &'static Bundle {
    let c = catalog();
    match l {
        Locale::EnUs => &c.en,
        Locale::TrTr => &c.tr,
    }
}

fn format(key: &str, args: Option<&FluentArgs<'_>>, l: Locale) -> String {
    let try_bundle = |b: &Bundle| -> Option<String> {
        let msg = b.get_message(key)?;
        let pattern = msg.value()?;
        let mut errors = vec![];
        let s = b.format_pattern(pattern, args, &mut errors).into_owned();
        if !errors.is_empty() {
            warn_once(
                &format!("{}:{key}", l.tag()),
                &format!("i18n: errors formatting {key} in {}: {errors:?}", l.tag()),
            );
        }
        Some(s)
    };
    if let Some(s) = try_bundle(bundle(l)) {
        return s;
    }
    if l != Locale::EnUs {
        warn_once(&format!("{}:{key}", l.tag()), &format!("i18n: {key} missing in {}, using en-US", l.tag()));
        if let Some(s) = try_bundle(bundle(Locale::EnUs)) {
            return s;
        }
    }
    warn_once(key, &format!("i18n: missing key {key}"));
    key.to_string()
}

/// Debug builds: log `msg` the first time `id` is seen.
fn warn_once(id: &str, msg: &str) {
    if cfg!(debug_assertions) {
        static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
        if SEEN.get_or_init(Default::default).lock().insert(id.to_string()) {
            log::warn!("{msg}");
        }
    }
}

/// Resolve `general.language` (`"auto"`, `"en-US"`, `"tr-TR"`; `"en"` / `"tr"` are
/// accepted too) to a supported locale. `auto` negotiates the OS UI language against
/// the available translations, defaulting to en-US.
pub fn resolve(setting: &str) -> Locale {
    let s = setting.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("auto") {
        return os_locale().unwrap_or_default();
    }
    negotiate(s).unwrap_or_default()
}

/// The OS UI language, if Soyuz has a translation for it (looked up once).
pub fn os_locale() -> Option<Locale> {
    static OS: OnceLock<Option<Locale>> = OnceLock::new();
    *OS.get_or_init(|| soyuz_fs::os_locale::ui_language().as_deref().and_then(negotiate))
}

/// Match a BCP-47 tag against the available locales (fluent-langneg filtering).
pub fn negotiate(tag: &str) -> Option<Locale> {
    use fluent_langneg::{NegotiationStrategy, negotiate_languages};
    let requested: LanguageIdentifier = tag.replace('_', "-").parse().ok()?;
    let available: Vec<LanguageIdentifier> = Locale::ALL.iter().map(|l| langid(*l)).collect();
    let got = negotiate_languages(&[requested], &available, None, NegotiationStrategy::Filtering);
    let first = got.first()?.to_string();
    Locale::ALL.into_iter().find(|l| l.tag() == first)
}

/// Switch the UI (strings, formatting, collation) to the language `setting` resolves
/// to. Cheap; call whenever the config is applied. Returns the active locale.
pub fn apply_setting(setting: &str) -> Locale {
    let l = resolve(setting);
    if l != soyuz_core::locale::current() {
        log::info!("i18n: language {}", l.tag());
        soyuz_core::locale::set_current(l);
        GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    l
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bumped on every language switch: caches of translated text (palette entries, built
/// menus, …) compare it to know when to rebuild.
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// Language choices for the settings picker: `(config value, label)`; the label of
/// `auto` names the detected OS language.
pub fn language_choices() -> Vec<(&'static str, String)> {
    let os = os_locale().unwrap_or_default();
    let mut v = vec![("auto", crate::tr!("settings-language-auto", lang = os.native_name()))];
    v.extend(Locale::ALL.iter().map(|l| (l.tag(), l.native_name().to_string())));
    v
}

pub mod labels;

#[cfg(test)]
mod tests;
