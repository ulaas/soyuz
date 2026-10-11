//! Desktop Entry files (freedesktop "Desktop Entry Specification" 1.5): parsing with
//! localized keys, visibility rules and the `Exec=` field-code expansion. Pure code,
//! tested on every OS; launching lives in `launcher.rs`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The `[Desktop Entry]` keys we use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DesktopEntry {
    pub kind: String,
    /// Localized when a matching `Name[…]` exists.
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    /// Raw value (string escapes not yet applied; see [`exec_tokens`]).
    pub exec: Option<String>,
    pub try_exec: Option<String>,
    /// `Path=`: working directory.
    pub path: Option<String>,
    pub url: Option<String>,
    pub icon: Option<String>,
    pub mime_types: Vec<String>,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
    pub no_display: bool,
    pub hidden: bool,
    pub terminal: bool,
    pub dbus_activatable: bool,
    pub startup_notify: bool,
}

/// The locale used for `Key[locale]` lookups, split as `lang_COUNTRY@MODIFIER`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Locale {
    pub lang: String,
    pub country: Option<String>,
    pub modifier: Option<String>,
}

impl Locale {
    /// Parse `ll_CC.ENCODING@mod` (the encoding is dropped). `C` / `POSIX` → `None`.
    pub fn parse(s: &str) -> Option<Locale> {
        let s = s.trim();
        if s.is_empty() || s == "C" || s == "POSIX" || s.starts_with("C.") {
            return None;
        }
        let (rest, modifier) = match s.split_once('@') {
            Some((r, m)) => (r, Some(m.to_string())),
            None => (s, None),
        };
        let rest = rest.split('.').next().unwrap_or(rest);
        let (lang, country) = match rest.split_once('_') {
            Some((l, c)) => (l.to_string(), Some(c.to_string())),
            None => (rest.to_string(), None),
        };
        (!lang.is_empty()).then_some(Locale { lang, country, modifier })
    }

    /// The process locale for messages (`LC_ALL`, `LC_MESSAGES`, `LANG`).
    pub fn current() -> Option<Locale> {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
            .and_then(|s| Locale::parse(&s))
    }

    /// Rank of a `[locale]` key suffix: lower is better (`ll_CC@MOD`, `ll_CC`, `ll@MOD`,
    /// `ll`); `None` when it does not apply.
    pub fn rank(&self, key_locale: &str) -> Option<u8> {
        let mut cands: Vec<String> = Vec::with_capacity(4);
        let l = &self.lang;
        if let (Some(c), Some(m)) = (&self.country, &self.modifier) {
            cands.push(format!("{l}_{c}@{m}"));
        }
        if let Some(c) = &self.country {
            cands.push(format!("{l}_{c}"));
        }
        if let Some(m) = &self.modifier {
            cands.push(format!("{l}@{m}"));
        }
        cands.push(l.clone());
        cands.iter().position(|c| c == key_locale).map(|p| p as u8)
    }
}

/// Parse a `.desktop` file for `locale`. `None` without a `[Desktop Entry]` group.
pub(crate) fn parse_desktop_with(text: &str, locale: Option<&Locale>) -> Option<DesktopEntry> {
    let mut e = DesktopEntry::default();
    let mut in_main = false;
    let mut seen_main = false;
    // Best rank seen so far for each localizable key (`u8::MAX` = unlocalized).
    let mut ranks: [Option<u8>; 3] = [None; 3];
    for line in text.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_main = line.trim_end() == "[Desktop Entry]";
            seen_main |= in_main;
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let (base, loc) = match k.split_once('[') {
            Some((b, rest)) => (b, Some(rest.trim_end_matches(']'))),
            None => (k, None),
        };
        let slot = match base {
            "Name" => Some(0),
            "GenericName" => Some(1),
            "Comment" => Some(2),
            _ => None,
        };
        if let Some(slot) = slot {
            let rank = match loc {
                None => u8::MAX,
                Some(l) => match locale.and_then(|lc| lc.rank(l)) {
                    Some(r) => r,
                    None => continue,
                },
            };
            if ranks[slot].is_some_and(|r| r <= rank) {
                continue;
            }
            ranks[slot] = Some(rank);
            let val = unescape_value(v);
            match slot {
                0 => e.name = val,
                1 => e.generic_name = Some(val),
                _ => e.comment = Some(val),
            }
            continue;
        }
        if loc.is_some() {
            continue;
        }
        let truth = v == "true" || v == "1";
        match base {
            "Type" => e.kind = v.to_string(),
            "Exec" => e.exec = Some(v.to_string()),
            "TryExec" => e.try_exec = Some(unescape_value(v)),
            "Path" => e.path = Some(unescape_value(v)).filter(|p| !p.is_empty()),
            "URL" => e.url = Some(unescape_value(v)),
            "Icon" => e.icon = Some(unescape_value(v)).filter(|i| !i.is_empty()),
            "MimeType" => e.mime_types = split_list(v),
            "OnlyShowIn" => e.only_show_in = split_list(v),
            "NotShowIn" => e.not_show_in = split_list(v),
            "NoDisplay" => e.no_display = truth,
            "Hidden" => e.hidden = truth,
            "Terminal" => e.terminal = truth,
            "DBusActivatable" => e.dbus_activatable = truth,
            "StartupNotify" => e.startup_notify = truth,
            _ => {}
        }
    }
    seen_main.then_some(e)
}

/// Parse with the process locale.
pub(crate) fn parse_desktop(text: &str) -> Option<DesktopEntry> {
    parse_desktop_with(text, Locale::current().as_ref())
}

/// `a;b\;c;` → `["a", "b;c"]`.
pub(crate) fn split_list(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut it = v.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some(';') => cur.push(';'),
                Some(o) => {
                    cur.push('\\');
                    cur.push(o);
                }
                None => cur.push('\\'),
            },
            ';' => out.push(unescape_value(&std::mem::take(&mut cur))),
            c => cur.push(c),
        }
    }
    out.push(unescape_value(&cur));
    out.retain(|s| !s.trim().is_empty());
    out
}

/// Desktop-entry string escapes (`\s \n \t \r \\`).
pub(crate) fn unescape_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('s') => out.push(' '),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// One `Exec=` argument after unquoting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExecToken {
    pub text: String,
    /// Came from a quoted string (field codes are not expanded inside quotes, except
    /// the lenient handling of a lone `"%f"`-style token).
    pub quoted: bool,
}

/// Split an `Exec=` value into arguments (string escapes, then the desktop-entry
/// quoting rules: `"…"` with `\" \` \$ \\` escapes).
pub(crate) fn exec_tokens(exec: &str) -> Vec<ExecToken> {
    let exec = unescape_value(exec);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut quoted = false;
    let mut has = false;
    let mut it = exec.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                in_q = !in_q;
                quoted = true;
                has = true;
            }
            '\\' if in_q => {
                if let Some(&n) = it.peek()
                    && matches!(n, '"' | '`' | '$' | '\\')
                {
                    cur.push(n);
                    it.next();
                } else {
                    cur.push('\\');
                }
            }
            c if (c == ' ' || c == '\t' || c == '\n') && !in_q => {
                if has || !cur.is_empty() {
                    out.push(ExecToken { text: std::mem::take(&mut cur), quoted });
                    has = false;
                    quoted = false;
                }
            }
            c => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(ExecToken { text: cur, quoted });
    }
    out
}

/// What a launch is about: local files or URLs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Path(PathBuf),
    Url(String),
}

impl Target {
    /// For `%f`: the local path (a `file:` URL decoded), else the URL as given.
    fn as_file(&self) -> OsString {
        match self {
            Target::Path(p) => p.as_os_str().to_owned(),
            Target::Url(u) => super::uri::uri_to_path(u).map_or_else(|| OsString::from(u), PathBuf::into_os_string),
        }
    }

    /// For `%u`: a URL (`file://` + percent-encoded path for local files).
    fn as_url(&self) -> OsString {
        match self {
            Target::Path(p) => OsString::from(super::uri::file_uri(p)),
            Target::Url(u) => OsString::from(u),
        }
    }
}

/// Values for `%i %c %k`.
#[derive(Clone, Debug, Default)]
pub(crate) struct ExecContext<'a> {
    pub icon: Option<&'a str>,
    pub name: &'a str,
    pub location: Option<&'a Path>,
}

/// Expand `Exec=` for `targets` into one or more command lines (several when the
/// entry takes a single `%f` / `%u` and more than one target is given). Without any
/// file field code, the targets are appended as paths (lenient: what GLib does too).
/// `None` when `Exec=` has no program.
pub(crate) fn expand_exec(exec: &str, targets: &[Target], ctx: &ExecContext<'_>) -> Option<Vec<Vec<OsString>>> {
    let tokens = exec_tokens(exec);
    tokens.first()?;
    let codes = |t: &ExecToken, set: &[char]| field_codes(&t.text).iter().any(|c| set.contains(c));
    let multi = tokens.iter().any(|t| codes(t, &['F', 'U']));
    let single = tokens.iter().any(|t| codes(t, &['f', 'u']));
    let groups: Vec<&[Target]> =
        if !multi && single && targets.len() > 1 { targets.chunks(1).collect() } else { vec![targets] };
    let mut lines = Vec::new();
    for group in groups {
        let mut argv: Vec<OsString> = Vec::new();
        for t in &tokens {
            expand_token(t, group, ctx, &mut argv);
        }
        if !multi && !single {
            argv.extend(group.iter().map(Target::as_file));
        }
        if argv.first().is_some_and(|p| !p.is_empty()) {
            lines.push(argv);
        }
    }
    (!lines.is_empty()).then_some(lines)
}

/// Field codes (`%x`) in a token, `%%` excluded.
fn field_codes(s: &str) -> Vec<char> {
    let mut v = Vec::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '%'
            && let Some(n) = it.next()
            && n != '%'
        {
            v.push(n);
        }
    }
    v
}

fn expand_token(t: &ExecToken, targets: &[Target], ctx: &ExecContext<'_>, argv: &mut Vec<OsString>) {
    // Whole-token codes that expand to zero or several arguments.
    match t.text.as_str() {
        "%F" => return argv.extend(targets.iter().map(Target::as_file)),
        "%U" => return argv.extend(targets.iter().map(Target::as_url)),
        "%f" if targets.is_empty() => return,
        "%u" if targets.is_empty() => return,
        "%i" => {
            if let Some(i) = ctx.icon {
                argv.push("--icon".into());
                argv.push(i.into());
            }
            return;
        }
        "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => return,
        _ => {}
    }
    let mut out = OsString::new();
    let mut it = t.text.chars();
    let mut lit = String::new();
    let flush = |lit: &mut String, out: &mut OsString| {
        if !lit.is_empty() {
            out.push(OsStr::new(lit.as_str()));
            lit.clear();
        }
    };
    while let Some(c) = it.next() {
        if c != '%' {
            lit.push(c);
            continue;
        }
        match it.next() {
            Some('%') => lit.push('%'),
            Some('f' | 'F') => {
                flush(&mut lit, &mut out);
                if let Some(f) = targets.first() {
                    out.push(f.as_file());
                }
            }
            Some('u' | 'U') => {
                flush(&mut lit, &mut out);
                if let Some(f) = targets.first() {
                    out.push(f.as_url());
                }
            }
            Some('c') => lit.push_str(ctx.name),
            Some('k') => {
                flush(&mut lit, &mut out);
                if let Some(l) = ctx.location {
                    out.push(l.as_os_str());
                }
            }
            Some('i') => lit.push_str(ctx.icon.unwrap_or_default()),
            // Deprecated / unknown codes are dropped.
            Some(_) | None => {}
        }
    }
    flush(&mut lit, &mut out);
    argv.push(out);
}

/// Quote one argument for an `Exec=` line (and escape it for the desktop-file string
/// syntax): reserved characters force double quotes, inside which `" \` $ \\` get a
/// backslash; then every backslash is doubled for the string-level escaping.
pub(crate) fn exec_quote(arg: &str) -> String {
    const RESERVED: &[char] =
        &[' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`', '='];
    let quoted = if arg.is_empty() || arg.contains(RESERVED) {
        let mut q = String::from("\"");
        for c in arg.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                q.push('\\');
            }
            q.push(c);
        }
        q.push('"');
        q
    } else {
        arg.to_string()
    };
    let quoted = quoted.replace('%', "%%");
    quoted.replace('\\', "\\\\")
}

/// `XDG_CURRENT_DESKTOP` entries (`GNOME`, `KDE`, `niri`, …).
pub(crate) fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

impl DesktopEntry {
    /// `OnlyShowIn` / `NotShowIn` against the current desktops.
    pub fn shown_in(&self, desktops: &[String]) -> bool {
        let any = |list: &[String]| desktops.iter().any(|d| list.iter().any(|l| l.eq_ignore_ascii_case(d)));
        if !self.only_show_in.is_empty() && !any(&self.only_show_in) {
            return false;
        }
        !any(&self.not_show_in)
    }

    /// Whether `TryExec` (if any) names an existing executable.
    pub fn try_exec_ok(&self) -> bool {
        match self.try_exec.as_deref().map(str::trim) {
            None | Some("") => true,
            Some(t) => which(t).is_some_and(|p| is_executable(&p)),
        }
    }

    /// Usable at all: an application with a command (`Hidden` entries do not exist).
    pub fn launchable(&self) -> bool {
        !self.hidden
            && (self.kind.is_empty() || self.kind == "Application")
            && self.exec.is_some()
            && self.try_exec_ok()
    }

    /// Listed in "Open with" menus: launchable, not `NoDisplay`, shown in this desktop.
    pub fn listed(&self, desktops: &[String]) -> bool {
        self.launchable() && !self.no_display && self.shown_in(desktops)
    }

    /// The program of `Exec=` (first argument).
    pub fn program(&self) -> Option<String> {
        exec_tokens(self.exec.as_deref()?).into_iter().next().map(|t| t.text)
    }
}

/// Resolve a bare program name through `$PATH`.
pub(crate) fn which(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file() && is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: &str = r#"
# comment
[Desktop Entry]
Type=Application
Name=Text Editor
Name[de]=Texteditor
Name[sr@latin]=Uređivač
Name[pt_BR]=Editor de texto
Name[pt]=Editor de texto (pt)
GenericName=Editor
Comment[de]=Bearbeiten
Exec=gedit --new-window "%U" %i
Icon=org.gnome.gedit
MimeType=text/plain;text/x-c;
NoDisplay=false
OnlyShowIn=GNOME;Unity;
DBusActivatable=true

[Desktop Action new-window]
Name=Other
Exec=gedit --other
"#;

    fn loc(s: &str) -> Locale {
        Locale::parse(s).unwrap()
    }

    #[test]
    fn locales_and_localized_names() {
        assert_eq!(loc("pt_BR.UTF-8"), Locale { lang: "pt".into(), country: Some("BR".into()), modifier: None });
        assert_eq!(Locale::parse("C.UTF-8"), None);
        assert_eq!(Locale::parse("POSIX"), None);
        let name = |l: Option<&str>| parse_desktop_with(DESKTOP, l.map(loc).as_ref()).unwrap().name;
        assert_eq!(name(None), "Text Editor");
        assert_eq!(name(Some("de_DE.UTF-8")), "Texteditor");
        assert_eq!(name(Some("pt_BR.UTF-8")), "Editor de texto");
        assert_eq!(name(Some("pt_PT")), "Editor de texto (pt)");
        assert_eq!(name(Some("sr_RS@latin")), "Uređivač");
        assert_eq!(name(Some("fr_FR")), "Text Editor");
        let e = parse_desktop_with(DESKTOP, Some(&loc("de"))).unwrap();
        assert_eq!(e.comment.as_deref(), Some("Bearbeiten"));
        assert_eq!(e.generic_name.as_deref(), Some("Editor"));
    }

    #[test]
    fn desktop_parsing_and_visibility() {
        let e = parse_desktop_with(DESKTOP, None).unwrap();
        assert_eq!(e.kind, "Application");
        assert_eq!(e.mime_types, ["text/plain", "text/x-c"]);
        assert_eq!(e.exec.as_deref(), Some(r#"gedit --new-window "%U" %i"#));
        assert!(e.dbus_activatable);
        assert!(e.shown_in(&["GNOME".into()]));
        assert!(!e.shown_in(&["KDE".into()]));
        assert!(!e.shown_in(&[]));
        let mut n = e.clone();
        n.only_show_in.clear();
        n.not_show_in = vec!["KDE".into()];
        assert!(n.shown_in(&["niri".into()]));
        assert!(!n.shown_in(&["niri".into(), "KDE".into()]));
        assert!(parse_desktop_with("[Other]\nName=x", None).is_none());
        assert_eq!(split_list(r"a;b\;c;;"), ["a", "b;c"]);
    }

    #[cfg(unix)]
    #[test]
    fn try_exec_and_hidden() {
        let mut e = parse_desktop_with("[Desktop Entry]\nType=Application\nExec=sh\nTryExec=sh\n", None).unwrap();
        assert!(e.launchable());
        e.try_exec = Some("definitely-not-installed-soyuz".into());
        assert!(!e.launchable());
        e.try_exec = None;
        e.hidden = true;
        assert!(!e.launchable());
        e.hidden = false;
        e.no_display = true;
        assert!(e.launchable() && !e.listed(&[]));
    }

    fn texts(tokens: Vec<ExecToken>) -> Vec<String> {
        tokens.into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn exec_quoting() {
        assert_eq!(texts(exec_tokens(r#"sh -c "echo \"hi\" \$HOME" %f"#)), ["sh", "-c", r#"echo "hi" $HOME"#, "%f"]);
        assert_eq!(texts(exec_tokens(r#"app "" x"#)), ["app", "", "x"]);
        assert_eq!(texts(exec_tokens(r"my\sapp %F")), ["my", "app", "%F"]);
        // String escape first (`\\\\` → `\\`), then the quoting rule (`\\` → `\`).
        assert_eq!(texts(exec_tokens(r#""/opt/a\\\\b/app" %u"#)), [r"/opt/a\b/app", "%u"]);
    }

    #[cfg(unix)]
    fn s(v: &[Vec<OsString>]) -> Vec<Vec<String>> {
        v.iter().map(|l| l.iter().map(|a| a.to_string_lossy().into_owned()).collect()).collect()
    }

    #[cfg(unix)]
    #[test]
    fn exec_expansion() {
        let ctx = ExecContext { icon: Some("gedit"), name: "Text Editor", location: Some(Path::new("/a/x.desktop")) };
        let files = [Target::Path("/tmp/a b.txt".into()), Target::Path("/tmp/c%.txt".into())];
        let ex = |e: &str, t: &[Target]| s(&expand_exec(e, t, &ctx).unwrap());
        assert_eq!(ex("ed %F", &files), [vec!["ed", "/tmp/a b.txt", "/tmp/c%.txt"]]);
        assert_eq!(ex("ed %U", &files), [vec!["ed", "file:///tmp/a%20b.txt", "file:///tmp/c%25.txt"]]);
        assert_eq!(ex("ed %f", &files), [vec!["ed", "/tmp/a b.txt"], vec!["ed", "/tmp/c%.txt"]], "one per file");
        assert_eq!(ex("ed --file=%f -x", &files[..1]), [vec!["ed", "--file=/tmp/a b.txt", "-x"]]);
        assert_eq!(
            ex("ed %i %c %k 100%%", &[]),
            [vec!["ed", "--icon", "gedit", "Text Editor", "/a/x.desktop", "100%"]]
        );
        assert_eq!(ex("ed %f %d %m", &[]), [vec!["ed"]]);
        assert_eq!(ex("ed", &files), [vec!["ed", "/tmp/a b.txt", "/tmp/c%.txt"]], "no field code: appended");
        let url = [Target::Url("https://example.com/x?y".into())];
        assert_eq!(ex("br %u", &url), [vec!["br", "https://example.com/x?y"]]);
        assert_eq!(ex("ed %f", &[Target::Url("file:///tmp/x%20y".into())]), [vec!["ed", "/tmp/x y"]]);
        assert!(expand_exec("", &files, &ctx).is_none());
        // Non-UTF-8 file names are passed through untouched.
        use std::os::unix::ffi::OsStrExt;
        let raw = PathBuf::from(OsStr::from_bytes(b"/tmp/\xff.bin"));
        let out = expand_exec("ed --x=%f", &[Target::Path(raw)], &ctx).unwrap();
        assert_eq!(out[0][1].as_bytes(), b"--x=/tmp/\xff.bin");
    }

    #[test]
    fn exec_quote_round_trips() {
        for arg in ["/usr/bin/soyuz", "/opt/My Apps/soyuz", r#"/x/"q"$`\y"#, "/a/100%"] {
            let line = format!("{} %U", exec_quote(arg));
            let toks = texts(exec_tokens(&line));
            let first = toks[0].replace("%%", "%");
            assert_eq!(first, arg, "{line}");
            assert_eq!(toks[1], "%U");
        }
    }
}
