//! The "Create link" dialog for the selection (`FrontendCmd::MetaDialog(MetaDialog::Link)`:
//! New symbolic link / hard link / junction), without drawing: the form state
//! ([`LinkForm`]: kind, target, relative target, name), the background probe of the typed
//! target ([`probe_target`]: exists, file or folder, same volume), the folder listing of
//! the target picker ([`list_folder`]), the validation ([`LinkForm::error`]) and the
//! command that creates the link ([`LinkForm::create`]: a `Create` job, undoable).
//!
//! Both frontends draw it: the GUI (`ui::dialogs::links`, with a small folder browser
//! under the target field) and the terminal UI (`soyuz-tui`, `input::dialogs::link`).
//! Probes and listings run off the UI thread (`metadata` on a dead network share can
//! block for tens of seconds).

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use soyuz_core::{EntryInfo, Location};
use soyuz_fs::{LinkKind, ListOptions};
use soyuz_jobs::CreateKind;

use crate::cmd::Cmd;
use crate::core::AppCore;
use crate::models::properties::MetaCmd;

pub use crate::models::dialogs::LinkChoice;

/// Typing pause before the typed target is probed.
pub const PROBE_DEBOUNCE: Duration = Duration::from_millis(250);

/// `target` relative to `base` (`None` when they share no root, e.g. another drive).
pub fn relative_to(target: &Path, base: &Path) -> Option<PathBuf> {
    let t: Vec<Component<'_>> = target.components().collect();
    let b: Vec<Component<'_>> = base.components().collect();
    let eq = |x: &Component<'_>, y: &Component<'_>| {
        if cfg!(windows) {
            x.as_os_str().to_string_lossy().eq_ignore_ascii_case(&y.as_os_str().to_string_lossy())
        } else {
            x == y
        }
    };
    if t.is_empty() || b.is_empty() || !eq(&t[0], &b[0]) {
        return None;
    }
    let common = t.iter().zip(&b).take_while(|(x, y)| eq(x, y)).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    Some(out)
}

/// What the typed target refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetState {
    Empty,
    /// The background probe of the typed target has not finished yet.
    Checking,
    Missing,
    File,
    Dir,
}

/// The result of probing a target off the UI thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub state: TargetState,
    /// Target and link folder are on the same volume (for hard links).
    pub same_volume: bool,
    /// The folder the picker should switch to after a typed edit (the target or its parent).
    pub browse: Option<PathBuf>,
}

impl Probe {
    /// The probe of an empty target (nothing to check).
    pub fn empty() -> Probe {
        Probe { state: TargetState::Empty, same_volume: true, browse: None }
    }
}

/// Probe `target` for a link in `dir` (blocking: run it on a worker thread).
pub fn probe_target(target: &Path, dir: &Path) -> Probe {
    let state = match std::fs::metadata(target) {
        Ok(m) if m.is_dir() => TargetState::Dir,
        Ok(_) => TargetState::File,
        Err(_) => TargetState::Missing,
    };
    let same_volume = match state {
        TargetState::File => soyuz_fs::device_of(target).ok() == soyuz_fs::device_of(dir).ok(),
        _ => true,
    };
    let browse = if state == TargetState::Dir {
        Some(target.to_path_buf())
    } else {
        target.parent().filter(|pp| pp.is_dir()).map(Path::to_path_buf)
    };
    Probe { state, same_volume, browse }
}

/// The picker's listing of `dir`: hidden items left out, folders first, natural order
/// (blocking: run it on a worker thread).
pub fn list_folder(dir: &Path) -> Result<Vec<EntryInfo>, String> {
    let dot_hidden = crate::services::lister::dot_hidden();
    let opts = ListOptions { include_hidden: false, dot_hidden, ..ListOptions::default() };
    soyuz_fs::read_dir(dir, &opts).map_err(|e| e.to_string()).map(|mut v| {
        v.sort_by(|a, b| {
            b.is_dir_like().cmp(&a.is_dir_like()).then_with(|| soyuz_core::natural_cmp(&a.name, &b.name, true))
        });
        v
    })
}

/// The form of the "Create link" dialog.
#[derive(Clone, Debug)]
pub struct LinkForm {
    /// The folder the link is created in.
    pub dir: PathBuf,
    /// The typed target (relative paths are relative to [`LinkForm::dir`]).
    pub target: String,
    pub name: String,
    pub kind: LinkChoice,
    /// Names already in [`LinkForm::dir`].
    pub existing: Vec<String>,
    /// The user edited the name (it no longer follows the target).
    pub name_touched: bool,
    /// Store a relative path (symbolic links).
    pub relative: bool,
    /// Validate the name with the Windows rules (see `rename::windows_rules_for`).
    pub windows_rules: bool,
}

impl LinkForm {
    pub fn new(dir: PathBuf, target: Option<PathBuf>, kind: LinkChoice, existing: Vec<String>) -> LinkForm {
        let target_s = target.as_ref().map(|t| soyuz_fs::strip_verbatim(t).display().to_string()).unwrap_or_default();
        let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
        let name = target.as_ref().map(|t| crate::ops::fileops::names::link_name(t, &refs)).unwrap_or_default();
        LinkForm {
            dir,
            target: target_s,
            name,
            kind,
            existing,
            name_touched: false,
            relative: false,
            windows_rules: cfg!(windows),
        }
    }

    /// Validate names with the Windows rules (Windows-family volume or strict names).
    pub fn windows_rules(mut self, on: bool) -> LinkForm {
        self.windows_rules = on;
        self
    }

    /// The form for the focused pane: its folder, the first selected item as the target.
    /// Junctions are NTFS-only: elsewhere a junction request makes a symbolic link. `Err`
    /// is the message for an info toast (not a folder on disk).
    pub fn open(core: &AppCore, kind: LinkChoice) -> Result<LinkForm, String> {
        let kind = if kind == LinkChoice::Junction && !cfg!(windows) { LinkChoice::Symlink } else { kind };
        let pane = core.ws.pane();
        let Location::Dir(dir) = &pane.location else {
            return Err(tr!("dialogs-link-need-folder").into());
        };
        let target =
            if pane.selection.is_empty() { None } else { pane.selected_entries().first().map(|e| e.path.clone()) };
        let names: Vec<String> = pane.entries.iter().map(|e| e.name.clone()).collect();
        let windows_rules = crate::ops::fileops::rename::windows_rules_for(
            core.config.general.strict_windows_names,
            &core.ui.volumes,
            Some(dir),
        );
        Ok(LinkForm::new(dir.clone(), target, kind, names).windows_rules(windows_rules))
    }

    /// The folder the picker starts in: the target's folder, else the link's folder.
    pub fn start_folder(&self) -> PathBuf {
        if self.target.trim().is_empty() {
            return self.dir.clone();
        }
        self.target_path().parent().map(Path::to_path_buf).unwrap_or_else(|| self.dir.clone())
    }

    /// The target as a path (environment variables expanded, relative to the folder).
    pub fn target_path(&self) -> PathBuf {
        let t = PathBuf::from(soyuz_fs::expand_env(self.target.trim()));
        if t.is_relative() { self.dir.join(t) } else { t }
    }

    /// The target was typed: the name follows it until the name is edited.
    pub fn target_typed(&mut self) {
        if !self.name_touched {
            let refs: Vec<&str> = self.existing.iter().map(String::as_str).collect();
            self.name = crate::ops::fileops::names::link_name(&self.target_path(), &refs);
        }
    }

    /// A target was picked (the picker): the text shows it, the name follows it.
    pub fn set_target(&mut self, p: &Path) {
        self.target = soyuz_fs::strip_verbatim(p).display().to_string();
        if !self.name_touched {
            let refs: Vec<&str> = self.existing.iter().map(String::as_str).collect();
            self.name = crate::ops::fileops::names::link_name(p, &refs);
        }
    }

    /// The user edited the name.
    pub fn name_edited(&mut self) {
        self.name_touched = true;
    }

    /// "Relative path" applies (symbolic links only).
    pub fn relative_applies(&self) -> bool {
        self.kind == LinkChoice::Symlink
    }

    /// Why the link cannot be created with the target in `state` (`same_volume`: the
    /// probe's answer for hard links).
    pub fn error(&self, state: &TargetState, same_volume: bool) -> Option<String> {
        let name = self.name.trim();
        if self.target.trim().is_empty() {
            Some(tr!("dialogs-link-choose-target").into())
        } else if let Err(e) = soyuz_core::validate_name_with(name, self.windows_rules) {
            Some(tr!("dialogs-props-invalid-name", error = e.to_string()))
        } else if self.existing.iter().any(|e| e.eq_ignore_ascii_case(name)) {
            Some(tr!("dialogs-link-exists", name = name))
        } else {
            match (self.kind, state) {
                (_, TargetState::Checking) => Some(tr!("dialogs-link-checking").into()),
                (LinkChoice::Hardlink, TargetState::Dir) => Some(tr!("dialogs-link-hard-files-only").into()),
                (LinkChoice::Hardlink, TargetState::Missing) => Some(tr!("dialogs-link-hard-needs-file").into()),
                (LinkChoice::Hardlink, _) if !same_volume => Some(tr!("dialogs-link-hard-same-volume").into()),
                (LinkChoice::Junction, TargetState::File) => Some(tr!("dialogs-link-junction-folders-only").into()),
                (LinkChoice::Junction, TargetState::Missing) => Some(tr!("dialogs-link-junction-needs-folder").into()),
                _ => None,
            }
        }
    }

    /// The command that creates the link (a `Create` job), or `None` while
    /// [`LinkForm::error`] has something to say.
    pub fn create(&self, state: &TargetState, same_volume: bool) -> Option<Cmd> {
        if self.error(state, same_volume).is_some() {
            return None;
        }
        let name = self.name.trim().to_string();
        let path = self.dir.join(&name);
        let target = self.target_path();
        let kind = match self.kind {
            LinkChoice::Symlink => {
                let lk = if *state == TargetState::Dir { LinkKind::Dir } else { LinkKind::File };
                let stored =
                    if self.relative { relative_to(&target, &self.dir).unwrap_or(target.clone()) } else { target };
                CreateKind::Symlink { target: stored, kind: lk }
            }
            LinkChoice::Hardlink => CreateKind::Hardlink { src: target },
            LinkChoice::Junction => CreateKind::Junction { target },
        };
        Some(Cmd::Meta(MetaCmd::Create { dir: self.dir.clone(), path, kind }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        let (t, b) = if cfg!(windows) {
            (PathBuf::from(r"C:\data\photos\a.jpg"), PathBuf::from(r"C:\data\links"))
        } else {
            (PathBuf::from("/data/photos/a.jpg"), PathBuf::from("/data/links"))
        };
        assert_eq!(relative_to(&t, &b).unwrap(), Path::new("..").join("photos").join("a.jpg"));
        assert_eq!(relative_to(&b, &b).unwrap(), PathBuf::from("."));
        let child = b.join("x");
        assert_eq!(relative_to(&child, &b).unwrap(), PathBuf::from("x"));
        if cfg!(windows) {
            assert!(relative_to(Path::new(r"D:\x"), Path::new(r"C:\y")).is_none(), "different drives");
            assert!(relative_to(Path::new(r"c:\data\x"), Path::new(r"C:\data")).is_some(), "drive letter case");
        }
    }

    #[test]
    fn validation_follows_kind_and_target() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let mut f = LinkForm::new(dir.path().to_path_buf(), None, LinkChoice::Hardlink, vec!["a.txt".into()]);
        assert_eq!(f.error(&TargetState::Empty, true).as_deref(), Some(tr!("dialogs-link-choose-target")));
        f.target = file.display().to_string();
        f.target_typed();
        assert!(!f.name.is_empty() && f.name != "a.txt", "the name follows the target: {}", f.name);
        let probe = probe_target(&f.target_path(), &f.dir);
        assert_eq!(probe.state, TargetState::File);
        assert_eq!(probe.browse.as_deref(), Some(dir.path()));
        assert!(f.error(&probe.state, probe.same_volume).is_none());
        assert_eq!(f.error(&TargetState::Dir, true).as_deref(), Some(tr!("dialogs-link-hard-files-only")));
        assert_eq!(f.error(&TargetState::File, false).as_deref(), Some(tr!("dialogs-link-hard-same-volume")));
        match f.create(&probe.state, true) {
            Some(Cmd::Meta(MetaCmd::Create { kind: CreateKind::Hardlink { src }, .. })) => assert_eq!(src, file),
            other => panic!("{other:?}"),
        }
        // A symbolic link with a relative target.
        f.kind = LinkChoice::Symlink;
        f.relative = true;
        match f.create(&TargetState::File, true) {
            Some(Cmd::Meta(MetaCmd::Create { kind: CreateKind::Symlink { target, kind: LinkKind::File }, .. })) => {
                assert_eq!(target, PathBuf::from("a.txt"))
            }
            other => panic!("{other:?}"),
        }
        f.name = "A.TXT".into();
        assert!(f.error(&TargetState::File, true).is_some(), "taken (any case)");
        let l = list_folder(dir.path()).unwrap();
        assert_eq!(l.len(), 1);
    }
}
