//! The preview pane (Phase T4): what the focused pane's cursor item (else the first
//! selected item, else the folder) looks like, beside the panes (`Regions::preview`, `[tui]
//! preview_ratio` of their width; below them when the terminal is narrow). Toggled by
//! `TogglePreviewPane` (Alt+P); `ToggleInfoPane` shows only the details,
//! `ToggleHexPreview` flips to bytes.
//!
//! The loading is the core's (`models::preview_load::PreviewLoader`, the same worker
//! requests as the GUI: one in flight, stale results dropped, nothing loads while the
//! pane is hidden or has no room). [`prepare`] keeps the target in step with the
//! selection, drains results and builds the model ([`PreviewVm`], kept in the input
//! state because hex rows load pages on demand); [`draw_preview`] draws it:
//!
//! - a header row: the file's icon and name, its kind (and `hex`) on the right;
//! - the body: text and code with the syntax colours (truecolor / 256 colours; the
//!   terminal's own colour in 16-colour and mono modes), Markdown (headings, emphasis,
//!   lists, quotes, code), the hex view, an archive listing with a cursor (Enter opens the
//!   archive at that entry), a folder summary, a two-file diff (two text files selected),
//!   pictures, or a message;
//! - the info section below a separator: type, size, dates, attributes and what the
//!   loader found (`models::preview_info::info_rows`).
//!
//! Pictures ([`Body::Picture`]) go through `crate::images` (`[tui] images`: kitty, sixel,
//! iTerm2, half-blocks; a `[W×H]` placeholder in mono colour mode): photos and other
//! images (an animated one shows its first frame), SVG and 3D models (the loader runs with
//! `PreviewLoader::terminal`, so the core rasterizes them: `services::preview_raster`), the
//! first page of a PDF (pdfium; its text when pdfium is missing), a video's poster frame,
//! an audio file's or e-book's cover and a font's specimen strips. A one-line summary
//! (dimensions, format) sits under the picture and the details (EXIF too) in the info
//! section; when the picture cannot be drawn (`images = "off"`, no room, encoding, gone) a
//! text line says so in its place.
//!
//! The media player (Phase V3b, [`Body::Player`], `crate::player`): an audio or video
//! file whose details have loaded plays here when `[media] player` is on and the FFmpeg
//! libraries are loaded. Its frames are pictures like any other (half-blocks or a
//! graphics protocol; a text line when images are off, the sound still plays), the poster
//! frame or cover shows until the first one, and a status row at the bottom of the body
//! holds the play / pause mark, elapsed / total, a progress bar, the volume, `muted` and
//! `loop` (words instead in screen-reader mode).
//!
//! Click targets: `PreviewScroll` over the pane (wheel scrolls, a click gives it the
//! keyboard), `ArchiveRow(i)` per listing row, `PlayerPlay` / `PlayerSeek` on the
//! player's status row; `crate::input::preview` handles them.

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span as RSpan};
use soyuz_appcore::color::Rgba8;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::file_class::{FileClass, classify};
use soyuz_appcore::icon::IconId;
use soyuz_appcore::models::preview_hex::{self, PENDING_ROW};
use soyuz_appcore::models::preview_info::{info_label, info_rows, kind_label};
use soyuz_appcore::models::preview_load::{PreviewLoader, diff_pair, target_entry};
use soyuz_appcore::services::preview::{Content, Kind, Loaded};
use soyuz_appcore::services::preview_diff::{Row as DiffRow, Tag};
use soyuz_appcore::services::preview_gfx::{Rgba, StyledLine, Tex};
use soyuz_appcore::services::preview_rich::Rich;
use soyuz_appcore::services::preview_text::{Block, Span as MdSpan};
use soyuz_appcore::tr;
use soyuz_core::config::TuiBorders;
use soyuz_core::{EntryInfo, SizeStyle, format_size, format_time};

use crate::dialog::safe;
use crate::images::{self, Drawn, ImageState};
use crate::input::InputState;
use crate::layout::{HitTarget, Hits, Regions};
use crate::player::{self, StatusVm, Want};
use crate::theme::{Role, TuiTheme, to_color};
use crate::view::text;
use crate::widgets::SelectList;

/// The colour the loader is asked to draw plain text in: runs in it take the theme's text
/// colour instead (syntax colours stay as they are).
pub const PLAIN_FG: Rgba = [0x01, 0x02, 0x03, 0xFF];
/// Shown after this long without a result (like the GUI's spinner delay).
const LOADING_AFTER_MS: u128 = 120;

/// Whether the preview pane is open (`TogglePreviewPane` / `ToggleInfoPane`).
pub fn shown(core: &AppCore) -> bool {
    core.ui.preview.open
}

/// One styled run of a body line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    /// A syntax colour (`None`: the role's).
    pub color: Option<[u8; 3]>,
    pub role: Option<Role>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Seg {
    pub fn new(text: impl Into<String>, role: Role) -> Seg {
        Seg { text: text.into(), role: Some(role), ..Seg::default() }
    }
}

/// One line of the body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PLine {
    pub segs: Vec<Seg>,
}

impl PLine {
    pub fn plain(text: impl Into<String>, role: Role) -> PLine {
        PLine { segs: vec![Seg::new(text, role)] }
    }

    /// The line's text (tests).
    pub fn text(&self) -> String {
        self.segs.iter().map(|s| s.text.as_str()).collect()
    }
}

/// One row of an archive listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArchiveRowVm {
    pub name: String,
    pub depth: u8,
    pub dir: bool,
    pub size: String,
}

/// One picture of the body (a texture made through the terminal UI's uploader).
#[derive(Clone, Debug)]
pub struct Pic {
    pub tex: Tex,
    /// A line above the picture (font specimen sizes).
    pub caption: Option<String>,
}

impl PartialEq for Pic {
    fn eq(&self, other: &Pic) -> bool {
        self.tex.size == other.tex.size && self.caption == other.caption
    }
}

impl Eq for Pic {}

/// What the body shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Body {
    /// Nothing to preview.
    #[default]
    Nothing,
    /// A request is taking a while.
    Loading,
    /// Lines (text, code, Markdown, hex, folder summary, diff, details): `lines` are the
    /// ones in view, from line `top` of `total`; `gutter` digits of line numbers (0: none).
    Lines { lines: Vec<PLine>, top: usize, total: usize, gutter: usize },
    /// An archive listing: `rows` in view (the first is entry `list.top`).
    Archive { header: String, sub: String, rows: Vec<ArchiveRowVm>, list: SelectList, total: usize },
    /// A message.
    Message { title: String, body: String },
    /// Pictures stacked top to bottom, `lines` under them (title, summary).
    Picture { pics: Vec<Pic>, lines: Vec<PLine> },
    /// The media player (Phase V3b): the latest frame (else the poster or cover) with
    /// `lines` under it, and the status row at the bottom; `serial` counts the frames.
    Player { status: StatusVm, pics: Vec<Pic>, lines: Vec<PLine>, serial: u64 },
}

/// The preview pane as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviewVm {
    /// The pane is drawn.
    pub visible: bool,
    /// It has the keyboard.
    pub focused: bool,
    /// The file's name (or the diff's two names).
    pub title: String,
    /// The icon's file class (`None`: no icon).
    pub class: Option<FileClass>,
    /// Its kind on the right of the header (`Text`, `Diff · +3 −1`, …).
    pub kind: String,
    /// The hex view is forced on.
    pub hex: bool,
    pub body: Body,
    /// The info section: translated label, value.
    pub info: Vec<(String, String)>,
    /// The content is text-like (it gets most of the height; `parts`).
    pub textual: bool,
}

impl PreviewVm {
    pub fn from_core(core: &AppCore, input: &InputState) -> PreviewVm {
        let visible = shown(core);
        if !visible {
            return PreviewVm::default();
        }
        PreviewVm { visible, focused: input.preview.focused, ..input.preview.vm.clone() }
    }

    /// Every body line in view as text (tests).
    pub fn body_text(&self) -> Vec<String> {
        match &self.body {
            Body::Lines { lines, .. } => lines.iter().map(PLine::text).collect(),
            Body::Archive { header, sub, rows, .. } => {
                [header.clone(), sub.clone()].into_iter().chain(rows.iter().map(|r| r.name.clone())).collect()
            }
            Body::Message { title, body } => vec![title.clone(), body.clone()],
            Body::Picture { pics, lines } => {
                pics.iter().filter_map(|p| p.caption.clone()).chain(lines.iter().map(PLine::text)).collect()
            }
            Body::Player { status, lines, .. } => {
                lines.iter().map(PLine::text).chain(std::iter::once(status.words())).collect()
            }
            Body::Loading => vec![tr!("tui-preview-loading").to_string()],
            Body::Nothing => vec![tr!("views-preview-nothing-selected").to_string()],
        }
    }
}

/// The pane's parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Parts {
    /// Inside the border.
    pub inner: Rect,
    pub header: Rect,
    pub body: Rect,
    /// The separator above the info rows.
    pub rule: Rect,
    pub info: Rect,
}

/// Split the pane `area` (`textual`: the body is text-like and gets most of the room).
pub fn parts(area: Rect, borders: TuiBorders, info_rows: usize, textual: bool, info_only: bool) -> Parts {
    let t = crate::borders::thickness(borders);
    let inner = if area.width > 2 * t && area.height > 2 * t {
        Rect::new(area.x + t, area.y + t, area.width - 2 * t, area.height - 2 * t)
    } else {
        Rect::new(area.x, area.y, 0, 0)
    };
    let mut p = Parts { inner, ..Parts::default() };
    if inner.is_empty() {
        return p;
    }
    p.header = Rect { height: 1, ..inner };
    let rest = Rect { y: inner.y + 1, height: inner.height - 1, ..inner };
    p.body = rest;
    p.rule = Rect { y: rest.bottom(), height: 0, ..rest };
    p.info = p.rule;
    if info_only || info_rows == 0 || rest.height < 4 {
        return p;
    }
    let share = if textual { rest.height / 3 } else { rest.height / 2 };
    let rows = (info_rows as u16).min(share.saturating_sub(1)).min(if textual { 8 } else { u16::MAX });
    if rows == 0 {
        return p;
    }
    p.body.height = rest.height - rows - 1;
    p.rule = Rect { y: p.body.bottom(), height: 1, ..rest };
    p.info = Rect { y: p.rule.bottom(), height: rows, ..rest };
    p
}

fn textual(loaded: Option<&Loaded>) -> bool {
    match loaded.map(|l| &l.content) {
        Some(Content::Text(_) | Content::Markdown { .. } | Content::Hex(_) | Content::Archive(_)) => true,
        Some(Content::Rich(r)) => r.is_textual(),
        _ => false,
    }
}

/// Step 1 of a frame (`view::prepare`): follow the selection, drain the loader and build
/// the model. Lets go of everything while the pane is hidden or has no room.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    let hex_toggle = std::mem::take(&mut core.ui.preview.hex_toggle);
    let st = &mut input.preview;
    if !shown(core) || regions.preview.is_empty() {
        if hex_toggle && let Some(l) = &mut st.loader {
            l.toggle_hex();
        }
        st.release();
        if !shown(core) {
            st.focused = false;
        }
        return;
    }
    let dark = TuiTheme::cached(core, false).is_dark();
    let loader = st.loader.get_or_insert_with(|| {
        let mut l = PreviewLoader::new(core.wake.clone(), core.uploader.clone(), false, 1.0);
        // Only pixels reach the terminal: SVG and 3D models come rasterized.
        l.terminal = true;
        l
    });
    if hex_toggle {
        loader.toggle_hex();
    }
    let limits = loader.limits(&core.config, dark, PLAIN_FG);
    let pair = diff_pair(core);
    let target = target_entry(core);
    let mut moved = loader.set_target(target.as_ref(), limits);
    loader.poll();
    match &pair {
        Some((a, b)) => {
            let key = Some((a.path.clone(), b.path.clone()));
            moved |= st.diff_key != key;
            st.diff_key = key;
            let cap = core.config.preview.max_diff_kib.max(4) * 1024;
            st.diff.sync(a, b, cap, &core.wake);
        }
        None => {
            if st.diff_key.take().is_some() {
                st.diff.clear();
                moved = true;
            }
        }
    }
    if moved {
        st.scroll = 0;
        st.archive = SelectList::new();
    }
    let info_only = core.ui.preview.info_only;
    let borders = crate::borders::from_core(core);
    let loader = st.loader.as_mut().expect("loader");
    let info: Vec<(String, String)> = match &loader.entry {
        Some(e) if pair.is_none() || info_only => {
            info_rows(e, loader.loaded.as_ref(), loader.folder_size(), &core.config)
                .into_iter()
                .map(|(k, v)| (safe(info_label(&k)), safe(&v)))
                .collect()
        }
        _ => Vec::new(),
    };
    let is_textual = textual(loader.loaded.as_ref());
    let p = parts(regions.preview, borders, info.len(), is_textual, info_only);
    let (w, h) = (usize::from(p.body.width), usize::from(p.body.height));
    st.page = h;
    let mut vm = PreviewVm { hex: loader.force_hex, textual: is_textual, ..PreviewVm::default() };
    match (&pair, &loader.entry) {
        (Some((a, b)), _) if !info_only => {
            vm.title = safe(&format!("{} ↔ {}", a.name, b.name));
            vm.class = Some(classify(b));
            vm.kind = tr!("views-preview-diff").to_string();
        }
        (_, Some(e)) => {
            vm.title = safe(&e.name);
            vm.class = Some(classify(e));
            vm.kind = loader.loaded.as_ref().map(|l| kind_label(l.kind).to_string()).unwrap_or_default();
        }
        _ => vm.title = tr!("views-preview-title").to_string(),
    }
    if vm.title.is_empty() {
        vm.title = tr!("views-preview-title").to_string();
    }
    vm.body = if info_only {
        let lines: Vec<PLine> = info_lines(&info, w);
        window(lines, &mut st.scroll, h, 0)
    } else if pair.is_some() {
        diff_body(st, &mut vm.kind, w, h)
    } else {
        content_body(loader, &mut st.scroll, &mut st.archive, w, h)
    };
    if !info_only {
        vm.info = info;
    }
    // The player (Phase V3b): asked for the file shown; its body while it is open.
    let loader = input.preview.loader.as_ref().expect("loader");
    let want = if info_only || pair.is_some() { None } else { Want::of(loader, longest_edge_px(p.body)) };
    if let Some(want) = &want
        && player::shows(core, &input.player, &want.key)
    {
        vm.body = player_body(core, &input.player, loader, want);
    }
    input.player.ask.preview = want;
    input.preview.vm = vm;
}

/// The longest edge of `r` in pixels (the frames the player is asked for).
pub(crate) fn longest_edge_px(r: Rect) -> u32 {
    let (cw, ch) = images::cell_size();
    (u32::from(r.width) * u32::from(cw)).max(u32::from(r.height) * u32::from(ch))
}

/// The body while the player is open (or opening, or failed) for `want`: the latest frame,
/// else the poster frame or cover from the loaded preview; title lines for audio; the
/// status row.
pub(crate) fn player_body(core: &AppCore, st: &player::State, loader: &PreviewLoader, want: &Want) -> Body {
    let (Some(entry), Some(loaded)) = (loader.entry.as_ref(), loader.loaded.as_ref()) else { return Body::Loading };
    let (status, frame) = match core.ui.player.state() {
        Some(s) => (StatusVm::from_state(&s, core.ui.player.loop_enabled()), st.frame()),
        None => match st.error(&want.key) {
            Some(e) => (StatusVm::failed(e, want.video), None),
            None => (StatusVm::opening(want.video), None),
        },
    };
    let still = match &loaded.content {
        Content::Rich(Rich::Video { poster, .. }) => poster.clone(),
        Content::Rich(Rich::Audio { cover, .. }) => cover.clone(),
        _ => None,
    };
    let (tex, serial) = match frame.filter(|_| want.video) {
        Some(f) => (Some(f.tex), f.serial),
        None => (still, 0),
    };
    let pics = tex.into_iter().map(|tex| Pic { tex, caption: None }).collect();
    let lines = if want.video { Vec::new() } else { media_lines(loaded, entry) };
    Body::Player { status, pics, lines, serial }
}

/// The lines of `all` in view from `*scroll` (clamped), `h` rows.
fn window(all: Vec<PLine>, scroll: &mut usize, h: usize, gutter: usize) -> Body {
    let total = all.len();
    *scroll = (*scroll).min(total.saturating_sub(h));
    let lines = all.into_iter().skip(*scroll).take(h).collect();
    Body::Lines { lines, top: *scroll, total, gutter }
}

fn info_lines(info: &[(String, String)], w: usize) -> Vec<PLine> {
    let label_w = info.iter().map(|(k, _)| text::width(k)).max().unwrap_or(0).min(w / 2);
    let mut out = Vec::new();
    for (k, v) in info {
        let label = text::fit(k, label_w);
        let value_w = w.saturating_sub(label_w + 1).max(1);
        for (i, part) in text::wrap(v, value_w).into_iter().enumerate() {
            let left = if i == 0 { label.clone() } else { " ".repeat(label_w) };
            out.push(PLine { segs: vec![Seg::new(format!("{left} "), Role::Muted), Seg::new(part, Role::Text)] });
        }
    }
    out
}

fn diff_body(st: &mut crate::input::preview::State, kind: &mut String, w: usize, h: usize) -> Body {
    let m = match &st.diff.result {
        None => return Body::Loading,
        Some(Err(e)) => return Body::Message { title: tr!("views-diff-failed").into(), body: safe(e) },
        Some(Ok(m)) => m.clone(),
    };
    *kind = format!("{} · +{} −{}", tr!("views-preview-diff"), m.added, m.removed);
    if m.binary {
        let t = if m.identical { tr!("views-diff-identical") } else { tr!("views-diff-binary-differ") };
        return Body::Message { title: t.into(), body: tr!("views-diff-binary-sub").into() };
    }
    if m.identical {
        return Body::Message { title: tr!("views-diff-none").into(), body: tr!("views-diff-none-sub").into() };
    }
    let total = m.unified.len() + usize::from(m.truncated);
    st.scroll = st.scroll.min(total.saturating_sub(h));
    let mut lines = Vec::new();
    for i in st.scroll..(st.scroll + h).min(total) {
        let Some(row) = m.unified.get(i) else {
            lines.push(PLine::plain(tr!("views-diff-truncated"), Role::Faint));
            continue;
        };
        lines.push(match row {
            DiffRow::Hunk(hunk) => PLine::plain(safe(hunk), Role::Accent),
            DiffRow::Line(l) => {
                let (mark, role) = match l.tag {
                    Tag::Equal => (" ", Role::Text),
                    Tag::Delete => ("-", Role::Danger),
                    Tag::Insert => ("+", Role::Success),
                };
                let mut segs = vec![Seg::new(mark, role)];
                for (em, t) in &l.segs {
                    let mut s = Seg::new(safe(t), role);
                    s.bold = *em && l.tag != Tag::Equal;
                    s.underline = s.bold;
                    segs.push(s);
                }
                PLine { segs }
            }
        });
    }
    let _ = w;
    Body::Lines { lines, top: st.scroll, total, gutter: 0 }
}

pub(crate) fn content_body(
    loader: &mut PreviewLoader,
    scroll: &mut usize,
    archive: &mut SelectList,
    w: usize,
    h: usize,
) -> Body {
    if loader.entry.is_none() {
        return Body::Nothing;
    }
    if matches!(loader.loaded.as_ref().map(|l| &l.content), Some(Content::Hex(_))) {
        return hex_body(loader, scroll, w, h);
    }
    let folder_size = loader.folder_size();
    let (Some(entry), Some(loaded)) = (loader.entry.as_ref(), loader.loaded.as_ref()) else {
        let slow = loader.loading_since.is_some_and(|t| t.elapsed().as_millis() > LOADING_AFTER_MS);
        return if slow || !loader.is_loading() { Body::Loading } else { Body::empty_lines() };
    };
    match &loaded.content {
        Content::Text(t) => {
            let total = t.lines.len() + usize::from(t.truncated);
            *scroll = (*scroll).min(total.saturating_sub(h));
            let mut lines = Vec::with_capacity(h);
            for i in *scroll..(*scroll + h).min(total) {
                lines.push(match t.lines.get(i) {
                    Some(l) => styled(l),
                    None => PLine::plain(tr!("views-preview-text-truncated"), Role::Faint),
                });
            }
            let digits = total.to_string().len().max(2);
            let gutter = if w >= digits + 8 { digits } else { 0 };
            Body::Lines { lines, top: *scroll, total, gutter }
        }
        Content::Markdown { blocks, truncated } => {
            let mut all = markdown(blocks, w.max(1));
            if *truncated {
                all.push(PLine::default());
                all.push(PLine::plain(tr!("views-preview-truncated"), Role::Faint));
            }
            window(all, scroll, h, 0)
        }
        Content::Hex(_) => Body::Loading,
        Content::Archive(l) => {
            let total = l.entries.len();
            let rows_h = h.saturating_sub(2);
            archive.clamp(total, rows_h);
            let rows = archive
                .visible(total, rows_h)
                .map(|i| {
                    let (path, dir, size) = &l.entries[i];
                    let p = path.trim_end_matches('/');
                    ArchiveRowVm {
                        name: safe(p.rsplit('/').next().unwrap_or(p)),
                        depth: p.matches('/').count().min(8) as u8,
                        dir: *dir,
                        size: if *dir { String::new() } else { format_size(*size, SizeStyle::Binary) },
                    }
                })
                .collect();
            Body::Archive {
                header: tr!("views-preview-archive-entries", count = l.total_entries),
                sub: tr!(
                    "views-preview-archive-unpacked",
                    format = safe(&l.format),
                    size = format_size(l.total_size, SizeStyle::Binary)
                ),
                rows,
                list: archive.clone(),
                total,
            }
        }
        Content::Folder(s) => window(folder_lines(s, folder_size), scroll, h, 0),
        Content::Media { .. } => window(media_lines(loaded, entry), scroll, h, 0),
        Content::Image(img) => {
            let summary = summary(loaded, &["Dimensions", "Format", "Triangles"]);
            picture(vec![Pic { tex: img.tex.clone(), caption: None }], summary.into_iter().collect())
        }
        // Not produced for the terminal (`PreviewLoader::terminal` rasterizes SVG).
        Content::Svg { .. } => Body::Message { title: kind_label(loaded.kind).into(), body: hex_hint() },
        Content::Rich(r) => rich_body(r, loaded, entry, scroll, w, h),
        Content::Message { title, body, .. } => {
            let mut body = safe(body);
            if matches!(loaded.kind, Kind::Pdf | Kind::Image) {
                body = if body.is_empty() { hex_hint() } else { format!("{body} {}", hex_hint()) };
            }
            Body::Message { title: safe(title), body }
        }
    }
}

/// A picture body (the model keeps the textures alive until the next frame).
fn picture(pics: Vec<Pic>, lines: Vec<PLine>) -> Body {
    Body::Picture { pics, lines }
}

/// The loader's fact `k`, made safe for the terminal.
fn fact(loaded: &Loaded, k: &str) -> Option<String> {
    loaded.facts.iter().find(|(fk, _)| fk == k).map(|(_, v)| safe(v)).filter(|v| !v.is_empty())
}

/// `Dimensions · Format …` from the facts `keys` that exist (faint).
fn summary(loaded: &Loaded, keys: &[&str]) -> Option<PLine> {
    let parts: Vec<String> = keys.iter().filter_map(|k| fact(loaded, k)).collect();
    (!parts.is_empty()).then(|| PLine::plain(parts.join(" · "), Role::Faint))
}

/// Title, artist and a summary line of an audio or video file.
fn media_lines(loaded: &Loaded, entry: &EntryInfo) -> Vec<PLine> {
    let mut lines = vec![PLine::plain(fact(loaded, "Title").unwrap_or_else(|| safe(&entry.name)), Role::Title)];
    if let Some(a) = fact(loaded, "Artist") {
        lines.push(PLine::plain(a, Role::Muted));
    }
    lines.extend(summary(loaded, &["Duration", "Dimensions", "Frame rate", "Video", "Audio", "Format", "Container"]));
    lines
}

/// The body of a rich preview: what the core rendered as pixels (PDF first page, video
/// poster, audio / e-book cover, font specimen); text or a message otherwise.
fn rich_body(r: &Rich, loaded: &Loaded, entry: &EntryInfo, scroll: &mut usize, w: usize, h: usize) -> Body {
    let rich_msg = || Body::Message {
        title: kind_label(loaded.kind).into(),
        body: tr!("tui-preview-rich", kind = kind_label(loaded.kind)),
    };
    match r {
        Rich::Pdf(v) if v.locked.is_some() => {
            Body::Message { title: tr!("views-rich-pdf-locked").into(), body: tr!("tui-preview-pdf-locked").into() }
        }
        Rich::Pdf(v) => {
            let pages = PLine::plain(tr!("tui-preview-pages", count = v.info.pages), Role::Faint);
            if let Some(first) = &v.first {
                let mut lines: Vec<PLine> = v.info.title.iter().map(|t| PLine::plain(safe(t), Role::Title)).collect();
                lines.push(pages);
                return picture(vec![Pic { tex: first.clone(), caption: None }], lines);
            }
            if v.info.text.trim().is_empty() {
                let why = v.no_render.as_deref().map(safe).unwrap_or_default();
                return Body::Message { title: kind_label(loaded.kind).into(), body: why };
            }
            // No page pictures (pdfium missing): the text of the first pages.
            let mut all = vec![pages];
            if let Some(why) = &v.no_render {
                all.push(PLine::plain(tr!("tui-preview-pdf-text-only", reason = safe(why)), Role::Faint));
            }
            all.push(PLine::default());
            for para in v.info.text.lines() {
                for l in text::wrap(&safe(para), w.max(1)) {
                    all.push(PLine::plain(l, Role::Text));
                }
            }
            window(all, scroll, h, 0)
        }
        // The facts carry what the libraries read (`info`), so the summary line and the
        // info section already show them.
        Rich::Video { poster, note, info: _ } => {
            let mut lines = media_lines(loaded, entry);
            match poster {
                Some(p) => picture(vec![Pic { tex: p.clone(), caption: None }], lines),
                None => {
                    if let Some(n) = note {
                        lines.push(PLine::plain(safe(n), Role::Faint));
                    }
                    window(lines, scroll, h, 0)
                }
            }
        }
        Rich::Audio { cover, .. } => {
            let lines = media_lines(loaded, entry);
            match cover {
                Some(c) => picture(vec![Pic { tex: c.clone(), caption: None }], lines),
                None => window(lines, scroll, h, 0),
            }
        }
        Rich::Epub { cover: Some(c), .. } => {
            let mut lines: Vec<PLine> =
                fact(loaded, "Title").map(|t| PLine::plain(t, Role::Title)).into_iter().collect();
            lines.extend(fact(loaded, "Author").map(|a| PLine::plain(a, Role::Muted)));
            picture(vec![Pic { tex: c.clone(), caption: None }], lines)
        }
        Rich::Font { strips, .. } if !strips.is_empty() => {
            let pics = strips.iter().map(|(cap, t)| Pic { tex: t.clone(), caption: Some(safe(cap)) }).collect();
            let lines = summary(loaded, &["Family", "Style", "Format"]).into_iter().collect();
            picture(pics, lines)
        }
        Rich::Office(_) | Rich::Epub { .. } | Rich::Font { .. } | Rich::Model { .. } => rich_msg(),
    }
}

fn hex_body(loader: &mut PreviewLoader, scroll: &mut usize, w: usize, h: usize) -> Body {
    let Some((doc, client)) = loader.hex_doc() else { return Body::Loading };
    let total = usize::try_from(preview_hex::row_count(doc.len)).unwrap_or(usize::MAX);
    *scroll = (*scroll).min(total.saturating_sub(h));
    let digits = preview_hex::offset_digits(doc.len);
    let ascii = w >= digits + 2 + 49 + 2 + 16;
    let mut lines = Vec::with_capacity(h);
    for row in *scroll..(*scroll + h).min(total) {
        let offset = row as u64 * preview_hex::BYTES_PER_ROW;
        let mut segs = vec![Seg::new(format!("{offset:0digits$X}"), Role::Faint)];
        match doc.row(row as u64, client) {
            Some(bytes) => {
                segs.push(Seg::new(format!("  {}", preview_hex::hex_part(&bytes)), Role::Text));
                if ascii {
                    segs.push(Seg::new(format!("  {}", preview_hex::ascii_part(&bytes)), Role::Muted));
                }
            }
            None => segs.push(Seg::new(format!("  {PENDING_ROW}"), Role::Faint)),
        }
        lines.push(PLine { segs });
    }
    doc.trim_rows(*scroll as u64, (*scroll + h) as u64);
    Body::Lines { lines, top: *scroll, total, gutter: 0 }
}

/// How to see the bytes of a file the preview cannot show.
fn hex_hint() -> String {
    let action = soyuz_appcore::palette::catalog::title(&soyuz_core::config::Action::ToggleHexPreview);
    tr!("tui-preview-hex-hint", action = action)
}

impl Body {
    fn empty_lines() -> Body {
        Body::Lines { lines: Vec::new(), top: 0, total: 0, gutter: 0 }
    }
}

fn folder_lines(
    s: &soyuz_appcore::services::preview::FolderSummary,
    size: Option<(u64, u64, u64, bool)>,
) -> Vec<PLine> {
    let mut out = Vec::new();
    if let Some(err) = &s.error {
        out.push(PLine::plain(tr!("views-preview-folder-unreadable"), Role::Title));
        out.push(PLine::plain(safe(err), Role::Danger));
        return out;
    }
    out.push(PLine::plain(tr!("views-preview-folder-counts", dirs = s.dirs, files = s.files), Role::Title));
    match size {
        Some((bytes, files, _, done)) => {
            let size = format_size(bytes, SizeStyle::Binary);
            let (t, role) = if done {
                (tr!("views-preview-folder-size", size = size, files = files), Role::Muted)
            } else {
                (tr!("views-preview-folder-size-running", size = size, files = files), Role::Accent)
            };
            out.push(PLine::plain(t, role));
        }
        None => out.push(PLine::plain(
            tr!("views-preview-folder-direct", size = format_size(s.direct_bytes, SizeStyle::Binary)),
            Role::Muted,
        )),
    }
    if s.hidden > 0 {
        out.push(PLine::plain(tr!("views-preview-folder-hidden", count = s.hidden), Role::Faint));
    }
    if !s.top_ext.is_empty() {
        let chips: Vec<String> = s.top_ext.iter().map(|(e, n)| format!(".{} {n}", safe(e))).collect();
        out.push(PLine::plain(chips.join("  "), Role::Muted));
    }
    if let Some((name, t)) = &s.newest {
        let rel = format_time(*t, &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Relative));
        out.push(PLine::plain(tr!("views-preview-folder-newest", name = safe(name), when = rel), Role::Faint));
    }
    out
}

/// A highlighted line as segments: plain runs in the theme's text colour, the others in
/// their syntax colour.
fn styled(l: &StyledLine) -> PLine {
    let mut segs = Vec::new();
    if l.runs.is_empty() && !l.text.is_empty() {
        segs.push(Seg::new(safe(&l.text), Role::Text));
    }
    for r in &l.runs {
        let Some(t) = l.text.get(r.range.clone()) else { continue };
        let c = r.style.color;
        segs.push(Seg {
            text: safe(t),
            color: (c != PLAIN_FG).then_some([c[0], c[1], c[2]]),
            role: (c == PLAIN_FG).then_some(Role::Text),
            bold: r.style.bold,
            italic: r.style.italic,
            underline: r.style.underline,
        });
    }
    PLine { segs }
}

// ---- Markdown -----------------------------------------------------------------------

fn md_segs(spans: &[MdSpan], base: Role) -> Vec<Seg> {
    spans
        .iter()
        .map(|s| {
            let role = if s.code {
                Role::Code
            } else if s.link {
                Role::Accent
            } else {
                base
            };
            Seg {
                text: safe(&s.text),
                role: Some(role),
                bold: s.bold,
                italic: s.italic,
                underline: s.link,
                ..Seg::default()
            }
        })
        .collect()
}

/// `segs` wrapped into lines of `w` cells: the first line starts with `first`, the
/// others with `indent` blanks.
fn wrap_segs(segs: &[Seg], w: usize, first: Vec<Seg>, indent: usize) -> Vec<PLine> {
    let mut out = Vec::new();
    let lead_w = first.iter().map(|s| text::width(&s.text)).sum::<usize>();
    let mut cur = PLine { segs: first };
    let mut cur_w = lead_w;
    let mut start_w = lead_w;
    let new_line = |out: &mut Vec<PLine>, cur: &mut PLine| {
        out.push(std::mem::take(cur));
        cur.segs.push(Seg::new(" ".repeat(indent), Role::Text));
    };
    for s in segs {
        for (i, word) in s.text.split(' ').enumerate() {
            let piece_sep = i > 0;
            if piece_sep {
                if cur_w > start_w && cur_w < w {
                    push_text(&mut cur, s, " ");
                    cur_w += 1;
                } else if cur_w >= w {
                    new_line(&mut out, &mut cur);
                    cur_w = indent;
                    start_w = indent;
                }
            }
            if word.is_empty() {
                continue;
            }
            let mut word = word.to_string();
            loop {
                let ww = text::width(&word);
                if cur_w + ww <= w {
                    push_text(&mut cur, s, &word);
                    cur_w += ww;
                    break;
                }
                if cur_w > start_w {
                    // Drop the trailing space we may have put and start a new line.
                    if let Some(last) = cur.segs.last_mut()
                        && last.text.ends_with(' ')
                    {
                        last.text.pop();
                    }
                    new_line(&mut out, &mut cur);
                    cur_w = indent;
                    start_w = indent;
                    continue;
                }
                // Longer than a whole line: split it.
                let room = w.saturating_sub(cur_w).max(1);
                let (head, tail) = split_at_width(&word, room);
                push_text(&mut cur, s, &head);
                new_line(&mut out, &mut cur);
                cur_w = indent;
                start_w = indent;
                if tail.is_empty() {
                    break;
                }
                word = tail;
            }
        }
    }
    out.push(cur);
    out
}

fn push_text(line: &mut PLine, style: &Seg, t: &str) {
    match line.segs.last_mut() {
        Some(last)
            if last.role == style.role
                && last.color == style.color
                && last.bold == style.bold
                && last.italic == style.italic
                && last.underline == style.underline =>
        {
            last.text.push_str(t)
        }
        _ => line.segs.push(Seg { text: t.to_string(), ..style.clone() }),
    }
}

fn split_at_width(s: &str, w: usize) -> (String, String) {
    use unicode_segmentation::UnicodeSegmentation;
    let mut used = 0;
    let mut cut = s.len();
    for (i, g) in s.grapheme_indices(true) {
        let gw = text::width(g);
        if used + gw > w && i > 0 {
            cut = i;
            break;
        }
        used += gw;
    }
    (s[..cut].to_string(), s[cut..].to_string())
}

/// Markdown blocks as lines of `w` cells.
pub fn markdown(blocks: &[Block], w: usize) -> Vec<PLine> {
    let mut out: Vec<PLine> = Vec::new();
    for (n, b) in blocks.iter().enumerate() {
        let gap = n > 0
            && !matches!(
                (b, &blocks[n - 1]),
                (
                    Block::Bullet(..) | Block::Numbered(..) | Block::Task(..),
                    Block::Bullet(..) | Block::Numbered(..) | Block::Task(..)
                )
            );
        if gap {
            out.push(PLine::default());
        }
        match b {
            Block::Heading(level, spans) => {
                let role = if *level <= 2 { Role::Accent } else { Role::Title };
                let mut segs = md_segs(spans, role);
                for s in &mut segs {
                    s.bold = true;
                }
                let first = if *level <= 1 {
                    Vec::new()
                } else {
                    vec![Seg::new("#".repeat(usize::from(*level)) + " ", Role::Faint)]
                };
                out.extend(wrap_segs(&segs, w, first, 0));
                if *level <= 2 {
                    out.push(PLine::plain("─".repeat(w.min(60)), Role::Faint));
                }
            }
            Block::Para(spans) => out.extend(wrap_segs(&md_segs(spans, Role::Text), w, Vec::new(), 0)),
            Block::Bullet(depth, spans) | Block::Numbered(depth, _, spans) | Block::Task(depth, _, spans) => {
                let marker = match b {
                    Block::Numbered(_, label, _) => format!("{} ", safe(label)),
                    Block::Task(_, true, _) => "[x] ".into(),
                    Block::Task(_, false, _) => "[ ] ".into(),
                    _ => if *depth == 0 { "• " } else { "◦ " }.into(),
                };
                let pad = 2 * usize::from(*depth);
                let indent = pad + text::width(&marker);
                let first = vec![Seg::new(" ".repeat(pad), Role::Text), Seg::new(marker, Role::Muted)];
                out.extend(wrap_segs(&md_segs(spans, Role::Text), w, first, indent));
            }
            Block::Quote(spans) => {
                let mut segs = md_segs(spans, Role::Muted);
                for s in &mut segs {
                    s.italic = true;
                }
                let lines = wrap_segs(&segs, w, vec![Seg::new("│ ", Role::Faint)], 2);
                for mut l in lines {
                    if let Some(first) = l.segs.first_mut()
                        && first.text == "  "
                    {
                        *first = Seg::new("│ ", Role::Faint);
                    }
                    out.push(l);
                }
            }
            Block::Code { lines, .. } => {
                for l in lines {
                    out.push(PLine { segs: vec![Seg::new("  ", Role::Text), Seg::new(safe(l), Role::Code)] });
                }
            }
            Block::Table(rows) => {
                for r in rows {
                    out.push(PLine::plain(safe(r), Role::Code));
                }
            }
            Block::Rule => out.push(PLine::plain("─".repeat(w.min(60)), Role::Faint)),
        }
    }
    out
}

// ---- drawing ------------------------------------------------------------------------

fn seg_style(s: &Seg, theme: &TuiTheme) -> Style {
    let mut st = theme.style(s.role.unwrap_or(Role::Text));
    if let Some([r, g, b]) = s.color {
        let c = to_color(Rgba8::rgb(r, g, b), theme.mode());
        if c != ratatui::style::Color::Reset {
            st = st.fg(c);
        }
    }
    if s.bold {
        st = st.add_modifier(Modifier::BOLD);
    }
    if s.italic {
        st = st.add_modifier(Modifier::ITALIC);
    }
    if s.underline {
        st = st.add_modifier(Modifier::UNDERLINED);
    }
    st
}

fn put_line(buf: &mut Buffer, x: u16, y: u16, w: u16, l: &PLine, theme: &TuiTheme) {
    let spans: Vec<RSpan<'_>> = l.segs.iter().map(|s| RSpan::styled(s.text.as_str(), seg_style(s, theme))).collect();
    buf.set_line(x, y, &Line::from(spans), w);
}

fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, w: u16, style: Style) {
    buf.set_stringn(x, y, s, usize::from(w), style);
}

/// Draw the pane into `area` (`Regions::preview`).
pub fn draw_preview(f: &mut Frame<'_>, area: Rect, m: &PreviewVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() || !m.visible {
        return;
    }
    hits.push(area, HitTarget::PreviewScroll);
    let borders = theme.borders();
    let p = parts(area, borders, m.info.len(), m.textual, false);
    let buf = f.buffer_mut();
    buf.set_style(area, theme.style(Role::Text));
    let edge = theme.style(if m.focused { Role::BorderFocused } else { Role::Border });
    f.render_widget(crate::borders::block(borders, edge), area);
    let buf = f.buffer_mut();
    if p.inner.is_empty() {
        return;
    }
    // Header: icon, name; kind (and hex) on the right.
    let h = p.header;
    let mut x = h.x;
    if let Some(c) = m.class {
        let g = crate::icons::class_glyph(c, theme.icons());
        let gw = text::width(g) as u16;
        if gw + 1 < h.width {
            put(buf, x, h.y, g, gw, theme.style(Role::of_class(c)));
            x += gw + 1;
        }
    }
    let mut kind = m.kind.clone();
    if m.hex {
        kind = if kind.is_empty() {
            tr!("tui-preview-hex").into()
        } else {
            format!("{kind} · {}", tr!("tui-preview-hex"))
        };
    }
    let room = usize::from(h.right() - x);
    let kind_w = text::width(&kind);
    let (title_w, kind_shown) =
        if kind_w > 0 && kind_w + 8 <= room { (room - kind_w - 1, true) } else { (room, false) };
    let title_role = if m.focused { Role::Accent } else { Role::Title };
    let title = text::truncate_name(&m.title, title_w, true);
    put(buf, x, h.y, &title, title_w as u16, theme.style(title_role).add_modifier(Modifier::BOLD));
    if kind_shown {
        put(buf, h.right() - kind_w as u16, h.y, &kind, kind_w as u16, theme.style(Role::Muted));
    }
    draw_body(buf, p.body, m, theme, hits);
    if !p.rule.is_empty() {
        let line = crate::borders::lines(borders).horizontal.repeat(usize::from(p.rule.width));
        put(buf, p.rule.x, p.rule.y, &line, p.rule.width, theme.style(Role::Border));
        let w = usize::from(p.info.width);
        for (l, y) in info_lines(&m.info, w).iter().zip(p.info.y..p.info.bottom()) {
            put_line(buf, p.info.x, y, p.info.width, l, theme);
        }
    }
}

pub(crate) fn draw_body(buf: &mut Buffer, r: Rect, m: &PreviewVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let center = |buf: &mut Buffer, y: u16, s: &str, role: Role| {
        let s = text::truncate_end(s, usize::from(r.width));
        let x = r.x + (r.width - text::width(&s) as u16) / 2;
        put(buf, x, y, &s, r.width, theme.style(role));
    };
    let mid = r.y + r.height.saturating_sub(1) / 2;
    match &m.body {
        Body::Nothing => {
            center(buf, mid, tr!("views-preview-nothing-selected"), Role::Muted);
            if mid + 1 < r.bottom() {
                center(buf, mid + 1, tr!("views-preview-nothing-selected-sub"), Role::Faint);
            }
        }
        Body::Loading => center(buf, mid, tr!("tui-preview-loading"), Role::Faint),
        Body::Message { title, body } => {
            center(buf, mid, title, Role::Title);
            for (l, y) in text::wrap(body, usize::from(r.width)).iter().zip(mid + 1..r.bottom()) {
                center(buf, y, l, Role::Muted);
            }
        }
        Body::Picture { pics, lines } => draw_pictures(buf, r, pics, lines, theme, false),
        Body::Player { status, pics, lines, .. } => draw_player(buf, r, status, pics, lines, theme, hits),
        Body::Lines { lines, top, gutter, .. } => {
            let gw = if *gutter > 0 && usize::from(r.width) > *gutter + 2 { *gutter as u16 + 1 } else { 0 };
            for (i, (l, y)) in lines.iter().zip(r.y..r.bottom()).enumerate() {
                if gw > 0 {
                    let no = format!("{:>width$}", top + i + 1, width = *gutter);
                    put(buf, r.x, y, &no, gw, theme.style(Role::Faint));
                }
                put_line(buf, r.x + gw, y, r.width - gw, l, theme);
            }
        }
        Body::Archive { header, sub, rows, list, .. } => {
            put(buf, r.x, r.y, &text::truncate_end(header, usize::from(r.width)), r.width, theme.style(Role::Title));
            if r.height > 1 {
                put(
                    buf,
                    r.x,
                    r.y + 1,
                    &text::truncate_end(sub, usize::from(r.width)),
                    r.width,
                    theme.style(Role::Muted),
                );
            }
            let lr = Rect { y: r.y + 2.min(r.height), height: r.height.saturating_sub(2), ..r };
            for (i, (row, y)) in rows.iter().zip(lr.y..lr.bottom()).enumerate() {
                let index = list.top + i;
                let rect = Rect::new(lr.x, y, lr.width, 1);
                let cursor = index == list.cursor;
                let mut base = theme.style(Role::Text);
                if cursor {
                    base = base.patch(theme.style(if m.focused { Role::Cursor } else { Role::CursorInactive }));
                }
                buf.set_style(rect, base);
                let icon = crate::icons::glyph(if row.dir { IconId::Folder } else { IconId::File }, theme.icons());
                let indent = " ".repeat(usize::from(row.depth) * 2);
                let size_w = text::width(&row.size);
                let w = usize::from(lr.width);
                let left_w = if size_w > 0 && size_w + 4 < w { w - size_w - 1 } else { w };
                let role = if row.dir { Role::Folder } else { Role::Text };
                let name_style = if cursor { base } else { theme.style(role) };
                let left = Line::from(vec![
                    RSpan::styled(indent, base),
                    RSpan::styled(format!("{icon} "), if cursor { base } else { theme.style(role) }),
                    RSpan::styled(row.name.as_str(), name_style),
                ]);
                buf.set_line(lr.x, y, &left, left_w as u16);
                if left_w < w {
                    let st = if cursor { base } else { theme.style(Role::Muted) };
                    put(buf, lr.x + (w - size_w) as u16, y, &row.size, size_w as u16, st);
                }
                hits.push(rect, HitTarget::ArchiveRow(index));
            }
        }
    }
}

/// What stands in for a picture that is not on the screen (`None`: it is).
pub fn status_text(d: Drawn) -> Option<&'static str> {
    match d {
        Drawn::Image | Drawn::Placeholder => None,
        Drawn::Pending => Some(tr!("tui-preview-image-pending")),
        Drawn::Off => Some(tr!("tui-preview-image-off")),
        Drawn::Missing => Some(tr!("tui-preview-image-missing")),
        Drawn::NoRoom => Some(tr!("tui-preview-image-no-room")),
    }
}

/// Pictures stacked in `r` (equal shares; a caption row above each when it has one and
/// there is room), `lines` at the bottom (as many as leave the pictures two rows).
/// `stream`: the pictures are video frames replacing one another
/// ([`ImageState::stream`]).
fn draw_pictures(buf: &mut Buffer, r: Rect, pics: &[Pic], lines: &[PLine], theme: &TuiTheme, stream: bool) {
    let n_lines = (lines.len() as u16).min(r.height.saturating_sub(2));
    let area = Rect { height: r.height - n_lines, ..r };
    for (l, y) in lines.iter().zip(area.bottom()..r.bottom()) {
        let w = text::width(&l.text()).min(usize::from(r.width)) as u16;
        put_line(buf, r.x + (r.width - w) / 2, y, w.max(1), l, theme);
    }
    if pics.is_empty() || area.is_empty() {
        return;
    }
    let n = (pics.len() as u16).min(area.height).max(1);
    let share = area.height / n;
    for (i, pic) in pics.iter().take(usize::from(n)).enumerate() {
        let i = i as u16;
        let h = if i + 1 == n { area.height - share * i } else { share };
        let mut slot = Rect { y: area.y + share * i, height: h, ..area };
        if let Some(c) = &pic.caption
            && slot.height >= 3
        {
            let c = text::truncate_end(c, usize::from(slot.width));
            put(buf, slot.x, slot.y, &c, slot.width, theme.style(Role::Muted));
            slot.y += 1;
            slot.height -= 1;
        }
        let mut st = ImageState { stream, ..ImageState::default() };
        // Screen-reader mode (T8): no graphics (no kitty / sixel requests), a line instead.
        let msg = if theme.screen_reader() {
            Some(tr!("tui-sr-picture"))
        } else {
            status_text(images::draw_in(buf, slot, &pic.tex, &mut st))
        };
        let Some(msg) = msg else { continue };
        // Graphics images blank their cells while they encode: say so under them.
        let y = match st.drawn {
            Some(d) if d.bottom() < slot.bottom() => d.bottom(),
            Some(_) => continue,
            None => slot.y + slot.height.saturating_sub(1) / 2,
        };
        if slot.is_empty() || y >= slot.bottom() {
            continue;
        }
        let s = text::truncate_end(msg, usize::from(slot.width));
        let x = slot.x + (slot.width - text::width(&s) as u16) / 2;
        put(buf, x, y, &s, slot.width, theme.style(Role::Faint));
    }
}

/// The player's body: the pictures and lines above, the status row at the bottom (words
/// only in screen-reader mode, which draws no pictures either). The mark and the bar are
/// click targets (`PlayerPlay`, `PlayerSeek`).
fn draw_player(
    buf: &mut Buffer,
    r: Rect,
    status: &StatusVm,
    pics: &[Pic],
    lines: &[PLine],
    theme: &TuiTheme,
    hits: &mut Hits,
) {
    let row = Rect { y: r.bottom() - 1, height: 1, ..r };
    let above = Rect { height: r.height - 1, ..r };
    if theme.screen_reader() {
        draw_pictures(buf, above, &[], lines, theme, false);
        let s = text::truncate_end(&status.words(), usize::from(row.width));
        put(buf, row.x, row.y, &s, row.width, theme.style(Role::Text));
        hits.push(row, HitTarget::PlayerPlay);
        return;
    }
    draw_pictures(buf, above, pics, lines, theme, true);
    let icons = theme.icons();
    let l = status.layout(usize::from(row.width), icons);
    let mark_w = text::width(&l.mark) as u16;
    let mark_role = match status.status {
        player::Status::Error(_) => Role::Danger,
        player::Status::Opening => Role::Muted,
        _ => Role::Accent,
    };
    put(buf, row.x, row.y, &l.mark, mark_w, theme.style(mark_role).add_modifier(Modifier::BOLD));
    hits.push(Rect { width: mark_w.min(row.width), ..row }, HitTarget::PlayerPlay);
    let times_role = if matches!(status.status, player::Status::Error(_)) { Role::Danger } else { Role::Text };
    put(buf, row.x + mark_w + 1, row.y, &l.times, row.width.saturating_sub(mark_w + 1), theme.style(times_role));
    if let Some((bx, bw)) = l.bar {
        let bar = player::bar(bw, status.fraction().unwrap_or(0.0), icons);
        let rect = Rect::new(row.x + bx as u16, row.y, bw as u16, 1);
        put(buf, rect.x, rect.y, &bar, rect.width, theme.style(Role::Accent));
        hits.push(rect, HitTarget::PlayerSeek);
    }
    let fw = text::width(&l.flags) as u16;
    if fw > 0 && fw <= row.width {
        put(buf, row.right() - fw, row.y, &l.flags, fw, theme.style(Role::Muted));
    }
}

/// The path to open for archive row `index` of `loader`'s listing: the archive with the
/// row's folder as the location and the row selected (a folder row opens itself).
pub fn archive_target(loader: &PreviewLoader, index: usize) -> Option<(PathBuf, String, Option<String>)> {
    let entry: &EntryInfo = loader.entry.as_ref()?;
    let Some(Content::Archive(l)) = loader.loaded.as_ref().map(|l| &l.content) else { return None };
    let (path, dir, _) = l.entries.get(index)?;
    let p = path.trim_matches('/');
    if *dir {
        return Some((entry.path.clone(), p.to_string(), None));
    }
    let (inner, name) = p.rsplit_once('/').unwrap_or(("", p));
    Some((entry.path.clone(), inner.to_string(), Some(name.to_string())))
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn draw(m: &PreviewVm, w: u16, h: u16) -> (Vec<String>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_preview(f, Rect::new(0, 0, w, h), m, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect();
        (lines, hits)
    }

    fn vm(body: Body) -> PreviewVm {
        PreviewVm {
            visible: true,
            title: "main.rs".into(),
            class: Some(FileClass::Code),
            kind: "Text".into(),
            body,
            info: vec![("Size".into(), "12 bytes".into()), ("Type".into(), "Rust".into())],
            ..PreviewVm::default()
        }
    }

    #[test]
    fn parts_split_and_shrink() {
        let p = parts(Rect::new(0, 0, 30, 20), TuiBorders::Rounded, 3, true, false);
        assert_eq!(p.inner, Rect::new(1, 1, 28, 18));
        assert_eq!(p.header, Rect::new(1, 1, 28, 1));
        assert_eq!(p.info.height, 3);
        assert_eq!(p.rule.bottom(), p.info.y);
        assert_eq!(p.body.bottom(), p.rule.y);
        let p = parts(Rect::new(0, 0, 30, 20), TuiBorders::None, 3, true, true);
        assert!(p.info.is_empty() && p.body.height == 19);
        assert!(parts(Rect::new(0, 0, 2, 2), TuiBorders::Plain, 3, true, false).inner.is_empty());
    }

    #[test]
    fn lines_with_gutter_and_info() {
        let body = Body::Lines {
            lines: vec![PLine::plain("fn main() {}", Role::Text), PLine::plain("", Role::Text)],
            top: 4,
            total: 10,
            gutter: 2,
        };
        let (lines, hits) = draw(&vm(body), 30, 12);
        assert!(lines[1].contains("main.rs") && lines[1].contains("Text"), "{lines:?}");
        assert!(lines[2].contains(" 5 fn main() {}"), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("Size") && l.contains("12 bytes")), "{lines:?}");
        assert_eq!(hits.at(5, 5), Some(HitTarget::PreviewScroll));
    }

    #[test]
    fn archive_rows_record_targets() {
        let body = Body::Archive {
            header: "2 entries".into(),
            sub: "zip".into(),
            rows: vec![
                ArchiveRowVm { name: "dir".into(), depth: 0, dir: true, size: String::new() },
                ArchiveRowVm { name: "a.txt".into(), depth: 1, dir: false, size: "1 KiB".into() },
            ],
            list: SelectList { cursor: 1, top: 0 },
            total: 2,
        };
        let (lines, hits) = draw(&vm(body), 30, 14);
        assert!(lines.iter().any(|l| l.contains("a.txt") && l.contains("1 KiB")), "{lines:?}");
        let y = lines.iter().position(|l| l.contains("a.txt")).unwrap() as u16;
        assert_eq!(hits.at(4, y), Some(HitTarget::ArchiveRow(1)));
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let bodies = [
            Body::Nothing,
            Body::Loading,
            Body::Message { title: "Too large".into(), body: "a long explanation of it".into() },
            Body::Lines { lines: vec![PLine::plain("x".repeat(50), Role::Text)], top: 0, total: 1, gutter: 3 },
            Body::Archive {
                header: "h".into(),
                sub: "s".into(),
                rows: vec![ArchiveRowVm { name: "n".into(), depth: 8, dir: false, size: "1 B".into() }],
                list: SelectList::default(),
                total: 1,
            },
        ];
        for b in bodies {
            let mut m = vm(b);
            m.hex = true;
            for w in 0..=20 {
                for h in 0..=6 {
                    draw(&m, w, h);
                }
            }
        }
    }

    fn red(up: &soyuz_appcore::services::preview_gfx::Uploader, w: usize, h: usize) -> Tex {
        let rgba: Vec<u8> = (0..w * h).flat_map(|_| [255, 0, 0, 255]).collect();
        let px = soyuz_appcore::services::preview_gfx::Pixels { size: [w, h], rgba: &rgba, premultiplied: false };
        Tex::upload(up, "red".into(), px)
    }

    #[test]
    fn pictures_draw_with_lines_and_status() {
        use soyuz_core::config::{Config, TuiImages};
        let mut c = Config::default();
        c.tui.images = TuiImages::Halfblocks;
        c.tui.color_mode = soyuz_core::config::TuiColorMode::Truecolor;
        let up = images::install(&c, soyuz_appcore::host::no_wake());
        images::begin_frame(&c);
        let pics = vec![
            Pic { tex: red(&up, 100, 60), caption: Some("12 pt".into()) },
            Pic { tex: red(&up, 100, 60), caption: None },
        ];
        let body = Body::Picture { pics, lines: vec![PLine::plain("100 × 60 px · PNG", Role::Faint)] };
        let m = PreviewVm { info: Vec::new(), ..vm(body.clone()) };
        let (lines, _) = draw(&m, 30, 20);
        let all = lines.join(
            "
",
        );
        assert!(all.contains("12 pt") && all.contains("100 × 60 px · PNG"), "{all}");
        assert!(all.contains('█') || all.contains('▀'), "{all}");
        assert!(m.body_text().iter().any(|l| l == "12 pt"));

        // Off: the text line in place of each picture.
        c.tui.images = TuiImages::Off;
        images::begin_frame(&c);
        let (lines, _) = draw(&m, 40, 20);
        let all = lines.join(
            "
",
        );
        assert!(all.contains(tr!("tui-preview-image-off")) && !all.contains('█'), "{all}");
        // A texture that is not the terminal's: missing.
        c.tui.images = TuiImages::Halfblocks;
        images::begin_frame(&c);
        let foreign = red(&soyuz_appcore::services::preview_gfx::no_upload(), 4, 4);
        let m2 = vm(Body::Picture { pics: vec![Pic { tex: foreign, caption: None }], lines: Vec::new() });
        let (lines, _) = draw(&m2, 40, 20);
        assert!(
            lines
                .join(
                    "
"
                )
                .contains(tr!("tui-preview-image-missing"))
        );
        assert_eq!(status_text(Drawn::Image), None);
        assert_eq!(status_text(Drawn::Placeholder), None);
        assert!(status_text(Drawn::NoRoom).is_some() && status_text(Drawn::Pending).is_some());
        // Tiny sizes never panic.
        for w in 0..=20 {
            for h in 0..=6 {
                draw(&m, w, h);
                draw(&m2, w, h);
            }
        }
    }

    /// The player's body: the status row at the bottom with its click targets, the lines
    /// above; words only in screen-reader mode.
    #[test]
    fn player_body_draws_the_status_row_with_targets() {
        use crate::player::{Status, StatusVm};
        let status = StatusVm {
            status: Status::Playing,
            pos_ds: 123,
            duration_ds: Some(900),
            volume: 80,
            muted: true,
            loop_: false,
            video: false,
        };
        let body = Body::Player {
            status: status.clone(),
            pics: Vec::new(),
            lines: vec![PLine::plain("Song title", Role::Title)],
            serial: 1,
        };
        let m = PreviewVm { info: Vec::new(), ..vm(body) };
        assert_eq!(m.body_text(), vec!["Song title".to_string(), status.words()]);
        let (lines, hits) = draw(&m, 40, 12);
        let y = lines.iter().position(|l| l.contains("0:12 / 1:30")).expect("the times") as u16;
        assert_eq!(y, 10, "the row above the bottom border");
        assert!(lines[y as usize].contains("│|| 0:12"), "the pause mark (ASCII): {lines:?}");
        assert!(lines[y as usize].contains("==") && lines[y as usize].ends_with("80% muted│"), "{lines:?}");
        assert!(lines[y as usize - 1].contains("Song title"), "{lines:?}");
        assert_eq!(hits.at(1, y), Some(HitTarget::PlayerPlay));
        let bar = hits.iter().find(|(_, t)| *t == HitTarget::PlayerSeek).map(|(r, _)| *r).expect("the bar");
        assert!(bar.y == y && bar.width >= 4 && hits.at(bar.x + 1, y) == Some(HitTarget::PlayerSeek));
        // Screen-reader mode: words, no bar.
        let mut t = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let mut sr_hits = Hits::default();
        let theme = TuiTheme::plain().with_screen_reader(true);
        t.draw(|f| draw_preview(f, Rect::new(0, 0, 60, 12), &m, &theme, &mut sr_hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let text: Vec<String> =
            (0..12).map(|y| (0..60).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect();
        assert!(text.iter().any(|l| l.contains("Playing, 0:12 of 1:30, volume 80 %, muted")), "{text:?}");
        assert!(!text.iter().any(|l| l.contains("==")), "{text:?}");
        assert!(sr_hits.iter().all(|(_, t)| *t != HitTarget::PlayerSeek));
        // Opening and failed states, tiny sizes.
        for st in [StatusVm::opening(true), StatusVm::failed("no codec", true)] {
            let m = vm(Body::Player { status: st, pics: Vec::new(), lines: Vec::new(), serial: 0 });
            let (lines, _) = draw(&m, 40, 10);
            assert!(lines.iter().any(|l| l.contains("Opening") || l.contains("no codec")), "{lines:?}");
            for w in 0..=20 {
                for h in 0..=6 {
                    draw(&m, w, h);
                }
            }
        }
    }

    #[test]
    fn markdown_renders_headings_lists_and_wraps() {
        let md = soyuz_appcore::services::preview_text::parse_markdown(
            "# Title\n\nSome **bold** text that wraps around.\n\n- one\n- two\n\n```\ncode\n```\n",
        );
        let lines = markdown(&md, 16);
        let text: Vec<String> = lines.iter().map(PLine::text).collect();
        assert_eq!(text[0], "Title");
        assert!(lines[0].segs.iter().all(|s| s.bold));
        assert!(text.iter().any(|t| t == "• one") && text.iter().any(|t| t == "• two"), "{text:?}");
        assert!(text.iter().any(|t| t.contains("code")));
        assert!(text.iter().all(|t| text::width(t) <= 16), "{text:?}");
        let bold = lines.iter().flat_map(|l| &l.segs).find(|s| s.text.contains("bold")).unwrap();
        assert!(bold.bold);
        // A word longer than the line is split.
        let long = soyuz_appcore::services::preview_text::parse_markdown(&"x".repeat(40));
        assert!(markdown(&long, 10).iter().all(|l| text::width(&l.text()) <= 10));
    }

    #[test]
    fn styled_lines_keep_syntax_colours() {
        use soyuz_appcore::services::preview_gfx::{RunStyle, TextFont};
        let mut l = StyledLine::default();
        l.append("fn", RunStyle::plain(TextFont::monospace(12.0), [200, 100, 50, 255]));
        l.append(" main\x1b[2J", RunStyle::plain(TextFont::monospace(12.0), PLAIN_FG));
        let p = styled(&l);
        assert_eq!(p.segs[0].color, Some([200, 100, 50]));
        assert_eq!(p.segs[1].color, None);
        assert!(!p.text().contains('\x1b'), "escape sequences never reach the terminal");
    }
}
