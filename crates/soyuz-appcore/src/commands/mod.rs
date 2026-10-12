//! User-defined commands (`[[custom_commands]]`): placeholder expansion, running them
//! detached or in the integrated terminal, and which context-menu entries apply
//! (`ui::commands` draws them). Every command is also an
//! [`Action::CustomCommand`](soyuz_core::config::Action::CustomCommand), so it appears in
//! the palette and can be bound.
//!
//! Placeholders (see `soyuz_core::config::CustomCommand`): `%p` focused path, `%n`
//! selected names, `%s` selected paths, `%d` current folder, `%o` the other pane's
//! folder, `%%` a literal percent sign; legacy upper-case forms `%P` (paths), `%D`,
//! `%N`, `%E` (extension of the focused item) and `%F` (first selected path).

use std::path::{Path, PathBuf};

use soyuz_core::Location;
use soyuz_core::config::{CommandContext, Config, CustomCommand};

use crate::core::AppCore;
use crate::host::Host;
use crate::models::toasts::Toast;

/// What the placeholders expand to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaceholderCtx {
    /// Item under the cursor (falls back to the first selected item).
    pub focused: Option<PathBuf>,
    pub selected: Vec<PathBuf>,
    pub dir: Option<PathBuf>,
    pub other_dir: Option<PathBuf>,
}

impl PlaceholderCtx {
    /// From the focused pane of `app`.
    pub fn from_app(app: &AppCore) -> Self {
        let pane = app.ws.pane();
        let selected: Vec<PathBuf> = pane.selected_entries().iter().map(|e| clean(&e.path)).collect();
        let focused = pane
            .cursor
            .as_deref()
            .and_then(|c| pane.entry(c))
            .filter(|e| selected.is_empty() || pane.selection.contains(e.name.as_str()))
            .map(|e| clean(&e.path))
            .or_else(|| selected.first().cloned());
        let dir = pane.location.as_fs_path().map(clean);
        let other_dir = app.ws.tab().other().and_then(|p| p.location.as_fs_path().map(clean));
        PlaceholderCtx { focused, selected, dir, other_dir }
    }

    fn focused_or_dir(&self) -> Option<&Path> {
        self.focused.as_deref().or(self.dir.as_deref())
    }

    fn selection_or_focused(&self) -> Vec<PathBuf> {
        if self.selected.is_empty() { self.focused.iter().cloned().collect() } else { self.selected.clone() }
    }
}

fn clean(p: &Path) -> PathBuf {
    soyuz_fs::strip_verbatim(p)
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

/// How list values are joined inside a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quoting {
    /// Values are inserted verbatim (detached processes get one argv entry per argument).
    None,
    /// POSIX shells: `'it'\''s'`.
    Posix,
    /// PowerShell: `'it''s'`.
    PowerShell,
    /// cmd.exe: `"C:\a b"`.
    Cmd,
}

impl Quoting {
    /// Quoting style for a terminal shell setting (`""` = platform default).
    pub fn for_shell(shell: &str) -> Quoting {
        let s = shell.to_ascii_lowercase();
        if s.contains("pwsh") || s.contains("powershell") {
            Quoting::PowerShell
        } else if s.contains("cmd") {
            Quoting::Cmd
        } else if s.is_empty() {
            if cfg!(windows) { Quoting::PowerShell } else { Quoting::Posix }
        } else {
            Quoting::Posix
        }
    }

    /// Quote one value when it needs it.
    pub fn quote(self, v: &str) -> String {
        // `,` and `@` are not plain for PowerShell (array / splatting) or cmd (delimiter).
        let plain = !v.is_empty()
            && v.chars().all(|c| {
                c.is_alphanumeric()
                    || matches!(c, '/' | '\\' | '.' | '_' | '-' | ':' | '+')
                    || (self == Quoting::Posix && matches!(c, ',' | '@'))
            });
        match self {
            Quoting::None => v.to_string(),
            _ if plain => v.to_string(),
            Quoting::Posix => format!("'{}'", v.replace('\'', "'\\''")),
            Quoting::PowerShell => soyuz_fs::launch::powershell_quote(v),
            Quoting::Cmd => cmd_quote(v),
        }
    }
}

/// Quote a value for a cmd.exe command line. Inside `"…"` cmd leaves
/// `& | < > ^ ( )` alone, but it still expands `%VAR%`, and `"` toggles its quote state.
/// Both are therefore emitted *outside* the quotes, caret-escaped: `a%b` becomes
/// `"a"^%"b"`. The percent phase then only sees names like `"b"^`, which never exist, so
/// nothing expands, and the caret phase turns `^%` into a literal `%`. Backslashes before
/// a closing quote are doubled so the target's CRT argv parser does not read `\"` as a
/// literal quote. (`!` is only special with delayed expansion, which neither `cmd /C`
/// nor a default interactive cmd enables.)
pub fn cmd_quote(v: &str) -> String {
    fn flush(out: &mut String, seg: &mut String) {
        let trailing = seg.len() - seg.trim_end_matches('\\').len();
        out.push('"');
        out.push_str(seg);
        out.extend(std::iter::repeat_n('\\', trailing));
        out.push('"');
        seg.clear();
    }
    let mut out = String::with_capacity(v.len() + 2);
    let mut seg = String::new();
    for c in v.chars() {
        if matches!(c, '%' | '"') {
            flush(&mut out, &mut seg);
            out.push('^');
            out.push(c);
        } else {
            seg.push(c);
        }
    }
    flush(&mut out, &mut seg);
    out
}

/// Expand the placeholders inside one string. List placeholders are joined with spaces
/// (each value quoted per `q`).
pub fn expand_str(s: &str, ctx: &PlaceholderCtx, q: Quoting) -> String {
    let path = |p: &Path| q.quote(&p.to_string_lossy());
    let list = |v: Vec<String>| v.iter().map(|x| q.quote(x)).collect::<Vec<_>>().join(" ");
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(&n) = chars.peek() else {
            out.push('%');
            break;
        };
        let rep = match n {
            '%' => Some("%".to_string()),
            'p' => Some(ctx.focused_or_dir().map(path).unwrap_or_default()),
            'F' => Some(ctx.selection_or_focused().first().map(|p| path(p)).unwrap_or_default()),
            'd' | 'D' => Some(ctx.dir.as_deref().map(path).unwrap_or_default()),
            'o' => Some(ctx.other_dir.as_deref().or(ctx.dir.as_deref()).map(path).unwrap_or_default()),
            's' | 'P' => {
                Some(list(ctx.selection_or_focused().iter().map(|p| p.to_string_lossy().into_owned()).collect()))
            }
            'n' => Some(list(ctx.selection_or_focused().iter().map(|p| name_of(p)).collect())),
            'N' => Some(ctx.focused.as_deref().map(|p| q.quote(&name_of(p))).unwrap_or_default()),
            // Quoted like every other value: an extension can hold `&`, `;`, quotes… (#5).
            'E' => Some(
                ctx.focused
                    .as_deref()
                    .and_then(|p| p.extension())
                    .map(|e| q.quote(&e.to_string_lossy()))
                    .unwrap_or_default(),
            ),
            _ => None,
        };
        match rep {
            Some(r) => {
                chars.next();
                out.push_str(&r);
            }
            None => out.push('%'),
        }
    }
    out
}

/// Expand an argument list for a detached process: an argument that is exactly `%s`,
/// `%n` or `%P` becomes one argument per selected item; others expand in place.
pub fn expand_args(args: &[String], ctx: &PlaceholderCtx) -> Vec<String> {
    let mut out = Vec::new();
    for a in args {
        match a.as_str() {
            "%s" | "%P" => out.extend(ctx.selection_or_focused().iter().map(|p| p.to_string_lossy().into_owned())),
            "%n" => out.extend(ctx.selection_or_focused().iter().map(|p| name_of(p))),
            _ => out.push(expand_str(a, ctx, Quoting::None)),
        }
    }
    out
}

/// The full command line typed into a terminal shell.
pub fn command_line(cmd: &CustomCommand, ctx: &PlaceholderCtx, q: Quoting) -> String {
    let program = expand_str(cmd.command.trim(), ctx, Quoting::None);
    let mut parts = Vec::new();
    if cmd.args.is_empty() {
        // A command line typed as one string: placeholders are quoted in place.
        return expand_str(cmd.command.trim(), ctx, q);
    }
    let quoted = q.quote(&program);
    // A quoted string as the first PowerShell token is an expression, not a command.
    let program = if q == Quoting::PowerShell && quoted != program { format!("& {quoted}") } else { quoted };
    parts.push(program);
    for a in &cmd.args {
        match a.as_str() {
            "%s" | "%P" | "%n" => parts.push(expand_str(a, ctx, q)),
            _ => parts.push(q.quote(&expand_str(a, ctx, Quoting::None))),
        }
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" ")
}

/// Whether the command shows for the given context (no contexts = everywhere).
pub fn applies(cmd: &CustomCommand, kinds: &[CommandContext]) -> bool {
    cmd.context.is_empty() || kinds.iter().any(|k| cmd.context.contains(k))
}

/// Split a user-typed argument string (`a "b c" 'd'`) into arguments.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                any = true;
            }
            None if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            None => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Join arguments back into an editable string (inverse of [`split_args`]).
pub fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|a| if a.is_empty() || a.contains(char::is_whitespace) { format!("\"{a}\"") } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run the custom command called `name` against the focused pane.
pub fn run_named(app: &mut AppCore, name: &str, host: &dyn Host) {
    let Some(cmd) = app.config.custom_commands.iter().find(|c| c.name == name).cloned() else {
        app.ui.toasts.push(Toast::warning(tr!("actions-command-not-found", name = name)));
        return;
    };
    run(app, &cmd, host);
}

/// Run `cmd` (detached, or typed into the terminal panel).
pub fn run(app: &mut AppCore, cmd: &CustomCommand, host: &dyn Host) {
    if cmd.command.trim().is_empty() {
        app.ui.toasts.push(Toast::warning(tr!("actions-command-empty", name = &cmd.name)));
        return;
    }
    let pctx = PlaceholderCtx::from_app(app);
    let dir = pctx.dir.clone().unwrap_or_else(soyuz_fs::home);
    if cmd.terminal {
        let line = command_line(cmd, &pctx, Quoting::for_shell(&app.config.terminal.shell));
        crate::models::terminal::run_command(app, host, dir, &line);
        return;
    }
    let program = expand_str(cmd.command.trim(), &pctx, Quoting::None);
    let args = expand_args(&cmd.args, &pctx);
    let shell_line = cmd.args.is_empty() && program.contains(char::is_whitespace) && !Path::new(&program).exists();
    let line = if shell_line { expand_str(cmd.command.trim(), &pctx, Quoting::for_shell("")) } else { String::new() };
    let name = cmd.name.clone();
    let postman = app.services.mailbox.postman();
    let spawned = std::thread::Builder::new().name("soyuz-custom-cmd".into()).spawn(move || {
        let result = if shell_line { spawn_shell(&line, &dir) } else { spawn_direct(&program, &args, &dir) };
        if let Err(e) = result {
            postman.send(crate::services::mailbox::UiMsg::Notice(
                Toast::error(tr!("actions-command-run-failed", name = &name)).body(e.to_string()),
            ));
        }
    });
    if let Err(e) = spawned {
        app.ui.toasts.push(Toast::error(tr!("actions-command-start-failed")).body(e.to_string()));
    } else {
        app.ui.toasts.push(Toast::info(tr!("actions-command-running", name = &cmd.name)));
    }
}

fn spawn_direct(program: &str, args: &[String], dir: &Path) -> std::io::Result<()> {
    std::process::Command::new(program).args(args).current_dir(dir).spawn().map(drop)
}

/// Run a whole command line detached. On Windows the line was quoted for PowerShell
/// (`Quoting::for_shell("")`), so it runs in PowerShell — previously it went to `cmd /C`
/// with PowerShell quoting, so `a&calc&.txt` injected a command. The script
/// travels as `-EncodedCommand`, so neither cmd.exe nor the CRT re-parses it.
fn spawn_shell(line: &str, dir: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
            .arg(soyuz_fs::launch::powershell_encoded_command(line))
            .current_dir(dir)
            .spawn()
            .map(drop)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh").arg("-c").arg(line).current_dir(dir).spawn().map(drop)
    }
}

/// The commands whose context-menu entries apply to `kinds` (named ones only).
pub fn context_entries<'a>(config: &'a Config, kinds: &[CommandContext]) -> Vec<&'a CustomCommand> {
    config.custom_commands.iter().filter(|c| !c.name.trim().is_empty() && applies(c, kinds)).collect()
}

/// The context kinds for a selection (`(any file, any folder)`), or the background.
pub fn kinds_for(location: &Location, any_file: bool, any_dir: bool, background: bool) -> Vec<CommandContext> {
    let mut v = Vec::new();
    if background || !matches!(location, Location::Dir(_)) && !any_file && !any_dir {
        v.push(CommandContext::Background);
    }
    if any_file {
        v.push(CommandContext::File);
    }
    if any_dir {
        v.push(CommandContext::Dir);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> PlaceholderCtx {
        PlaceholderCtx {
            focused: Some(PathBuf::from("/w/my file.txt")),
            selected: vec![PathBuf::from("/w/my file.txt"), PathBuf::from("/w/b.rs")],
            dir: Some(PathBuf::from("/w")),
            other_dir: Some(PathBuf::from("/x y")),
        }
    }

    #[test]
    fn expands_in_strings() {
        let c = ctx();
        assert_eq!(expand_str("%d", &c, Quoting::None), "/w");
        assert_eq!(expand_str("--file=%p", &c, Quoting::None), "--file=/w/my file.txt");
        assert_eq!(expand_str("%n", &c, Quoting::Posix), "'my file.txt' b.rs");
        assert_eq!(expand_str("%s", &c, Quoting::PowerShell), "'/w/my file.txt' /w/b.rs");
        assert_eq!(expand_str("%o", &c, Quoting::Cmd), "\"/x y\"");
        assert_eq!(expand_str("100%% %E %N", &c, Quoting::None), "100% txt my file.txt");
        assert_eq!(expand_str("%q stays, trailing %", &c, Quoting::None), "%q stays, trailing %");
        assert_eq!(expand_str("%F|%D|%P", &c, Quoting::None), "/w/my file.txt|/w|/w/my file.txt /w/b.rs");
    }

    #[test]
    fn empty_selection_falls_back() {
        let c = PlaceholderCtx { dir: Some(PathBuf::from("/w")), ..Default::default() };
        assert_eq!(expand_str("%p", &c, Quoting::None), "/w", "%p falls back to the folder");
        assert_eq!(expand_str("[%s]", &c, Quoting::None), "[]");
        assert_eq!(expand_str("%o", &c, Quoting::None), "/w", "no other pane: current folder");
    }

    #[test]
    fn list_args_split() {
        let c = ctx();
        let args = vec!["-n".to_string(), "%s".to_string(), "--in=%d".to_string()];
        assert_eq!(expand_args(&args, &c), vec!["-n", "/w/my file.txt", "/w/b.rs", "--in=/w"]);
        assert_eq!(expand_args(&["%n".into()], &c), vec!["my file.txt", "b.rs"]);
    }

    #[test]
    fn terminal_lines() {
        let c = ctx();
        let cmd = CustomCommand { command: "code".into(), args: vec!["%p".into()], ..Default::default() };
        assert_eq!(command_line(&cmd, &c, Quoting::Posix), "code '/w/my file.txt'");
        let cmd = CustomCommand { command: "git log -- %s".into(), ..Default::default() };
        assert_eq!(command_line(&cmd, &c, Quoting::PowerShell), "git log -- '/w/my file.txt' /w/b.rs");
        let cmd =
            CustomCommand { command: "C:/Program Files/x.exe".into(), args: vec!["%s".into()], ..Default::default() };
        assert_eq!(command_line(&cmd, &c, Quoting::PowerShell), "& 'C:/Program Files/x.exe' '/w/my file.txt' /w/b.rs");
        assert_eq!(Quoting::Posix.quote("it's"), "'it'\\''s'");
        assert_eq!(Quoting::PowerShell.quote("it's"), "'it''s'");
    }

    const HOSTILE: &[&str] = &[
        "a&calc&.txt",
        "%PATH%.txt",
        "it’s.txt",
        "‘x‛‚.txt",
        "it's.txt",
        "$(calc).txt",
        "`calc`.txt",
        "a;calc;.txt",
        "a|calc.txt",
        "a^b.txt",
        "!x!.txt",
        "(a) <b> c.txt",
        "a,b@c.txt",
        "C:\\dir with space\\",
        "a\"&calc&\".txt",
        "100%",
    ];

    /// Simulates cmd.exe's tokenizer: no `%` inside quotes, and outside quotes every
    /// special character is caret-escaped.
    fn cmd_safe(s: &str) -> bool {
        let mut in_q = false;
        let mut it = s.chars();
        while let Some(c) = it.next() {
            match (in_q, c) {
                (_, '"') => in_q = !in_q,
                (true, '%') => return false,
                (false, '^') => {
                    if it.next().is_none() {
                        return false;
                    }
                }
                (false, '&' | '|' | '<' | '>' | '%' | '(' | ')' | ' ' | ';' | ',' | '!') => return false,
                _ => {}
            }
        }
        !in_q
    }

    #[test]
    fn hostile_names_quote_safely() {
        for &name in HOSTILE {
            let c = cmd_quote(name);
            assert!(cmd_safe(&c), "cmd: {name:?} -> {c}");
            let p = Quoting::PowerShell.quote(name);
            // Inside a PowerShell single-quoted string only (typographic) single quotes are
            // special, and those must come in pairs.
            let inner = &p[1..p.len() - 1];
            let mut run = 0;
            for ch in inner.chars().chain(std::iter::once('x')) {
                if matches!(ch, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
                    run += 1;
                } else {
                    assert!(run % 2 == 0, "powershell: {name:?} -> {p}");
                    run = 0;
                }
            }
            assert!(p.starts_with('\'') && p.ends_with('\''), "{p}");
            let x = Quoting::Posix.quote(name);
            // `,` and `@` are harmless to POSIX shells, so `a,b@c.txt` may stay bare.
            assert!(x == "a,b@c.txt" || (x.starts_with('\'') && x.ends_with('\'')), "{x}");
        }
        assert_eq!(cmd_quote("a&calc&.txt"), "\"a&calc&.txt\"");
        assert_eq!(cmd_quote("%PATH%.txt"), "\"\"^%\"PATH\"^%\".txt\"");
        assert_eq!(cmd_quote(r"C:\dir\"), r#""C:\dir\\""#);
        assert_eq!(Quoting::PowerShell.quote("it’s.txt"), "'it’’s.txt'");
        assert_eq!(Quoting::PowerShell.quote("a,b"), "'a,b'", "comma would build an array");
        assert_eq!(Quoting::Cmd.quote("plain.txt"), "plain.txt");
    }

    #[test]
    fn extension_placeholder_is_quoted() {
        let c = PlaceholderCtx { focused: Some(PathBuf::from("/w/x.a&calc")), ..Default::default() };
        assert_eq!(expand_str("%E", &c, Quoting::PowerShell), "'a&calc'");
        assert_eq!(expand_str("%E", &c, Quoting::Cmd), "\"a&calc\"");
        assert_eq!(expand_str("%E", &c, Quoting::None), "a&calc");
        let cmd = CustomCommand { command: "C:/x(1).exe".into(), args: vec!["%p".into()], ..Default::default() };
        assert_eq!(command_line(&cmd, &c, Quoting::PowerShell), "& 'C:/x(1).exe' '/w/x.a&calc'");
    }

    #[test]
    fn contexts_and_arg_strings() {
        let mut cmd = CustomCommand::default();
        assert!(applies(&cmd, &[CommandContext::Background]));
        cmd.context = vec![CommandContext::File];
        assert!(applies(&cmd, &[CommandContext::File, CommandContext::Dir]));
        assert!(!applies(&cmd, &[CommandContext::Dir]));
        assert_eq!(split_args(r#"-a "b c" 'd e' f"#), vec!["-a", "b c", "d e", "f"]);
        assert_eq!(split_args(r#"x "" y"#), vec!["x", "", "y"]);
        let v = vec!["-a".to_string(), "b c".to_string(), String::new()];
        assert_eq!(split_args(&join_args(&v)), v);
        assert_eq!(kinds_for(&Location::Trash, false, false, false), vec![CommandContext::Background]);
        assert_eq!(
            kinds_for(&Location::Dir("/w".into()), true, true, false),
            vec![CommandContext::File, CommandContext::Dir]
        );
    }
}
