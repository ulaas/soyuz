//! Shared preview state and renderers used by both the preview pane and Quick Look:
//! request management (one in-flight load per consumer, stale requests dropped), hex
//! paging, folder-size streaming, and painters for every content kind plus the info
//! section.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use egui::{
    Align2, Color32, CornerRadius, FontId, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};
use soyuz_core::config::Config;
use soyuz_core::{EntryInfo, OpControl, format_size, format_time};

use super::hex::{self, HexDoc};
use super::markdown;
use crate::services::preview::{Content, Kind, Limits, Loaded, Msg, PreviewClient, Source};
use crate::services::preview_gfx::{StyledLine, TextFont};
use crate::theme::icons::{self, ph};
use crate::theme::{Palette, palette};
use crate::ui::gfx::{self, TexExt};
use crate::ui::views::columns::{attr_letters, mode_string};
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{pill_button, spinner, trunc_galley};

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
    changed_at: Instant,
}

impl Drop for SizeJob {
    fn drop(&mut self) {
        self.ctl.cancel();
    }
}

/// One preview consumer's state.
pub struct PreviewState {
    client: PreviewClient,
    key: Option<TargetKey>,
    pub entry: Option<EntryInfo>,
    pub loaded: Option<Loaded>,
    loading_since: Option<Instant>,
    hex: Option<HexDoc>,
    sizes: Option<SizeJob>,
    pub force_hex: bool,
    /// Quick Look image zoom (1 = fit) and pan offset.
    pub zoom: f32,
    pub pan: Vec2,
    big: bool,
    /// Cache key of the in-flight request (the result is remembered when it lands).
    cache_key: Option<crate::services::preview_cache::CacheKey>,
    /// Display scale for page-like renders (refreshed every [`Self::poll`]).
    ppp: f32,
}

impl PreviewState {
    pub fn new(ctx: &egui::Context, big: bool) -> Self {
        PreviewState {
            client: PreviewClient::new(crate::host::egui_wake(ctx), gfx::egui_uploader(ctx)),
            key: None,
            entry: None,
            loaded: None,
            loading_since: None,
            hex: None,
            sizes: None,
            force_hex: false,
            zoom: 1.0,
            pan: Vec2::ZERO,
            big,
            cache_key: None,
            ppp: ctx.pixels_per_point(),
        }
    }

    /// Request limits from the config and palette.
    pub fn limits(&self, cfg: &Config, p: &Palette) -> Limits {
        Limits {
            max_image_px: if self.big { 4096 } else { 1600 },
            max_image_mp: cfg.preview.max_image_mp.max(1),
            max_text_bytes: (cfg.preview.max_text_kib.max(4) * 1024) as usize,
            dark: p.dark,
            mono: TextFont::monospace(12.0),
            fg: gfx::rgba(p.text_primary),
            pixels_per_point: self.ppp,
            rich: crate::services::preview_rich::RichLimits::from_config(&cfg.preview, self.big),
        }
    }

    /// Show `entry` (or nothing). Issues a new request only when the target changed.
    pub fn set_target(&mut self, entry: Option<&EntryInfo>, limits: Limits) {
        let key = entry.map(|e| TargetKey {
            path: e.path.clone(),
            modified: e.modified,
            size: e.size,
            force_hex: self.force_hex,
        });
        if key == self.key {
            return;
        }
        let same_path = matches!((&key, &self.key), (Some(a), Some(b)) if a.path == b.path);
        self.key = key;
        self.entry = entry.cloned();
        self.hex = None;
        self.sizes = None;
        if !same_path {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
        match entry {
            Some(e) => {
                let ck = (!self.force_hex && !e.is_dir_like()).then(|| crate::services::preview_cache::CacheKey {
                    path: e.path.clone(),
                    modified: e.modified,
                    size: e.size,
                    variant: (limits.max_image_px, limits.dark, limits.max_text_bytes),
                });
                if let Some(hit) = ck.as_ref().and_then(crate::services::preview_cache::get) {
                    self.client.cancel();
                    self.cache_key = None;
                    self.loaded = Some(hit);
                    self.loading_since = None;
                    return;
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
    }

    /// Forget the current target so the next `set_target` reloads.
    pub fn invalidate(&mut self) {
        self.key = None;
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

    /// Drain worker results (call once per frame).
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.ppp = ctx.pixels_per_point();
        while let Some(msg) = self.client.try_recv() {
            match msg {
                Msg::Loaded { id, loaded } => {
                    if id != self.client.latest() {
                        continue;
                    }
                    self.loading_since = None;
                    if let Content::Hex(init) = &loaded.content {
                        self.hex = Some(HexDoc::new(init));
                    }
                    if let (Content::Folder(_), Some(e)) = (&loaded.content, &self.entry)
                        && !Source::from_path(&e.path).is_entry()
                    {
                        let c = ctx.clone();
                        let (rx, ctl) =
                            soyuz_search::folder_sizes(vec![e.path.clone()], Arc::new(move || c.request_repaint()));
                        self.sizes = Some(SizeJob {
                            path: e.path.clone(),
                            rx,
                            ctl,
                            bytes: 0,
                            files: 0,
                            dirs: 0,
                            done: false,
                            changed_at: Instant::now(),
                        });
                    }
                    if let Some(k) = self.cache_key.take() {
                        crate::services::preview_cache::put(k, &loaded);
                    }
                    self.loaded = Some(*loaded);
                }
                Msg::Page { doc, page, bytes } => {
                    if let Some(h) = &mut self.hex {
                        h.on_page(doc, page, bytes);
                    }
                }
            }
        }
        if let Some(s) = &mut self.sizes {
            for ev in s.rx.try_iter() {
                match ev {
                    soyuz_search::SizeEvent::Partial { path, bytes, files, dirs } if path == s.path => {
                        (s.bytes, s.files, s.dirs) = (bytes, files, dirs);
                        s.changed_at = Instant::now();
                    }
                    soyuz_search::SizeEvent::Done { path, bytes, files, dirs, .. } if path == s.path => {
                        (s.bytes, s.files, s.dirs, s.done) = (bytes, files, dirs, true);
                        s.changed_at = Instant::now();
                    }
                    _ => {}
                }
            }
        }
    }

    /// `(bytes, files, dirs, done)` of the recursive folder size, when computing.
    pub fn folder_size(&self) -> Option<(u64, u64, u64, bool)> {
        self.sizes.as_ref().map(|s| (s.bytes, s.files, s.dirs, s.done))
    }
}

// ---- info ---------------------------------------------------------------------------

/// Info rows for the selection: type, size, dates, attributes, owner + loader facts.
pub fn info_rows(
    entry: &EntryInfo,
    loaded: Option<&Loaded>,
    folder_size: Option<(u64, u64, u64, bool)>,
    cfg: &Config,
) -> Vec<(String, String)> {
    let style = cfg.general.size_style;
    let fmt_t = |t: SystemTime| format_time(t, &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Long));
    let meta = loaded.and_then(|l| l.meta.as_ref());
    let mut rows = vec![("Type".to_string(), crate::state::listing::kind_label(entry))];
    if entry.is_dir_like() {
        if let Some((bytes, files, dirs, done)) = folder_size {
            let v = format!("{}{}", format_size(bytes, style), if done { "" } else { "…" });
            rows.push(("Size".into(), v));
            rows.push(("Contains".into(), tr!("views-info-contains", files = files, dirs = dirs)));
        }
    } else {
        let size = if entry.size == 0 { meta.map_or(0, |m| m.size) } else { entry.size };
        rows.push(("Size".into(), tr!("views-info-size-bytes", size = format_size(size, style), bytes = size)));
        if let Some(a) = entry.alloc_size.filter(|&a| a != entry.size && Source::from_path(&entry.path).is_entry()) {
            rows.push(("Compressed".into(), format_size(a, style)));
        }
    }
    let parent = match Source::from_path(&entry.path) {
        Source::Entry { archive, inner } => {
            let dir = inner.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            format!("{}!/{dir}", soyuz_fs::strip_verbatim(&archive).display())
        }
        Source::File(p) => p.parent().map(|d| soyuz_fs::strip_verbatim(d).display().to_string()).unwrap_or_default(),
    };
    rows.push(("Where".into(), parent));
    let pick = |a: Option<SystemTime>, b: Option<SystemTime>| a.or(b);
    if let Some(t) = pick(entry.modified, meta.and_then(|m| m.modified)) {
        rows.push(("Modified".into(), fmt_t(t)));
    }
    if let Some(t) = pick(entry.created, meta.and_then(|m| m.created)) {
        rows.push(("Created".into(), fmt_t(t)));
    }
    if let Some(t) = pick(entry.accessed, meta.and_then(|m| m.accessed)) {
        rows.push(("Accessed".into(), fmt_t(t)));
    }
    let attrs = if entry.attrs.is_empty() { meta.map(|m| m.attrs).unwrap_or_default() } else { entry.attrs };
    if !attrs.is_empty() {
        rows.push(("Attributes".into(), attr_letters(attrs)));
    }
    if let Some(m) = entry.mode.or(meta.and_then(|m| m.mode)) {
        rows.push(("Mode".into(), format!("{} ({:o})", mode_string(m), m & 0o7777)));
    }
    if let Some(o) = entry.owner.as_deref().or(meta.and_then(|m| m.owner.as_deref())) {
        rows.push(("Owner".into(), o.to_string()));
    }
    if let Some(l) = loaded {
        for (k, v) in &l.facts {
            if !rows.iter().any(|(rk, _)| rk == k) {
                rows.push((k.clone(), v.clone()));
            }
        }
    }
    rows
}

/// `1234567` → `1,234,567` (`1.234.567` in Turkish).
pub fn group_digits(n: u64) -> String {
    soyuz_core::locale::group_int(n)
}

/// Translated label of an info row key. [`info_rows`] and the preview loaders key rows
/// by their English name (rows are deduplicated and looked up by it); unknown keys are
/// shown as they are.
pub fn info_label(key: &str) -> &str {
    match key {
        "Type" => tr!("views-info-type"),
        "Size" => tr!("views-info-size"),
        "Contains" => tr!("views-info-contains-label"),
        "Compressed" => tr!("views-info-compressed"),
        "Where" => tr!("views-info-where"),
        "Modified" => tr!("views-info-modified"),
        "Created" => tr!("views-info-created"),
        "Accessed" => tr!("views-info-accessed"),
        "Attributes" => tr!("views-info-attributes"),
        "Mode" => tr!("views-info-mode"),
        "Owner" => tr!("views-info-owner"),
        "Title" => tr!("views-info-title"),
        "Artist" => tr!("views-info-artist"),
        "Album" => tr!("views-info-album"),
        "Album artist" => tr!("views-info-album-artist"),
        "Year" => tr!("views-info-year"),
        "Genre" => tr!("views-info-genre"),
        "Track" => tr!("views-info-track"),
        "Composer" => tr!("views-info-composer"),
        "Container" => tr!("views-info-container"),
        "Format" => tr!("views-info-format"),
        "Sample rate" => tr!("views-info-sample-rate"),
        "Channels" => tr!("views-info-channels"),
        "Bit depth" => tr!("views-info-bit-depth"),
        "Duration" => tr!("views-info-duration"),
        "Dimensions" => tr!("views-info-dimensions"),
        "Frame rate" => tr!("views-info-frame-rate"),
        "Bitrate" => tr!("views-info-bitrate"),
        "Tracks" => tr!("views-info-tracks"),
        "Color" => tr!("views-info-color"),
        "Compression" => tr!("views-info-compression"),
        "Encoding" => tr!("views-info-encoding"),
        "Encrypted" => tr!("views-info-encrypted"),
        "Entries" => tr!("views-info-entries"),
        "Language" => tr!("views-info-language"),
        "Lines" => tr!("views-info-lines"),
        "Note" => tr!("views-info-note"),
        "PDF version" => tr!("views-info-pdf-version"),
        "Previewer" => tr!("views-info-previewer"),
        "Pages" => tr!("views-info-pages"),
        "Author" => tr!("views-info-author"),
        "Producer" => tr!("views-info-producer"),
        "Publisher" => tr!("views-info-publisher"),
        "Published" => tr!("views-info-published"),
        "Version" => tr!("views-info-version"),
        "EPUB version" => tr!("views-info-epub-version"),
        "Page size" => tr!("views-info-page-size"),
        "Paragraphs" => tr!("views-info-paragraphs"),
        "Codec" => tr!("views-info-codec"),
        "Copyright" => tr!("views-info-copyright"),
        "Designer" => tr!("views-info-designer"),
        "Weight" => tr!("views-info-weight"),
        "Spacing" => tr!("views-info-spacing"),
        "Variable axes" => tr!("views-info-variable-axes"),
        "Bounds" => tr!("views-info-bounds"),
        "Faces" => tr!("views-info-faces"),
        "Meshes" => tr!("views-info-meshes"),
        "Drawn" => tr!("views-info-drawn"),
        "Shown" => tr!("views-info-shown"),
        "Renderer" => tr!("views-info-renderer"),
        "Waveform" => tr!("views-info-waveform"),
        "Words" => tr!("views-info-words"),
        "Sheets" => tr!("views-info-sheets"),
        "Slides" => tr!("views-info-slides"),
        "Chapters" => tr!("views-info-chapters"),
        "Family" => tr!("views-info-family"),
        "Style" => tr!("views-info-style"),
        "Glyphs" => tr!("views-info-glyphs"),
        "Triangles" => tr!("views-info-triangles"),
        "Vertices" => tr!("views-info-vertices"),
        other => other,
    }
}

/// Two-column info grid.
pub fn info_grid(ui: &mut Ui, rows: &[(String, String)], label_w: f32) {
    let p = palette(ui.ctx());
    let font = FontId::proportional(11.5);
    // Translated labels can be longer: widen the label column up to a limit.
    let widest = rows
        .iter()
        .map(|(k, _)| ui.painter().layout_no_wrap(info_label(k).to_string(), font.clone(), p.text_tertiary).size().x)
        .fold(0.0_f32, f32::max);
    let label_w = label_w.max(widest + 4.0).min(label_w * 1.6);
    for (k, v) in rows {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (r, _) = ui.allocate_exact_size(vec2(label_w, 18.0), Sense::hover());
            let label = info_label(k);
            ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, font.clone(), p.text_tertiary);
            ui.add(egui::Label::new(egui::RichText::new(v).size(12.0).color(p.text_primary)).wrap());
        });
    }
}

// ---- content ------------------------------------------------------------------------

fn centered_message(ui: &Ui, rect: Rect, icon: &str, color: Color32, title: &str, body: &str) {
    let p = palette(ui.ctx());
    let c = rect.center();
    ui.painter().text(c - vec2(0.0, 30.0), Align2::CENTER_CENTER, icon, icons::font_fill(34.0), color);
    if !title.is_empty() {
        ui.painter().text(c + vec2(0.0, 8.0), Align2::CENTER_CENTER, title, semibold(13.5), p.text_primary);
    }
    if !body.is_empty() {
        let g = ui.painter().layout(
            body.to_string(),
            FontId::proportional(12.0),
            p.text_tertiary,
            (rect.width() - 40.0).clamp(80.0, 360.0),
        );
        let pos = pos2(c.x - g.size().x / 2.0, c.y + 22.0);
        ui.painter().galley(pos, g, p.text_tertiary);
    }
}

/// Subtle checkerboard behind images (transparency).
fn checker(ui: &Ui, r: Rect, p: &Palette) {
    let cell = 8.0;
    let (a, b) = if p.dark {
        (Color32::from_gray(38), Color32::from_gray(30))
    } else {
        (Color32::from_gray(238), Color32::from_gray(226))
    };
    ui.painter().rect_filled(r, 0.0, a);
    let cols = (r.width() / cell).ceil() as i32;
    let rows = (r.height() / cell).ceil() as i32;
    if cols * rows > 40_000 {
        return;
    }
    for y in 0..rows {
        for x in 0..cols {
            if (x + y) % 2 == 0 {
                let cr =
                    Rect::from_min_size(pos2(r.left() + x as f32 * cell, r.top() + y as f32 * cell), vec2(cell, cell))
                        .intersect(r);
                ui.painter().rect_filled(cr, 0.0, b);
            }
        }
    }
}

/// Draw the preview content into `rect`. `interactive` enables zoom/pan (Quick Look).
pub fn content(ui: &mut Ui, rect: Rect, st: &mut PreviewState, interactive: bool) -> Option<PreviewAction> {
    let p = palette(ui.ctx());
    let mut action = None;
    let Some(entry) = st.entry.clone() else {
        centered_message(
            ui,
            rect,
            ph::EYE,
            p.text_tertiary,
            tr!("views-preview-nothing-selected"),
            tr!("views-preview-nothing-selected-sub"),
        );
        return None;
    };
    let Some(loaded) = st.loaded.clone() else {
        if st.loading_since.is_some_and(|t| t.elapsed().as_secs_f32() > 0.12) {
            spinner(ui, rect.center(), 11.0, p.accent);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
        return None;
    };
    // A newer request is still loading: dim the old content slightly.
    let stale = st.loading_since.is_some();
    match &loaded.content {
        Content::Image(img) => {
            let tex_size = img.tex.size_vec2();
            let ppp = ui.ctx().pixels_per_point();
            let natural = vec2(img.width as f32, img.height as f32) / ppp;
            let avail = rect.shrink(if interactive { 0.0 } else { 12.0 });
            let fit = (avail.width() / natural.x).min(avail.height() / natural.y).clamp(0.0001, 1.0);
            let mut scale = fit;
            if interactive {
                let resp = ui.interact(rect, ui.id().with("ql.image"), Sense::click_and_drag());
                if resp.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 200.0);
                    if scroll != 0.0 {
                        let old = st.zoom;
                        st.zoom = (st.zoom * (scroll / 300.0).exp()).clamp(0.1, 32.0);
                        if let Some(ptr) = resp.hover_pos() {
                            let c = rect.center() + st.pan;
                            st.pan += (ptr - c) * (1.0 - st.zoom / old);
                        }
                    }
                }
                if resp.dragged() {
                    st.pan += resp.drag_delta();
                }
                if resp.double_clicked() {
                    if (st.zoom - 1.0).abs() < 0.01 {
                        st.zoom = 1.0 / fit; // 100%
                    } else {
                        st.zoom = 1.0;
                        st.pan = Vec2::ZERO;
                    }
                }
                scale = fit * st.zoom;
            }
            let size = natural * scale;
            let r = Rect::from_center_size(avail.center() + if interactive { st.pan } else { Vec2::ZERO }, size);
            let clip = ui.clip_rect().intersect(rect);
            let painter = ui.painter().with_clip_rect(clip);
            if !interactive {
                checker(ui, r, &p);
            }
            let tint = if stale { Color32::from_white_alpha(200) } else { Color32::WHITE };
            painter.image(img.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
            if !interactive {
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, p.border_subtle), StrokeKind::Outside);
            }
            let _ = tex_size;
            if interactive {
                let pct = format!("{:.0}%", scale * ppp * 100.0);
                let badge = Rect::from_min_size(rect.left_bottom() + vec2(12.0, -34.0), vec2(56.0, 22.0));
                painter.rect_filled(badge, CornerRadius::same(6), p.bg_surface3.gamma_multiply(0.9));
                painter.text(badge.center(), Align2::CENTER_CENTER, pct, FontId::proportional(11.5), p.text_secondary);
            }
        }
        Content::Svg { uri, bytes } => {
            let avail = rect.shrink(16.0);
            let img = egui::Image::from_bytes(uri.clone(), egui::load::Bytes::Shared(bytes.clone()))
                .max_size(avail.size() * if interactive { st.zoom } else { 1.0 })
                .maintain_aspect_ratio(true);
            let mut child = ui.new_child(
                UiBuilder::new().max_rect(avail).layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
            );
            child.add(img);
        }
        Content::Text(t) => text_view(ui, rect, &t.lines, t.widest, t.truncated),
        Content::Markdown { blocks, truncated } => {
            let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 8.0))));
            ScrollArea::vertical().id_salt(("md", entry.path.clone())).auto_shrink([false, false]).show(
                &mut child,
                |ui| {
                    markdown::show(ui, blocks, &p);
                    if *truncated {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(tr!("views-preview-truncated")).size(11.0).color(p.text_tertiary));
                    }
                },
            );
        }
        Content::Hex(_) => {
            if let Some(doc) = &mut st.hex {
                hex::show(ui, rect, doc, &st.client, &p, if interactive { 13.0 } else { 11.5 });
            }
        }
        Content::Archive(l) => {
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), 40.0));
            ui.painter().text(
                header.left_center() + vec2(14.0, -7.0),
                Align2::LEFT_CENTER,
                tr!("views-preview-archive-entries", count = l.total_entries),
                semibold(12.5),
                p.text_primary,
            );
            ui.painter().text(
                header.left_center() + vec2(14.0, 9.0),
                Align2::LEFT_CENTER,
                tr!(
                    "views-preview-archive-unpacked",
                    format = &l.format,
                    size = format_size(l.total_size, soyuz_core::SizeStyle::Binary)
                ),
                FontId::proportional(11.0),
                p.text_tertiary,
            );
            let br = Rect::from_min_size(pos2(header.right() - 84.0, header.top() + 8.0), vec2(74.0, 24.0));
            let mut bchild = ui.new_child(UiBuilder::new().max_rect(br));
            if pill_button(&mut bchild, Some(ph::FOLDER_OPEN), tr!("views-preview-browse"), false).clicked() {
                action = Some(PreviewAction::Open(entry.path.clone()));
            }
            let list = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
            let mut child = ui.new_child(UiBuilder::new().max_rect(list));
            child.set_clip_rect(list.intersect(ui.clip_rect()));
            let row_h = 22.0;
            ScrollArea::vertical().id_salt(("arch", entry.path.clone())).auto_shrink([false, false]).show_rows(
                &mut child,
                row_h,
                l.entries.len(),
                |ui, range| {
                    for i in range {
                        let (path, is_dir, size) = &l.entries[i];
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                        if resp.hovered() {
                            ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), CornerRadius::same(4), p.bg_surface2);
                        }
                        let depth = path.trim_end_matches('/').matches('/').count() as f32;
                        let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path);
                        let x = r.left() + 14.0 + depth.min(8.0) * 12.0;
                        let (g, c) =
                            if *is_dir { (ph::FOLDER_SIMPLE, p.glyph_folder) } else { (ph::FILE, p.text_tertiary) };
                        ui.painter().text(pos2(x + 7.0, r.center().y), Align2::CENTER_CENTER, g, icons::font(13.0), c);
                        let tg = trunc_galley(
                            ui.painter(),
                            name,
                            FontId::proportional(12.0),
                            p.text_primary,
                            r.right() - x - 100.0,
                        );
                        ui.painter().galley(pos2(x + 20.0, r.center().y - tg.size().y / 2.0), tg, p.text_primary);
                        if !is_dir {
                            ui.painter().text(
                                pos2(r.right() - 14.0, r.center().y),
                                Align2::RIGHT_CENTER,
                                format_size(*size, soyuz_core::SizeStyle::Binary),
                                FontId::proportional(11.0),
                                p.text_tertiary,
                            );
                        }
                    }
                },
            );
        }
        Content::Folder(s) => {
            let c = rect.center() - vec2(0.0, rect.height() * 0.12);
            ui.painter().text(c, Align2::CENTER_CENTER, ph::FOLDER_SIMPLE, icons::font_fill(54.0), p.glyph_folder);
            let mut y = c.y + 48.0;
            let line = |ui: &Ui, y: &mut f32, text: String, font: FontId, color: Color32| {
                ui.painter().text(pos2(rect.center().x, *y), Align2::CENTER_CENTER, text, font, color);
                *y += 20.0;
            };
            if let Some(err) = &s.error {
                line(ui, &mut y, tr!("views-preview-folder-unreadable").into(), semibold(13.0), p.text_primary);
                line(ui, &mut y, err.clone(), FontId::proportional(11.5), p.danger);
            } else {
                line(
                    ui,
                    &mut y,
                    tr!("views-preview-folder-counts", dirs = s.dirs, files = s.files),
                    semibold(13.5),
                    p.text_primary,
                );
                match st.folder_size() {
                    Some((bytes, files, _, done)) => {
                        let size = format_size(bytes, soyuz_core::SizeStyle::Binary);
                        let text = if done {
                            tr!("views-preview-folder-size", size = size, files = files)
                        } else {
                            tr!("views-preview-folder-size-running", size = size, files = files)
                        };
                        line(
                            ui,
                            &mut y,
                            text,
                            FontId::proportional(12.0),
                            if done { p.text_secondary } else { p.accent },
                        );
                        if !done {
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                        }
                    }
                    None => line(
                        ui,
                        &mut y,
                        tr!(
                            "views-preview-folder-direct",
                            size = format_size(s.direct_bytes, soyuz_core::SizeStyle::Binary)
                        ),
                        FontId::proportional(12.0),
                        p.text_secondary,
                    ),
                }
                if s.hidden > 0 {
                    line(
                        ui,
                        &mut y,
                        tr!("views-preview-folder-hidden", count = s.hidden),
                        FontId::proportional(11.5),
                        p.text_tertiary,
                    );
                }
                if !s.top_ext.is_empty() {
                    y += 6.0;
                    let chips: Vec<String> = s.top_ext.iter().map(|(e, n)| format!(".{e} {n}")).collect();
                    let font = FontId::proportional(11.0);
                    let widths: Vec<f32> = chips
                        .iter()
                        .map(|c| ui.painter().layout_no_wrap(c.clone(), font.clone(), p.text_secondary).size().x + 16.0)
                        .collect();
                    let total: f32 = widths.iter().sum::<f32>() + 6.0 * (chips.len() as f32 - 1.0);
                    let mut x = rect.center().x - total.min(rect.width() - 20.0) / 2.0;
                    for (chip, w) in chips.iter().zip(widths) {
                        if x + w > rect.right() - 8.0 {
                            break;
                        }
                        let r = Rect::from_min_size(pos2(x, y - 10.0), vec2(w, 20.0));
                        ui.painter().rect_filled(r, CornerRadius::same(10), p.bg_surface2);
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, chip, font.clone(), p.text_secondary);
                        x += w + 6.0;
                    }
                    y += 22.0;
                }
                if let Some((name, t)) = &s.newest {
                    let rel = format_time(*t, &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Relative));
                    line(
                        ui,
                        &mut y,
                        tr!("views-preview-folder-newest", name = name, when = rel),
                        FontId::proportional(11.0),
                        p.text_tertiary,
                    );
                }
            }
            let _ = y;
        }
        Content::Media { video } => {
            let c = rect.center() - vec2(0.0, rect.height() * 0.18);
            let (g, col) = if *video { (ph::FILM_SLATE, p.glyph_video) } else { (ph::WAVEFORM, p.glyph_audio) };
            let disc = Rect::from_center_size(c, vec2(96.0, 96.0));
            ui.painter().rect_filled(disc, CornerRadius::same(22), col.gamma_multiply(0.14));
            ui.painter().text(c, Align2::CENTER_CENTER, g, icons::font_fill(46.0), col);
            let mut y = disc.bottom() + 22.0;
            let title = loaded.facts.iter().find(|(k, _)| k == "Title").map(|(_, v)| v.clone());
            let artist = loaded.facts.iter().find(|(k, _)| k == "Artist").map(|(_, v)| v.clone());
            let headline = title.unwrap_or_else(|| entry.name.clone());
            let tg = trunc_galley(ui.painter(), &headline, semibold(14.0), p.text_primary, rect.width() - 32.0);
            ui.painter().galley(pos2(rect.center().x - tg.size().x / 2.0, y - tg.size().y / 2.0), tg, p.text_primary);
            y += 20.0;
            if let Some(a) = artist {
                ui.painter().text(
                    pos2(rect.center().x, y),
                    Align2::CENTER_CENTER,
                    a,
                    FontId::proportional(12.0),
                    p.text_secondary,
                );
                y += 20.0;
            }
            let summary: Vec<String> = ["Duration", "Dimensions", "Format", "Container"]
                .iter()
                .filter_map(|k| loaded.facts.iter().find(|(fk, _)| fk == k).map(|(_, v)| v.clone()))
                .collect();
            if !summary.is_empty() {
                ui.painter().text(
                    pos2(rect.center().x, y),
                    Align2::CENTER_CENTER,
                    summary.join(" · "),
                    FontId::proportional(11.5),
                    p.text_tertiary,
                );
            }
        }
        Content::Rich(r) => {
            if let Some(a) = super::rich::show(ui, rect, r, &entry, interactive) {
                action = Some(a);
            }
        }
        Content::Message { icon, title, body } => {
            centered_message(ui, rect, crate::ui::icons::glyph(*icon), p.text_tertiary, title, body);
            if loaded.kind == Kind::Pdf || loaded.kind == Kind::Image {
                let br = Rect::from_center_size(rect.center() + vec2(0.0, 74.0), vec2(120.0, 26.0));
                let mut child = ui.new_child(UiBuilder::new().max_rect(br));
                if pill_button(&mut child, Some(ph::BINARY), tr!("views-preview-show-bytes"), false).clicked() {
                    action = Some(PreviewAction::ToggleHex);
                }
            }
        }
    }
    action
}

/// Virtualized, highlighted text with a line-number gutter.
fn text_view(ui: &mut Ui, rect: Rect, lines: &Arc<Vec<StyledLine>>, widest: usize, truncated: bool) {
    let p = palette(ui.ctx());
    let font = FontId::monospace(12.0);
    let row_h = 18.0;
    let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
    let n = lines.len() + usize::from(truncated);
    let digits = n.to_string().len().max(2);
    let gutter = char_w * digits as f32 + 18.0;
    let content_w = gutter + 12.0 + char_w * widest.min(crate::services::preview_text::MAX_LINE_CHARS) as f32 + 24.0;
    ui.painter().rect_filled(rect, 0.0, p.bg_sunken);
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    ScrollArea::both().id_salt(("text", Arc::as_ptr(lines) as usize)).auto_shrink([false, false]).show_rows(
        &mut child,
        row_h,
        n,
        |ui, range| {
            let w = content_w.max(ui.available_width());
            for i in range {
                let (r, _) = ui.allocate_exact_size(vec2(w, row_h), Sense::hover());
                if i >= lines.len() {
                    ui.painter().text(
                        pos2(r.left() + gutter + 12.0, r.center().y),
                        Align2::LEFT_CENTER,
                        tr!("views-preview-text-truncated"),
                        FontId::proportional(11.0),
                        p.text_tertiary,
                    );
                    continue;
                }
                ui.painter().text(
                    pos2(r.left() + gutter - 8.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    (i + 1).to_string(),
                    font.clone(),
                    p.text_tertiary.gamma_multiply(0.8),
                );
                let galley = ui.painter().layout_job(gfx::layout_job(&lines[i]));
                ui.painter().galley(
                    pos2(r.left() + gutter + 12.0, r.center().y - galley.size().y / 2.0),
                    galley,
                    p.text_primary,
                );
            }
        },
    );
    // Gutter separator.
    ui.painter().line_segment(
        [pos2(rect.left() + gutter + 2.0, rect.top()), pos2(rect.left() + gutter + 2.0, rect.bottom())],
        Stroke::new(1.0, p.border_subtle),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::{Attributes, EntryKind};

    #[test]
    fn digit_grouping() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1234567), "1,234,567");
    }

    #[test]
    fn multi_plural_messages_format_on_one_line() {
        use crate::i18n::{FluentArgs, t_args_in};
        use soyuz_core::Locale;
        let mut a = FluentArgs::new();
        a.set("dirs", 2);
        a.set("files", 1);
        assert_eq!(t_args_in("views-preview-folder-counts", &a, Locale::EnUs), "2 folders · 1 file");
        assert_eq!(t_args_in("views-preview-folder-counts", &a, Locale::TrTr), "2 klasör · 1 dosya");
        let mut a = FluentArgs::new();
        a.set("files", 1234);
        a.set("folders", 1);
        assert_eq!(t_args_in("views-index-idle", &a, Locale::EnUs), "Indexed · 1,234 files, 1 folder");
        assert_eq!(t_args_in("views-index-idle", &a, Locale::TrTr), "Dizine eklendi · 1.234 dosya, 1 klasör");
    }

    #[test]
    fn info_rows_for_file_and_folder() {
        let cfg = Config::default();
        let mut e = EntryInfo::minimal(PathBuf::from("/data/report.pdf"), EntryKind::File);
        e.size = 2048;
        e.attrs = Attributes::READONLY;
        e.mode = Some(0o100644);
        e.modified = Some(SystemTime::UNIX_EPOCH);
        let rows = info_rows(&e, None, None, &cfg);
        let get = |k: &str| rows.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        assert!(get("Size").unwrap().contains("2,048 bytes"));
        assert!(get("Mode").unwrap().contains("644"));
        assert!(get("Attributes").is_some());
        assert!(get("Modified").is_some());
        assert_eq!(get("Type").unwrap(), crate::state::listing::kind_label(&e));

        let d = EntryInfo::minimal(PathBuf::from("/data/sub"), EntryKind::Dir);
        let rows = info_rows(&d, None, Some((4096, 3, 1, false)), &cfg);
        let size = rows.iter().find(|(a, _)| a == "Size").unwrap();
        assert!(size.1.ends_with('…'), "still computing");
        assert!(rows.iter().any(|(a, v)| a == "Contains" && v == "3 files, 1 folder"));

        let a = EntryInfo::minimal(PathBuf::from("/x/a.zip!/dir/f.txt"), EntryKind::File);
        let rows = info_rows(&a, None, None, &cfg);
        assert!(rows.iter().any(|(k, v)| k == "Where" && v.ends_with("a.zip!/dir")));
    }
}
