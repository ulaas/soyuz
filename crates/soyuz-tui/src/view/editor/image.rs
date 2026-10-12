//! The image editor's view ([`soyuz_appcore::editors::image::ImageDoc`]): the bars (crop
//! by numbers, resize, save as), the "saving re-encodes / cannot save" warning, the image
//! fitted into the panel through [`crate::images::draw`] (the edited display copy the core
//! builds on a worker; the previous copy while the next one is on its way), a key hint
//! and the status line (size, format, edits, save mode).
//!
//! The keys are the core's (`ImageDoc::key`, as in the GUI): R / Shift+R rotate, H / V
//! flip, C crop (here the crop bar: x, y, width, height in pixels, Enter crops), Ctrl+Z /
//! Ctrl+Y undo / redo; `EditorResizeImage` opens the resize bar, `EditorSave` /
//! `EditorSaveAs` save.

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::editors::image::ImageDoc;
use soyuz_appcore::editors::image::ops::SaveMode;
use soyuz_appcore::services::preview_gfx::Tex;
use soyuz_appcore::tr;

use super::{BarVm, Keys, bar, draw_bars, fill_row, put};
use crate::images::{Drawn, ImageState};
use crate::input::editor::{Bar, DocView, SaveAs};
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::text as fit;

/// The crop bar's fields: x, y, width, height (pixels of the image as edited).
pub const CROP_FIELDS: usize = 4;
/// The resize bar's fields: width, height.
pub const RESIZE_FIELDS: usize = 2;

/// A texture in the view model (compared by pixel size: the model is rebuilt every frame).
#[derive(Clone, Debug)]
pub struct TexRef(pub Tex);

impl PartialEq for TexRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.size == other.0.size
    }
}

impl Eq for TexRef {}

/// The image editor as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageVm {
    pub bars: Vec<BarVm>,
    /// Saving re-encodes (JPEG) or is not possible.
    pub warning: Option<String>,
    /// The edited display copy, and whether it shows the current edits.
    pub tex: Option<(TexRef, bool)>,
    /// The size after the edits.
    pub size: (u32, u32),
    /// The keys.
    pub hint: String,
    pub status: bool,
    pub status_left: String,
    pub status_right: String,
}

/// `fields[i]` as a pixel count.
fn px(s: &str) -> Option<u32> {
    s.trim().parse::<u32>().ok()
}

/// Fill the crop bar's fields for an image `size` (the whole image).
pub fn crop_fields(size: (u32, u32)) -> Vec<String> {
    vec!["0".into(), "0".into(), size.0.to_string(), size.1.to_string()]
}

/// The crop rectangle `(x, y, w, h)` the crop fields describe in an image of `size`
/// (clamped inside it), or `None` when a field is not a number or the area is empty.
pub fn parse_crop(fields: &[String], size: (u32, u32)) -> Option<(u32, u32, u32, u32)> {
    let [x, y, w, h] = [0, 1, 2, 3].map(|i| fields.get(i).and_then(|s| px(s)));
    let (x, y, w, h) = (x?, y?, w?, h?);
    let (iw, ih) = size;
    if x >= iw || y >= ih || w == 0 || h == 0 {
        return None;
    }
    Some((x, y, w.min(iw - x), h.min(ih - y)))
}

/// The size the resize fields ask for from an image of `size`: a width (or `50%`) and a
/// height; an empty field keeps the aspect ratio. `None` when nothing valid is asked.
pub fn parse_resize(fields: &[String], size: (u32, u32)) -> Option<(u32, u32)> {
    let (iw, ih) = (f64::from(size.0.max(1)), f64::from(size.1.max(1)));
    let dim = |s: &str, of: f64| -> Option<Option<f64>> {
        let s = s.trim();
        if s.is_empty() {
            return Some(None);
        }
        let v = match s.strip_suffix('%') {
            Some(p) => p.trim().parse::<f64>().ok()? / 100.0 * of,
            None => s.parse::<u32>().ok()? as f64,
        };
        (v.is_finite() && v >= 0.5).then_some(Some(v))
    };
    let w = dim(fields.first().map_or("", String::as_str), iw)?;
    let h = dim(fields.get(1).map_or("", String::as_str), ih)?;
    let (w, h) = match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * ih / iw),
        (None, Some(h)) => (h * iw / ih, h),
        (None, None) => return None,
    };
    let (w, h) = (w.round().max(1.0), h.round().max(1.0));
    (w <= 65_535.0 && h <= 65_535.0).then_some((w as u32, h as u32))
}

/// The geometry and model of one frame. `quality`: `[editor] jpeg_quality`.
pub fn prepare(
    i: &mut ImageDoc,
    v: &mut DocView,
    save_as: Option<&mut SaveAs>,
    area: Rect,
    focused: bool,
    keys: &Keys,
    quality: u8,
) -> ImageVm {
    i.pump();
    let size = i.size();
    let mut vm = ImageVm { size, ..ImageVm::default() };
    // The crop bar follows the core's crop mode (C, Esc).
    if i.crop_mode && v.bar != Bar::Crop {
        v.fields = crop_fields(size);
        v.field = 2;
        v.field_edits = v.fields.iter().map(|s| crate::widgets::LineEdit::at_end(s)).collect();
        v.field_error = None;
        v.bar = Bar::Crop;
    } else if !i.crop_mode && v.bar == Bar::Crop {
        v.bar = Bar::Text;
    }
    if matches!(v.bar, Bar::Find | Bar::Replace | Bar::Goto) || v.bar == Bar::SaveAs && save_as.is_none() {
        v.bar = Bar::Text;
    }
    let w = area.width;
    let fields = match v.bar {
        Bar::Crop => Some((
            [tr!("tui-editor-crop-x"), tr!("tui-editor-crop-y"), tr!("tui-editor-width"), tr!("tui-editor-height")]
                .to_vec(),
            tr!("tui-editor-crop-keys"),
        )),
        Bar::Resize => {
            Some(([tr!("tui-editor-width"), tr!("tui-editor-height")].to_vec(), tr!("tui-editor-resize-keys")))
        }
        _ => None,
    };
    if let Some((labels, keys_note)) = fields {
        v.field_edits.resize_with(v.fields.len(), crate::widgets::LineEdit::new);
        let n = labels.len().min(v.fields.len());
        for (k, label) in labels.iter().enumerate().take(n) {
            let last = k + 1 == n;
            let (note, error) = match (&v.field_error, last) {
                (Some(e), true) => (e.clone(), true),
                (None, true) => (keys_note.to_string(), false),
                _ => (String::new(), false),
            };
            let text = v.fields[k].clone();
            let fo = focused && v.field == k;
            vm.bars.push(bar(label.to_string(), &text, &mut v.field_edits[k], w, 24, fo, Vec::new(), note, error));
        }
    }
    if let Some(s) = save_as {
        let fo = focused && v.bar == Bar::SaveAs;
        let (note, error) = match &s.error {
            Some(e) => (e.clone(), true),
            None if s.busy => (tr!("tui-editor-saving").into(), false),
            None => (String::new(), false),
        };
        let text = s.text.clone();
        vm.bars.push(bar(tr!("tui-editor-save-as").into(), &text, &mut s.edit, w, 0, fo, Vec::new(), note, error));
    }
    vm.warning = match i.save_mode() {
        SaveMode::Lossy(_) if i.is_dirty() => {
            Some(tr!("editors-image-lossy-warning", quality = quality.to_string()).to_string())
        }
        SaveMode::Unsupported if i.is_dirty() => Some(tr!("editors-image-unsupported-warning").into()),
        _ => None,
    };
    vm.tex = i.texture().map(|(t, current)| (TexRef(t.clone()), current));
    vm.hint = tr!("tui-editor-image-keys", resize = keys.resize.as_str(), save = keys.save.as_str());
    let head = vm.bars.len() as u16 + u16::from(vm.warning.is_some());
    vm.status = area.height >= head + 3;
    let fmt = i.format.map(|f| format!("{f:?}").to_uppercase()).unwrap_or_else(|| tr!("editors-image-kind").into());
    // The terminal fits the image: no zoom in the status, unlike the GUI's.
    let mut s = tr!("tui-editor-image-status", width = size.0.to_string(), height = size.1.to_string(), format = fmt);
    if !i.ops.is_empty() {
        s.push_str("  ·  ");
        s.push_str(&tr!("editors-image-status-edits", count = i.ops.len()));
    }
    vm.status_left = s;
    vm.status_right = match i.save_mode() {
        SaveMode::Lossless(_) => tr!("editors-image-mode-lossless"),
        SaveMode::Lossy(_) => tr!("editors-image-mode-lossy"),
        SaveMode::Unsupported => tr!("editors-image-mode-readonly"),
    }
    .to_string();
    vm
}

/// Draw the image editor into `area` (the panel's body).
pub fn draw(f: &mut Frame<'_>, area: Rect, m: &ImageVm, doc: u64, theme: &TuiTheme, hits: &mut Hits) {
    let mut cursor = None;
    let mut y = draw_bars(f.buffer_mut(), area, &m.bars, doc, theme, hits, &mut cursor);
    if let Some(w) = &m.warning
        && y < area.bottom()
    {
        put(f.buffer_mut(), area, area.x + 1, y, w, usize::from(area.width), theme.style(Role::Warning));
        y += 1;
    }
    let mut bottom = if m.status { area.bottom().saturating_sub(1) } else { area.bottom() };
    // The key hint above the status line, when there is room.
    let hint_row = (bottom > y + 2).then(|| bottom - 1);
    if let Some(hy) = hint_row {
        put(
            f.buffer_mut(),
            area,
            area.x + 1,
            hy,
            &m.hint,
            usize::from(area.width.saturating_sub(1)),
            theme.style(Role::Faint),
        );
        bottom = hy;
    }
    let pic = Rect::new(area.x, y, area.width, bottom.saturating_sub(y));
    if !pic.is_empty() {
        hits.push(pic, HitTarget::EditorCell { doc, top: 0, left: 0 });
        let placeholder = |f: &mut Frame<'_>, text: &str| {
            let s = fit::truncate_end(text, usize::from(pic.width));
            let w = fit::width(&s) as u16;
            let x = pic.x + pic.width.saturating_sub(w) / 2;
            put(f.buffer_mut(), pic, x, pic.y + pic.height / 2, &s, usize::from(pic.width), theme.style(Role::Muted));
        };
        match &m.tex {
            Some((t, _)) => {
                let mut state = ImageState { upscale: true, ..ImageState::default() };
                match crate::images::draw(f, pic, &t.0, &mut state) {
                    Drawn::Off => placeholder(f, &tr!("tui-editor-image-off", width = m.size.0, height = m.size.1)),
                    Drawn::Missing => placeholder(f, "…"),
                    _ => {}
                }
            }
            None => placeholder(f, "…"),
        }
    }
    if m.status && area.height > 0 {
        let buf = f.buffer_mut();
        let sy = area.bottom() - 1;
        fill_row(buf, area, area.x, sy, theme.style(Role::StatusBar));
        let rw = fit::width(&m.status_right) as u16;
        let lw = area.width.saturating_sub(rw + 3);
        let left = fit::truncate_end(&m.status_left, usize::from(lw));
        put(buf, area, area.x + 1, sy, &left, usize::from(lw), theme.style(Role::StatusBar));
        if area.width > rw + 1 {
            put(buf, area, area.right() - rw - 1, sy, &m.status_right, usize::from(rw), theme.style(Role::StatusBar));
        }
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position(ratatui::layout::Position { x, y });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_vm() -> ImageVm {
        ImageVm {
            bars: vec![BarVm {
                label: "Width".into(),
                field: "640".into(),
                field_w: 6,
                cursor: Some(3),
                ..BarVm::default()
            }],
            warning: Some("Saving re-encodes".into()),
            size: (640, 480),
            hint: "R: rotate".into(),
            status: true,
            status_left: "640 × 480".into(),
            status_right: "Lossless".into(),
            ..ImageVm::default()
        }
    }

    #[test]
    fn crop_fields_parse_and_clamp() {
        let f = |v: [&str; 4]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_crop(&crop_fields((10, 8)), (10, 8)), Some((0, 0, 10, 8)));
        assert_eq!(parse_crop(&f(["2", "3", "100", "100"]), (10, 8)), Some((2, 3, 8, 5)), "clamped");
        assert_eq!(parse_crop(&f(["10", "0", "1", "1"]), (10, 8)), None, "outside");
        assert_eq!(parse_crop(&f(["0", "0", "0", "1"]), (10, 8)), None, "empty");
        assert_eq!(parse_crop(&f(["a", "0", "1", "1"]), (10, 8)), None);
    }

    #[test]
    fn resize_fields_keep_the_aspect() {
        let f = |a: &str, b: &str| vec![a.to_string(), b.to_string()];
        assert_eq!(parse_resize(&f("320", ""), (640, 480)), Some((320, 240)));
        assert_eq!(parse_resize(&f("", "240"), (640, 480)), Some((320, 240)));
        assert_eq!(parse_resize(&f("50%", "10"), (640, 480)), Some((320, 10)));
        assert_eq!(parse_resize(&f("200%", ""), (3, 2)), Some((6, 4)));
        assert_eq!(parse_resize(&f("", ""), (640, 480)), None);
        assert_eq!(parse_resize(&f("x", ""), (640, 480)), None);
        assert_eq!(parse_resize(&f("0", ""), (640, 480)), None);
        assert_eq!(parse_resize(&f("70000", "1"), (640, 480)), None);
    }

    #[test]
    fn draws_at_tiny_sizes() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let m = sample_vm();
        for w in 0..=20u16 {
            for h in 0..=6u16 {
                let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut hits = Hits::default();
                t.draw(|f| draw(f, Rect::new(0, 0, w, h), &m, 1, &TuiTheme::plain(), &mut hits)).unwrap();
            }
        }
        let mut t = Terminal::new(TestBackend::new(60, 10)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw(f, Rect::new(0, 0, 60, 10), &m, 4, &TuiTheme::plain(), &mut hits)).unwrap();
        assert!(hits.iter().any(|(_, t)| matches!(t, HitTarget::EditorCell { doc: 4, top: 0, left: 0 })));
    }
}
