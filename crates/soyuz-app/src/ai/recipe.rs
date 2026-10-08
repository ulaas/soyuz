//! Recipes (roadmap A5 "determinism"): the write steps of a finished task saved as a
//! TOML job graph you can edit and re-run later without a model. Re-running plans every
//! step again (fresh dry run against the current disk) and shows the same plan cards and
//! approvals; nothing runs unasked.
//!
//! ```toml
//! name = "Sort downloads"
//! description = "Images and PDFs into folders"
//! created = "2026-10-08T12:00:00+02:00"
//! base = 'C:\Users\me\Downloads'
//!
//! [[steps]]
//! tool = "mkdir"
//! args = { path = "{folder}\\Images" }
//!
//! [[steps]]
//! tool = "move"
//! args = { sources = ["{folder}\\a.jpg"], dest_dir = "{folder}\\Images" }
//! ```
//!
//! `{folder}` stands for the folder the recipe runs in (the recording folder when saved).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Placeholder for the folder a recipe runs in.
pub const FOLDER_VAR: &str = "{folder}";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Recipe {
    pub name: String,
    pub description: String,
    pub created: String,
    /// Folder the recipe was recorded in (`{folder}` defaults to it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<PathBuf>,
    pub steps: Vec<RecipeStep>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecipeStep {
    pub tool: String,
    pub args: toml::Table,
}

/// `ai-recipes/` next to `config.toml`.
pub fn dir() -> PathBuf {
    soyuz_core::config::config_dir().join("ai-recipes")
}

/// JSON → TOML (nulls dropped; TOML has none).
pub fn json_to_toml(v: &Value) -> Option<toml::Value> {
    Some(match v {
        Value::Null => return None,
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => toml::Value::Integer(i),
            None => toml::Value::Float(n.as_f64()?),
        },
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(a) => toml::Value::Array(a.iter().filter_map(json_to_toml).collect()),
        Value::Object(m) => {
            toml::Value::Table(m.iter().filter_map(|(k, v)| json_to_toml(v).map(|t| (k.clone(), t))).collect())
        }
    })
}

pub fn toml_to_json(v: &toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => Value::from(*f),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Value::Object(t.iter().map(|(k, v)| (k.clone(), toml_to_json(v))).collect()),
    }
}

fn map_strings(v: &Value, f: &dyn Fn(&str) -> String) -> Value {
    match v {
        Value::String(s) => Value::String(f(s)),
        Value::Array(a) => Value::Array(a.iter().map(|x| map_strings(x, f)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), map_strings(x, f))).collect()),
        other => other.clone(),
    }
}

fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && if cfg!(windows) { s[..prefix.len()].eq_ignore_ascii_case(prefix) } else { s.starts_with(prefix) }
}

impl Recipe {
    /// Build from executed `(tool, args)` steps recorded in `base` (paths under it become
    /// `{folder}`; `dry_run` flags are dropped).
    pub fn from_steps(name: &str, description: &str, base: Option<&Path>, steps: &[(String, Value)]) -> Recipe {
        let prefix = base.map(|b| b.display().to_string().trim_end_matches(['/', '\\']).to_string());
        let steps = steps
            .iter()
            .map(|(tool, args)| {
                let mut args = args.clone();
                if let Some(m) = args.as_object_mut() {
                    m.remove("dry_run");
                }
                if let Some(p) = &prefix {
                    args = map_strings(&args, &|s| {
                        if starts_with_ci(s, p) && (s.len() == p.len() || s[p.len()..].starts_with(['/', '\\'])) {
                            format!("{FOLDER_VAR}{}", &s[p.len()..])
                        } else {
                            s.to_string()
                        }
                    });
                }
                let args = match json_to_toml(&args) {
                    Some(toml::Value::Table(t)) => t,
                    _ => toml::Table::new(),
                };
                RecipeStep { tool: tool.clone(), args }
            })
            .collect();
        Recipe {
            name: name.to_string(),
            description: description.to_string(),
            created: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
            base: base.map(Path::to_path_buf),
            steps,
        }
    }

    /// Steps with `{folder}` replaced, as JSON tool arguments (`dry_run: false`).
    pub fn resolved(&self, folder: &Path) -> Vec<(String, Value)> {
        let f = folder.display().to_string();
        let f = f.trim_end_matches(['/', '\\']).to_string();
        self.steps
            .iter()
            .map(|s| {
                let mut args =
                    map_strings(&toml_to_json(&toml::Value::Table(s.args.clone())), &|x| x.replace(FOLDER_VAR, &f));
                if let Some(m) = args.as_object_mut() {
                    m.remove("dry_run");
                }
                (s.tool.clone(), args)
            })
            .collect()
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    pub fn parse(text: &str) -> Result<Recipe, String> {
        let r: Recipe = toml::from_str(text).map_err(|e| e.to_string())?;
        for s in &r.steps {
            let d = soyuz_ai::tools::def(&s.tool).ok_or_else(|| format!("unknown tool “{}”", s.tool))?;
            if d.risk == soyuz_ai::tools::Risk::Read && d.category != soyuz_ai::tools::Category::Ui {
                return Err(format!("“{}” is a read tool; recipes only hold changes", s.tool));
            }
        }
        Ok(r)
    }
}

/// File name for a recipe name (`Sort downloads` → `sort-downloads.toml`).
pub fn file_name(name: &str) -> String {
    let mut s: String = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-');
    format!("{}.toml", if s.is_empty() { "recipe" } else { s })
}

/// Save atomically into `dir`; returns the file path.
pub fn save(dir: &Path, r: &Recipe) -> Result<PathBuf, String> {
    let _ = soyuz_fs::create_dir_all(dir);
    let path = dir.join(file_name(&r.name));
    soyuz_fs::write_atomic(&path, r.to_toml()?.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Every parsable recipe in `dir`, by name (worker thread).
pub fn list(dir: &Path) -> Vec<(PathBuf, Result<Recipe, String>)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(PathBuf, Result<Recipe, String>)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")))
        .map(|p| {
            let r = std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| Recipe::parse(&t));
            (p, r)
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn record_save_and_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("Downloads");
        let steps = vec![
            ("mkdir".to_string(), json!({"path": base.join("Images"), "dry_run": false})),
            (
                "move".to_string(),
                json!({"sources": [base.join("a.jpg")], "dest_dir": base.join("Images"), "conflict": "ask", "x": null}),
            ),
        ];
        let r = Recipe::from_steps("Sort downloads!", "imgs", Some(&base), &steps);
        let text = r.to_toml().unwrap();
        assert!(text.contains("{folder}"), "{text}");
        assert!(!text.contains("dry_run"));
        let path = save(dir.path(), &r).unwrap();
        assert!(path.ends_with("sort-downloads.toml"));
        let listed = list(dir.path());
        assert_eq!(listed.len(), 1);
        let back = listed[0].1.clone().unwrap();
        assert_eq!(back, r);
        let other = dir.path().join("Elsewhere");
        let res = back.resolved(&other);
        assert_eq!(res[0].1["path"], json!(other.join("Images").display().to_string()));
        assert_eq!(res[1].1["sources"][0], json!(other.join("a.jpg").display().to_string()));
        assert!(Recipe::parse("[[steps]]\ntool = \"list_dir\"\nargs = {}").is_err(), "read tools rejected");
        assert!(Recipe::parse("[[steps]]\ntool = \"nope\"\nargs = {}").is_err());
    }
}
