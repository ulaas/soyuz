//! Inline rename in the views: F2 starts it (stem selected), F2 again cycles the
//! selection stem → extension → whole name, Enter / click-away commits, Tab commits and
//! moves to the next item (Shift+Tab: previous), Esc cancels. Validation errors are shown
//! under the editor and block the commit.
//!
//! This is the state and the commit logic; the GUI editor is `crate::ui::views::rename`.

use std::path::{Path, PathBuf};

use soyuz_core::EntryInfo;
use soyuz_core::config::RenameSelects;

use super::FileOpCmd;
use crate::cmd::Cmd;
use crate::state::pane::{Pane, PaneId};
use crate::state::selection::EntryKey;

/// Which part of the name is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameSel {
    Stem,
    Ext,
    All,
}

/// Character range (start, end) of `sel` in `name`. Folders and names without an
/// extension (incl. dot-files) have no separate stem / extension.
pub fn sel_range(name: &str, is_dir: bool, sel: RenameSel) -> (usize, usize) {
    let total = name.chars().count();
    let dot = if is_dir {
        None
    } else {
        name.char_indices()
            .filter(|(_, c)| *c == '.')
            .map(|(i, _)| i)
            .next_back()
            .filter(|&i| i > 0 && i + 1 < name.len())
    };
    let Some(dot_byte) = dot else { return (0, total) };
    let stem_chars = name[..dot_byte].chars().count();
    match sel {
        RenameSel::Stem => (0, stem_chars),
        RenameSel::Ext => (stem_chars + 1, total),
        RenameSel::All => (0, total),
    }
}

/// The next selection for F2, skipping states that select the same range.
pub fn next_sel(name: &str, is_dir: bool, cur: RenameSel) -> RenameSel {
    let order = [RenameSel::Stem, RenameSel::Ext, RenameSel::All];
    let start = order.iter().position(|s| *s == cur).unwrap_or(0);
    let here = sel_range(name, is_dir, cur);
    for k in 1..=3 {
        let cand = order[(start + k) % 3];
        if sel_range(name, is_dir, cand) != here {
            return cand;
        }
    }
    cur
}

/// The volume holding `dir` uses a Windows-family file system (FAT, exFAT, NTFS, SMB …;
/// [`soyuz_core::fs_needs_windows_names`]): names there must follow the Windows rules
/// even on Linux / macOS. Reads the cached volume list only (no I/O): the volume with
/// the longest mount path that contains `dir`.
pub fn volume_needs_windows_names(volumes: &[soyuz_fs::VolumeInfo], dir: &Path) -> bool {
    volumes
        .iter()
        .filter(|v| !v.mount.as_os_str().is_empty() && dir.starts_with(&v.mount))
        .max_by_key(|v| v.mount.components().count())
        .is_some_and(|v| soyuz_core::fs_needs_windows_names(&v.fs))
}

/// Should names created in `dir` follow the Windows rules? Always on Windows; elsewhere
/// with `[general] strict_windows_names`, or when `dir` is on a Windows-family volume.
pub fn windows_rules_for(strict: bool, volumes: &[soyuz_fs::VolumeInfo], dir: Option<&Path>) -> bool {
    cfg!(windows) || strict || dir.is_some_and(|d| volume_needs_windows_names(volumes, d))
}

/// Validate a new name for an item called `original` among `siblings`.
pub fn validate<'a>(
    new: &str,
    original: &str,
    siblings: impl IntoIterator<Item = &'a str>,
    windows_rules: bool,
    case_insensitive: bool,
) -> Result<(), String> {
    use soyuz_core::NameError as E;
    soyuz_core::validate_name_with(new, windows_rules).map_err(|e| match e {
        E::Empty => tr!("ops-rename-required").to_string(),
        E::Reserved(n) => tr!("ops-rename-reserved", name = n),
        E::InvalidChar(c) => tr!("ops-rename-invalid-char", char = c.to_string()),
        E::TrailingDotOrSpace => tr!("ops-rename-trailing-dot").to_string(),
        E::TooLong(n) => tr!("ops-rename-too-long", count = n),
    })?;
    let same = |a: &str, b: &str| if case_insensitive { a.to_lowercase() == b.to_lowercase() } else { a == b };
    if same(new, original) {
        return Ok(()); // unchanged or a case-only change of the item itself
    }
    if siblings.into_iter().any(|s| s != original && same(s, new)) {
        return Err(tr!("ops-rename-exists", name = new));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct InlineRename {
    pub pane: PaneId,
    /// Entry key (current name) of the item being renamed.
    pub key: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub text: String,
    pub sel: RenameSel,
    /// Apply `sel` to the editor on the next frame.
    pub apply_sel: bool,
    pub error: Option<String>,
}

impl InlineRename {
    pub fn new(pane: PaneId, e: &EntryInfo, selects: RenameSelects) -> Self {
        InlineRename {
            pane,
            key: e.name.clone(),
            path: e.path.clone(),
            is_dir: e.is_dir_like(),
            text: e.name.clone(),
            sel: match selects {
                RenameSelects::Stem => RenameSel::Stem,
                RenameSelects::All => RenameSel::All,
            },
            apply_sel: true,
            error: None,
        }
    }
}

/// How the edit ends this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Editing,
    /// Esc.
    Cancel,
    /// Enter or a click elsewhere.
    Commit,
    /// Tab (+1) / Shift+Tab (-1): commit and rename the next / previous item.
    CommitAndMove(i32),
}

/// Validate the edited name against the listing of `pane` (no I/O) and store the error;
/// returns the trimmed name.
pub fn revalidate(
    r: &mut InlineRename,
    pane: &Pane,
    strict_windows_names: bool,
    volumes: &[soyuz_fs::VolumeInfo],
) -> String {
    let dir = pane.location.as_fs_path();
    let windows_rules = windows_rules_for(strict_windows_names, volumes, dir);
    // FAT / exFAT / NTFS / SMB volumes compare names case-insensitively, also off Windows.
    let ci = super::names::case_insensitive_fs() || dir.is_some_and(|d| volume_needs_windows_names(volumes, d));
    let text = r.text.trim().to_string();
    r.error = validate(&text, &r.key, pane.entries.iter().map(|e| e.name.as_str()), windows_rules, ci).err();
    text
}

/// Apply `outcome` to the inline rename in `slot` (the item of `pane`); `text` is the
/// trimmed new name. `clicked_away`: the editor lost the focus without Enter / Tab.
/// Returns `true` when the editor must keep (take back) the focus: the name is invalid.
pub fn finish(
    slot: &mut Option<InlineRename>,
    pane: &mut Pane,
    outcome: Outcome,
    text: String,
    clicked_away: bool,
    selects: RenameSelects,
    cmds: &mut Vec<Cmd>,
) -> bool {
    let Some(r) = slot.as_ref() else { return false };
    match outcome {
        Outcome::Editing => false,
        Outcome::Cancel => {
            *slot = None;
            false
        }
        Outcome::Commit | Outcome::CommitAndMove(_) => {
            if r.error.is_some() {
                if clicked_away {
                    // Clicked away with an invalid name: drop the edit.
                    *slot = None;
                    return false;
                }
                // Keep editing until the name is valid.
                return true;
            }
            let (key, path) = (r.key.clone(), r.path.clone());
            let changed = text != key;
            if changed {
                cmds.push(Cmd::FileOp(FileOpCmd::Rename { pane: pane.id, from: path, to: text.clone() }));
            }
            *slot = None;
            if let Outcome::CommitAndMove(d) = outcome {
                let pos = pane.visible_pos(&key).map(|p| p as i64 + d as i64).filter(|p| *p >= 0).map(|p| p as usize);
                if let Some(pos) = pos
                    && let (Some(next), Some(k)) = (pane.entry_at(pos).cloned(), pane.key_at(pos).map(EntryKey::from))
                {
                    pane.selection.select_only(k.clone());
                    pane.cursor = Some(k);
                    pane.scroll_to_cursor = true;
                    *slot = Some(InlineRename::new(pane.id, &next, selects));
                }
            } else if changed {
                pane.pending_cursor = Some(text);
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_ranges() {
        assert_eq!(sel_range("report.final.pdf", false, RenameSel::Stem), (0, 12));
        assert_eq!(sel_range("report.final.pdf", false, RenameSel::Ext), (13, 16));
        assert_eq!(sel_range("report.final.pdf", false, RenameSel::All), (0, 16));
        assert_eq!(sel_range(".bashrc", false, RenameSel::Stem), (0, 7), "dot-files have no extension");
        assert_eq!(sel_range("my.dir", true, RenameSel::Stem), (0, 6), "folders select the whole name");
        assert_eq!(sel_range("trailing.", false, RenameSel::Stem), (0, 9));
        // Character (not byte) offsets.
        assert_eq!(sel_range("çağrı.txt", false, RenameSel::Stem), (0, 5));
        assert_eq!(sel_range("çağrı.txt", false, RenameSel::Ext), (6, 9));
    }

    #[test]
    fn f2_cycles() {
        assert_eq!(next_sel("a.txt", false, RenameSel::Stem), RenameSel::Ext);
        assert_eq!(next_sel("a.txt", false, RenameSel::Ext), RenameSel::All);
        assert_eq!(next_sel("a.txt", false, RenameSel::All), RenameSel::Stem);
        // No extension: stem == all, so the cycle stays on the whole name.
        assert_eq!(next_sel("folder", true, RenameSel::Stem), RenameSel::Stem);
        assert_eq!(next_sel("Makefile", false, RenameSel::All), RenameSel::All);
    }

    fn vol(mount: &str, fs: &str) -> soyuz_fs::VolumeInfo {
        soyuz_fs::VolumeInfo {
            mount: PathBuf::from(mount),
            label: String::new(),
            fs: fs.into(),
            kind: soyuz_fs::VolumeKind::Fixed,
            total: 0,
            free: 0,
            read_only: false,
            device: soyuz_core::DeviceId(0),
            supports: soyuz_fs::FsCaps::empty(),
        }
    }

    #[test]
    fn windows_names_follow_the_volume() {
        let vols = [vol("/", "btrfs"), vol("/run/media/u/STICK", "vfat"), vol("/mnt/win", "fuseblk")];
        assert!(!volume_needs_windows_names(&vols, Path::new("/home/u")));
        assert!(volume_needs_windows_names(&vols, Path::new("/run/media/u/STICK/photos")));
        assert!(volume_needs_windows_names(&vols, Path::new("/mnt/win")));
        assert!(!volume_needs_windows_names(&vols, Path::new("/run/media/u/STICK2")), "path components, not prefixes");
        assert!(!volume_needs_windows_names(&[], Path::new("/x")));
        assert!(windows_rules_for(true, &vols, Some(Path::new("/home"))), "strict setting");
        assert_eq!(windows_rules_for(false, &vols, Some(Path::new("/home"))), cfg!(windows));
        assert!(windows_rules_for(false, &vols, Some(Path::new("/mnt/win/x"))));
        assert_eq!(windows_rules_for(false, &vols, None), cfg!(windows));
    }

    #[test]
    fn validation() {
        let sib = ["a.txt", "B.txt", "c"];
        assert!(validate("new.txt", "a.txt", sib, true, true).is_ok());
        assert!(validate("A.TXT", "a.txt", sib, true, true).is_ok(), "case-only rename of itself");
        assert!(validate("b.TXT", "a.txt", sib, true, true).unwrap_err().contains("already exists"));
        assert!(validate("b.txt", "a.txt", sib, false, false).is_ok(), "case-sensitive fs");
        assert!(validate("", "a.txt", sib, true, true).unwrap_err().contains("required"));
        assert!(validate("x?y", "a.txt", sib, true, true).unwrap_err().contains('?'));
        assert!(validate("x?y", "a.txt", sib, false, false).is_ok(), "unix allows ?");
        assert!(validate("CON", "a.txt", sib, true, true).is_err());
        assert!(validate("end.", "a.txt", sib, true, true).is_err());
    }
}
