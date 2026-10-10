//! Color tags / labels: eight colors matching the `tag.*` design tokens,
//! stored per path in `tags.toml` next to the config (portable-aware), plus a
//! best-effort read of macOS Finder tags (`com.apple.metadata:_kMDItemUserTags`, a
//! binary plist) shown alongside.
//!
//! The store is UI-thread-local; it loads on a worker at startup and is written back
//! (debounced, atomically) on a worker after changes. `Location::Tags(slug)` lists every
//! path carrying a tag through the normal listing path ([`attach`]).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, unbounded};
use serde::{Deserialize, Serialize};
use soyuz_core::{Column, ColumnSpec, EntryInfo};

use super::mailbox::{Postman, UiMsg};
use crate::state::pane::{ListingStatus, Pane};

/// Number of tag colors (`tag.red` … `tag.gray`).
pub const TAG_COUNT: usize = 8;
/// English names (slugs, parsing), index-aligned with `Palette::tags`; see
/// [`display_name`] for the translated label.
pub const TAG_NAMES: [&str; TAG_COUNT] = ["Red", "Orange", "Yellow", "Green", "Cyan", "Blue", "Purple", "Gray"];

/// `"red"` for tag 0, used in `Location::Tags` and `tags.toml`.
pub fn slug(tag: u8) -> String {
    TAG_NAMES.get(tag as usize).map(|n| n.to_ascii_lowercase()).unwrap_or_default()
}

/// Tag index from a name (`red`, `Red`, `grey`, Finder names, `1`..`8`).
pub fn from_name(s: &str) -> Option<u8> {
    let s = s.trim().to_ascii_lowercase();
    if let Ok(n) = s.parse::<u8>() {
        return (1..=TAG_COUNT as u8).contains(&n).then(|| n - 1);
    }
    let s = if s == "grey" { "gray".to_string() } else { s };
    TAG_NAMES.iter().position(|n| n.eq_ignore_ascii_case(&s)).map(|i| i as u8)
}

/// Finder color label number (1 gray … 7 orange) → our tag index.
pub fn from_finder_color(n: u8) -> Option<u8> {
    match n {
        1 => Some(7),
        2 => Some(3),
        3 => Some(6),
        4 => Some(5),
        5 => Some(2),
        6 => Some(0),
        7 => Some(1),
        _ => None,
    }
}

/// Tag set of one path as a bit mask (bit `i` = tag `i`).
pub type TagMask = u8;

pub fn mask_tags(mask: TagMask) -> impl Iterator<Item = u8> {
    (0..TAG_COUNT as u8).filter(move |i| mask & (1 << i) != 0)
}

#[derive(Default, Serialize, Deserialize)]
struct TagFile {
    #[serde(default)]
    version: u32,
    /// path → tag slugs.
    #[serde(default)]
    items: BTreeMap<String, Vec<String>>,
}

/// Path → tags, with TOML (de)serialization.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagStore {
    map: HashMap<PathBuf, TagMask>,
}

fn norm(p: &Path) -> PathBuf {
    soyuz_fs::strip_verbatim(p)
}

impl TagStore {
    pub fn get(&self, path: &Path) -> TagMask {
        self.map.get(&norm(path)).copied().unwrap_or(0)
    }

    pub fn set(&mut self, path: &Path, mask: TagMask) {
        if mask == 0 {
            self.map.remove(&norm(path));
        } else {
            self.map.insert(norm(path), mask);
        }
    }

    pub fn toggle(&mut self, path: &Path, tag: u8) {
        let m = self.get(path) ^ (1 << tag);
        self.set(path, m);
    }

    pub fn add(&mut self, path: &Path, tag: u8) {
        let m = self.get(path) | (1 << tag);
        self.set(path, m);
    }

    pub fn clear(&mut self, path: &Path) {
        self.map.remove(&norm(path));
    }

    /// Paths carrying `tag`, sorted.
    pub fn paths_with(&self, tag: u8) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> =
            self.map.iter().filter(|(_, m)| **m & (1 << tag) != 0).map(|(p, _)| p.clone()).collect();
        v.sort();
        v
    }

    pub fn counts(&self) -> [usize; TAG_COUNT] {
        let mut c = [0; TAG_COUNT];
        for m in self.map.values() {
            for t in mask_tags(*m) {
                c[t as usize] += 1;
            }
        }
        c
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Re-key a renamed / moved path (and everything below it).
    pub fn rename(&mut self, from: &Path, to: &Path) {
        let (from, to) = (norm(from), norm(to));
        let moved: Vec<(PathBuf, TagMask)> = self
            .map
            .iter()
            .filter(|(p, _)| p.starts_with(&from))
            .map(|(p, m)| (to.join(p.strip_prefix(&from).unwrap_or(Path::new(""))), *m))
            .collect();
        if moved.is_empty() {
            return;
        }
        self.map.retain(|p, _| !p.starts_with(&from));
        for (p, m) in moved {
            let p = if p.as_os_str().is_empty() { to.clone() } else { p };
            self.map.insert(p, m);
        }
    }

    pub fn to_toml(&self) -> String {
        let mut f = TagFile { version: 1, items: BTreeMap::new() };
        for (p, m) in &self.map {
            f.items.insert(p.to_string_lossy().into_owned(), mask_tags(*m).map(slug).collect());
        }
        toml::to_string_pretty(&f).unwrap_or_default()
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        let f: TagFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut s = TagStore::default();
        for (p, tags) in f.items {
            let mask = tags.iter().filter_map(|t| from_name(t)).fold(0u8, |m, t| m | (1 << t));
            if mask != 0 {
                s.map.insert(PathBuf::from(p), mask);
            }
        }
        Ok(s)
    }
}

// ---------------------------------------------------------------------------------------
// macOS Finder tags: a minimal binary plist reader for an array of strings.

/// Most Finder tags we read from one file (real files carry a handful).
const MAX_FINDER_TAGS: usize = 64;

/// Parse a `bplist00` whose top object is an array of strings (Finder's
/// `_kMDItemUserTags`: `"Red\n6"`, `"Work"`). Returns `(name, finder color)`.
///
/// The xattr is attacker-controlled (any downloaded file can carry one), so every count
/// and offset is bounds-checked with checked arithmetic, object references must be
/// below the trailer's object count, and the array is capped at [`MAX_FINDER_TAGS`]
/// before anything is allocated.
pub fn parse_finder_tags(data: &[u8]) -> Option<Vec<(String, Option<u8>)>> {
    if data.len() < 40 || &data[..8] != b"bplist00" {
        return None;
    }
    let trailer = &data[data.len() - 32..];
    let off_size = trailer[6] as usize;
    let ref_size = trailer[7] as usize;
    let be = |b: &[u8]| b.iter().fold(0u64, |a, x| (a << 8) | *x as u64);
    // Values wider than usize (or than the data) are rejected rather than truncated.
    let num_objects = usize::try_from(be(&trailer[8..16])).ok()?;
    let top = usize::try_from(be(&trailer[16..24])).ok()?;
    let table = usize::try_from(be(&trailer[24..32])).ok()?;
    if off_size == 0 || ref_size == 0 || off_size > 8 || ref_size > 8 || top >= num_objects {
        return None;
    }
    // The offset table must fit in the data before the trailer.
    let table_end = num_objects.checked_mul(off_size).and_then(|n| n.checked_add(table))?;
    if table < 8 || table_end > data.len() - 32 {
        return None;
    }
    let offset_of = |i: usize| -> Option<usize> {
        if i >= num_objects {
            return None;
        }
        let s = table.checked_add(i.checked_mul(off_size)?)?;
        let o = usize::try_from(be(data.get(s..s.checked_add(off_size)?)?)).ok()?;
        (o < data.len()).then_some(o)
    };
    // Length that follows a marker: low nibble, or an int object when it is 0xF.
    let length = |pos: usize| -> Option<(usize, usize)> {
        let m = *data.get(pos)?;
        let n = (m & 0x0F) as usize;
        if n != 0x0F {
            return Some((n, pos.checked_add(1)?));
        }
        let im = *data.get(pos.checked_add(1)?)?;
        // Int marker 0x1N with 2^N bytes; we accept up to 8 bytes.
        if im >> 4 != 0x1 || im & 0x0F > 3 {
            return None;
        }
        let bytes = 1usize << (im & 0x0F);
        let at = pos.checked_add(2)?;
        let end = at.checked_add(bytes)?;
        let v = usize::try_from(be(data.get(at..end)?)).ok()?;
        Some((v, end))
    };
    let string_at = |obj: usize| -> Option<String> {
        let pos = offset_of(obj)?;
        let m = *data.get(pos)?;
        let (n, start) = length(pos)?;
        match m >> 4 {
            0x5 => data.get(start..start.checked_add(n)?).map(|b| String::from_utf8_lossy(b).into_owned()),
            0x6 => {
                let b = data.get(start..start.checked_add(n.checked_mul(2)?)?)?;
                let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                Some(String::from_utf16_lossy(&units))
            }
            _ => None,
        }
    };
    let pos = offset_of(top)?;
    if *data.get(pos)? >> 4 != 0xA {
        return None;
    }
    let (count, start) = length(pos)?;
    if count > MAX_FINDER_TAGS || count > num_objects {
        return None;
    }
    // All references must be inside the data before any allocation.
    let refs_end = count.checked_mul(ref_size).and_then(|n| n.checked_add(start))?;
    if refs_end > data.len() {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let at = start + k * ref_size;
        let r = data.get(at..at + ref_size)?;
        let s = string_at(usize::try_from(be(r)).ok()?)?;
        let (name, color) = match s.split_once('\n') {
            Some((n, c)) => (n.to_string(), c.trim().parse::<u8>().ok()),
            None => (s, None),
        };
        out.push((name, color));
    }
    Some(out)
}

/// Finder tags → our mask (by color number, else by name).
pub fn finder_mask(tags: &[(String, Option<u8>)]) -> TagMask {
    tags.iter()
        .filter_map(|(name, color)| color.and_then(from_finder_color).or_else(|| from_name(name)))
        .fold(0, |m, t| m | (1 << t))
}

#[cfg(target_os = "macos")]
fn read_finder(path: &Path) -> TagMask {
    match xattr::get(path, "com.apple.metadata:_kMDItemUserTags") {
        Ok(Some(data)) => parse_finder_tags(&data).map(|t| finder_mask(&t)).unwrap_or(0),
        _ => 0,
    }
}

#[cfg(not(target_os = "macos"))]
fn read_finder(_path: &Path) -> TagMask {
    0
}

// ---------------------------------------------------------------------------------------
// UI-thread registry.

enum Reply {
    Loaded(TagStore),
    Finder { dir: PathBuf, tags: Vec<(PathBuf, TagMask)> },
}

struct Registry {
    store: TagStore,
    loaded: bool,
    started: bool,
    dirty: bool,
    last_change: Instant,
    rx: Option<Receiver<Reply>>,
    tx: Option<crossbeam_channel::Sender<Reply>>,
    path: PathBuf,
    /// Finder tags per path (read-only, macOS).
    finder: HashMap<PathBuf, TagMask>,
    finder_dirs: HashSet<PathBuf>,
    /// Bumped on every change (sidebar counts cache).
    version: u64,
}

impl Default for Registry {
    fn default() -> Self {
        Registry {
            store: TagStore::default(),
            loaded: false,
            started: false,
            dirty: false,
            last_change: Instant::now(),
            rx: None,
            tx: None,
            path: soyuz_core::config::config_dir().join("tags.toml"),
            finder: HashMap::new(),
            finder_dirs: HashSet::new(),
            version: 0,
        }
    }
}

thread_local! {
    static REG: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Start loading `tags.toml` (idempotent) and drain worker replies; writes pending
/// changes back after a short debounce. Returns `true` in the frame the store arrives.
pub fn pump(wake: &std::sync::Arc<dyn Fn() + Send + Sync>) -> bool {
    REG.with_borrow_mut(|r| {
        let mut arrived = false;
        if !r.started {
            r.started = true;
            let (tx, rx) = unbounded();
            r.rx = Some(rx);
            r.tx = Some(tx.clone());
            let path = r.path.clone();
            let wake = wake.clone();
            let _ = std::thread::Builder::new().name("soyuz-tags-load".into()).spawn(move || {
                // A broken file is backed up and never overwritten (store_file).
                let store = super::store_file::load(&path, TagStore::from_toml).value();
                let _ = tx.send(Reply::Loaded(store));
                wake();
            });
        }
        let replies: Vec<Reply> = r.rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for reply in replies {
            match reply {
                Reply::Loaded(mut s) => {
                    // Merge edits made before the load finished.
                    for (p, m) in std::mem::take(&mut r.store.map) {
                        s.set(&p, m);
                    }
                    r.store = s;
                    r.loaded = true;
                    arrived = true;
                    r.version += 1;
                }
                Reply::Finder { dir, tags } => {
                    r.finder.retain(|p, _| p.parent() != Some(dir.as_path()));
                    for (p, m) in tags {
                        r.finder.insert(p, m);
                    }
                    r.version += 1;
                }
            }
        }
        if r.dirty && r.loaded && r.last_change.elapsed() > Duration::from_millis(400) {
            queue_save(r);
        }
        arrived
    })
}

/// Hand the current store to the single ordered save worker.
fn queue_save(r: &mut Registry) {
    r.dirty = false;
    super::store_file::save_async(r.path.clone(), r.store.to_toml().into_bytes());
}

/// Queue pending changes now, skipping the debounce (`App::save` then calls
/// `store_file::flush`). Edits made before the file finished loading stay in memory.
pub fn flush() {
    REG.with_borrow_mut(|r| {
        if r.dirty && r.loaded {
            queue_save(r);
        }
    });
}

/// Display name of a tag slug (`red` → `Red`, "Kırmızı" in Turkish).
pub fn display_name(slug: &str) -> String {
    let Some(t) = from_name(slug) else { return slug.to_string() };
    match t {
        0 => tr!("services-tag-red"),
        1 => tr!("services-tag-orange"),
        2 => tr!("services-tag-yellow"),
        3 => tr!("services-tag-green"),
        4 => tr!("services-tag-cyan"),
        5 => tr!("services-tag-blue"),
        6 => tr!("services-tag-purple"),
        _ => tr!("services-tag-gray"),
    }
    .to_string()
}

/// Whether there are unsaved changes (the UI keeps repainting until they are written).
pub fn pending_save() -> bool {
    REG.with_borrow(|r| r.dirty)
}

/// Tags of a path: user tags plus Finder tags. Cheap; call per row.
pub fn tags_of(path: &Path) -> TagMask {
    REG.with_borrow(|r| {
        let own = r.store.get(path);
        if r.finder.is_empty() { own } else { own | r.finder.get(path).copied().unwrap_or(0) }
    })
}

/// Read Finder tags for the entries of `dir` once (macOS; no-op elsewhere).
pub fn request_finder(dir: &Path, wake: &std::sync::Arc<dyn Fn() + Send + Sync>) {
    if !cfg!(target_os = "macos") {
        return;
    }
    REG.with_borrow_mut(|r| {
        if !r.finder_dirs.insert(dir.to_path_buf()) {
            return;
        }
        let Some(tx) = r.tx.clone() else { return };
        let dir = dir.to_path_buf();
        let wake = wake.clone();
        let _ = std::thread::Builder::new().name("soyuz-finder-tags".into()).spawn(move || {
            let mut tags = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    let m = read_finder(&p);
                    if m != 0 {
                        tags.push((p, m));
                    }
                }
            }
            let _ = tx.send(Reply::Finder { dir, tags });
            wake();
        });
    });
}

fn changed(r: &mut Registry) {
    r.dirty = true;
    r.last_change = Instant::now();
    r.version += 1;
}

/// Toggle `tag` on every path: when all already carry it, remove it; else add it.
pub fn toggle_many(paths: &[PathBuf], tag: u8) -> bool {
    REG.with_borrow_mut(|r| {
        let all = paths.iter().all(|p| r.store.get(p) & (1 << tag) != 0);
        for p in paths {
            if all {
                let m = r.store.get(p) & !(1 << tag);
                r.store.set(p, m);
            } else {
                r.store.add(p, tag);
            }
        }
        changed(r);
        !all
    })
}

pub fn clear_many(paths: &[PathBuf]) {
    REG.with_borrow_mut(|r| {
        for p in paths {
            r.store.clear(p);
        }
        changed(r);
    });
}

/// Keep tags attached to a renamed / moved item.
pub fn rename(from: &Path, to: &Path) {
    REG.with_borrow_mut(|r| {
        if r.store.get(from) != 0 || r.store.map.keys().any(|p| p.starts_with(norm(from))) {
            r.store.rename(from, to);
            changed(r);
        }
    });
}

pub fn counts() -> [usize; TAG_COUNT] {
    REG.with_borrow(|r| r.store.counts())
}

pub fn version() -> u64 {
    REG.with_borrow(|r| r.version)
}

/// Every tagged path with its tag slugs (feeds the index's `tag:` filter).
pub fn snapshot() -> Vec<(PathBuf, Vec<String>)> {
    REG.with_borrow(|r| {
        r.store
            .map
            .iter()
            .filter(|(_, m)| **m != 0)
            .map(|(p, m)| (p.clone(), mask_tags(*m).map(slug).collect()))
            .collect()
    })
}

/// Columns for a tag listing.
pub fn listing_columns() -> Vec<ColumnSpec> {
    let mut path = ColumnSpec::new(Column::Path);
    path.width = 260.0;
    let mut tags = ColumnSpec::new(Column::Tags);
    tags.width = 110.0;
    vec![ColumnSpec::new(Column::Name), path, tags, ColumnSpec::new(Column::Modified), ColumnSpec::new(Column::Size)]
}

/// Start listing `Location::Tags(slug)` for this pane generation.
pub fn attach(pane: &mut Pane, tag_slug: &str, postman: Postman) {
    let Some(tag) = from_name(tag_slug) else {
        pane.status = ListingStatus::Failed(soyuz_core::FsError::with_message(
            soyuz_core::FsErrorKind::NotFound,
            "tag",
            None,
            tr!("services-tag-unknown", name = tag_slug),
        ));
        return;
    };
    if !pane.view.columns.iter().any(|c| c.column == Column::Path && c.visible) {
        pane.view.columns = listing_columns();
    }
    pane.view.group = soyuz_core::GroupBy::None;
    pane.status = ListingStatus::Loading { since: Instant::now() };
    let mut paths = REG.with_borrow(|r| r.store.paths_with(tag));
    REG.with_borrow(|r| {
        for (p, m) in &r.finder {
            if m & (1 << tag) != 0 && !paths.contains(p) {
                paths.push(p.clone());
            }
        }
    });
    stat_listing(pane, paths, postman, |_, _| {});
}

/// Stat `paths` on a worker and stream them into the pane as one listing generation.
/// Names are made unique (`name (parent)`) because selection is keyed by name.
pub fn stat_listing(
    pane: &Pane,
    paths: Vec<PathBuf>,
    postman: Postman,
    adjust: impl Fn(&Path, &mut EntryInfo) + Send + 'static,
) {
    let (id, generation) = (pane.id, pane.generation);
    let spawned = std::thread::Builder::new().name("soyuz-virtual-list".into()).spawn(move || {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = Vec::with_capacity(paths.len());
        for p in paths {
            let Ok(mut e) = soyuz_fs::stat(&p, false) else { continue };
            if !seen.insert(e.name.to_lowercase()) {
                let parent = p.parent().map(soyuz_core::entry::display_name).unwrap_or_default();
                e.name = format!("{} ({parent})", e.name);
                if !seen.insert(e.name.to_lowercase()) {
                    e.name = soyuz_fs::strip_verbatim(&p).to_string_lossy().into_owned();
                    seen.insert(e.name.to_lowercase());
                }
            }
            adjust(&p, &mut e);
            out.push(e);
        }
        postman.send(UiMsg::Listing { pane: id, generation, chunk: out, done: true, err: None });
    });
    if spawned.is_err() {
        log::warn!("cannot spawn listing thread");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trip() {
        let mut s = TagStore::default();
        s.add(Path::new("/a/one.txt"), 0);
        s.add(Path::new("/a/one.txt"), 5);
        s.toggle(Path::new("/b/two"), 7);
        s.toggle(Path::new("/c/gone"), 2);
        s.toggle(Path::new("/c/gone"), 2);
        assert_eq!(s.len(), 2, "toggling twice removes the entry");
        let text = s.to_toml();
        assert!(text.contains("red") && text.contains("blue") && text.contains("gray"), "{text}");
        let back = TagStore::from_toml(&text).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.get(Path::new("/a/one.txt")), 0b0010_0001);
        assert_eq!(back.paths_with(7), vec![PathBuf::from("/b/two")]);
        assert_eq!(back.counts(), [1, 0, 0, 0, 0, 1, 0, 1]);
        assert!(TagStore::from_toml("items = 3").is_err());
        assert!(TagStore::from_toml("").unwrap().is_empty());
    }

    #[test]
    fn rename_keeps_tags_below() {
        let mut s = TagStore::default();
        s.add(Path::new("/p/dir"), 1);
        s.add(Path::new("/p/dir/x.txt"), 3);
        s.add(Path::new("/p/other"), 4);
        s.rename(Path::new("/p/dir"), Path::new("/q/moved"));
        assert_eq!(s.get(Path::new("/q/moved")), 1 << 1);
        assert_eq!(s.get(Path::new("/q/moved/x.txt")), 1 << 3);
        assert_eq!(s.get(Path::new("/p/dir/x.txt")), 0);
        assert_eq!(s.get(Path::new("/p/other")), 1 << 4);
    }

    #[test]
    fn names_and_slugs() {
        for i in 0..TAG_COUNT as u8 {
            assert_eq!(from_name(&slug(i)), Some(i));
        }
        assert_eq!(from_name("Grey"), Some(7));
        assert_eq!(from_name("3"), Some(2));
        assert_eq!(from_name("9"), None);
        assert_eq!(from_name("teal"), None);
    }

    /// Build a bplist00 with an array of ASCII strings (test encoder).
    fn bplist(strings: &[&str]) -> Vec<u8> {
        let mut out = b"bplist00".to_vec();
        let mut offsets = Vec::new();
        // Object 0: the array; objects 1..: strings.
        offsets.push(out.len());
        assert!(strings.len() < 15);
        out.push(0xA0 | strings.len() as u8);
        for i in 0..strings.len() {
            out.push((i + 1) as u8);
        }
        for s in strings {
            offsets.push(out.len());
            if s.len() < 15 {
                out.push(0x50 | s.len() as u8);
            } else {
                out.push(0x5F);
                out.push(0x10);
                out.push(s.len() as u8);
            }
            out.extend_from_slice(s.as_bytes());
        }
        let table = out.len();
        for o in &offsets {
            out.push(*o as u8);
        }
        let mut trailer = vec![0u8; 6];
        trailer.push(1); // offset size
        trailer.push(1); // ref size
        trailer.extend_from_slice(&(offsets.len() as u64).to_be_bytes());
        trailer.extend_from_slice(&0u64.to_be_bytes());
        trailer.extend_from_slice(&(table as u64).to_be_bytes());
        out.extend_from_slice(&trailer);
        out
    }

    #[test]
    fn finder_tags_from_bplist() {
        let data = bplist(&["Red\n6", "Work", "Project Blue Long Name\n4", "Green"]);
        let tags = parse_finder_tags(&data).unwrap();
        assert_eq!(tags[0], ("Red".to_string(), Some(6)));
        assert_eq!(tags[1], ("Work".to_string(), None));
        assert_eq!(tags[2], ("Project Blue Long Name".to_string(), Some(4)));
        let m = finder_mask(&tags);
        assert_eq!(m, (1 << 0) | (1 << 5) | (1 << 3));
        assert!(parse_finder_tags(b"not a plist at all, definitely not one").is_none());
    }

    /// Patch the 32-byte trailer of a plist built by [`bplist`].
    fn with_trailer(mut data: Vec<u8>, num_objects: u64, top: u64, table: u64) -> Vec<u8> {
        let t = data.len() - 32;
        data[t + 8..t + 16].copy_from_slice(&num_objects.to_be_bytes());
        data[t + 16..t + 24].copy_from_slice(&top.to_be_bytes());
        data[t + 24..t + 32].copy_from_slice(&table.to_be_bytes());
        data
    }

    #[test]
    fn hostile_bplists_are_rejected_without_panic() {
        let good = bplist(&["Red\n6", "Work"]);
        let table = (good.len() - 32 - 3) as u64;
        // Offsets / counts near usize::MAX must not overflow or allocate.
        assert!(parse_finder_tags(&with_trailer(good.clone(), u64::MAX, 0, table)).is_none());
        assert!(parse_finder_tags(&with_trailer(good.clone(), 3, 0, u64::MAX)).is_none());
        assert!(parse_finder_tags(&with_trailer(good.clone(), 3, 0, u64::MAX - 1)).is_none());
        assert!(parse_finder_tags(&with_trailer(good.clone(), 3, 2, 0)).is_none());
        // Array count announced as 2^62 via an extended int length.
        let mut huge = b"bplist00".to_vec();
        huge.extend_from_slice(&[0xAF, 0x13]);
        huge.extend_from_slice(&(1u64 << 62).to_be_bytes());
        huge.push(8); // offset table: object 0 at offset 8
        let table = (huge.len() - 1) as u64;
        let mut trailer = vec![0u8; 6];
        trailer.extend_from_slice(&[1, 1]);
        trailer.extend_from_slice(&1u64.to_be_bytes());
        trailer.extend_from_slice(&0u64.to_be_bytes());
        trailer.extend_from_slice(&table.to_be_bytes());
        huge.extend_from_slice(&trailer);
        assert!(parse_finder_tags(&huge).is_none());
        // A string whose length runs past the end.
        let mut long = bplist(&["Red"]);
        let s = long.iter().position(|b| *b == 0x53).unwrap();
        long[s] = 0x5F;
        let _ = parse_finder_tags(&long);
        // Every truncation of a valid plist is handled.
        for n in 0..good.len() {
            let _ = parse_finder_tags(&good[..n]);
        }
        assert_eq!(parse_finder_tags(&good).unwrap().len(), 2);
    }
}
