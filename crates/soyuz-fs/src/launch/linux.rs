//! Linux / BSD: `xdg-open` (then `gio open`) for the default application; the "Open with"
//! list comes from the shared-mime-info glob database (`mime/globs2`), `mimeapps.list` and
//! `.desktop` files in the XDG data directories. Parsing is pure and tested everywhere.
#![cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::AppInfo;

/// The `[Desktop Entry]` keys we use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DesktopEntry {
    pub kind: String,
    pub name: String,
    pub exec: Option<String>,
    pub try_exec: Option<String>,
    pub url: Option<String>,
    pub icon: Option<String>,
    pub mime_types: Vec<String>,
    pub no_display: bool,
    pub hidden: bool,
    pub terminal: bool,
}

/// Parse a `.desktop` file (unlocalized keys; `Name[xx]` variants ignored).
pub(crate) fn parse_desktop(text: &str) -> Option<DesktopEntry> {
    let mut e = DesktopEntry::default();
    let mut in_main = false;
    let mut seen_main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            seen_main |= in_main;
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "Type" => e.kind = v.to_string(),
            "Name" => e.name = unescape_value(v),
            "Exec" => e.exec = Some(v.to_string()),
            "TryExec" => e.try_exec = Some(unescape_value(v)),
            "URL" => e.url = Some(unescape_value(v)),
            "Icon" => e.icon = Some(unescape_value(v)),
            "MimeType" => e.mime_types = v.split(';').filter(|s| !s.is_empty()).map(str::to_string).collect(),
            "NoDisplay" => e.no_display = v == "true",
            "Hidden" => e.hidden = v == "true",
            "Terminal" => e.terminal = v == "true",
            _ => {}
        }
    }
    seen_main.then_some(e)
}

/// Desktop-entry string escapes (`\s \n \t \r \\`).
fn unescape_value(v: &str) -> String {
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

/// Split an `Exec=` value into arguments (desktop-entry quoting rules); field codes are
/// kept except the ones that never take a file (`%i %c %k` and deprecated codes).
pub(crate) fn exec_tokens(exec: &str) -> Vec<String> {
    let exec = unescape_value(exec);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut has = false;
    let mut it = exec.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                in_q = !in_q;
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
            c if c.is_whitespace() && !in_q => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .filter(|t| !matches!(t.as_str(), "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m"))
        .map(|t| t.replace("%%", "%"))
        .collect()
}

/// `(weight, mime, glob)` lines of `globs2`.
pub(crate) fn parse_globs2(text: &str) -> Vec<(u32, String, String)> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut it = l.splitn(4, ':');
            let w = it.next()?.parse().ok()?;
            let mime = it.next()?.to_string();
            let glob = it.next()?.to_string();
            Some((w, mime, glob))
        })
        .collect()
}

/// Best MIME type for a file name: literal names, then the longest/heaviest `*.ext` glob.
pub(crate) fn mime_for_name(globs: &[(u32, String, String)], name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let mut best: Option<(u32, usize, &str)> = None;
    for (w, mime, glob) in globs {
        let matched = if let Some(suffix) = glob.strip_prefix('*') {
            !suffix.contains(['*', '?', '[']) && lower.ends_with(&suffix.to_lowercase())
        } else {
            !glob.contains(['*', '?', '[']) && lower == glob.to_lowercase()
        };
        if matched {
            let key = (*w, glob.len(), mime.as_str());
            if best.is_none_or(|b| (key.0, key.1) > (b.0, b.1)) {
                best = Some(key);
            }
        }
    }
    best.map(|b| b.2.to_string())
}

/// `[Default Applications]`, `[Added Associations]` and `[Removed Associations]` of a
/// `mimeapps.list`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct MimeApps {
    pub defaults: HashMap<String, Vec<String>>,
    pub added: HashMap<String, Vec<String>>,
    pub removed: HashMap<String, Vec<String>>,
}

pub(crate) fn parse_mimeapps(text: &str) -> MimeApps {
    let mut m = MimeApps::default();
    let mut section = "";
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = match line {
                "[Default Applications]" => "d",
                "[Added Associations]" => "a",
                "[Removed Associations]" => "r",
                _ => "",
            };
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let ids: Vec<String> = v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
        let map = match section {
            "d" => &mut m.defaults,
            "a" => &mut m.added,
            "r" => &mut m.removed,
            _ => continue,
        };
        map.entry(k.trim().to_string()).or_default().extend(ids);
    }
    m
}

/// AppInfo from a desktop entry (`id` = desktop file id).
pub(crate) fn app_from_entry(id: &str, e: &DesktopEntry) -> Option<AppInfo> {
    let tokens = exec_tokens(e.exec.as_deref()?);
    let (exe, args) = tokens.split_first()?;
    Some(AppInfo {
        id: id.to_string(),
        name: if e.name.is_empty() { id.trim_end_matches(".desktop").to_string() } else { e.name.clone() },
        exec: which(exe).unwrap_or_else(|| PathBuf::from(exe)),
        args: args.to_vec(),
        icon: e.icon.as_ref().filter(|i| i.starts_with('/')).map(PathBuf::from),
        launchable: true,
    })
}

/// Resolve a bare program name through `$PATH`.
pub(crate) fn which(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return Some(PathBuf::from(name));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

fn xdg_dirs(home_var: &str, home_default: &str, dirs_var: &str, dirs_default: &str) -> Vec<PathBuf> {
    let mut out = vec![
        std::env::var_os(home_var)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::path::home().join(home_default)),
    ];
    let dirs = std::env::var(dirs_var).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| dirs_default.to_string());
    out.extend(dirs.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    out
}

fn data_dirs() -> Vec<PathBuf> {
    xdg_dirs("XDG_DATA_HOME", ".local/share", "XDG_DATA_DIRS", "/usr/local/share:/usr/share")
}

fn config_dirs() -> Vec<PathBuf> {
    xdg_dirs("XDG_CONFIG_HOME", ".config", "XDG_CONFIG_DIRS", "/etc/xdg")
}

/// All desktop entries by desktop-file id (earlier data dirs win).
fn desktop_entries() -> Vec<(String, DesktopEntry)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for base in data_dirs() {
        let apps = base.join("applications");
        let mut stack = vec![apps.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for de in rd.flatten() {
                let p = de.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().is_none_or(|e| e != "desktop") {
                    continue;
                }
                let id = p.strip_prefix(&apps).map(|r| r.to_string_lossy().replace('/', "-")).unwrap_or_default();
                if !seen.insert(id.clone()) {
                    continue;
                }
                if let Some(e) = std::fs::read_to_string(&p).ok().and_then(|t| parse_desktop(&t)) {
                    out.push((id, e));
                }
            }
        }
    }
    out
}

/// MIME type of `path` from the glob database (directories are `inode/directory`).
pub(crate) fn mime_of(path: &Path) -> Option<String> {
    if path.is_dir() {
        return Some("inode/directory".into());
    }
    let name = path.file_name()?.to_string_lossy().into_owned();
    for base in data_dirs() {
        if let Ok(t) = std::fs::read_to_string(base.join("mime").join("globs2"))
            && let Some(m) = mime_for_name(&parse_globs2(&t), &name)
        {
            return Some(m);
        }
    }
    None
}

/// Applications for `path`: defaults from `mimeapps.list` first, then added associations,
/// then every desktop entry declaring the MIME type.
pub(crate) fn apps_for(path: &Path) -> Vec<AppInfo> {
    let Some(mime) = mime_of(path) else { return Vec::new() };
    let entries = desktop_entries();
    let mut order: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    let mut lists: Vec<PathBuf> = config_dirs().into_iter().map(|d| d.join("mimeapps.list")).collect();
    lists.extend(data_dirs().into_iter().map(|d| d.join("applications").join("mimeapps.list")));
    for list in lists {
        if let Ok(t) = std::fs::read_to_string(list) {
            let m = parse_mimeapps(&t);
            for ids in [m.defaults.get(&mime), m.added.get(&mime)].into_iter().flatten() {
                order.extend(ids.iter().cloned());
            }
            removed.extend(m.removed.get(&mime).cloned().unwrap_or_default());
        }
    }
    for (id, e) in &entries {
        if e.mime_types.iter().any(|m| m == &mime) {
            order.push(id.clone());
        }
    }
    let mut out: Vec<AppInfo> = Vec::new();
    for id in order {
        if removed.contains(&id) || out.iter().any(|a| a.id == id) {
            continue;
        }
        if let Some((_, e)) = entries.iter().find(|(eid, _)| *eid == id)
            && !e.hidden
            && let Some(app) = app_from_entry(&id, e)
        {
            out.push(app);
        }
    }
    out
}

/// Target of a `.desktop` file: `URL=` for links (`file://` decoded), the resolved
/// executable for applications.
pub(crate) fn desktop_target(path: &Path) -> std::io::Result<Option<PathBuf>> {
    let text = std::fs::read_to_string(path)?;
    let Some(e) = parse_desktop(&text) else { return Ok(None) };
    if e.kind == "Link" {
        return Ok(e
            .url
            .as_deref()
            .and_then(|u| u.strip_prefix("file://"))
            .map(|p| PathBuf::from(String::from_utf8_lossy(&crate::trash::percent_decode_pub(p)).into_owned())));
    }
    Ok(e.exec
        .as_deref()
        .and_then(|x| exec_tokens(x).into_iter().next())
        .map(|exe| which(&exe).unwrap_or_else(|| PathBuf::from(exe))))
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
Exec=gedit --new-window "%U" %i
Icon=org.gnome.gedit
MimeType=text/plain;text/x-c;
NoDisplay=false

[Desktop Action new-window]
Exec=gedit --other
"#;

    #[test]
    fn desktop_parsing() {
        let e = parse_desktop(DESKTOP).unwrap();
        assert_eq!(e.name, "Text Editor");
        assert_eq!(e.kind, "Application");
        assert_eq!(e.mime_types, ["text/plain", "text/x-c"]);
        assert_eq!(e.exec.as_deref(), Some(r#"gedit --new-window "%U" %i"#));
        let app = app_from_entry("org.gnome.gedit.desktop", &e).unwrap();
        assert_eq!(app.args, ["--new-window", "%U"]);
        assert!(parse_desktop("[Other]\nName=x").is_none());
    }

    #[test]
    fn exec_quoting() {
        assert_eq!(exec_tokens(r#"sh -c "echo \"hi\" \$HOME" %f"#), ["sh", "-c", r#"echo "hi" $HOME"#, "%f"]);
        assert_eq!(exec_tokens(r#"app "" x"#), ["app", "", "x"]);
        assert_eq!(exec_tokens("app 100%%"), ["app", "100%"]);
        assert_eq!(exec_tokens(r"my\sapp %F"), ["my", "app", "%F"]);
    }

    #[test]
    fn globs_and_mimeapps() {
        let globs = parse_globs2(
            "# x\n50:text/plain:*.txt\n50:text/x-csrc:*.c\n55:application/x-tar-gz:*.tar.gz\n50:application/gzip:*.gz\n50:text/x-makefile:Makefile\n",
        );
        assert_eq!(mime_for_name(&globs, "README.TXT").as_deref(), Some("text/plain"));
        assert_eq!(mime_for_name(&globs, "a.tar.gz").as_deref(), Some("application/x-tar-gz"));
        assert_eq!(mime_for_name(&globs, "Makefile").as_deref(), Some("text/x-makefile"));
        assert_eq!(mime_for_name(&globs, "x.unknown"), None);
        let m = parse_mimeapps(
            "[Default Applications]\ntext/plain=a.desktop;b.desktop\n[Added Associations]\ntext/plain=c.desktop;\n[Removed Associations]\ntext/plain=b.desktop\n",
        );
        assert_eq!(m.defaults["text/plain"], ["a.desktop", "b.desktop"]);
        assert_eq!(m.added["text/plain"], ["c.desktop"]);
        assert_eq!(m.removed["text/plain"], ["b.desktop"]);
    }

    #[test]
    fn desktop_link_target() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("l.desktop");
        std::fs::write(&p, "[Desktop Entry]\nType=Link\nURL=file:///home/u/My%20Docs\n").unwrap();
        assert_eq!(desktop_target(&p).unwrap(), Some(PathBuf::from("/home/u/My Docs")));
    }
}
