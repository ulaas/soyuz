//! "Add to Claude Desktop / Claude Code / VS Code": the client config snippet that starts
//! `soyuz --mcp --mcp-client <name>`, merged into that tool's JSON config file. The
//! settings page shows the snippet and a line diff and writes only after confirmation
//! (with a `.soyuz-backup` copy of the old file).
//!
//! Config locations and formats (verified against the official docs, 2026-10):
//! * Claude Desktop — `%APPDATA%\Claude\claude_desktop_config.json` (macOS:
//!   `~/Library/Application Support/Claude/…`), `{"mcpServers": {name: {command, args}}}`.
//! * Claude Code — user scope in `~/.claude.json`, top-level
//!   `{"mcpServers": {name: {"type": "stdio", command, args, env}}}`.
//! * VS Code — user `mcp.json` (`%APPDATA%\Code\User\mcp.json`),
//!   `{"servers": {name: {"type": "stdio", command, args}}}`.
//!
//! Key order and the rest of the file are preserved (JSON is re-printed with two-space
//! indentation). Files with comments (JSONC) are refused; the snippet can be pasted by
//! hand.

use std::path::{Path, PathBuf};

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

/// Name of the server entry written into the client config.
pub const ENTRY_NAME: &str = "soyuz";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    ClaudeDesktop,
    ClaudeCode,
    VsCode,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::ClaudeDesktop, Target::ClaudeCode, Target::VsCode];

    pub fn label(self) -> &'static str {
        match self {
            Target::ClaudeDesktop => "Claude Desktop",
            Target::ClaudeCode => "Claude Code",
            Target::VsCode => "VS Code",
        }
    }

    /// The `--mcp-client` name (and `[[ai.mcp_clients]]` entry) for this tool.
    pub fn client_name(self) -> &'static str {
        match self {
            Target::ClaudeDesktop => "claude-desktop",
            Target::ClaudeCode => "claude-code",
            Target::VsCode => "vscode",
        }
    }

    fn servers_key(self) -> &'static str {
        match self {
            Target::VsCode => "servers",
            _ => "mcpServers",
        }
    }

    /// The tool's user-level config file.
    pub fn config_path(self) -> Option<PathBuf> {
        let home = soyuz_fs::home();
        let roaming = || -> Option<PathBuf> {
            if cfg!(windows) {
                std::env::var_os("APPDATA").map(PathBuf::from)
            } else if cfg!(target_os = "macos") {
                Some(home.join("Library").join("Application Support"))
            } else {
                Some(std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")))
            }
        };
        match self {
            Target::ClaudeDesktop => roaming().map(|d| d.join("Claude").join("claude_desktop_config.json")),
            Target::ClaudeCode => Some(home.join(".claude.json")),
            Target::VsCode => roaming().map(|d| d.join("Code").join("User").join("mcp.json")),
        }
    }

    /// The server entry for `exe`.
    pub fn entry(self, exe: &Path) -> OJson {
        let args = OJson::Arr(
            ["--mcp", "--mcp-client", self.client_name()].iter().map(|s| OJson::Str(s.to_string())).collect(),
        );
        let command = OJson::Str(soyuz_fs::strip_verbatim(exe).to_string_lossy().into_owned());
        let mut fields = Vec::new();
        if self != Target::ClaudeDesktop {
            fields.push(("type".to_string(), OJson::Str("stdio".into())));
        }
        fields.push(("command".to_string(), command));
        fields.push(("args".to_string(), args));
        if self == Target::ClaudeCode {
            fields.push(("env".to_string(), OJson::Obj(Vec::new())));
        }
        OJson::Obj(fields)
    }

    /// The snippet as the user would paste it (`{"mcpServers": {"soyuz": …}}`).
    pub fn snippet(self, exe: &Path) -> String {
        let v = OJson::Obj(vec![(
            self.servers_key().to_string(),
            OJson::Obj(vec![(ENTRY_NAME.to_string(), self.entry(exe))]),
        )]);
        v.pretty()
    }
}

/// An insertion-ordered JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum OJson {
    Null,
    Bool(bool),
    Num(serde_json::Number),
    Str(String),
    Arr(Vec<OJson>),
    Obj(Vec<(String, OJson)>),
}

impl<'de> Deserialize<'de> for OJson {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = OJson;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_unit<E: de::Error>(self) -> Result<OJson, E> {
                Ok(OJson::Null)
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<OJson, E> {
                Ok(OJson::Bool(b))
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<OJson, E> {
                Ok(OJson::Num(n.into()))
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<OJson, E> {
                Ok(OJson::Num(n.into()))
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<OJson, E> {
                serde_json::Number::from_f64(n).map(OJson::Num).ok_or_else(|| E::custom("bad number"))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<OJson, E> {
                Ok(OJson::Str(s.to_string()))
            }
            fn visit_string<E: de::Error>(self, s: String) -> Result<OJson, E> {
                Ok(OJson::Str(s))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<OJson, A::Error> {
                let mut v = Vec::new();
                while let Some(x) = a.next_element()? {
                    v.push(x);
                }
                Ok(OJson::Arr(v))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<OJson, A::Error> {
                let mut v: Vec<(String, OJson)> = Vec::new();
                while let Some((k, x)) = a.next_entry::<String, OJson>()? {
                    if let Some(slot) = v.iter_mut().find(|(kk, _)| *kk == k) {
                        slot.1 = x;
                    } else {
                        v.push((k, x));
                    }
                }
                Ok(OJson::Obj(v))
            }
        }
        d.deserialize_any(V)
    }
}

impl OJson {
    /// Pretty print with two-space indentation (serde_json style).
    pub fn pretty(&self) -> String {
        let mut s = String::new();
        self.write(&mut s, 0);
        s
    }

    fn write(&self, out: &mut String, depth: usize) {
        let pad = |out: &mut String, d: usize| out.push_str(&"  ".repeat(d));
        match self {
            OJson::Null => out.push_str("null"),
            OJson::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            OJson::Num(n) => out.push_str(&n.to_string()),
            OJson::Str(s) => out.push_str(&serde_json::to_string(s).unwrap_or_default()),
            OJson::Arr(v) if v.is_empty() => out.push_str("[]"),
            OJson::Obj(v) if v.is_empty() => out.push_str("{}"),
            OJson::Arr(v) => {
                out.push_str("[\n");
                for (i, x) in v.iter().enumerate() {
                    pad(out, depth + 1);
                    x.write(out, depth + 1);
                    out.push_str(if i + 1 < v.len() { ",\n" } else { "\n" });
                }
                pad(out, depth);
                out.push(']');
            }
            OJson::Obj(v) => {
                out.push_str("{\n");
                for (i, (k, x)) in v.iter().enumerate() {
                    pad(out, depth + 1);
                    out.push_str(&serde_json::to_string(k).unwrap_or_default());
                    out.push_str(": ");
                    x.write(out, depth + 1);
                    out.push_str(if i + 1 < v.len() { ",\n" } else { "\n" });
                }
                pad(out, depth);
                out.push('}');
            }
        }
    }

    /// Set `key` in an object (replacing in place, else appending).
    fn set(&mut self, key: &str, value: OJson) -> Result<(), String> {
        let OJson::Obj(fields) = self else { return Err("expected a JSON object".into()) };
        match fields.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => fields.push((key.to_string(), value)),
        }
        Ok(())
    }

    fn get_mut(&mut self, key: &str) -> Option<&mut OJson> {
        match self {
            OJson::Obj(fields) => fields.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Merge the Soyuz entry into `existing` (empty / missing file = new object).
pub fn merge(target: Target, existing: Option<&str>, exe: &Path) -> Result<String, String> {
    let mut root = match existing.map(str::trim) {
        None | Some("") => OJson::Obj(Vec::new()),
        Some(text) => serde_json::from_str::<OJson>(text).map_err(|e| {
            format!("cannot parse the existing file ({e}); it may contain comments — paste the snippet by hand")
        })?,
    };
    if !matches!(root, OJson::Obj(_)) {
        return Err("the existing file is not a JSON object".into());
    }
    let key = target.servers_key();
    if root.get_mut(key).is_none() {
        root.set(key, OJson::Obj(Vec::new()))?;
    }
    let servers = root.get_mut(key).ok_or("internal error")?;
    if !matches!(servers, OJson::Obj(_)) {
        return Err(format!("“{key}” in the existing file is not an object"));
    }
    servers.set(ENTRY_NAME, target.entry(exe))?;
    let mut text = root.pretty();
    text.push('\n');
    Ok(text)
}

/// One line of a diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLine {
    Same(String),
    Added(String),
    Removed(String),
}

/// Line diff (common prefix / suffix trimmed, LCS on the rest, capped).
pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut out: Vec<DiffLine> = a[..pre].iter().map(|l| DiffLine::Same(l.to_string())).collect();
    if am.len() * bm.len() > 4_000_000 {
        out.extend(am.iter().map(|l| DiffLine::Removed(l.to_string())));
        out.extend(bm.iter().map(|l| DiffLine::Added(l.to_string())));
    } else {
        // LCS table.
        let (n, m) = (am.len(), bm.len());
        let mut t = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                t[i * (m + 1) + j] = if am[i] == bm[j] {
                    t[(i + 1) * (m + 1) + j + 1] + 1
                } else {
                    t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                out.push(DiffLine::Same(am[i].to_string()));
                i += 1;
                j += 1;
            } else if j < m && (i == n || t[i * (m + 1) + j + 1] >= t[(i + 1) * (m + 1) + j]) {
                out.push(DiffLine::Added(bm[j].to_string()));
                j += 1;
            } else {
                out.push(DiffLine::Removed(am[i].to_string()));
                i += 1;
            }
        }
    }
    out.extend(a[a.len() - suf..].iter().map(|l| DiffLine::Same(l.to_string())));
    out
}

/// What writing a target would do.
#[derive(Clone, Debug)]
pub struct InstallPreview {
    pub target: Target,
    pub path: PathBuf,
    /// The file as read (`None` = it does not exist yet).
    pub old_text: Option<String>,
    pub new_text: String,
    pub diff: Vec<DiffLine>,
}

impl InstallPreview {
    pub fn changed(&self) -> bool {
        self.old_text.as_deref() != Some(self.new_text.as_str())
    }
}

/// Build the preview for `target` (reads the config file).
pub fn preview(target: Target, exe: &Path) -> Result<InstallPreview, String> {
    let path = target.config_path().ok_or("cannot find the config folder")?;
    preview_at(target, &path, exe)
}

pub fn preview_at(target: Target, path: &Path, exe: &Path) -> Result<InstallPreview, String> {
    let old_text = match std::fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let new_text = merge(target, old_text.as_deref(), exe)?;
    let diff = line_diff(old_text.as_deref().unwrap_or(""), &new_text);
    Ok(InstallPreview { target, path: path.to_path_buf(), old_text, new_text, diff })
}

/// Write a confirmed preview. Refuses when the file changed since the preview.
pub fn apply(p: &InstallPreview) -> Result<(), String> {
    let now = match std::fs::read_to_string(&p.path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", p.path.display())),
    };
    if now != p.old_text {
        return Err("the file changed since the preview; open the preview again".into());
    }
    if let Some(dir) = p.path.parent() {
        soyuz_fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if let Some(old) = &p.old_text {
        let mut backup = p.path.as_os_str().to_owned();
        backup.push(".soyuz-backup");
        soyuz_fs::write_atomic(Path::new(&backup), old.as_bytes()).map_err(|e| format!("cannot write backup: {e}"))?;
    }
    soyuz_fs::write_atomic(&p.path, p.new_text.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_order_and_other_servers() {
        let exe = Path::new(r"C:\Apps\Soyuz\soyuz.exe");
        let existing = r#"{"zeta": 1, "mcpServers": {"git": {"command": "uvx", "args": ["mcp-server-git"]}}, "alpha": [true, null, 1.5]}"#;
        let out = merge(Target::ClaudeDesktop, Some(existing), exe).unwrap();
        let zeta = out.find("\"zeta\"").unwrap();
        let alpha = out.find("\"alpha\"").unwrap();
        assert!(zeta < alpha, "key order kept:\n{out}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mcpServers"]["git"]["command"], "uvx");
        assert_eq!(v["mcpServers"]["soyuz"]["command"], r"C:\Apps\Soyuz\soyuz.exe");
        assert_eq!(v["mcpServers"]["soyuz"]["args"][2], "claude-desktop");
        assert!(v["mcpServers"]["soyuz"].get("type").is_none());
        // Idempotent.
        assert_eq!(merge(Target::ClaudeDesktop, Some(&out), exe).unwrap(), out);

        let code = merge(Target::ClaudeCode, None, exe).unwrap();
        let v: serde_json::Value = serde_json::from_str(&code).unwrap();
        assert_eq!(v["mcpServers"]["soyuz"]["type"], "stdio");
        let vs = merge(Target::VsCode, Some("{}"), exe).unwrap();
        let v: serde_json::Value = serde_json::from_str(&vs).unwrap();
        assert_eq!(v["servers"]["soyuz"]["args"][2], "vscode");
        assert!(merge(Target::VsCode, Some("// comment\n{}"), exe).is_err());
        assert!(merge(Target::VsCode, Some("[1]"), exe).is_err());
    }

    #[test]
    fn diff_marks_only_the_change() {
        let d = line_diff("a\nb\nc\n", "a\nb\nx\nc\n");
        assert_eq!(
            d,
            vec![
                DiffLine::Same("a".into()),
                DiffLine::Same("b".into()),
                DiffLine::Added("x".into()),
                DiffLine::Same("c".into())
            ]
        );
        let d = line_diff("", "{\n}");
        assert!(d.iter().all(|l| matches!(l, DiffLine::Added(_))));
    }

    #[test]
    fn preview_and_apply_with_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg").join("claude_desktop_config.json");
        let exe = Path::new("soyuz.exe");
        let p = preview_at(Target::ClaudeDesktop, &path, exe).unwrap();
        assert!(p.old_text.is_none() && p.changed());
        apply(&p).unwrap();
        let p2 = preview_at(Target::ClaudeDesktop, &path, exe).unwrap();
        assert!(!p2.changed());
        std::fs::write(&path, "{\"x\": 1}").unwrap();
        assert!(apply(&p2).is_err(), "stale preview refused");
        let p3 = preview_at(Target::ClaudeDesktop, &path, exe).unwrap();
        apply(&p3).unwrap();
        assert_eq!(std::fs::read_to_string(path.with_extension("json.soyuz-backup")).unwrap(), "{\"x\": 1}");
    }
}
