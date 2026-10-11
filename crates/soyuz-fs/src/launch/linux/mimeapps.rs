//! Default and associated applications (freedesktop "Association between MIME types and
//! applications" spec 1.0.1): `mimeapps.list` files in precedence order, desktop-specific
//! `$desktop-mimeapps.list` variants, legacy `defaults.list`, `[Added Associations]`,
//! `[Removed Associations]`, `[Default Applications]`, then the applications each data
//! directory declares (`mimeinfo.cache`, or the scanned `MimeType=` keys).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// The groups of one `mimeapps.list` / `defaults.list`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MimeApps {
    pub defaults: HashMap<String, Vec<String>>,
    pub added: HashMap<String, Vec<String>>,
    pub removed: HashMap<String, Vec<String>>,
}

/// Parse `mimeapps.list` (or `defaults.list`, which only has `[Default Applications]`).
pub(crate) fn parse_mimeapps(text: &str) -> MimeApps {
    let mut m = MimeApps::default();
    let mut section = "";
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
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
        let e = map.entry(k.trim().to_string()).or_default();
        for id in ids {
            if !e.contains(&id) {
                e.push(id);
            }
        }
    }
    m
}

/// `[MIME Cache]` of `mimeinfo.cache`: type → desktop ids.
pub(crate) fn parse_mimeinfo_cache(text: &str) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_cache = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_cache = line == "[MIME Cache]";
            continue;
        }
        if !in_cache {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.entry(k.trim().to_string())
                .or_default()
                .extend(v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
        }
    }
    out
}

/// One source in precedence order.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// A `mimeapps.list` / `defaults.list`.
    List(MimeApps),
    /// The applications a data directory declares for each type.
    Installed(HashMap<String, Vec<String>>),
}

/// The `mimeapps.list` locations in precedence order. `desktops` are the
/// `XDG_CURRENT_DESKTOP` entries (lowercased for the file names).
pub(crate) fn list_paths(config_dirs: &[PathBuf], desktops: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in config_dirs {
        for desk in desktops {
            out.push(d.join(format!("{}-mimeapps.list", desk.to_lowercase())));
        }
        out.push(d.join("mimeapps.list"));
    }
    out
}

/// All sources: config `mimeapps.list` files, then per data directory its
/// `applications/{$desktop-mimeapps.list, mimeapps.list, defaults.list}` followed by the
/// applications installed there. `installed_in(i)` gives the ids for data dir `i`.
pub(crate) fn sources(
    config_dirs: &[PathBuf],
    data_dirs: &[PathBuf],
    desktops: &[String],
    mut installed_in: impl FnMut(usize) -> HashMap<String, Vec<String>>,
) -> Vec<Source> {
    let read = |p: &PathBuf| std::fs::read_to_string(p).ok().map(|t| Source::List(parse_mimeapps(&t)));
    let mut out: Vec<Source> = list_paths(config_dirs, desktops).iter().filter_map(read).collect();
    for (i, d) in data_dirs.iter().enumerate() {
        let apps = d.join("applications");
        let mut files = list_paths(std::slice::from_ref(&apps), desktops);
        files.push(apps.join("defaults.list"));
        out.extend(files.iter().filter_map(read));
        out.push(Source::Installed(installed_in(i)));
    }
    out
}

/// Applications associated with one type, in preference order: per source, added
/// associations / installed apps not removed by a higher-precedence source.
pub(crate) fn associated(sources: &[Source], mime: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut removed: HashSet<&str> = HashSet::new();
    for s in sources {
        match s {
            Source::List(m) => {
                for id in m.added.get(mime).into_iter().flatten() {
                    if !removed.contains(id.as_str()) && !out.contains(id) {
                        out.push(id.clone());
                    }
                }
                removed.extend(m.removed.get(mime).into_iter().flatten().map(String::as_str));
            }
            Source::Installed(map) => {
                for id in map.get(mime).into_iter().flatten() {
                    if !removed.contains(id.as_str()) && !out.contains(id) {
                        out.push(id.clone());
                    }
                }
            }
        }
    }
    out
}

/// The default application for a type and its ancestors (`chain` = type first, then
/// its parents): the first installed id from `[Default Applications]` in precedence
/// order; failing that, the first associated application, for each type of the chain in
/// turn.
pub(crate) fn default_app(sources: &[Source], chain: &[String], usable: &dyn Fn(&str) -> bool) -> Option<String> {
    for mime in chain {
        for s in sources {
            if let Source::List(m) = s
                && let Some(id) = m.defaults.get(mime).into_iter().flatten().find(|id| usable(id))
            {
                return Some(id.clone());
            }
        }
        if let Some(id) = associated(sources, mime).into_iter().find(|id| usable(id)) {
            return Some(id);
        }
    }
    None
}

/// Every application for a type and its ancestors (default first), deduplicated.
pub(crate) fn all_apps(sources: &[Source], chain: &[String], usable: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(d) = default_app(sources, chain, usable) {
        out.push(d);
    }
    for mime in chain {
        for id in associated(sources, mime) {
            if usable(&id) && !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(t: &str) -> Source {
        Source::List(parse_mimeapps(t))
    }

    fn installed(pairs: &[(&str, &[&str])]) -> Source {
        Source::Installed(
            pairs.iter().map(|(m, ids)| (m.to_string(), ids.iter().map(|s| s.to_string()).collect())).collect(),
        )
    }

    #[test]
    fn parsing() {
        let m = parse_mimeapps(
            "[Default Applications]\ntext/plain=a.desktop;b.desktop\n[Added Associations]\ntext/plain=c.desktop;\n[Removed Associations]\ntext/plain=b.desktop\n",
        );
        assert_eq!(m.defaults["text/plain"], ["a.desktop", "b.desktop"]);
        assert_eq!(m.added["text/plain"], ["c.desktop"]);
        assert_eq!(m.removed["text/plain"], ["b.desktop"]);
        let c = parse_mimeinfo_cache("[MIME Cache]\nimage/png=eog.desktop;gimp.desktop;\n");
        assert_eq!(c["image/png"], ["eog.desktop", "gimp.desktop"]);
    }

    #[test]
    fn precedence_and_removals() {
        let srcs = vec![
            // User config: default for text/plain is not installed, so the next one counts.
            list(
                "[Default Applications]\ntext/plain=gone.desktop;user.desktop\n[Removed Associations]\ntext/plain=sys.desktop\n",
            ),
            list("[Default Applications]\ntext/plain=vendor.desktop\n[Added Associations]\nimage/png=viewer.desktop\n"),
            installed(&[("text/plain", &["sys.desktop", "other.desktop"]), ("image/png", &["gimp.desktop"])]),
        ];
        let usable = |id: &str| id != "gone.desktop";
        let chain = |m: &str| vec![m.to_string()];
        assert_eq!(default_app(&srcs, &chain("text/plain"), &usable).as_deref(), Some("user.desktop"));
        assert_eq!(associated(&srcs, "text/plain"), ["other.desktop"], "removed by higher precedence");
        assert_eq!(default_app(&srcs, &chain("image/png"), &usable).as_deref(), Some("viewer.desktop"));
        assert_eq!(all_apps(&srcs, &chain("image/png"), &usable), ["viewer.desktop", "gimp.desktop"]);
        // Subclass walk: a C source falls back to text/plain handlers.
        let c = vec!["text/x-csrc".to_string(), "text/plain".to_string()];
        assert_eq!(default_app(&srcs, &c, &usable).as_deref(), Some("user.desktop"));
        assert_eq!(all_apps(&srcs, &c, &usable), ["user.desktop", "other.desktop"]);
        assert_eq!(default_app(&srcs, &chain("x/none"), &usable), None);
    }

    #[test]
    fn removal_does_not_affect_higher_sources() {
        let srcs = vec![
            list("[Added Associations]\ntext/plain=a.desktop\n"),
            list("[Removed Associations]\ntext/plain=a.desktop;b.desktop\n"),
            installed(&[("text/plain", &["a.desktop", "b.desktop", "c.desktop"])]),
        ];
        assert_eq!(associated(&srcs, "text/plain"), ["a.desktop", "c.desktop"]);
    }

    #[test]
    fn list_paths_order() {
        let p = list_paths(&[PathBuf::from("/c")], &["GNOME".into(), "niri".into()]);
        let s: Vec<String> = p.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(s, ["/c/gnome-mimeapps.list", "/c/niri-mimeapps.list", "/c/mimeapps.list"]);
    }
}
