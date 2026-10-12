//! Breadcrumb segmentation, path completion and the address field's editing logic,
//! shared by both frontends (the GUI's widget lives in `soyuz-app`'s `ui/breadcrumb.rs`,
//! the terminal UI's in `soyuz-tui`'s `view/crumbs.rs` and `input/address.rs`).

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use soyuz_core::{EntryInfo, Location};

use crate::cmd::Cmd;
use crate::core::AddressState;
use crate::services::{AuxState, Services};
use crate::state::pane::PaneId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrumbKind {
    /// Filesystem root / drive.
    Root,
    /// The user's home directory (collapsed prefix).
    Home,
    Folder,
    /// The archive file itself.
    Archive,
    /// Trash, Computer, Recent, Tags, Search.
    Virtual,
    /// The server of a remote location (`user@host`).
    Server,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Crumb {
    pub label: String,
    pub location: Location,
    pub kind: CrumbKind,
}

/// Split a location into clickable segments. With `home` set, a path inside the home
/// directory starts with a single "Home" crumb instead of the root / Users / name chain.
pub fn crumbs(loc: &Location, home: Option<&Path>) -> Vec<Crumb> {
    match loc {
        Location::Dir(p) => path_crumbs(p, home),
        Location::Archive { archive, inner } => {
            let mut v = path_crumbs(archive.parent().unwrap_or(archive), home);
            v.push(Crumb {
                label: soyuz_core::entry::display_name(archive),
                location: Location::Archive { archive: archive.clone(), inner: String::new() },
                kind: CrumbKind::Archive,
            });
            let mut acc = String::new();
            for seg in inner.split('/').filter(|s| !s.is_empty()) {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(seg);
                v.push(Crumb {
                    label: seg.to_string(),
                    location: Location::Archive { archive: archive.clone(), inner: acc.clone() },
                    kind: CrumbKind::Folder,
                });
            }
            v
        }
        Location::Remote(u) => match soyuz_vfs::RemoteUri::parse(u) {
            Ok(uri) => {
                let mut v = vec![Crumb {
                    label: uri.server_label(),
                    location: Location::Remote(uri.root().display()),
                    kind: CrumbKind::Server,
                }];
                let mut acc = String::new();
                for seg in uri.path.split('/').filter(|s| !s.is_empty()) {
                    acc.push('/');
                    acc.push_str(seg);
                    v.push(Crumb {
                        label: seg.to_string(),
                        location: Location::Remote(uri.with_path(&acc).display()),
                        kind: CrumbKind::Folder,
                    });
                }
                v
            }
            Err(_) => vec![Crumb { label: u.clone(), location: loc.clone(), kind: CrumbKind::Server }],
        },
        other => vec![Crumb { label: location_title(other), location: other.clone(), kind: CrumbKind::Virtual }],
    }
}

fn path_crumbs(p: &Path, home: Option<&Path>) -> Vec<Crumb> {
    let p = soyuz_fs::strip_verbatim(p);
    let mut out = Vec::new();
    let mut acc = PathBuf::new();
    let mut rest: &Path = &p;
    if let Some(h) = home.map(soyuz_fs::strip_verbatim)
        && !h.as_os_str().is_empty()
        && let Ok(rel) = p.strip_prefix(&h)
    {
        out.push(Crumb {
            label: tr!("ops-place-home").into(),
            location: Location::Dir(h.clone()),
            kind: CrumbKind::Home,
        });
        acc = h.clone();
        rest = rel;
        for c in rest.components() {
            if let Component::Normal(s) = c {
                acc.push(s);
                out.push(Crumb {
                    label: s.to_string_lossy().into_owned(),
                    location: Location::Dir(acc.clone()),
                    kind: CrumbKind::Folder,
                });
            }
        }
        return out;
    }
    let mut prefix_label: Option<String> = None;
    for c in rest.components() {
        match c {
            Component::Prefix(pre) => {
                acc.push(pre.as_os_str());
                prefix_label = Some(pre.as_os_str().to_string_lossy().trim_start_matches(r"\\").to_string());
            }
            Component::RootDir => {
                acc.push(c.as_os_str());
                let label = prefix_label.take().unwrap_or_else(|| "/".to_string());
                out.push(Crumb { label, location: Location::Dir(acc.clone()), kind: CrumbKind::Root });
            }
            Component::Normal(s) => {
                if let Some(label) = prefix_label.take() {
                    out.push(Crumb { label, location: Location::Dir(acc.clone()), kind: CrumbKind::Root });
                }
                acc.push(s);
                out.push(Crumb {
                    label: s.to_string_lossy().into_owned(),
                    location: Location::Dir(acc.clone()),
                    kind: CrumbKind::Folder,
                });
            }
            Component::CurDir | Component::ParentDir => {}
        }
    }
    if let Some(label) = prefix_label {
        out.push(Crumb { label, location: Location::Dir(acc), kind: CrumbKind::Root });
    }
    out
}

/// Title for a location (tab titles, window title, single-crumb virtual places).
pub fn location_title(loc: &Location) -> String {
    match loc {
        Location::Dir(p) => {
            let p = soyuz_fs::strip_verbatim(p);
            match p.file_name() {
                Some(n) => n.to_string_lossy().into_owned(),
                None => {
                    let s = p.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
                    if s.is_empty() { "/".into() } else { s }
                }
            }
        }
        Location::Archive { archive, inner } => match inner.rsplit('/').next() {
            Some(last) if !last.is_empty() => last.to_string(),
            _ => soyuz_core::entry::display_name(archive),
        },
        Location::Trash => tr!("ops-place-trash").into(),
        Location::Computer => {
            if cfg!(windows) { tr!("ops-place-computer") } else { tr!("ops-place-computer-unix") }.into()
        }
        Location::Recent => tr!("ops-place-recent").into(),
        Location::Timeline => tr!("ops-place-timeline").into(),
        Location::Tags(t) => tr!("ops-place-tag", name = crate::services::tags::display_name(t)),
        // Smart folders show their name.
        Location::Search(id) => {
            crate::services::search::spec(*id).and_then(|s| s.smart).unwrap_or_else(|| tr!("ops-place-search").into())
        }
        Location::Remote(u) => match soyuz_vfs::RemoteUri::parse(u) {
            Ok(uri) => uri.file_name().map(str::to_string).unwrap_or_else(|| uri.server_label()),
            Err(_) => u.clone(),
        },
    }
}

/// Split typed path text into the directory to list and the partial name being typed:
/// `"C:\Users\ul"` → `("C:\Users\", "ul")`, `"/usr/"` → `("/usr/", "")`.
pub fn split_for_completion(text: &str) -> Option<(PathBuf, String)> {
    let idx = text.rfind(['/', '\\'])?;
    let (dir, rest) = text.split_at(idx + 1);
    let dir = if dir.is_empty() { "/" } else { dir };
    let expanded = match Location::parse(dir) {
        Some(Location::Dir(p)) => p,
        _ => PathBuf::from(dir),
    };
    Some((expanded, rest.to_string()))
}

/// Names from `candidates` that start with `prefix` (case-insensitive), sorted naturally
/// and capped at `limit`.
pub fn completions<'a>(candidates: impl Iterator<Item = &'a str>, prefix: &str, limit: usize) -> Vec<String> {
    let lp = prefix.to_lowercase();
    let mut v: Vec<String> =
        candidates.filter(|n| n.to_lowercase().starts_with(&lp) && *n != prefix).map(str::to_string).collect();
    v.sort_by(|a, b| soyuz_core::natural_cmp(a, b, true));
    v.truncate(limit);
    v
}

// ---------------------------------------------------------------------------------------
// The address field's editing logic, shared by the GUI (`ui/breadcrumb.rs`) and the
// terminal UI (`soyuz-tui`, `input/address.rs`): both frontends call these, so typing,
// completing and submitting a path behave the same in both.
// ---------------------------------------------------------------------------------------

/// Folder completions offered at most.
pub const COMPLETION_LIMIT: usize = 8;

/// The completion state of the typed text: the folder being listed, the partial name and
/// the matching subfolder names (empty while the folder is still loading).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    /// The typed parent folder (empty when the text has no separator).
    pub dir: PathBuf,
    /// The partial name after the last separator.
    pub prefix: String,
    pub names: Vec<String>,
}

/// The last computed completion, keyed on the typed text and the identity of the aux
/// listing it came from: a frame with the same text and listing reuses it instead of
/// lowercasing every entry again.
struct Memo {
    text: String,
    entries: Option<Arc<Vec<EntryInfo>>>,
    names: Vec<String>,
}

thread_local! {
    static MEMO: RefCell<Option<Memo>> = const { RefCell::new(None) };
}

/// Folder names of `entries` completing `prefix`, memoized on (`text`, `entries`).
fn memo_names(text: &str, prefix: &str, entries: Option<Arc<Vec<EntryInfo>>>) -> Vec<String> {
    MEMO.with_borrow_mut(|memo| {
        if let Some(m) = memo.as_ref()
            && m.text == text
            && match (&m.entries, &entries) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        {
            return m.names.clone();
        }
        let names = entries.as_ref().map_or_else(Vec::new, |e| {
            completions(e.iter().filter(|e| e.is_dir_like()).map(|e| e.name.as_str()), prefix, COMPLETION_LIMIT)
        });
        *memo = Some(Memo { text: text.to_string(), entries, names: names.clone() });
        names
    })
}

/// The completion of `text`, requesting the typed folder's listing (or its refresh) from
/// the aux cache. Call it once per frame while the field is edited.
pub fn completion(services: &mut Services, text: &str) -> Completion {
    let Some((dir, prefix)) = split_for_completion(text) else { return Completion::default() };
    if dir.as_os_str().is_empty() {
        return Completion::default();
    }
    let entries = services.aux_entries(&dir);
    let names = memo_names(text, &prefix, entries);
    Completion { dir, prefix, names }
}

/// [`completion`] from what the aux cache already holds, without requesting anything
/// (for views that only read the core).
pub fn completion_cached(services: &Services, text: &str) -> Completion {
    let Some((dir, prefix)) = split_for_completion(text) else { return Completion::default() };
    if dir.as_os_str().is_empty() {
        return Completion::default();
    }
    let entries = match services.aux_state(&dir) {
        Some(AuxState::Ready { entries, .. }) => Some(entries.clone()),
        _ => None,
    };
    let names = memo_names(text, &prefix, entries);
    Completion { dir, prefix, names }
}

/// Open the address field on `location` (a click on the current crumb or the empty
/// field).
pub fn start_edit(address: &mut AddressState, location: &Location) {
    address.editing = true;
    address.text = location.display();
    address.request_focus = true;
    address.completion = None;
}

/// Leave the field without navigating (Esc).
pub fn cancel(address: &mut AddressState) {
    address.editing = false;
    address.completion = None;
}

/// Move the highlighted completion down (`down`) or up among `n` names, wrapping.
pub fn cycle(address: &mut AddressState, n: usize, down: bool) {
    if n == 0 {
        return;
    }
    address.completion = Some(match (address.completion, down) {
        (None, true) => 0,
        (None, false) => n - 1,
        (Some(s), true) => (s + 1) % n,
        (Some(s), false) => (s + n - 1) % n,
    });
}

/// Replace the partial name after the last separator of `text` with `name` and a
/// separator (the text's own kind of separator when it has backslashes).
pub fn accept(text: &mut String, name: &str) {
    let base = text.rfind(['/', '\\']).map_or(String::new(), |i| text[..=i].to_string());
    let sep = if text.contains('\\') { '\\' } else { std::path::MAIN_SEPARATOR };
    *text = format!("{base}{name}{sep}");
}

/// Tab: accept the highlighted completion, or the first one. Returns whether the text
/// changed (the caller puts the cursor at the end).
pub fn tab(address: &mut AddressState, names: &[String]) -> bool {
    let Some(name) = address.completion.and_then(|s| names.get(s)).or(names.first()) else { return false };
    let name = name.clone();
    accept(&mut address.text, &name);
    address.completion = None;
    true
}

/// Enter: accept the highlighted completion (if any), then [`submit`]. `c` is the
/// completion of the text as it was before.
pub fn enter(address: &mut AddressState, services: &mut Services, pane: PaneId, cmds: &mut Vec<Cmd>, c: &Completion) {
    if let Some(name) = address.completion.and_then(|s| c.names.get(s)) {
        let name = name.clone();
        accept(&mut address.text, &name);
    }
    submit(address, services, pane, cmds, &c.dir, &c.prefix);
}

/// Navigate pane `pane` to the typed location (paths with `~`, archives, remote URIs,
/// `trash:` …: [`Location::parse`]). A path to a file in the listed folder `dir`
/// (typed name `prefix`) opens its folder and selects it.
pub fn submit(
    address: &mut AddressState,
    services: &mut Services,
    pane: PaneId,
    cmds: &mut Vec<Cmd>,
    dir: &Path,
    prefix: &str,
) {
    address.editing = false;
    let text = address.text.trim().to_string();
    let Some(loc) = Location::parse(&text) else { return };
    if let Location::Dir(path) = &loc {
        // Typed a file name inside a folder we already listed → reveal it.
        let is_file = services
            .aux_entries(dir)
            .is_some_and(|entries| entries.iter().any(|e| e.name == prefix && !e.is_dir_like() && !prefix.is_empty()));
        if is_file {
            cmds.push(Cmd::RevealPath(path.clone()));
            return;
        }
    }
    cmds.push(Cmd::FocusPane(pane));
    cmds.push(Cmd::Navigate { pane, location: loc, select: None });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(v: &[Crumb]) -> Vec<&str> {
        v.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    #[cfg(unix)]
    fn unix_paths() {
        let c = crumbs(&Location::Dir("/usr/local/bin".into()), None);
        assert_eq!(labels(&c), ["/", "usr", "local", "bin"]);
        assert_eq!(c[0].kind, CrumbKind::Root);
        assert_eq!(c[2].location, Location::Dir("/usr/local".into()));
        let c = crumbs(&Location::Dir("/".into()), None);
        assert_eq!(labels(&c), ["/"]);
    }

    #[test]
    #[cfg(windows)]
    fn windows_paths() {
        let c = crumbs(&Location::Dir(r"C:\Windows\System32".into()), None);
        assert_eq!(labels(&c), ["C:", "Windows", "System32"]);
        assert_eq!(c[0].location, Location::Dir(r"C:\".into()));
        assert_eq!(c[2].location, Location::Dir(r"C:\Windows\System32".into()));
        let c = crumbs(&Location::Dir(r"\\?\D:\data".into()), None);
        assert_eq!(labels(&c), ["D:", "data"]);
        let c = crumbs(&Location::Dir(r"\\server\share\x".into()), None);
        assert_eq!(labels(&c), [r"server\share", "x"]);
        let c = crumbs(&Location::Dir(r"C:\".into()), None);
        assert_eq!(labels(&c), ["C:"]);
    }

    #[test]
    fn home_is_collapsed() {
        let home = std::env::temp_dir().join("soyuz-home");
        let p = home.join("src").join("proj");
        let c = crumbs(&Location::Dir(p.clone()), Some(&home));
        assert_eq!(labels(&c), ["Home", "src", "proj"]);
        assert_eq!(c[0].kind, CrumbKind::Home);
        assert_eq!(c[2].location, Location::Dir(p));
        let c = crumbs(&Location::Dir(home.clone()), Some(&home));
        assert_eq!(labels(&c), ["Home"]);
    }

    #[test]
    fn archive_and_virtual() {
        let a = std::env::temp_dir().join("a.zip");
        let c = crumbs(&Location::Archive { archive: a.clone(), inner: "x/y".into() }, None);
        let l = labels(&c);
        assert_eq!(&l[l.len() - 3..], ["a.zip", "x", "y"]);
        assert_eq!(c.last().unwrap().location, Location::Archive { archive: a, inner: "x/y".into() });
        let t = crumbs(&Location::Trash, None);
        assert_eq!(labels(&t), ["Trash"]);
        assert_eq!(t[0].kind, CrumbKind::Virtual);
    }

    #[test]
    fn titles() {
        assert_eq!(location_title(&Location::Dir(std::env::temp_dir().join("abc"))), "abc");
        assert_eq!(location_title(&Location::Computer), if cfg!(windows) { "This PC" } else { "Computer" });
        #[cfg(windows)]
        assert_eq!(location_title(&Location::Dir(r"C:\".into())), "C:");
    }

    #[test]
    fn completion_helpers() {
        let (dir, rest) = split_for_completion("/usr/lo").unwrap();
        assert_eq!(dir, PathBuf::from("/usr/"));
        assert_eq!(rest, "lo");
        let (_, rest) = split_for_completion(r"C:\Users\").unwrap();
        assert_eq!(rest, "");
        assert!(split_for_completion("nothing").is_none());
        let names = ["local", "lib", "Lib64", "share", "lo"];
        assert_eq!(completions(names.iter().copied(), "l", 10), ["lib", "Lib64", "lo", "local"]);
        assert_eq!(completions(names.iter().copied(), "lo", 10), ["local"], "exact match is not offered");
        assert_eq!(completions(names.iter().copied(), "l", 2).len(), 2);
    }

    fn services() -> Services {
        Services::new(
            crate::host::no_wake(),
            crate::services::thumbs::Thumbs::new(crate::host::no_wake(), crate::services::preview_gfx::no_upload(), 1),
            std::time::Duration::from_millis(50),
        )
    }

    /// `dir` listed into the aux cache (as the lister would).
    fn listed(s: &mut Services, dir: &Path) -> Arc<Vec<EntryInfo>> {
        let entries: Vec<EntryInfo> =
            std::fs::read_dir(dir).unwrap().map(|e| soyuz_fs::list::stat(&e.unwrap().path(), false).unwrap()).collect();
        let entries = Arc::new(entries);
        s.aux.insert(dir.to_path_buf(), AuxState::Ready { entries: entries.clone(), at: std::time::Instant::now() });
        entries
    }

    fn typed(dir: &Path, rest: &str) -> String {
        format!("{}{}{rest}", dir.display(), std::path::MAIN_SEPARATOR)
    }

    #[test]
    fn completion_lists_subfolders_and_is_memoized() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        for n in ["sub", "Src", "other"] {
            std::fs::create_dir(d.join(n)).unwrap();
        }
        std::fs::write(d.join("sunny.txt"), b"x").unwrap();
        let mut s = services();
        // Not listed yet: nothing, and the listing was requested.
        let text = typed(d, "s");
        let c = completion(&mut s, &text);
        assert!(c.names.is_empty());
        assert_eq!(c.prefix, "s");
        assert!(matches!(s.aux_state(&c.dir), Some(AuxState::Loading(_))));
        let entries = listed(&mut s, &c.dir);
        let c = completion(&mut s, &text);
        assert_eq!(c.names, ["Src", "sub"], "folders only, case-insensitive, natural order");
        assert_eq!(completion_cached(&s, &text), c);
        // Same text and listing: the memo answers (even if the entries were to change in
        // place, which they never do: a reload is a new list).
        MEMO.with_borrow(|m| assert!(Arc::ptr_eq(m.as_ref().unwrap().entries.as_ref().unwrap(), &entries)));
        // A new listing of the same folder is a new key.
        listed(&mut s, &c.dir);
        assert_eq!(completion_cached(&s, &text).names, ["Src", "sub"]);
        assert_eq!(completion(&mut s, &typed(d, "su")).names, ["sub"]);
        assert_eq!(completion(&mut s, "no separator"), Completion::default());
    }

    #[test]
    fn editing_keys() {
        let mut a = AddressState::default();
        let loc = Location::Dir(std::env::temp_dir());
        start_edit(&mut a, &loc);
        assert!(a.editing && a.request_focus);
        assert_eq!(a.text, loc.display());
        // Up / Down wrap; nothing to move through: no highlight.
        cycle(&mut a, 0, true);
        assert_eq!(a.completion, None);
        cycle(&mut a, 3, false);
        assert_eq!(a.completion, Some(2));
        cycle(&mut a, 3, true);
        assert_eq!(a.completion, Some(0));
        cycle(&mut a, 3, true);
        assert_eq!(a.completion, Some(1));
        // Tab takes the highlighted name, else the first.
        let names = vec!["alpha".to_string(), "beta".to_string()];
        a.text = "/x/a".into();
        assert!(tab(&mut a, &names));
        assert_eq!(a.text, format!("/x/beta{}", std::path::MAIN_SEPARATOR));
        assert_eq!(a.completion, None);
        a.text = r"C:\x\".into();
        assert!(tab(&mut a, &names));
        assert_eq!(a.text, r"C:\x\alpha\", "backslash paths keep backslashes");
        assert!(!tab(&mut a, &[]));
        cancel(&mut a);
        assert!(!a.editing);
    }

    #[test]
    fn enter_navigates_or_reveals() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::create_dir(d.join("sub")).unwrap();
        std::fs::write(d.join("f.txt"), b"x").unwrap();
        let mut s = services();
        let pane = PaneId(3);
        let mut a = AddressState { editing: true, text: typed(d, "s"), ..Default::default() };
        listed(&mut s, &split_for_completion(&a.text).unwrap().0);
        let c = completion(&mut s, &a.text);
        assert_eq!(c.names, ["sub"]);
        // Enter with a highlighted completion accepts it first.
        a.completion = Some(0);
        let mut cmds = Vec::new();
        enter(&mut a, &mut s, pane, &mut cmds, &c);
        assert!(!a.editing);
        match cmds.as_slice() {
            [Cmd::FocusPane(p), Cmd::Navigate { pane: p2, location: Location::Dir(to), select: None }] => {
                assert_eq!((*p, *p2), (pane, pane));
                assert_eq!(to, &d.join("sub"));
            }
            other => panic!("{other:?}"),
        }
        // A typed file name reveals the file.
        let mut a = AddressState { editing: true, text: typed(d, "f.txt"), ..Default::default() };
        let c = completion(&mut s, &a.text);
        let mut cmds = Vec::new();
        enter(&mut a, &mut s, pane, &mut cmds, &c);
        assert!(matches!(cmds.as_slice(), [Cmd::RevealPath(p)] if p == &d.join("f.txt")), "{cmds:?}");
        // Virtual places parse too; empty text only closes the field.
        let mut a = AddressState { editing: true, text: "trash:".into(), ..Default::default() };
        let mut cmds = Vec::new();
        enter(&mut a, &mut s, pane, &mut cmds, &Completion::default());
        assert!(matches!(cmds.as_slice(), [_, Cmd::Navigate { location: Location::Trash, .. }]), "{cmds:?}");
        let mut a = AddressState { editing: true, text: "  ".into(), ..Default::default() };
        let mut cmds = Vec::new();
        enter(&mut a, &mut s, pane, &mut cmds, &Completion::default());
        assert!(cmds.is_empty() && !a.editing);
    }
}
