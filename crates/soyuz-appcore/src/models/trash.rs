//! The Trash location's state (frontend-agnostic): items of every bin listed by
//! `soyuz_fs::trash::list` on a worker thread, the selection, loading / error state.
//! The GUI's `ui::views::trash` and the terminal UI's Trash page draw it; restore, empty
//! and purge run as jobs. Both draw the same header line ([`TrashState::summary`]),
//! origin column ([`origin`]) and row menu ([`menu`]).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_core::{Location, SizeStyle, format_size};
use soyuz_fs::TrashItem;

use crate::cmd::Cmd;
use crate::host::Wake;
use crate::icon::IconId;
use crate::models::context_menu::{Entry, Item, ItemStyle, MenuIcon, Run};
use crate::models::toasts::Toast;
use crate::ops::fileops::FileOpCmd;
use crate::state::pane::PaneId;

/// What lists the bins: `soyuz_fs::trash::list` unless a test replaced it ([`set_lister`]).
pub type Lister = Arc<dyn Fn() -> Result<Vec<TrashItem>, String> + Send + Sync>;

static LISTER: RwLock<Option<Lister>> = RwLock::new(None);

/// Test seam: list the trash with `f` instead of the real bins (`None`: the real ones).
/// Headless tests point it at a fake bin in a temporary folder, so they never read or
/// change the user's trash.
#[doc(hidden)]
pub fn set_lister(f: Option<Lister>) {
    *LISTER.write().unwrap_or_else(|e| e.into_inner()) = f;
}

fn list_bins() -> Result<Vec<TrashItem>, String> {
    let custom = LISTER.read().unwrap_or_else(|e| e.into_inner()).clone();
    match custom {
        Some(f) => f(),
        None => soyuz_fs::trash::list().map_err(|e| e.to_string()),
    }
}

enum TrashMsg {
    Listed(Result<Vec<TrashItem>, String>),
}

pub struct TrashState {
    pub items: Vec<TrashItem>,
    /// Selected items by [`key`].
    pub selected: BTreeSet<String>,
    anchor: Option<String>,
    pub loading: bool,
    pub loaded: bool,
    stale: bool,
    pub error: Option<String>,
    tx: Sender<TrashMsg>,
    rx: Receiver<TrashMsg>,
    wake: Wake,
    /// A message for the toast stack (set by `poll`, taken by the view).
    notice: Option<Toast>,
    last_load: Option<Instant>,
}

/// Selection key of a trashed item (unique across bins).
pub fn key(i: &TrashItem) -> String {
    format!("{}|{}", i.trash_root.display(), i.id)
}

impl TrashState {
    pub fn new(wake: Wake) -> Self {
        let (tx, rx) = unbounded();
        TrashState {
            items: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            loading: false,
            loaded: false,
            stale: true,
            error: None,
            tx,
            rx,
            wake,
            notice: None,
            last_load: None,
        }
    }

    pub fn items(&self) -> &[TrashItem] {
        &self.items
    }

    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// Load when shown for the first time or after a trash-related job.
    pub fn ensure_loaded(&mut self) {
        if (self.stale || !self.loaded) && !self.loading {
            self.reload();
        }
    }

    pub fn reload(&mut self) {
        self.loading = true;
        self.stale = false;
        self.last_load = Some(Instant::now());
        let tx = self.tx.clone();
        let wake = self.wake.clone();
        let spawned = std::thread::Builder::new().name("soyuz-trash-list".into()).spawn(move || {
            let r = list_bins();
            let _ = tx.send(TrashMsg::Listed(r));
            wake();
        });
        if spawned.is_err() {
            self.loading = false;
            self.error = Some(tr!("views-trash-reader-failed").into());
        }
    }

    pub fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                TrashMsg::Listed(Ok(items)) => {
                    self.items = items;
                    let keys: BTreeSet<String> = self.items.iter().map(key).collect();
                    self.selected.retain(|k| keys.contains(k));
                    self.loading = false;
                    self.loaded = true;
                    self.error = None;
                }
                TrashMsg::Listed(Err(e)) => {
                    self.loading = false;
                    self.loaded = true;
                    self.error = Some(e);
                }
            }
        }
    }

    pub fn take_notice(&mut self) -> Option<Toast> {
        self.notice.take()
    }

    pub fn selected_items(&self) -> Vec<TrashItem> {
        self.items.iter().filter(|i| self.selected.contains(&key(i))).cloned().collect()
    }

    /// Select every item (the anchor stays).
    pub fn select_all(&mut self) {
        self.selected = self.items.iter().map(key).collect();
    }

    /// Whether item `idx` is selected.
    pub fn is_selected(&self, idx: usize) -> bool {
        self.items.get(idx).is_some_and(|i| self.selected.contains(&key(i)))
    }

    /// The header's second line: reading, empty, or the item count and total size (and
    /// how many are selected).
    pub fn summary(&self, style: SizeStyle) -> String {
        if self.loading && !self.loaded {
            tr!("views-trash-reading").to_string()
        } else if self.items.is_empty() {
            tr!("views-trash-empty-short").to_string()
        } else {
            let n = self.items.len();
            let total: u64 = self.items.iter().map(|i| i.size).sum();
            let size = format_size(total, style);
            if self.selected.is_empty() {
                tr!("views-trash-summary", count = n, size = size)
            } else {
                tr!("views-trash-summary-selected", count = n, size = size, selected = self.selected.len())
            }
        }
    }

    /// A click on row `idx`: `shift` extends from the anchor, `command` toggles.
    pub fn click(&mut self, idx: usize, shift: bool, command: bool) {
        let k = key(&self.items[idx]);
        if shift
            && let Some(a) = self.anchor.clone()
            && let Some(ai) = self.items.iter().position(|i| key(i) == a)
        {
            let (lo, hi) = if ai <= idx { (ai, idx) } else { (idx, ai) };
            if !command {
                self.selected.clear();
            }
            for i in &self.items[lo..=hi] {
                self.selected.insert(key(i));
            }
            return;
        }
        if command {
            if !self.selected.remove(&k) {
                self.selected.insert(k.clone());
            }
        } else {
            self.selected.clear();
            self.selected.insert(k.clone());
        }
        self.anchor = Some(k);
    }
}

/// The path a trashed item is shown by: where it came from, else its bin id.
pub fn name_path(it: &TrashItem) -> PathBuf {
    if it.original.as_os_str().is_empty() { PathBuf::from(&it.id) } else { it.original.clone() }
}

/// The "Original location" column: the folder it came from, or "Unknown".
pub fn origin(it: &TrashItem) -> String {
    name_path(it)
        .parent()
        .map(|d| soyuz_fs::strip_verbatim(d).to_string_lossy().into_owned())
        .filter(|s| !s.is_empty() && !it.original.as_os_str().is_empty())
        .unwrap_or_else(|| tr!("views-trash-unknown-origin").into())
}

/// The menu of a trashed item (right click; `selected`: the items it acts on, `original`:
/// the clicked item's original path): restore, open the original folder (when known),
/// copy the original path, delete permanently.
pub fn menu(selected: Vec<TrashItem>, original: PathBuf, pane: PaneId) -> Vec<Entry> {
    let row = |icon: IconId, label: &str, c: Cmd| {
        Entry::Item(Item {
            icon: MenuIcon::Id(icon),
            label: label.to_string(),
            chord: None,
            enabled: true,
            style: ItemStyle::Plain,
            run: Some(Run::Cmd(c)),
            keep_open: false,
            tip: None,
        })
    };
    let mut out = vec![row(
        IconId::ArrowCounterClockwise,
        tr!("views-trash-restore"),
        Cmd::FileOp(FileOpCmd::TrashRestore(selected.clone())),
    )];
    if let Some(dir) = original.parent().filter(|d| !d.as_os_str().is_empty()) {
        out.push(row(
            IconId::FolderOpen,
            tr!("views-trash-open-original"),
            Cmd::Navigate { pane, location: Location::Dir(dir.to_path_buf()), select: None },
        ));
    }
    out.push(row(
        IconId::ClipboardText,
        tr!("views-trash-copy-original"),
        Cmd::CopyText(soyuz_fs::strip_verbatim(&original).to_string_lossy().into_owned()),
    ));
    out.push(Entry::Separator);
    out.push(row(
        IconId::XCircle,
        tr!("views-trash-delete-permanently"),
        Cmd::FileOp(FileOpCmd::ConfirmPurge(selected)),
    ));
    out
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use soyuz_core::EntryKind;

    use super::*;

    fn item(id: &str, original: &str, size: u64) -> TrashItem {
        TrashItem {
            id: id.into(),
            original: original.into(),
            deleted_at: SystemTime::UNIX_EPOCH,
            size,
            kind: EntryKind::File,
            trash_root: "/bin".into(),
        }
    }

    fn state(items: Vec<TrashItem>) -> TrashState {
        let mut s = TrashState::new(crate::host::no_wake());
        s.items = items;
        s.loaded = true;
        s
    }

    #[test]
    fn selection_summary_and_columns() {
        let mut s = state(vec![item("a", "/x/a.txt", 10), item("b", "", 5), item("c", "/y/c", 1)]);
        assert_eq!(
            s.summary(SizeStyle::Decimal),
            tr!("views-trash-summary", count = 3, size = format_size(16, SizeStyle::Decimal))
        );
        s.click(0, false, false);
        s.click(2, true, false);
        assert!(s.is_selected(0) && s.is_selected(1) && s.is_selected(2));
        s.click(1, false, true);
        assert!(!s.is_selected(1) && !s.is_selected(7));
        assert_eq!(s.selected_items().len(), 2);
        s.selected.clear();
        s.select_all();
        assert_eq!(s.selected.len(), 3);
        assert_ne!(
            s.summary(SizeStyle::Decimal),
            tr!("views-trash-summary", count = 3, size = format_size(16, SizeStyle::Decimal))
        );
        assert_eq!(name_path(&s.items[1]), PathBuf::from("b"));
        assert_eq!(origin(&s.items[1]), tr!("views-trash-unknown-origin"));
        assert_eq!(origin(&s.items[0]), soyuz_fs::strip_verbatim(std::path::Path::new("/x")).display().to_string());
        assert_eq!(state(Vec::new()).summary(SizeStyle::Decimal), tr!("views-trash-empty-short"));
    }

    #[test]
    fn row_menu_runs_the_trash_commands() {
        let sel = vec![item("a", "/x/a.txt", 10)];
        let runs = |m: &[Entry]| -> Vec<Run> {
            m.iter()
                .filter_map(|e| match e {
                    Entry::Item(i) => i.run.clone(),
                    _ => None,
                })
                .collect()
        };
        let r = runs(&menu(sel.clone(), "/x/a.txt".into(), PaneId(3)));
        assert_eq!(r.len(), 4);
        assert!(matches!(&r[0], Run::Cmd(Cmd::FileOp(FileOpCmd::TrashRestore(v))) if v.len() == 1));
        assert!(matches!(&r[1], Run::Cmd(Cmd::Navigate { pane: PaneId(3), location: Location::Dir(d), .. })
            if d == std::path::Path::new("/x")));
        assert!(matches!(&r[2], Run::Cmd(Cmd::CopyText(_))));
        assert!(matches!(&r[3], Run::Cmd(Cmd::FileOp(FileOpCmd::ConfirmPurge(v))) if v.len() == 1));
        // An unknown origin has no "Open original location".
        assert_eq!(runs(&menu(sel, PathBuf::new(), PaneId(3))).len(), 3);
    }

    #[test]
    fn the_lister_seam_replaces_the_bins() {
        set_lister(Some(Arc::new(|| Ok(vec![item("z", "/q/z", 1)]))));
        assert_eq!(list_bins().map(|v| v.len()), Ok(1));
        set_lister(None);
    }
}
