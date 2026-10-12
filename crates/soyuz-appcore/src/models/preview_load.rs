//! What a preview consumer (the preview pane, Quick Look; in either frontend) keeps
//! besides its drawing: which entry it shows, the in-flight request (one per consumer,
//! stale requests dropped), the result, the hex document, the recursive folder size,
//! and the two-file diff. Frontends only draw [`PreviewLoader::loaded`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use soyuz_core::config::Config;
use soyuz_core::{EntryInfo, EntryKind, Location, OpControl};

use super::preview_hex::HexDoc;
use crate::core::AppCore;
use crate::host::Wake;
use crate::services::preview::{Content, Kind, Limits, Loaded, Msg, PreviewClient, Source, kind_by_name};
use crate::services::preview_cache::{self, CacheKey};
use crate::services::preview_diff::{DiffClient, DiffModel};
use crate::services::preview_gfx::{Rgba, TextFont, Uploader};

/// Something the user asked for from inside a preview.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewAction {
    ToggleHex,
    /// Open this path (archive / folder) in the pane.
    Open(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
struct TargetKey {
    path: PathBuf,
    modified: Option<SystemTime>,
    size: u64,
    force_hex: bool,
}

/// Recursive folder size being computed for a folder preview.
struct SizeJob {
    path: PathBuf,
    rx: crossbeam_channel::Receiver<soyuz_search::SizeEvent>,
    ctl: Arc<OpControl>,
    bytes: u64,
    files: u64,
    dirs: u64,
    done: bool,
}

impl Drop for SizeJob {
    fn drop(&mut self) {
        self.ctl.cancel();
    }
}

/// One preview consumer's loading state.
pub struct PreviewLoader {
    client: PreviewClient,
    wake: Wake,
    key: Option<TargetKey>,
    pub entry: Option<EntryInfo>,
    pub loaded: Option<Loaded>,
    /// When the current request was sent (`None`: nothing in flight).
    pub loading_since: Option<Instant>,
    pub hex: Option<HexDoc>,
    sizes: Option<SizeJob>,
    pub force_hex: bool,
    /// Quick Look: bigger images and rich renders.
    pub big: bool,
    /// Cache key of the in-flight request (the result is remembered when it lands).
    cache_key: Option<CacheKey>,
    /// Display scale for page-like renders (the frontend keeps it current).
    pub pixels_per_point: f32,
    /// The frontend draws only uploaded pixels (the terminal UI; `RichLimits::terminal`).
    pub terminal: bool,
}

impl std::fmt::Debug for PreviewLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewLoader")
            .field("entry", &self.entry.as_ref().map(|e| &e.path))
            .field("loaded", &self.loaded.as_ref().map(|l| l.kind))
            .field("loading", &self.loading_since.is_some())
            .field("force_hex", &self.force_hex)
            .finish_non_exhaustive()
    }
}

impl PreviewLoader {
    /// `wake` asks the frontend for a frame when a result lands, `upload` makes its
    /// textures; `big` for Quick Look.
    pub fn new(wake: Wake, upload: Uploader, big: bool, pixels_per_point: f32) -> Self {
        PreviewLoader {
            client: PreviewClient::new(wake.clone(), upload),
            wake,
            key: None,
            entry: None,
            loaded: None,
            loading_since: None,
            hex: None,
            sizes: None,
            force_hex: false,
            big,
            cache_key: None,
            pixels_per_point,
            terminal: false,
        }
    }

    /// The worker client (hex pages are requested through it).
    pub fn client(&self) -> &PreviewClient {
        &self.client
    }

    /// The hex document with the client its pages load through.
    pub fn hex_doc(&mut self) -> Option<(&mut HexDoc, &PreviewClient)> {
        self.hex.as_mut().map(|h| (h, &self.client))
    }

    /// Request limits from the config; `dark` / `fg`: the theme (syntax colours, text).
    pub fn limits(&self, cfg: &Config, dark: bool, fg: Rgba) -> Limits {
        Limits {
            max_image_px: if self.big { 4096 } else { 1600 },
            max_image_mp: cfg.preview.max_image_mp.max(1),
            max_text_bytes: (cfg.preview.max_text_kib.max(4) * 1024) as usize,
            dark,
            mono: TextFont::monospace(12.0),
            fg,
            pixels_per_point: self.pixels_per_point,
            rich: crate::services::preview_rich::RichLimits {
                terminal: self.terminal,
                ..crate::services::preview_rich::RichLimits::from_config(&cfg.preview, self.big)
            },
        }
    }

    /// Show `entry` (or nothing). Issues a new request only when the target changed.
    /// Returns whether the shown path changed (frontends reset zoom / scroll then).
    pub fn set_target(&mut self, entry: Option<&EntryInfo>, limits: Limits) -> bool {
        let key = entry.map(|e| TargetKey {
            path: e.path.clone(),
            modified: e.modified,
            size: e.size,
            force_hex: self.force_hex,
        });
        if key == self.key {
            return false;
        }
        let same_path = matches!((&key, &self.key), (Some(a), Some(b)) if a.path == b.path);
        self.key = key;
        self.entry = entry.cloned();
        self.hex = None;
        self.sizes = None;
        match entry {
            Some(e) => {
                let ck = (!self.force_hex && !e.is_dir_like()).then(|| CacheKey {
                    path: e.path.clone(),
                    modified: e.modified,
                    size: e.size,
                    variant: (limits.max_image_px, limits.dark, limits.max_text_bytes, limits.rich.terminal),
                });
                if let Some(hit) = ck.as_ref().and_then(preview_cache::get) {
                    self.client.cancel();
                    self.cache_key = None;
                    self.loaded = Some(hit);
                    self.loading_since = None;
                    if let Some(Content::Hex(init)) = self.loaded.as_ref().map(|l| &l.content) {
                        self.hex = Some(HexDoc::new(init));
                    }
                    return !same_path;
                }
                self.cache_key = ck;
                let source = Source::from_path(&e.path);
                self.client.request(source, e.is_dir_like(), self.force_hex, limits);
                self.loading_since = Some(Instant::now());
            }
            None => {
                self.client.cancel();
                self.loaded = None;
                self.loading_since = None;
            }
        }
        !same_path
    }

    /// Forget the current target so the next `set_target` reloads.
    pub fn invalidate(&mut self) {
        self.key = None;
    }

    /// Flip the hex view (reloads on the next `set_target`).
    pub fn toggle_hex(&mut self) {
        self.force_hex = !self.force_hex;
        self.invalidate();
    }

    pub fn clear(&mut self) {
        self.client.cancel();
        self.key = None;
        self.entry = None;
        self.loaded = None;
        self.hex = None;
        self.sizes = None;
        self.loading_since = None;
    }

    pub fn is_loading(&self) -> bool {
        self.loading_since.is_some()
    }

    /// Drain worker results (call once per frame). Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Some(msg) = self.client.try_recv() {
            match msg {
                Msg::Loaded { id, loaded } => {
                    if id != self.client.latest() {
                        continue;
                    }
                    changed = true;
                    self.loading_since = None;
                    if let Content::Hex(init) = &loaded.content {
                        self.hex = Some(HexDoc::new(init));
                    }
                    if let (Content::Folder(_), Some(e)) = (&loaded.content, &self.entry)
                        && !Source::from_path(&e.path).is_entry()
                    {
                        let (rx, ctl) = soyuz_search::folder_sizes(vec![e.path.clone()], self.wake.clone());
                        self.sizes =
                            Some(SizeJob { path: e.path.clone(), rx, ctl, bytes: 0, files: 0, dirs: 0, done: false });
                    }
                    if let Some(k) = self.cache_key.take() {
                        preview_cache::put(k, &loaded);
                    }
                    self.loaded = Some(*loaded);
                }
                Msg::Page { doc, page, bytes } => {
                    if let Some(h) = &mut self.hex {
                        h.on_page(doc, page, bytes);
                        changed = true;
                    }
                }
            }
        }
        if let Some(s) = &mut self.sizes {
            for ev in s.rx.try_iter() {
                match ev {
                    soyuz_search::SizeEvent::Partial { path, bytes, files, dirs } if path == s.path => {
                        (s.bytes, s.files, s.dirs) = (bytes, files, dirs);
                        changed = true;
                    }
                    soyuz_search::SizeEvent::Done { path, bytes, files, dirs, .. } if path == s.path => {
                        (s.bytes, s.files, s.dirs, s.done) = (bytes, files, dirs, true);
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        changed
    }

    /// `(bytes, files, dirs, done)` of the recursive folder size, when computing.
    pub fn folder_size(&self) -> Option<(u64, u64, u64, bool)> {
        self.sizes.as_ref().map(|s| (s.bytes, s.files, s.dirs, s.done))
    }
}

/// What the preview should show for the focused pane: the cursor entry, else the first
/// selected entry, else the folder itself.
pub fn target_entry(core: &AppCore) -> Option<EntryInfo> {
    let pane = core.ws.pane();
    if matches!(pane.location, Location::Remote(_)) {
        // Remote files preview from a downloaded copy; folders show nothing.
        let e =
            pane.cursor.as_deref().and_then(|c| pane.entry(c)).or_else(|| pane.selected_entries().first().copied())?;
        return super::remote::preview_target(core, e);
    }
    if let Some(e) = pane.cursor.as_deref().and_then(|c| pane.entry(c)) {
        return Some(e.clone());
    }
    if let Some(e) = pane.selected_entries().first() {
        return Some((*e).clone());
    }
    match &pane.location {
        Location::Dir(p) => Some(EntryInfo::minimal(p.clone(), EntryKind::Dir)),
        Location::Archive { archive, inner } if !inner.is_empty() => {
            Some(EntryInfo::minimal(soyuz_core::join_vpath(archive, inner), EntryKind::Dir))
        }
        Location::Archive { archive, .. } => Some(EntryInfo::minimal(archive.clone(), EntryKind::File)),
        _ => None,
    }
}

/// The two selected files of the focused pane, when the diff preview applies
/// (`[preview] diff`, exactly two text files): the older one first.
pub fn diff_pair(core: &AppCore) -> Option<(EntryInfo, EntryInfo)> {
    if !core.config.preview.diff {
        return None;
    }
    let pane = core.ws.pane();
    if pane.selection.len() != 2 {
        return None;
    }
    let sel = pane.selected_entries();
    let [a, b] = sel.as_slice() else { return None };
    // Older file on the left (the "before"), when the times tell.
    let (a, b) = match (a.modified, b.modified) {
        (Some(ta), Some(tb)) if tb < ta => (b, a),
        _ => (a, b),
    };
    let texty = |e: &EntryInfo| {
        !e.is_dir_like() && matches!(kind_by_name(&e.name), None | Some(Kind::Text | Kind::Markdown | Kind::Svg))
    };
    (texty(a) && texty(b)).then(|| ((*a).clone(), (*b).clone()))
}

type DiffKey = (PathBuf, Option<SystemTime>, u64, PathBuf, Option<SystemTime>, u64);

/// One consumer's two-file diff: the request, its result and the view choices.
#[derive(Default)]
pub struct DiffState {
    client: DiffClient,
    key: Option<DiffKey>,
    pub result: Option<Result<Arc<DiffModel>, String>>,
    /// `None` = automatic (side by side when wide).
    pub side: Option<bool>,
    /// Compare the other way round (newer → older).
    pub swapped: bool,
}

impl std::fmt::Debug for DiffState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiffState")
            .field("pending", &(self.key.is_some() && self.result.is_none()))
            .field("side", &self.side)
            .field("swapped", &self.swapped)
            .finish_non_exhaustive()
    }
}

impl DiffState {
    /// Whether a pair is being compared or shown.
    pub fn is_active(&self) -> bool {
        self.key.is_some()
    }

    pub fn clear(&mut self) {
        self.client.cancel();
        self.key = None;
        self.result = None;
    }

    /// Diff `a` → `b` (at most `cap` bytes each): requests when the pair changed, then
    /// drains results.
    pub fn sync(&mut self, a: &EntryInfo, b: &EntryInfo, cap: u64, wake: &Wake) {
        let key = (a.path.clone(), a.modified, a.size, b.path.clone(), b.modified, b.size);
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.result = None;
            self.client.request(a.path.clone(), b.path.clone(), cap, wake);
        }
        while let Some(r) = self.client.try_recv() {
            if r.id == self.client.latest() {
                self.result = Some(r.result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_loads_text_and_hex() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.rs");
        std::fs::write(&p, "fn main() {}\n").unwrap();
        let mut e = EntryInfo::minimal(p.clone(), EntryKind::File);
        e.size = 13;
        let cfg = Config::default();
        let mut l = PreviewLoader::new(crate::host::no_wake(), crate::services::preview_gfx::no_upload(), false, 1.0);
        let lim = l.limits(&cfg, true, [255; 4]);
        assert!(l.set_target(Some(&e), lim.clone()));
        assert!(!l.set_target(Some(&e), lim.clone()), "same target: no new request");
        let t0 = Instant::now();
        while l.loaded.is_none() && t0.elapsed().as_secs() < 30 {
            l.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            matches!(l.loaded.as_ref().map(|x| &x.content), Some(Content::Text(_))),
            "{:?}",
            l.loaded.as_ref().map(|x| x.kind)
        );
        l.toggle_hex();
        // Forgetting the target counts as a new one.
        l.set_target(Some(&e), l.limits(&cfg, true, [255; 4]));
        let t0 = Instant::now();
        while l.hex.is_none() && t0.elapsed().as_secs() < 30 {
            l.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(l.hex.is_some());
        l.clear();
        assert!(l.entry.is_none() && l.loaded.is_none() && !l.is_loading());
    }
}
