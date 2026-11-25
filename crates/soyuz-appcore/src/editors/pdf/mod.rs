//! The PDF viewer's document model (`DocKind::Pdf`): a [`PdfSession`] (worker threads
//! that render and extract text) plus everything a frontend needs to show it, without any
//! toolkit types:
//!
//! * the viewport ([`PdfDoc::set_viewport`]: size in view points and pixels per point)
//!   and scroll offset, the page [`layout::Layout`], zoom modes (`[pdf] default_zoom`)
//!   with anchor-preserving zoom, view rotation, the sidebar (`[pdf] sidebar`) and dark
//!   pages (`[pdf] dark_pages`, toggled per tab);
//! * [`PdfDoc::pump`] (every frame, after `set_viewport`): takes worker results, works
//!   out the wanted page bitmaps / sharp tiles / page texts, requests the missing ones and
//!   keeps the textures in a [`TexCache`] under `[pdf] cache_mib`;
//!   [`PdfDoc::visible_pages`] says what to paint (base layer, tiles on top, highlights);
//! * find (a background sweep with progressive counts, wrapping next / previous that
//!   scrolls the match into view), text selection across pages, go to page, links
//!   (internal ones scroll, web links come back as a [`LinkAction`] for the frontend to
//!   confirm / open per `[pdf] external_links`; nothing is opened here);
//! * a password-locked state (the frontend asks through
//!   `models::archive::ask_pdf_password`; a typed password reopens the document) and
//!   reloads that keep the page.
//!
//! The viewer never changes the file: [`PdfDoc::is_dirty`] is always `false`.

pub mod layout;
pub mod select;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use soyuz_core::config::{Action, KeyName, PdfDarkPages, PdfExternalLinks, PdfSidebar, PdfZoom};

use layout::{ACTUAL, Layout, Rect};
use select::{Selection, TextPos, Unit};

use super::{EditInput, KeyMods};
use crate::host::Wake;
use crate::services::pdf::nav::{Dest, LinkTarget, is_web_uri};
use crate::services::pdf::tiles::visible_tiles;
use crate::services::pdf::{
    DocInfo, Event, Job, Match, Outline, PageGeom, PageKey, PageText, PdfError, PdfSession, Query, RenderOpts,
    SessionCfg, TexCache, TileKey, prio,
};
use crate::services::preview_gfx::{Tex, Uploader};
use crate::services::preview_pdf;

/// Zoom presets in percent (the zoom menu; `+` / `-` step through them).
pub const ZOOM_STEPS: [f32; 20] = [
    10.0, 25.0, 33.0, 50.0, 67.0, 75.0, 90.0, 100.0, 110.0, 125.0, 150.0, 175.0, 200.0, 250.0, 300.0, 400.0, 500.0,
    600.0, 800.0, 1000.0,
];
/// Smallest zoom in percent.
pub const MIN_PERCENT: f32 = 5.0;
/// Largest zoom in percent.
pub const MAX_PERCENT: f32 = 1000.0;
/// Arrow-key scroll step (view points).
const LINE: f32 = 40.0;
/// Pages of text kept by the model (farthest from the current page dropped first).
const MAX_TEXTS: usize = 256;
/// Hit-test slack around a char box (page points).
const HIT_SLACK: f32 = 2.0;

/// The `[pdf]` settings as the model uses them.
#[derive(Clone, Debug)]
pub struct ViewerCfg {
    pub default_zoom: PdfZoom,
    pub sidebar: PdfSidebar,
    pub page_gap: f32,
    pub dark_pages: PdfDarkPages,
    pub external_links: PdfExternalLinks,
    /// Texture budget in bytes.
    pub cache_bytes: u64,
    pub session: SessionCfg,
}

impl ViewerCfg {
    /// From `[pdf]` and `[preview] max_document_mib` (the file size cap).
    pub fn from_config(pdf: &soyuz_core::config::Pdf, max_document_mib: u64) -> ViewerCfg {
        ViewerCfg {
            default_zoom: pdf.default_zoom,
            sidebar: pdf.sidebar,
            page_gap: pdf.page_gap_clamped(),
            dark_pages: pdf.dark_pages,
            external_links: pdf.external_links,
            cache_bytes: pdf.cache_mib_clamped() << 20,
            session: SessionCfg {
                timeout: Duration::from_secs(pdf.render_timeout_secs_clamped()),
                max_bytes: max_document_mib.max(1).saturating_mul(1 << 20),
                opts: RenderOpts { annotations: pdf.annotations, invert: false, system_fonts: pdf.system_fonts },
                ..SessionCfg::default()
            },
        }
    }
}

/// How the zoom is chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// The widest page fills the view width (follows the view size).
    FitWidth,
    /// The largest page fits wholly (follows the view size).
    FitPage,
    /// A fixed scale: view points per page point ([`layout::ACTUAL`] = 100 %).
    Scale(f32),
}

impl Zoom {
    pub fn from_config(z: PdfZoom) -> Zoom {
        match z {
            PdfZoom::FitWidth => Zoom::FitWidth,
            PdfZoom::FitPage => Zoom::FitPage,
            PdfZoom::Actual => Zoom::Scale(ACTUAL),
        }
    }

    /// A fixed zoom of `percent` %.
    pub fn percent(percent: f32) -> Zoom {
        Zoom::Scale(percent.clamp(MIN_PERCENT, MAX_PERCENT) / 100.0 * ACTUAL)
    }
}

/// Where the reader is (kept per file for the session, `[pdf] remember_position`, and
/// across reloads).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub page: u32,
    /// The view top as a fraction of the page height from the page top (negative in the
    /// gap above it).
    pub offset: f32,
    pub zoom: Zoom,
    pub rot: u8,
}

/// The document's state.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Opening,
    Ready,
    /// Opening failed; [`PdfError::Password`] = locked ([`PdfDoc::locked`]).
    Failed(PdfError),
}

/// What is under the pointer (for the cursor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hover {
    Nothing,
    /// Selectable text (I-beam).
    Text,
    /// A link (pointing hand).
    Link,
}

/// The result of clicking a link.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkAction {
    /// An internal link: the view moved.
    Jumped,
    /// A web link to open at once (`[pdf] external_links = "open"`).
    Open(String),
    /// A web link to confirm first (`"ask"`).
    Ask(String),
    /// A link that is never opened (not http / https / mailto).
    Blocked(String),
}

/// One page to paint, in viewport coordinates (view points from the viewport's
/// top-left).
#[derive(Clone, Debug)]
pub struct PageView {
    pub page: u32,
    pub rect: Rect,
    /// The whole page at base resolution (blurry when zoomed in; paint it first).
    pub base: Option<Tex>,
    /// Sharp tiles over the base, with their rectangles.
    pub tiles: Vec<(Tex, Rect)>,
    /// The page could not be rendered (show a card with Retry: [`PdfDoc::retry_page`]).
    pub failed: Option<PdfError>,
    /// Selected text, one rectangle per line.
    pub selection: Vec<Rect>,
    /// Find matches other than the current one.
    pub matches: Vec<Rect>,
    /// The current find match.
    pub current: Vec<Rect>,
}

/// The find bar.
#[derive(Default)]
pub struct PdfFind {
    pub open: bool,
    /// Edited by the frontend; call [`PdfDoc::find_changed`] after a change.
    pub query: Query,
    /// The frontend should focus (and select) the find field.
    pub focus_find: bool,
    id: Option<u64>,
    matches: BTreeMap<u32, Vec<Match>>,
    /// Pages searched so far of `total`.
    pub searched: u32,
    pub total: u32,
    /// The current match: page and index among that page's matches.
    pub current: Option<(u32, usize)>,
    /// Jump to the first match the sweep finds.
    jump: bool,
    /// The query changed before the document opened.
    deferred: bool,
}

impl PdfFind {
    /// Matches found so far.
    pub fn count(&self) -> usize {
        self.matches.values().map(Vec::len).sum()
    }

    /// 1-based number of the current match among all found so far.
    pub fn ordinal(&self) -> Option<usize> {
        let (page, i) = self.current?;
        Some(self.matches.range(..page).map(|(_, m)| m.len()).sum::<usize>() + i + 1)
    }

    /// The sweep has searched every page.
    pub fn done(&self) -> bool {
        self.id.is_some() && self.searched >= self.total
    }

    /// A search is running or finished.
    pub fn active(&self) -> bool {
        self.id.is_some()
    }

    fn flat(&self) -> Vec<(u32, usize)> {
        self.matches.iter().flat_map(|(p, m)| (0..m.len()).map(move |i| (*p, i))).collect()
    }
}

/// A texture held by the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TexKey {
    Tile(TileKey),
    Base(PageKey),
    Thumb(PageKey),
}

impl TexKey {
    fn page(&self) -> u32 {
        match self {
            TexKey::Tile(k) => k.page,
            TexKey::Base(k) | TexKey::Thumb(k) => k.page,
        }
    }

    /// Belongs to the render state `(scale_milli, rot, invert)`.
    fn current(&self, key: (u32, u8, bool)) -> bool {
        match self {
            TexKey::Tile(t) => (t.scale_milli, t.rot, t.invert) == key,
            TexKey::Base(p) | TexKey::Thumb(p) => (p.rot, p.invert) == (key.1, key.2),
        }
    }
}

pub struct PdfDoc {
    pub path: PathBuf,
    cfg: ViewerCfg,
    wake: Wake,
    up: Uploader,
    session: PdfSession,
    status: Status,
    info: Option<Arc<DocInfo>>,
    geoms: Vec<PageGeom>,
    layout: Layout,
    view: [f32; 2],
    ppp: f32,
    scroll: [f32; 2],
    zoom: Zoom,
    scale: f32,
    rot: u8,
    page: u32,
    /// The sidebar shown (`None`: hidden).
    pub sidebar: PdfSidebar,
    last_sidebar: PdfSidebar,
    dark_theme: bool,
    dark_override: Option<bool>,
    generation: u64,
    render_key: Option<(u32, u8, bool)>,
    tex: TexCache<TexKey>,
    requested: HashSet<TexKey>,
    failed: HashMap<u32, PdfError>,
    texts: HashMap<u32, Arc<PageText>>,
    text_requested: HashSet<u32>,
    thumbs_wanted: Range<u32>,
    pub find: PdfFind,
    /// The go-to-page field (`Some` while open), edited by the frontend.
    pub goto: Option<String>,
    /// The frontend should focus the go-to field.
    pub goto_focus: bool,
    sel: Option<Selection>,
    dragging: bool,
    drag_moved: bool,
    pending_copy: bool,
    copy_out: Option<String>,
    restore: Option<Position>,
    pending_script: Vec<String>,
    pw_gen: u64,
    /// The frontend should give the canvas keyboard focus.
    pub focus_request: bool,
    /// Set by the frontend every frame: a text field of this document (find, page) has
    /// the keyboard, so unmodified viewer keys type instead (`commands::binding`).
    pub field_focus: bool,
}

impl PdfDoc {
    /// Start opening `path` (on the session's worker; this returns at once). `restore`
    /// puts the view back where it was (remembered position, reload).
    pub fn new(path: PathBuf, cfg: ViewerCfg, wake: Wake, up: Uploader, restore: Option<Position>) -> PdfDoc {
        let session = PdfSession::open_path(path.clone(), cfg.session.clone(), wake.clone(), up.clone());
        let zoom = restore.map_or(Zoom::from_config(cfg.default_zoom), |r| r.zoom);
        PdfDoc {
            path,
            tex: TexCache::new(cfg.cache_bytes),
            sidebar: cfg.sidebar,
            last_sidebar: cfg.sidebar,
            cfg,
            wake,
            up,
            session,
            status: Status::Opening,
            info: None,
            geoms: Vec::new(),
            layout: Layout::default(),
            view: [800.0, 600.0],
            ppp: 1.0,
            scroll: [0.0; 2],
            zoom,
            scale: ACTUAL,
            rot: restore.map_or(0, |r| r.rot % 4),
            page: 0,
            dark_theme: false,
            dark_override: None,
            generation: 0,
            render_key: None,
            requested: HashSet::new(),
            failed: HashMap::new(),
            texts: HashMap::new(),
            text_requested: HashSet::new(),
            thumbs_wanted: 0..0,
            find: PdfFind::default(),
            goto: None,
            goto_focus: false,
            sel: None,
            dragging: false,
            drag_moved: false,
            pending_copy: false,
            copy_out: None,
            restore,
            pending_script: Vec::new(),
            pw_gen: preview_pdf::password_generation(),
            focus_request: true,
            field_focus: false,
        }
    }

    /// Never modified.
    pub fn is_dirty(&self) -> bool {
        false
    }

    /// Reopen the file (it changed on disk, or a password was typed), keeping the page,
    /// zoom and rotation.
    pub fn reload(&mut self) {
        if self.status == Status::Ready {
            self.restore = self.position();
        }
        self.session =
            PdfSession::open_path(self.path.clone(), self.cfg.session.clone(), self.wake.clone(), self.up.clone());
        self.status = Status::Opening;
        self.pw_gen = preview_pdf::password_generation();
        self.generation = 0;
        self.render_key = None;
        self.tex.clear();
        self.requested.clear();
        self.failed.clear();
        self.texts.clear();
        self.text_requested.clear();
        self.sel = None;
        self.dragging = false;
        self.pending_copy = false;
        if self.find.active() || self.find.deferred {
            self.find.id = None;
            self.find.matches.clear();
            self.find.current = None;
            self.find.deferred = !self.find.query.text.trim().is_empty();
        }
    }

    /// Apply changed `[pdf]` settings (cheap when nothing changed; call every frame).
    /// Render switches (annotations, system fonts, time budget, size cap) reopen the
    /// document.
    pub fn set_config(&mut self, cfg: &ViewerCfg) {
        let reopen = cfg.session.opts != self.cfg.session.opts
            || cfg.session.timeout != self.cfg.session.timeout
            || cfg.session.max_bytes != self.cfg.session.max_bytes;
        let relayout = cfg.page_gap != self.cfg.page_gap;
        if cfg.cache_bytes != self.cfg.cache_bytes {
            self.tex.set_budget(cfg.cache_bytes);
        }
        if cfg.dark_pages != self.cfg.dark_pages {
            self.dark_override = None;
        }
        self.cfg.default_zoom = cfg.default_zoom;
        self.cfg.sidebar = cfg.sidebar;
        self.cfg.page_gap = cfg.page_gap;
        self.cfg.dark_pages = cfg.dark_pages;
        self.cfg.external_links = cfg.external_links;
        self.cfg.cache_bytes = cfg.cache_bytes;
        if reopen {
            self.cfg.session = cfg.session.clone();
            self.reload();
        } else if relayout {
            self.relayout_keep();
        }
    }

    // ---- state ------------------------------------------------------------------------

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn is_ready(&self) -> bool {
        self.status == Status::Ready
    }

    /// Locked by a password: `Some(wrong)` (`wrong`: a typed password did not fit).
    pub fn locked(&self) -> Option<bool> {
        match &self.status {
            Status::Failed(PdfError::Password { wrong }) => Some(*wrong),
            _ => None,
        }
    }

    /// Document facts (title, engine, …) once open.
    pub fn info(&self) -> Option<&Arc<DocInfo>> {
        self.info.as_ref()
    }

    pub fn page_count(&self) -> u32 {
        self.geoms.len() as u32
    }

    /// The current page (0-based).
    pub fn page(&self) -> u32 {
        self.page
    }

    /// Size of `page` in millimetres (as printed, page rotation applied).
    pub fn page_size_mm(&self, page: u32) -> Option<(f32, f32)> {
        let g = self.geoms.get(page as usize)?;
        Some((g.w_pt * 25.4 / 72.0, g.h_pt * 25.4 / 72.0))
    }

    /// "Page 3 of 12" (toolbar, accessibility).
    pub fn page_label(&self, page: u32) -> String {
        tr!("pdf-page-of", page = (page + 1).to_string(), pages = self.page_count().to_string())
    }

    pub fn zoom(&self) -> Zoom {
        self.zoom
    }

    /// The zoom in percent (100 = printed size).
    pub fn percent(&self) -> f32 {
        self.scale / ACTUAL * 100.0
    }

    /// "150 %".
    pub fn zoom_label(&self) -> String {
        tr!("pdf-zoom-percent", percent = format!("{:.0}", self.percent()))
    }

    /// View points per page point.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// View rotation in quarter turns clockwise.
    pub fn rotation(&self) -> u8 {
        self.rot
    }

    /// Pages are drawn with inverted lightness.
    pub fn invert(&self) -> bool {
        self.dark_override.unwrap_or(match self.cfg.dark_pages {
            PdfDarkPages::Off => false,
            PdfDarkPages::Invert => true,
            PdfDarkPages::FollowTheme => self.dark_theme,
        })
    }

    /// The app's theme is dark (for `dark_pages = "follow_theme"`; call every frame).
    pub fn set_theme_dark(&mut self, dark: bool) {
        self.dark_theme = dark;
    }

    /// `[pdf] external_links`.
    pub fn external_links(&self) -> PdfExternalLinks {
        self.cfg.external_links
    }

    /// The outline (bookmarks; empty without one).
    pub fn outline(&self) -> &[Outline] {
        self.session.outline()
    }

    /// The whole document's size in view points.
    pub fn doc_size(&self) -> [f32; 2] {
        self.layout.size
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The scroll offset (doc space of the viewport's top-left).
    pub fn scroll(&self) -> [f32; 2] {
        self.scroll
    }

    /// The viewport size set last.
    pub fn viewport(&self) -> [f32; 2] {
        self.view
    }

    /// The text of `page` if extracted (accessibility description).
    pub fn page_text(&self, page: u32) -> Option<&str> {
        self.texts.get(&page).map(|t| t.text.as_str())
    }

    /// Where the reader is (`None` before the document opened).
    pub fn position(&self) -> Option<Position> {
        if self.status != Status::Ready {
            return self.restore;
        }
        let r = self.layout.rects.get(self.page as usize)?;
        let h = (r[3] - r[1]).max(1.0);
        Some(Position { page: self.page, offset: (self.scroll[1] - r[1]) / h, zoom: self.zoom, rot: self.rot })
    }

    // ---- view -------------------------------------------------------------------------

    /// The viewport: size in view points and pixels per view point. Fit zooms follow it.
    pub fn set_viewport(&mut self, size: [f32; 2], ppp: f32) {
        let size = [size[0].max(1.0), size[1].max(1.0)];
        let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
        if size == self.view && ppp == self.ppp {
            return;
        }
        self.ppp = ppp;
        if size != self.view {
            self.view = size;
            self.relayout_keep();
        }
    }

    fn compute_scale(&self) -> f32 {
        let gap = self.cfg.page_gap;
        let s = match self.zoom {
            Zoom::FitWidth => layout::fit_width(&self.geoms, self.rot, gap, self.view[0]),
            Zoom::FitPage => layout::fit_page(&self.geoms, self.rot, gap, self.view),
            Zoom::Scale(s) => s,
        };
        s.clamp(MIN_PERCENT / 100.0 * ACTUAL, MAX_PERCENT / 100.0 * ACTUAL)
    }

    fn rebuild_layout(&mut self) {
        self.scale = self.compute_scale();
        self.layout = Layout::build(&self.geoms, self.rot, self.scale, self.cfg.page_gap, self.view[0]);
    }

    /// Relayout keeping the current position.
    fn relayout_keep(&mut self) {
        let pos = self.position();
        self.rebuild_layout();
        if let Some(p) = pos.filter(|_| self.status == Status::Ready) {
            self.apply_position(p);
        }
    }

    fn apply_position(&mut self, p: Position) {
        let Some(last) = self.geoms.len().checked_sub(1) else { return };
        let page = (p.page as usize).min(last);
        let r = self.layout.rects[page];
        self.scroll = [(self.layout.size[0] - self.view[0]) / 2.0, r[1] + p.offset * (r[3] - r[1])];
        self.clamp_scroll();
        self.page = page as u32;
    }

    fn clamp_scroll(&mut self) {
        for i in 0..2 {
            let max = (self.layout.size[i] - self.view[i]).max(0.0);
            self.scroll[i] = if self.scroll[i].is_finite() { self.scroll[i].clamp(0.0, max) } else { 0.0 };
        }
    }

    fn view_rect(&self) -> Rect {
        [self.scroll[0], self.scroll[1], self.scroll[0] + self.view[0], self.scroll[1] + self.view[1]]
    }

    /// The current page from the scroll position: the most visible page (the first of
    /// equals).
    fn page_from_scroll(&mut self) {
        let v = self.view_rect();
        let mut best = (-1.0f32, self.page);
        for i in self.layout.visible(v) {
            let r = self.layout.rects[i];
            let h = r[3].min(v[3]) - r[1].max(v[1]);
            if h > best.0 + 0.5 {
                best = (h, i as u32);
            }
        }
        self.page = best.1;
    }

    /// Scroll by `delta` view points (wheel, keys, drag).
    pub fn scroll_by(&mut self, delta: [f32; 2]) {
        self.set_scroll([self.scroll[0] + delta[0], self.scroll[1] + delta[1]]);
    }

    /// Scroll to `offset` (doc space of the viewport's top-left; clamped).
    pub fn set_scroll(&mut self, offset: [f32; 2]) {
        self.scroll = offset;
        self.clamp_scroll();
        self.page_from_scroll();
    }

    /// Show `page` (0-based, clamped) at the top.
    pub fn go_to_page(&mut self, page: u32) {
        let Some(last) = self.geoms.len().checked_sub(1) else { return };
        let page = (page as usize).min(last);
        let r = self.layout.rects[page];
        self.scroll[1] = r[1] - self.cfg.page_gap;
        self.clamp_scroll();
        self.page = page as u32;
    }

    /// Show a link / outline destination.
    pub fn go_to_dest(&mut self, d: Dest) {
        let Some(g) = self.geoms.get(d.page as usize).copied() else { return };
        let Some(top) = d.top else { return self.go_to_page(d.page) };
        let p = layout::rotate_point(&g, self.rot, [d.left.unwrap_or(0.0), top]);
        let r = self.layout.rects[d.page as usize];
        self.scroll[1] = r[1] + p[1] * self.scale;
        self.clamp_scroll();
        self.page = d.page;
    }

    pub fn next_page(&mut self) {
        self.go_to_page(self.page.saturating_add(1));
    }

    pub fn prev_page(&mut self) {
        self.go_to_page(self.page.saturating_sub(1));
    }

    /// Set the zoom. `anchor` (viewport coordinates) stays over the same spot of the page;
    /// `None` anchors the view centre.
    pub fn zoom_to(&mut self, zoom: Zoom, anchor: Option<[f32; 2]>) {
        let anchor = anchor.unwrap_or([self.view[0] / 2.0, self.view[1] / 2.0]);
        let doc = [self.scroll[0] + anchor[0], self.scroll[1] + anchor[1]];
        let at = self.layout.page_at_y(doc[1]).filter(|_| self.status == Status::Ready);
        let local = at.map(|i| {
            let r = self.layout.rects[i];
            [(doc[0] - r[0]) / self.scale, (doc[1] - r[1]) / self.scale]
        });
        self.zoom = zoom;
        self.rebuild_layout();
        if let (Some(i), Some(l)) = (at, local) {
            let r = self.layout.rects[i];
            self.scroll = [r[0] + l[0] * self.scale - anchor[0], r[1] + l[1] * self.scale - anchor[1]];
            self.clamp_scroll();
            self.page_from_scroll();
        }
    }

    /// Ctrl+wheel / pinch: multiply the zoom by `factor` around `anchor`.
    pub fn zoom_by(&mut self, factor: f32, anchor: [f32; 2]) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom_to(Zoom::percent(self.percent() * factor), Some(anchor));
        }
    }

    /// The next preset up.
    pub fn zoom_in(&mut self) {
        let cur = self.percent();
        let next = ZOOM_STEPS.iter().copied().find(|s| *s > cur * 1.01).unwrap_or(MAX_PERCENT);
        self.zoom_to(Zoom::percent(next), None);
    }

    /// The next preset down.
    pub fn zoom_out(&mut self) {
        let cur = self.percent();
        let next = ZOOM_STEPS.iter().rev().copied().find(|s| *s < cur / 1.01).unwrap_or(MIN_PERCENT);
        self.zoom_to(Zoom::percent(next), None);
    }

    /// Turn the view a quarter (`cw`: clockwise), keeping the page.
    pub fn rotate(&mut self, cw: bool) {
        let pos = self.position();
        self.rot = (self.rot + if cw { 1 } else { 3 }) % 4;
        self.rebuild_layout();
        if let Some(p) = pos.filter(|_| self.status == Status::Ready) {
            self.apply_position(Position { rot: self.rot, ..p });
        }
    }

    /// Show / hide the sidebar (it comes back as it was, thumbnails the first time).
    pub fn toggle_sidebar(&mut self) {
        if self.sidebar == PdfSidebar::None {
            self.sidebar =
                if self.last_sidebar == PdfSidebar::None { PdfSidebar::Thumbnails } else { self.last_sidebar };
        } else {
            self.last_sidebar = self.sidebar;
            self.sidebar = PdfSidebar::None;
        }
    }

    pub fn set_sidebar(&mut self, s: PdfSidebar) {
        if s != PdfSidebar::None {
            self.last_sidebar = s;
        }
        self.sidebar = s;
    }

    /// Dark pages on / off for this tab (`[pdf] dark_pages` is left alone).
    pub fn toggle_dark(&mut self) {
        self.dark_override = Some(!self.invert());
    }

    // ---- coordinates ------------------------------------------------------------------

    /// A page-point rectangle of `page` in viewport coordinates.
    pub fn page_rect_to_view(&self, page: u32, r: Rect) -> Option<Rect> {
        let g = self.geoms.get(page as usize)?;
        let pr = self.layout.rects.get(page as usize)?;
        let rr = layout::rotate_rect(g, self.rot, r);
        let (ox, oy) = (pr[0] - self.scroll[0], pr[1] - self.scroll[1]);
        Some([ox + rr[0] * self.scale, oy + rr[1] * self.scale, ox + rr[2] * self.scale, oy + rr[3] * self.scale])
    }

    /// The page under a viewport point and the point in page points; `clamp` takes the
    /// nearest page when the point is between / beside pages.
    pub fn view_to_page(&self, pos: [f32; 2], clamp: bool) -> Option<(u32, [f32; 2])> {
        let doc = [pos[0] + self.scroll[0], pos[1] + self.scroll[1]];
        let i = self.layout.page_at_y(doc[1])?;
        let r = self.layout.rects[i];
        if !clamp && (doc[0] < r[0] || doc[0] > r[2] || doc[1] < r[1] || doc[1] > r[3]) {
            return None;
        }
        let local = [(doc[0] - r[0]) / self.scale, (doc[1] - r[1]) / self.scale];
        Some((i as u32, layout::unrotate_point(&self.geoms[i], self.rot, local)))
    }

    // ---- per frame --------------------------------------------------------------------

    /// Take worker results and request what the view needs (call every frame after
    /// [`PdfDoc::set_viewport`]). Returns whether anything changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        if self.locked().is_some() && preview_pdf::password_generation() != self.pw_gen {
            self.reload();
            changed = true;
        }
        for e in self.session.poll() {
            changed = true;
            self.on_event(e);
        }
        if self.status == Status::Ready {
            self.request_wanted();
        }
        changed
    }

    fn on_event(&mut self, e: Event) {
        match e {
            Event::Opened(info) => self.on_opened(info),
            Event::Failed(err) => {
                self.status = Status::Failed(err);
                self.pending_script.clear();
            }
            Event::Region { key, tex, .. } => {
                let k = TexKey::Tile(key);
                self.requested.remove(&k);
                if self.render_key.is_some_and(|rk| k.current(rk)) {
                    self.tex.insert(k, key.page, tex);
                }
            }
            Event::Base { key, tex } => {
                let k = TexKey::Base(key);
                self.requested.remove(&k);
                if (key.rot, key.invert) == (self.rot, self.invert()) {
                    self.failed.remove(&key.page);
                    self.tex.insert(k, key.page, tex);
                }
            }
            Event::Thumb { key, tex } => {
                let k = TexKey::Thumb(key);
                self.requested.remove(&k);
                if (key.rot, key.invert) == (self.rot, self.invert()) {
                    self.tex.insert(k, key.page, tex);
                }
            }
            Event::Text(page, t) => {
                self.text_requested.remove(&page);
                self.texts.insert(page, t);
                self.finish_copy();
            }
            Event::FindProgress { id, page, matches, searched, total } => {
                if self.find.id != Some(id) {
                    return;
                }
                self.find.searched = searched;
                self.find.total = total;
                if !matches.is_empty() {
                    self.find.matches.insert(page, matches);
                    if self.find.jump && self.find.current.is_none() {
                        self.find.jump = false;
                        self.find.current = Some((page, 0));
                        self.scroll_to_current();
                    }
                }
            }
            Event::PageFailed { job, error } => match job {
                // Failed jobs stay "requested": no retry loop (Retry clears them).
                Job::Base(k) => {
                    self.failed.insert(k.page, error);
                }
                Job::Text(p) => {
                    self.text_requested.remove(&p);
                    self.texts.insert(p, Arc::new(PageText::default()));
                    self.finish_copy();
                }
                _ => log::debug!("PDF {}: {job:?} failed: {error}", self.path.display()),
            },
        }
    }

    fn on_opened(&mut self, info: Arc<DocInfo>) {
        self.info = Some(info);
        self.geoms = self.session.geoms().to_vec();
        self.status = Status::Ready;
        self.rebuild_layout();
        match self.restore.take() {
            Some(p) => self.apply_position(p),
            None => {
                self.scroll = [(self.layout.size[0] - self.view[0]) / 2.0, 0.0];
                self.clamp_scroll();
                self.page = 0;
            }
        }
        if self.find.deferred {
            self.find.deferred = false;
            self.find_changed();
        }
        for s in std::mem::take(&mut self.pending_script) {
            self.devscript(&s);
        }
    }

    /// The render state of new jobs: `(scale_milli, rot, invert)`.
    fn current_key(&self) -> (u32, u8, bool) {
        (((self.scale * self.ppp) * 1000.0).round().max(1.0) as u32, self.rot, self.invert())
    }

    fn need_tiles(&self, page: usize, scale_milli: u32) -> bool {
        let (w, h) = self.geoms[page].px(scale_milli as f32 / 1000.0, self.rot);
        w.max(h) > self.cfg.session.base_px
    }

    /// Tiles of `page` the viewport shows, nearest the view centre first.
    fn tiles_for(&self, page: usize, key: (u32, u8, bool)) -> Vec<TileKey> {
        let r = self.layout.rects[page];
        let v = self.view_rect();
        let px = key.0 as f32 / 1000.0 / self.scale;
        let view = [(v[0] - r[0]).max(0.0) * px, (v[1] - r[1]).max(0.0) * px, (v[2] - r[0]) * px, (v[3] - r[1]) * px];
        visible_tiles(page as u32, &self.geoms[page], key.0, key.1, key.2, view)
    }

    fn request(&mut self, k: TexKey, job: Job, prio: u32) {
        if !self.tex.contains(&k) && self.requested.insert(k) {
            self.session.request(job, prio);
        }
    }

    fn request_text(&mut self, page: u32, prio: u32) {
        if !self.texts.contains_key(&page) && self.text_requested.insert(page) {
            self.session.request(Job::Text(page), prio);
        }
    }

    fn request_wanted(&mut self) {
        let key = self.current_key();
        if self.render_key != Some(key) {
            if self.render_key.is_some() {
                self.generation += 1;
                self.session.set_generation(self.generation);
            }
            self.tex.retain(|k| k.current(key));
            self.requested.retain(|k| k.current(key));
            self.render_key = Some(key);
        }
        let (_, rot, invert) = key;
        let vis = self.layout.visible(self.view_rect());
        let mut keep: HashSet<TexKey> = HashSet::new();
        let mut order = 0u32;
        for (n, i) in vis.clone().enumerate() {
            let pk = PageKey { page: i as u32, rot, invert };
            keep.insert(TexKey::Base(pk));
            self.request(TexKey::Base(pk), Job::Base(pk), prio::VISIBLE_BASE + n as u32);
            self.request_text(i as u32, prio::VISIBLE_TEXT + n as u32);
            if self.need_tiles(i, key.0) {
                for t in self.tiles_for(i, key) {
                    keep.insert(TexKey::Tile(t));
                    self.request(TexKey::Tile(t), Job::Region(t), prio::VISIBLE_REGION + order);
                    order += 1;
                }
            }
        }
        // Prefetch the neighbours' base layers.
        let n = self.layout.len();
        let around = [vis.start.checked_sub(1), (vis.end < n).then_some(vis.end)];
        for (j, i) in around.into_iter().flatten().enumerate() {
            let pk = PageKey { page: i as u32, rot, invert };
            keep.insert(TexKey::Base(pk));
            self.request(TexKey::Base(pk), Job::Base(pk), prio::PREFETCH + j as u32);
        }
        if self.sidebar == PdfSidebar::Thumbnails {
            let r = self.thumbs_wanted.start.min(n as u32)..self.thumbs_wanted.end.min(n as u32);
            for (j, p) in r.enumerate() {
                let pk = PageKey { page: p, rot, invert };
                keep.insert(TexKey::Thumb(pk));
                self.request(TexKey::Thumb(pk), Job::Thumb(pk), prio::THUMB + j as u32);
            }
        }
        self.tex.trim(self.page as f32, |k| keep.contains(k));
        if self.texts.len() > MAX_TEXTS && !self.pending_copy {
            let focus = self.page as i64;
            let mut pages: Vec<u32> = self.texts.keys().copied().filter(|p| !vis.contains(&(*p as usize))).collect();
            pages.sort_by_key(|p| std::cmp::Reverse((*p as i64 - focus).abs()));
            let extra = self.texts.len() - MAX_TEXTS;
            for p in pages.into_iter().take(extra) {
                self.texts.remove(&p);
            }
        }
    }

    /// The sidebar shows thumbnails of `pages` (call while the thumbnail list is shown).
    pub fn want_thumbs(&mut self, pages: Range<u32>) {
        self.thumbs_wanted = pages;
    }

    /// The thumbnail of `page`, if rendered.
    pub fn thumb(&mut self, page: u32) -> Option<Tex> {
        let pk = PageKey { page, rot: self.rot, invert: self.invert() };
        self.tex.get(&TexKey::Thumb(pk)).cloned()
    }

    /// Try a failed page again.
    pub fn retry_page(&mut self, page: u32) {
        self.failed.remove(&page);
        self.requested.retain(|k| k.page() != page);
    }

    /// The pages to paint this frame, top to bottom.
    pub fn visible_pages(&mut self) -> Vec<PageView> {
        if self.status != Status::Ready {
            return Vec::new();
        }
        let key = self.current_key();
        let vis = self.layout.visible(self.view_rect());
        let mut out = Vec::with_capacity(vis.len());
        for i in vis {
            let page = i as u32;
            let r = self.layout.rects[i];
            let rect = [r[0] - self.scroll[0], r[1] - self.scroll[1], r[2] - self.scroll[0], r[3] - self.scroll[1]];
            let base = self.tex.get(&TexKey::Base(PageKey { page, rot: key.1, invert: key.2 })).cloned();
            let mut tiles = Vec::new();
            if self.need_tiles(i, key.0) {
                let s = key.0 as f32 / 1000.0;
                for t in self.tiles_for(i, key) {
                    let Some(reg) = t.region(&self.geoms[i]) else { continue };
                    let Some(tex) = self.tex.get(&TexKey::Tile(t)).cloned() else { continue };
                    let to_view = |v: u32| v as f32 / s * self.scale;
                    let tr = [
                        rect[0] + to_view(reg.x),
                        rect[1] + to_view(reg.y),
                        rect[0] + to_view(reg.x + reg.w),
                        rect[1] + to_view(reg.y + reg.h),
                    ];
                    tiles.push((tex, tr));
                }
            }
            let to_view = |rs: Vec<Rect>| -> Vec<Rect> {
                rs.into_iter().filter_map(|r| self.page_rect_to_view(page, r)).collect()
            };
            let mut selection = Vec::new();
            if let (Some(sel), Some(t)) = (&self.sel, self.texts.get(&page))
                && let Some(range) = sel.on_page(page, t.chars.len())
            {
                selection = to_view(t.rects(range));
            }
            let (mut matches, mut current) = (Vec::new(), Vec::new());
            if self.find.open
                && let Some(ms) = self.find.matches.get(&page)
            {
                for (k, m) in ms.iter().enumerate() {
                    if self.find.current == Some((page, k)) {
                        current = to_view(m.rects.clone());
                    } else {
                        matches.extend(to_view(m.rects.clone()));
                    }
                }
            }
            out.push(PageView {
                page,
                rect,
                base,
                tiles,
                failed: self.failed.get(&page).cloned(),
                selection,
                matches,
                current,
            });
        }
        out
    }

    // ---- links and pointer ------------------------------------------------------------

    /// The link under a viewport point.
    pub fn link_at(&self, pos: [f32; 2]) -> Option<&LinkTarget> {
        let (page, p) = self.view_to_page(pos, false)?;
        self.session
            .links(page)
            .iter()
            .find(|l| p[0] >= l.rect[0] && p[0] <= l.rect[2] && p[1] >= l.rect[1] && p[1] <= l.rect[3])
            .map(|l| &l.target)
    }

    /// What the pointer is over (for the cursor).
    pub fn hover(&self, pos: [f32; 2]) -> Hover {
        if self.link_at(pos).is_some() {
            return Hover::Link;
        }
        match self.view_to_page(pos, false) {
            Some((page, p)) if self.texts.get(&page).is_some_and(|t| t.hit(p, 0.0).is_some()) => Hover::Text,
            _ => Hover::Nothing,
        }
    }

    /// A click on a link: internal ones scroll here; web links come back for the frontend
    /// per `[pdf] external_links` (nothing for `off`); others are blocked.
    pub fn click_link(&mut self, pos: [f32; 2]) -> Option<LinkAction> {
        match self.link_at(pos)?.clone() {
            LinkTarget::Dest(d) => {
                self.go_to_dest(d);
                Some(LinkAction::Jumped)
            }
            LinkTarget::Uri(u) if !is_web_uri(&u) => Some(LinkAction::Blocked(u)),
            LinkTarget::Uri(u) => match self.cfg.external_links {
                PdfExternalLinks::Off => None,
                PdfExternalLinks::Ask => Some(LinkAction::Ask(u)),
                PdfExternalLinks::Open => Some(LinkAction::Open(u)),
            },
        }
    }

    /// The char position under `pos` on its page (`clamp`: nearest text).
    fn text_pos(&self, pos: [f32; 2], unit: Unit) -> Option<(TextPos, TextPos)> {
        let (page, p) = self.view_to_page(pos, true)?;
        let t = self.texts.get(&page)?;
        let i = t.hit(p, HIT_SLACK).or_else(|| t.nearest(p))?;
        if unit == Unit::Char {
            let r = t.chars[i].rect();
            let c = if p[0] > (r[0] + r[2]) / 2.0 { i + 1 } else { i };
            return Some((TextPos::new(page, c), TextPos::new(page, c)));
        }
        Some(select::unit_range(t, page, i, unit))
    }

    /// Left button down at `pos`: `clicks` 1 / 2 / 3 select by char / word / line;
    /// `shift` extends the selection.
    pub fn pointer_down(&mut self, pos: [f32; 2], clicks: u8, shift: bool) {
        let unit = match clicks {
            0 | 1 => Unit::Char,
            2 => Unit::Word,
            _ => Unit::Line,
        };
        let Some(r) = self.text_pos(pos, unit) else {
            self.sel = None;
            self.dragging = false;
            return;
        };
        match (&mut self.sel, shift) {
            (Some(s), true) => s.head = r,
            _ => self.sel = Some(Selection { anchor: r, head: r, unit }),
        }
        self.dragging = true;
        self.drag_moved = clicks > 1 || shift;
    }

    /// The pointer moved with the button down.
    pub fn pointer_drag(&mut self, pos: [f32; 2]) {
        if !self.dragging {
            return;
        }
        let unit = self.sel.map_or(Unit::Char, |s| s.unit);
        let head = match self.text_pos(pos, unit) {
            Some(r) => r,
            None => {
                // A page without text (yet): up to its start / end.
                let Some((page, _)) = self.view_to_page(pos, true) else { return };
                let anchor_page = self.sel.map_or(page, |s| s.anchor.0.page);
                let p = if page > anchor_page { TextPos::new(page, 0) } else { TextPos::page_end(page) };
                (p, p)
            }
        };
        if let Some(s) = &mut self.sel {
            s.head = head;
            self.drag_moved = true;
        }
    }

    /// The button went up.
    pub fn pointer_up(&mut self) {
        self.dragging = false;
        if self.sel.is_some_and(|s| s.is_empty()) {
            self.sel = None;
        }
    }

    pub fn selection(&self) -> Option<&Selection> {
        self.sel.as_ref()
    }

    pub fn clear_selection(&mut self) {
        self.sel = None;
    }

    pub fn select_all(&mut self) {
        if let Some(last) = self.page_count().checked_sub(1) {
            self.sel = Some(Selection::range(TextPos::new(0, 0), TextPos::page_end(last)));
        }
    }

    /// Select the whole text of `page`.
    pub fn select_page(&mut self, page: u32) {
        if page < self.page_count() {
            self.sel = Some(Selection::range(TextPos::new(page, 0), TextPos::page_end(page)));
        }
    }

    /// The selected text, when every page it touches has its text.
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.sel.filter(|s| !s.is_empty())?;
        if sel.pages().any(|p| !self.texts.contains_key(&p)) {
            return None;
        }
        Some(select::selected_text(&sel, |p| self.texts.get(&p).map(|t| &**t)))
    }

    /// Selected chars (status bar; pages without text yet count as empty).
    pub fn selection_len(&self) -> usize {
        let Some(sel) = self.sel else { return 0 };
        sel.pages()
            .filter_map(|p| self.texts.get(&p).and_then(|t| sel.on_page(p, t.chars.len())))
            .map(|r| r.len())
            .sum()
    }

    /// Copy: the selected text now, or `None` while pages of it are still being read (it
    /// then arrives through [`PdfDoc::take_copy`]).
    pub fn copy(&mut self) -> Option<String> {
        let sel = self.sel.filter(|s| !s.is_empty())?;
        if let Some(t) = self.selected_text() {
            self.pending_copy = false;
            return Some(t);
        }
        self.pending_copy = true;
        for p in sel.pages() {
            self.request_text(p, prio::VISIBLE_TEXT);
        }
        None
    }

    fn finish_copy(&mut self) {
        if self.pending_copy
            && let Some(t) = self.selected_text()
        {
            self.pending_copy = false;
            self.copy_out = Some(t);
        }
    }

    /// Text a deferred copy finished with (the caller puts it on the clipboard).
    pub fn take_copy(&mut self) -> Option<String> {
        self.copy_out.take()
    }

    // ---- find and go to ----------------------------------------------------------------

    /// Open the find bar (with the selection as the query when it is short).
    pub fn open_find(&mut self) {
        self.goto = None;
        self.find.open = true;
        self.find.focus_find = true;
        if let Some(t) = self.selected_text().filter(|t| !t.is_empty() && t.len() <= 200 && !t.contains('\n')) {
            self.find.query.text = t;
            self.find_changed();
        }
    }

    /// The query changed: restart the sweep from the current page.
    pub fn find_changed(&mut self) {
        self.find.matches.clear();
        self.find.current = None;
        self.find.searched = 0;
        self.find.total = self.page_count();
        if self.find.query.text.trim().is_empty() {
            self.session.cancel_find();
            self.find.id = None;
            return;
        }
        if self.status != Status::Ready {
            self.find.deferred = true;
            return;
        }
        self.find.id = Some(self.session.find(self.find.query.clone(), self.page));
        self.find.jump = true;
    }

    /// Next / previous match (wrapping), scrolled into view.
    pub fn find_step(&mut self, forward: bool) {
        if !self.find.active() {
            if !self.find.query.text.trim().is_empty() {
                self.find_changed();
            }
            return;
        }
        let all = self.find.flat();
        if all.is_empty() {
            return;
        }
        let next = match self.find.current.and_then(|c| all.iter().position(|m| *m == c)) {
            Some(i) if forward => (i + 1) % all.len(),
            Some(i) => (i + all.len() - 1) % all.len(),
            None if forward => all.iter().position(|m| m.0 >= self.page).unwrap_or(0),
            None => all.iter().rposition(|m| m.0 <= self.page).unwrap_or(all.len() - 1),
        };
        self.find.current = Some(all[next]);
        self.find.jump = false;
        self.scroll_to_current();
    }

    /// Close the find bar (its highlights go).
    pub fn close_find(&mut self) {
        self.find.open = false;
        self.session.cancel_find();
        self.find.id = None;
        self.find.matches.clear();
        self.find.current = None;
        self.find.deferred = false;
    }

    /// Put the current match in view (a third from the top when it was not visible).
    fn scroll_to_current(&mut self) {
        let Some((page, i)) = self.find.current else { return };
        let Some(m) = self.find.matches.get(&page).and_then(|m| m.get(i)) else { return };
        let Some(r) =
            m.rects.iter().copied().reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
        else {
            return;
        };
        let Some(v) = self.page_rect_to_view(page, r) else { return };
        let inside = v[0] >= 0.0 && v[1] >= 0.0 && v[2] <= self.view[0] && v[3] <= self.view[1];
        if !inside {
            let mut s = self.scroll;
            s[1] += v[1] - self.view[1] / 3.0;
            if v[0] < 0.0 || v[2] > self.view[0] {
                s[0] += (v[0] + v[2]) / 2.0 - self.view[0] / 2.0;
            }
            self.scroll = s;
            self.clamp_scroll();
        }
        self.page = page;
    }

    pub fn open_goto(&mut self) {
        self.find.focus_find = false;
        self.goto = Some(String::new());
        self.goto_focus = true;
    }

    /// Go to the page typed in the go-to field (1-based); closes it. `false` when the text
    /// is not a number.
    pub fn submit_goto(&mut self, text: &str) -> bool {
        let Ok(n) = text.trim().parse::<u32>() else { return false };
        self.goto = None;
        self.go_to_page(n.saturating_sub(1));
        self.focus_request = true;
        true
    }

    /// Esc: close the go-to field, the find bar, or the selection. `false` when there was
    /// nothing to close.
    pub fn close_bars(&mut self) -> bool {
        if self.goto.take().is_some() {
            return true;
        }
        if self.find.open {
            self.close_find();
            return true;
        }
        self.sel.take().is_some()
    }

    // ---- commands and keys --------------------------------------------------------------

    /// A viewer command ([`Action::is_pdf`]) or one of the editors' it reuses (find,
    /// go to, match case, whole word). `false` when `a` is not one of them.
    pub fn command(&mut self, a: &Action) -> bool {
        match a {
            Action::PdfZoomIn => self.zoom_in(),
            Action::PdfZoomOut => self.zoom_out(),
            Action::PdfZoomActual => self.zoom_to(Zoom::Scale(ACTUAL), None),
            Action::PdfFitWidth => self.zoom_to(Zoom::FitWidth, None),
            Action::PdfFitPage => self.zoom_to(Zoom::FitPage, None),
            Action::PdfNextPage => self.next_page(),
            Action::PdfPrevPage => self.prev_page(),
            Action::PdfFirstPage => self.go_to_page(0),
            Action::PdfLastPage => self.go_to_page(u32::MAX),
            Action::PdfRotateCw => self.rotate(true),
            Action::PdfRotateCcw => self.rotate(false),
            Action::PdfToggleSidebar => self.toggle_sidebar(),
            Action::PdfToggleDarkPages => self.toggle_dark(),
            Action::EditorFind => self.open_find(),
            Action::EditorGoTo => self.open_goto(),
            Action::EditorFindNext | Action::EditorFindPrev => {
                if self.find.query.text.trim().is_empty() {
                    self.open_find();
                } else {
                    self.find.open = true;
                    self.find_step(*a == Action::EditorFindNext);
                }
            }
            Action::EditorMatchCase | Action::EditorWholeWord => {
                if !self.find.open {
                    return true;
                }
                if *a == Action::EditorMatchCase {
                    self.find.query.case = !self.find.query.case;
                } else {
                    self.find.query.whole_word = !self.find.query.whole_word;
                }
                self.find_changed();
            }
            _ => return false,
        }
        true
    }

    /// One input event while the canvas has the keyboard; returns text for the clipboard.
    pub fn input(&mut self, ev: EditInput) -> Option<String> {
        match ev {
            EditInput::Copy | EditInput::Cut => return self.copy(),
            EditInput::Key(key, m) => self.key(key, m),
            EditInput::Text(_) | EditInput::ImeCommit(_) | EditInput::Paste(_) => {}
        }
        None
    }

    /// The fixed keys: arrows, Page Up / Down, Space / Shift+Space, Home / End, Ctrl+A.
    pub fn key(&mut self, key: Option<KeyName>, m: KeyMods) {
        let Some(key) = key else { return };
        let page = (self.view[1] * 0.9).max(LINE);
        let wide = self.layout.size[0] > self.view[0] + 0.5;
        match key {
            KeyName::A if m.command => self.select_all(),
            _ if m.command || m.alt => {}
            KeyName::ArrowUp => self.scroll_by([0.0, -LINE]),
            KeyName::ArrowDown => self.scroll_by([0.0, LINE]),
            KeyName::ArrowLeft if wide => self.scroll_by([-LINE, 0.0]),
            KeyName::ArrowRight if wide => self.scroll_by([LINE, 0.0]),
            KeyName::ArrowLeft => self.prev_page(),
            KeyName::ArrowRight => self.next_page(),
            KeyName::PageUp => self.scroll_by([0.0, -page]),
            KeyName::PageDown => self.scroll_by([0.0, page]),
            KeyName::Space if m.shift => self.scroll_by([0.0, -page]),
            KeyName::Space => self.scroll_by([0.0, page]),
            KeyName::Home => self.set_scroll([self.scroll[0], 0.0]),
            KeyName::End => self.set_scroll([self.scroll[0], f32::MAX]),
            _ => {}
        }
    }

    // ---- SOYUZ_SCRIPT ---------------------------------------------------------------------

    /// `editor:pdf=<verb>`: `page=N`, `zoom=150`, `fit=width|page`, `rotate=cw|ccw`,
    /// `find=text`, `next`, `prev`, `sidebar=none|thumbnails|outline`, `select=all`,
    /// `dark`, `copy` (into [`PdfDoc::take_copy`]). Before the document opens they wait.
    pub fn devscript(&mut self, verb: &str) {
        if self.status == Status::Opening {
            self.pending_script.push(verb.to_string());
            return;
        }
        let (cmd, val) = verb.split_once('=').unwrap_or((verb, ""));
        match (cmd, val) {
            ("page", n) => self.go_to_page(n.trim().parse::<u32>().unwrap_or(1).saturating_sub(1)),
            ("zoom", z) => match z.trim().trim_end_matches('%').parse::<f32>() {
                Ok(p) => self.zoom_to(Zoom::percent(p), None),
                Err(_) => log::warn!("SOYUZ_SCRIPT editor:pdf={verb}: not a percentage"),
            },
            ("fit", "page") => self.zoom_to(Zoom::FitPage, None),
            ("fit", _) => self.zoom_to(Zoom::FitWidth, None),
            ("rotate", "ccw") => self.rotate(false),
            ("rotate", _) => self.rotate(true),
            ("find", q) => {
                self.find.open = true;
                self.find.query.text = q.to_string();
                self.find_changed();
            }
            ("next", _) => self.find_step(true),
            ("prev", _) => self.find_step(false),
            ("sidebar", "none") => self.set_sidebar(PdfSidebar::None),
            ("sidebar", "outline") => self.set_sidebar(PdfSidebar::Outline),
            ("sidebar", _) => self.set_sidebar(PdfSidebar::Thumbnails),
            ("select", _) => self.select_all(),
            ("dark", _) => self.toggle_dark(),
            ("copy", _) => {
                if let Some(t) = self.copy() {
                    self.copy_out = Some(t);
                }
            }
            _ => log::warn!("SOYUZ_SCRIPT editor:pdf={verb}: unknown"),
        }
    }
}
