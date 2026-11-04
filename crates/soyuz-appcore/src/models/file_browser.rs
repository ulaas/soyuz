//! Browsing folders for a file to pick, without drawing: one folder at a time, listed on
//! a worker thread (a slow or dead network folder never blocks the frontend), narrowed by
//! a filter ([`BrowseFilter`]) and a typed query. The GUI's path picker
//! (`soyuz-app/src/ui/path_picker.rs`) and the terminal UI's Open with… browser draw it.
//!
//! With [`BrowseFilter::Programs`], Windows shortcuts are read on the worker too: a
//! shortcut to a program is listed under the shortcut's name and picks the program
//! itself, so the Start menu folders work as a list of applications.

use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, unbounded};
use soyuz_core::EntryInfo;
use soyuz_fs::ListOptions;

use crate::host::{Wake, no_wake};

/// Which files a browser lists (folders always are).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BrowseFilter {
    /// Every file.
    #[default]
    All,
    /// Files with one of these lower-case extensions.
    Exts(Vec<String>),
    /// Programs (see [`is_program_name`] / the executable bit).
    Programs,
}

/// One listed folder or file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowseEntry {
    /// What to show.
    pub name: String,
    /// What picking it gives: the file, or the program a shortcut points at.
    pub path: PathBuf,
    pub is_dir: bool,
    /// The file passes the filter (the rest show only with "all files").
    matched: bool,
}

type Listing = (PathBuf, Result<Vec<BrowseEntry>, String>);

/// The file-browsing state of a picker.
pub struct FileBrowser {
    filter: BrowseFilter,
    wake: Wake,
    dir: PathBuf,
    query: String,
    show_all: bool,
    entries: Vec<BrowseEntry>,
    /// Indices into `entries` of the rows shown.
    visible: Vec<usize>,
    error: Option<String>,
    loading: bool,
    rx: Option<Receiver<Listing>>,
}

/// Whether the file name has a Windows program's extension.
pub fn is_program_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["exe", "com", "bat", "cmd"].iter().any(|p| e.eq_ignore_ascii_case(p)))
}

/// Whether a file looks like something to run: by extension on Windows, by the executable
/// bit elsewhere (an AppImage always; a filesystem without modes shows everything).
fn is_program(e: &EntryInfo) -> bool {
    if cfg!(windows) {
        is_program_name(&e.name)
    } else {
        let image = Path::new(&e.name).extension().is_some_and(|x| x.eq_ignore_ascii_case("appimage"));
        image || e.mode.is_none_or(|m| m & 0o111 != 0)
    }
}

/// The program a Windows shortcut points at, when it points at one.
fn shortcut_program(e: &EntryInfo) -> Option<PathBuf> {
    if !cfg!(windows) || !Path::new(&e.name).extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
        return None;
    }
    let target = soyuz_fs::launch::shortcut_target(&e.path).ok().flatten()?;
    is_program_name(&target.to_string_lossy()).then_some(target)
}

fn to_entry(e: EntryInfo, filter: &BrowseFilter) -> BrowseEntry {
    let is_dir = e.is_dir_like();
    if !is_dir
        && *filter == BrowseFilter::Programs
        && let Some(target) = shortcut_program(&e)
    {
        let name = Path::new(&e.name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(e.name);
        return BrowseEntry { name, path: target, is_dir: false, matched: true };
    }
    let matched = is_dir
        || match filter {
            BrowseFilter::All => true,
            BrowseFilter::Exts(exts) => Path::new(&e.name)
                .extension()
                .is_some_and(|x| exts.iter().any(|w| x.to_string_lossy().eq_ignore_ascii_case(w))),
            BrowseFilter::Programs => is_program(&e),
        };
    BrowseEntry { name: e.name, path: e.path, is_dir, matched }
}

impl FileBrowser {
    /// A browser that lists nothing until [`FileBrowser::navigate`]. `wake` runs when a
    /// listing arrives.
    pub fn new(filter: BrowseFilter, wake: Wake) -> FileBrowser {
        FileBrowser {
            filter,
            wake,
            dir: PathBuf::new(),
            query: String::new(),
            show_all: false,
            entries: Vec::new(),
            visible: Vec::new(),
            error: None,
            loading: false,
            rx: None,
        }
    }

    /// [`FileBrowser::new`] for a frontend that polls.
    pub fn polled(filter: BrowseFilter) -> FileBrowser {
        FileBrowser::new(filter, no_wake())
    }

    /// Show `dir`: clears the rows and the query, and lists it on a worker thread.
    pub fn navigate(&mut self, dir: PathBuf) {
        self.dir = dir.clone();
        self.query.clear();
        self.entries.clear();
        self.visible.clear();
        self.error = None;
        self.loading = true;
        let (tx, rx) = unbounded();
        self.rx = Some(rx);
        let (filter, wake) = (self.filter.clone(), self.wake.clone());
        let spawned = std::thread::Builder::new().name("soyuz-browse-list".into()).spawn(move || {
            let opts = ListOptions {
                include_hidden: false,
                dot_hidden: crate::services::lister::dot_hidden(),
                ..ListOptions::default()
            };
            let listed = soyuz_fs::read_dir(&dir, &opts).map(|mut entries| {
                soyuz_core::sort_entries(&mut entries, &soyuz_core::SortSpec::default());
                entries.into_iter().map(|e| to_entry(e, &filter)).collect()
            });
            let _ = tx.send((dir, listed.map_err(|e| e.to_string())));
            wake();
        });
        if let Err(e) = spawned {
            self.loading = false;
            self.rx = None;
            self.error = Some(e.to_string());
        }
    }

    /// Take a finished listing; `true` when one arrived. Call every frame.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let Ok((dir, listed)) = rx.try_recv() else { return false };
        self.rx = None;
        if dir != self.dir {
            return false;
        }
        self.loading = false;
        match listed {
            Ok(entries) => self.entries = entries,
            Err(e) => self.error = Some(e),
        }
        self.refilter();
        true
    }

    fn refilter(&mut self) {
        let query = self.query.trim().to_lowercase();
        let keep = |e: &BrowseEntry| {
            (e.is_dir || e.matched || self.show_all) && (query.is_empty() || e.name.to_lowercase().contains(&query))
        };
        self.visible = self.entries.iter().enumerate().filter(|(_, e)| keep(e)).map(|(i, _)| i).collect();
    }

    /// The folder shown.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The folder above, if any.
    pub fn parent(&self) -> Option<PathBuf> {
        self.dir.parent().map(Path::to_path_buf)
    }

    /// The typed query.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Narrow the rows to names containing `query` (any case).
    pub fn set_query(&mut self, query: &str) {
        if self.query != query {
            self.query = query.to_string();
            self.refilter();
        }
    }

    /// Whether the filter is ignored.
    pub fn show_all(&self) -> bool {
        self.show_all
    }

    /// List the files the filter leaves out too.
    pub fn set_show_all(&mut self, on: bool) {
        if self.show_all != on {
            self.show_all = on;
            self.refilter();
        }
    }

    /// The listing is on its way.
    pub fn loading(&self) -> bool {
        self.loading
    }

    /// Why the folder could not be listed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// How many rows are shown.
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    /// No row is shown.
    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// Row `i`.
    pub fn row(&self, i: usize) -> Option<&BrowseEntry> {
        self.visible.get(i).and_then(|&e| self.entries.get(e))
    }

    /// The rows shown.
    pub fn rows(&self) -> impl Iterator<Item = &BrowseEntry> {
        self.visible.iter().filter_map(|&i| self.entries.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(b: &mut FileBrowser) {
        for _ in 0..500 {
            if b.poll() || !b.loading() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the listing never arrived");
    }

    fn names(b: &FileBrowser) -> Vec<&str> {
        b.rows().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn filters_by_extension_and_query() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("a.toml"), "").unwrap();
        std::fs::write(d.path().join("b.txt"), "").unwrap();
        let mut b = FileBrowser::polled(BrowseFilter::Exts(vec!["toml".into()]));
        b.navigate(d.path().to_path_buf());
        wait(&mut b);
        assert_eq!(names(&b), ["sub", "a.toml"]);
        b.set_show_all(true);
        assert_eq!(names(&b), ["sub", "a.toml", "b.txt"]);
        b.set_query("B.T");
        assert_eq!(names(&b), ["b.txt"]);
        assert_eq!(b.row(0).map(|e| e.path.clone()), Some(d.path().join("b.txt")));
        assert_eq!(b.row(1), None);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_hang() {
        let d = tempfile::tempdir().unwrap();
        let mut b = FileBrowser::polled(BrowseFilter::All);
        b.navigate(d.path().join("missing"));
        wait(&mut b);
        assert!(b.error().is_some() && b.is_empty() && !b.loading());
    }

    #[test]
    fn program_names() {
        assert!(is_program_name("App.EXE") && is_program_name("run.cmd"));
        assert!(!is_program_name("notes.txt") && !is_program_name("exe"));
    }

    #[test]
    fn lists_programs() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("tool.exe"), "").unwrap();
        std::fs::write(d.path().join("notes.txt"), "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let exec = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(d.path().join("tool.exe"), exec).unwrap();
        }
        let mut b = FileBrowser::polled(BrowseFilter::Programs);
        b.navigate(d.path().to_path_buf());
        wait(&mut b);
        assert_eq!(names(&b), ["tool.exe"]);
    }

    #[cfg(windows)]
    #[test]
    fn a_program_shortcut_picks_the_program() {
        let d = tempfile::tempdir().unwrap();
        let e = EntryInfo {
            name: "App.lnk".into(),
            path: d.path().join("App.lnk"),
            ..soyuz_fs::stat(d.path(), true).unwrap()
        };
        assert_eq!(shortcut_program(&e), None, "a missing shortcut");
    }
}
