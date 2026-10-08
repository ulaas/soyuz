//! `docs/KEYBOARD.md` and `docs/keyboard.html` from soyuz-core's default key maps
//! (`config::defaults::preset`) and the English palette titles / descriptions.

use soyuz_app::palette::catalog::{description_in, title_in};
use soyuz_core::Locale;
use soyuz_core::config::defaults::preset;
use soyuz_core::config::{Action, ActionCategory, KeyChord, KeyContext, KeyPreset, Keymap, format_sequence};

const CATEGORIES: [ActionCategory; 7] = [
    ActionCategory::Navigation,
    ActionCategory::Tabs,
    ActionCategory::View,
    ActionCategory::Selection,
    ActionCategory::FileOps,
    ActionCategory::Tools,
    ActionCategory::Jobs,
];

/// One shortcut: a chord or a chord sequence, and where it applies.
#[derive(Clone, PartialEq)]
struct Key {
    /// Chords, each a list of key names (`["Ctrl", "Shift", "P"]`).
    chords: Vec<Vec<String>>,
    ctx: KeyContext,
}

struct Row {
    title: String,
    desc: String,
    keys: Vec<Key>,
}

fn key_parts(c: &KeyChord) -> Vec<String> {
    let s = c.to_string();
    // `Ctrl++` cannot occur (the key is named `Plus`), so `+` splits cleanly.
    s.split('+')
        .map(|p| match p {
            "Up" => "↑".to_string(),
            "Down" => "↓".to_string(),
            "Left" => "←".to_string(),
            "Right" => "→".to_string(),
            "Plus" => "+".to_string(),
            other => other.to_string(),
        })
        .collect()
}

fn keys_of(km: &Keymap, a: &Action) -> Vec<Key> {
    let mut out = Vec::new();
    for ctx in KeyContext::ALL {
        if let Some(chords) = km.bindings.get(&(ctx, a.clone())) {
            for c in chords {
                out.push(Key { chords: vec![key_parts(c)], ctx });
            }
        }
        if let Some(seqs) = km.sequences.get(&(ctx, a.clone())) {
            for s in seqs {
                debug_assert!(!format_sequence(s).is_empty());
                out.push(Key { chords: s.iter().map(key_parts).collect(), ctx });
            }
        }
    }
    out
}

/// Numbered slot actions (bookmarks, tabs, workspaces, tags) collapse into one row.
fn slot(a: &Action) -> Option<(&'static str, u8)> {
    match a {
        Action::OpenBookmark(n) => Some(("bookmark", *n)),
        Action::SelectTab(n) => Some(("tab", *n)),
        Action::OpenWorkspace(n) => Some(("workspace", *n)),
        Action::AddTag(n) => Some(("tag", *n)),
        _ => None,
    }
}

fn rows(km: &Keymap, cat: ActionCategory) -> Vec<Row> {
    let actions: Vec<Action> = Action::all().into_iter().filter(|a| a.category() == cat).collect();
    let mut out: Vec<Row> = Vec::new();
    let mut i = 0;
    while i < actions.len() {
        let a = &actions[i];
        let keys = keys_of(km, a);
        if let Some((kind, 1)) = slot(a)
            && keys.len() == 1
            && keys[0].chords.len() == 1
            && keys[0].chords[0].last().map(String::as_str) == Some("1")
        {
            // Extend while slot n is bound to the same modifiers + digit n.
            let prefix: Vec<String> = keys[0].chords[0][..keys[0].chords[0].len() - 1].to_vec();
            let mut n = 1u8;
            while let Some(next) = actions.get(i + n as usize) {
                let nk = keys_of(km, next);
                let want: Vec<String> = prefix.iter().cloned().chain([(n + 1).to_string()]).collect();
                if slot(next) == Some((kind, n + 1))
                    && nk.len() == 1
                    && nk[0].chords == vec![want]
                    && title_in(next, Locale::EnUs).ends_with(&format!(" {}", n + 1))
                {
                    n += 1;
                } else {
                    break;
                }
            }
            if n >= 3 {
                let title = title_in(a, Locale::EnUs);
                let title = title.strip_suffix(" 1").map(|t| format!("{t} 1–{n}")).unwrap_or(title);
                let first = keys[0].clone();
                let mut last = first.clone();
                if let Some(l) = last.chords[0].last_mut() {
                    *l = n.to_string();
                }
                out.push(Row { title, desc: description_in(a, Locale::EnUs), keys: vec![first, last] });
                i += n as usize;
                continue;
            }
        }
        if !keys.is_empty() {
            out.push(Row { title: title_in(a, Locale::EnUs), desc: description_in(a, Locale::EnUs), keys });
        }
        i += 1;
    }
    out
}

fn is_range(r: &Row) -> bool {
    r.title.contains('–') && r.keys.len() == 2
}

/// Bindings of `p` that differ from the Explorer preset: `(title, explorer keys, preset keys)`.
fn differences(p: KeyPreset) -> Vec<(String, Vec<Key>, Vec<Key>)> {
    let base = preset(KeyPreset::Explorer);
    let km = preset(p);
    let mut out = Vec::new();
    for cat in CATEGORIES {
        for a in Action::all().into_iter().filter(|a| a.category() == cat) {
            let (b, k) = (keys_of(&base, &a), keys_of(&km, &a));
            if b != k {
                out.push((title_in(&a, Locale::EnUs), b, k));
            }
        }
    }
    out
}

fn preset_blurb(p: KeyPreset) -> &'static str {
    match p {
        KeyPreset::Explorer => "Windows Explorer conventions (the default).",
        KeyPreset::Commander => "Total Commander / Norton Commander F-keys for two-pane work.",
        KeyPreset::Finder => "macOS Finder conventions (Cmd is Ctrl on Windows and Linux).",
        KeyPreset::Vim => "Vim / ranger-style letter keys and two-key sequences.",
    }
}

fn ctx_note(ctx: KeyContext) -> Option<String> {
    (ctx != KeyContext::Global).then(|| ctx.label().to_lowercase())
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|")
}

fn md_chord(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| {
            let p = match p.as_str() {
                "`" => "&#96;".to_string(),
                "|" => "&#124;".to_string(),
                "<" => "&lt;".to_string(),
                other => other.to_string(),
            };
            format!("<kbd>{p}</kbd>")
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn md_key(k: &Key) -> String {
    let s = k.chords.iter().map(|c| md_chord(c)).collect::<Vec<_>>().join(" ");
    match ctx_note(k.ctx) {
        Some(n) => format!("{s} *({n})*"),
        None => s,
    }
}

fn md_keys(keys: &[Key], range: bool) -> String {
    if keys.is_empty() {
        return "—".into();
    }
    let sep = if range { " … " } else { ", " };
    keys.iter().map(md_key).collect::<Vec<_>>().join(sep)
}

pub fn markdown() -> String {
    let mut s = String::new();
    s.push_str("# Keyboard shortcuts\n\n");
    s.push_str(
        "<!-- Generated from soyuz-core's default key maps by crates/soyuz-app/tests/docs_sync.rs.\n     \
         Do not edit by hand: run `SOYUZ_BLESS_DOCS=1 cargo test -p soyuz-app --test docs_sync`. -->\n\n",
    );
    s.push_str(
        "Soyuz ships four key presets. Pick one in **Settings → Keyboard** (or `[keys] preset` in\n\
         `config.toml`), then override single actions there or under `[keys.bindings]`; see\n\
         [CONFIG.md](CONFIG.md#keys). The command palette (<kbd>Ctrl</kbd>+<kbd>K</kbd>) lists every\n\
         action with its current shortcut, and <kbd>Ctrl</kbd>+<kbd>/</kbd> shows the active map in the app.\n\
         A printable one-page version is in [keyboard.html](keyboard.html) (open it in a browser and print to PDF).\n\n\
         Shortcuts marked *(file list)*, *(sidebar)* and so on only apply while that part of the window has\n\
         keyboard focus; the rest work everywhere. Text fields, the terminal and the built-in editors keep\n\
         their usual editing keys.\n\n",
    );
    s.push_str("| Preset | Style |\n|---|---|\n");
    for p in KeyPreset::ALL {
        s.push_str(&format!("| {} | {} |\n", p.label(), preset_blurb(p)));
    }
    s.push_str("\n## Explorer preset (default)\n\n");
    let km = preset(KeyPreset::Explorer);
    for cat in CATEGORIES {
        let rows = rows(&km, cat);
        if rows.is_empty() {
            continue;
        }
        s.push_str(&format!("### {}\n\n| Action | Shortcut | What it does |\n|---|---|---|\n", cat.label()));
        for r in &rows {
            s.push_str(&format!(
                "| {} | {} | {} |\n",
                md_escape(&r.title),
                md_keys(&r.keys, is_range(r)),
                md_escape(&r.desc)
            ));
        }
        s.push('\n');
    }
    for p in [KeyPreset::Commander, KeyPreset::Finder, KeyPreset::Vim] {
        s.push_str(&format!(
            "## {} preset\n\n{} Only the bindings that differ from the Explorer preset are listed.\n\n",
            p.label(),
            preset_blurb(p)
        ));
        s.push_str("| Action | Explorer | ");
        s.push_str(p.label());
        s.push_str(" |\n|---|---|---|\n");
        for (title, b, k) in differences(p) {
            s.push_str(&format!("| {} | {} | {} |\n", md_escape(&title), md_keys(&b, false), md_keys(&k, false)));
        }
        s.push('\n');
    }
    s.push_str(
        "## Not bound by default\n\n\
         Every other action (folder tools, checksums, sort and group choices, AI commands, …) has no\n\
         default shortcut. Run it from the command palette, or bind it in Settings → Keyboard.\n",
    );
    s
}

// ---------------------------------------------------------------------------
// Printable HTML
// ---------------------------------------------------------------------------

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn html_key(k: &Key) -> String {
    let s = k
        .chords
        .iter()
        .map(|c| c.iter().map(|p| format!("<kbd>{}</kbd>", html_escape(p))).collect::<Vec<_>>().join("+"))
        .collect::<Vec<_>>()
        .join(" ");
    match ctx_note(k.ctx) {
        Some(n) => format!("{s}<span class=\"ctx\">{n}</span>"),
        None => s,
    }
}

fn html_keys(keys: &[Key], range: bool) -> String {
    if keys.is_empty() {
        return "<span class=\"none\">—</span>".into();
    }
    let sep = if range { " … " } else { "<br>" };
    keys.iter().map(html_key).collect::<Vec<_>>().join(sep)
}

const CSS: &str = r#"
:root { --bg: #ffffff; --fg: #1d2026; --muted: #5d6573; --line: #dfe3ea; --card: #f6f7f9;
        --kbd-bg: #ffffff; --kbd-line: #b9c0cc; --accent: #3d6fe0; }
@media (prefers-color-scheme: dark) {
  :root { --bg: #15171c; --fg: #e7e9ee; --muted: #9aa3b2; --line: #2b2f38; --card: #1c1f26;
          --kbd-bg: #262a33; --kbd-line: #434956; --accent: #7aa2ff; }
}
* { box-sizing: border-box; }
html { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
body { margin: 0; padding: 24px 16px 48px; background: var(--bg); color: var(--fg);
       font: 12px/1.35 "Segoe UI", Inter, system-ui, sans-serif; }
main { max-width: 1180px; margin: 0 auto; }
header { display: flex; align-items: baseline; gap: 12px; flex-wrap: wrap;
         border-bottom: 2px solid var(--accent); padding-bottom: 8px; margin-bottom: 14px; }
h1 { font-size: 22px; margin: 0; letter-spacing: 0.2px; }
header p { margin: 0; color: var(--muted); }
h2 { font-size: 15px; margin: 22px 0 8px; }
.grid { columns: 3 300px; column-gap: 14px; }
section { break-inside: avoid; background: var(--card); border: 1px solid var(--line);
          border-radius: 8px; padding: 8px 10px 6px; margin: 0 0 14px; }
h3 { font-size: 11px; text-transform: uppercase; letter-spacing: 0.8px; color: var(--accent);
     margin: 0 0 4px; }
table { width: 100%; border-collapse: collapse; }
td { padding: 3px 0; vertical-align: top; border-top: 1px solid var(--line); }
tr:first-child td { border-top: 0; }
td.k { text-align: right; white-space: nowrap; padding-left: 8px; }
th { text-align: left; font-weight: 600; color: var(--muted); padding: 2px 0; }
kbd { display: inline-block; min-width: 1.6em; padding: 0 4px; border: 1px solid var(--kbd-line);
      border-bottom-width: 2px; border-radius: 4px; background: var(--kbd-bg); text-align: center;
      font: 600 10.5px/1.5 "Cascadia Mono", Consolas, ui-monospace, monospace; }
.ctx { display: block; color: var(--muted); font-size: 10px; }
.none { color: var(--muted); }
.diff td { padding-right: 8px; }
.diff td.k { text-align: left; }
footer { margin-top: 16px; color: var(--muted); font-size: 11px; }
@media (max-width: 640px) { body { padding: 16px 16px 32px; } td.k { white-space: normal; } }
@page { size: A4; margin: 12mm; }
@media print {
  body { padding: 0; font-size: 10px; background: #fff; color: #000; }
  :root { --bg: #fff; --fg: #000; --card: #fff; --kbd-bg: #fff; }
  .grid { columns: 3; }
  .page { break-before: page; }
}
"#;

pub fn html() -> String {
    let mut s = String::new();
    s.push_str("<!doctype html>\n<!-- Generated by crates/soyuz-app/tests/docs_sync.rs; do not edit by hand. -->\n");
    s.push_str("<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    s.push_str("<title>Soyuz keyboard shortcuts</title>\n<style>");
    s.push_str(CSS);
    s.push_str("</style>\n</head>\n<body>\n<main>\n<header><h1>Soyuz keyboard shortcuts</h1>");
    s.push_str(
        "<p>Explorer preset (default) · Ctrl+K opens the command palette with every action · \
         Settings → Keyboard changes any binding</p></header>\n<div class=\"grid\">\n",
    );
    let km = preset(KeyPreset::Explorer);
    for cat in CATEGORIES {
        let rows = rows(&km, cat);
        if rows.is_empty() {
            continue;
        }
        s.push_str(&format!("<section><h3>{}</h3><table>\n", html_escape(cat.label())));
        for r in &rows {
            s.push_str(&format!(
                "<tr><td title=\"{}\">{}</td><td class=\"k\">{}</td></tr>\n",
                html_escape(&r.desc),
                html_escape(&r.title),
                html_keys(&r.keys, is_range(r))
            ));
        }
        s.push_str("</table></section>\n");
    }
    s.push_str("</div>\n<div class=\"page\">\n<h2>Other presets: what changes</h2>\n<div class=\"grid\">\n");
    for p in [KeyPreset::Commander, KeyPreset::Finder, KeyPreset::Vim] {
        s.push_str(&format!(
            "<section class=\"diff\"><h3>{}</h3><table>\n<tr><th>Action</th><th>Explorer</th><th>{}</th></tr>\n",
            html_escape(p.label()),
            html_escape(p.label())
        ));
        for (title, b, k) in differences(p) {
            s.push_str(&format!(
                "<tr><td>{}</td><td class=\"k\">{}</td><td class=\"k\">{}</td></tr>\n",
                html_escape(&title),
                html_keys(&b, false),
                html_keys(&k, false)
            ));
        }
        s.push_str("</table></section>\n");
    }
    s.push_str(
        "</div>\n</div>\n<footer>Generated from the default key maps in soyuz-core \
         (see docs/KEYBOARD.md). Print with your browser's “Save as PDF” for a two-page A4 sheet.</footer>\n\
         </main>\n</body>\n</html>\n",
    );
    s
}
