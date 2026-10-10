//! Drag-out: an internal drag that leaves the window becomes an OS drag, so items
//! can be dropped on the desktop, into Explorer, editors, mail clients, ...
//!
//! * **Trigger** ([`should_hand_off`]): an internal [`DragPayload`] is active, the primary
//!   button is down, and the pointer is outside the window (egui reports it outside the
//!   viewport, or gone after winit's `CursorLeft`). Drags between panes never leave the
//!   window, so they behave exactly as before.
//! * **Hand-off** ([`end_frame`]): the egui payload is cleared and
//!   `soyuz_fs::dragout::do_drag` runs the modal OLE loop (`DoDragDrop`, allowed effects
//!   copy | move | link; the target reads Ctrl / Shift / Alt itself). winit buffers its
//!   window events meanwhile. Afterwards a synthetic button release + `PointerGone` is fed
//!   to egui on the next frame ([`raw_input`]), because the real button-up went to the
//!   OLE loop. Coming back over the Soyuz window cancels the OLE drag and resumes the
//!   internal one (payload restored, no release injected: the button is still down).
//! * **After the drop** ([`after_drop`]): copy / link need nothing. A move the target
//!   performed itself (optimized move: `DROPEFFECT_NONE` + "Logical Performed DropEffect"
//!   = move, or the sources are gone) only refreshes the panes; nothing is recorded in
//!   undo. Only an *unoptimized* move (the target returned `DROPEFFECT_MOVE`, did not
//!   report a different performed effect, and every source still exists) makes Soyuz
//!   remove the sources itself, through a normal **trash** job (undoable). When unsure,
//!   nothing is deleted.
//! * Only local paths are dragged out: the internal drag sources (file list, tree, search
//!   / tag / recent listings, shelf) all carry local paths; archive and remote listings
//!   don't start internal drags.

use std::path::{Path, PathBuf};

use egui::{Id, Pos2, Rect};
use soyuz_core::config::DragDefault;
use soyuz_fs::dragout::{DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DragData, DragReport, Format, Outcome};

use super::dnd::{self, DragPayload};
use crate::app::SoyuzApp;

/// Whether the active internal drag should become an OS drag this frame.
///
/// `pointer` is egui's latest pointer position (`None` after `PointerGone`), `screen`
/// the window's content rect in points.
pub fn should_hand_off(active: bool, primary_down: bool, pointer: Option<Pos2>, screen: Rect) -> bool {
    if !active || !primary_down {
        return false;
    }
    match pointer {
        None => true,
        // One point of slack so the window's last pixel row / column still counts as inside.
        Some(p) => !screen.expand(1.0).contains(p),
    }
}

/// Path as handed to other apps: without the `\\?\` prefix when the plain form fits
/// `MAX_PATH` (many apps reject verbatim paths), verbatim otherwise.
fn external_path(p: &Path) -> PathBuf {
    let plain = soyuz_fs::strip_verbatim(p);
    if plain.as_os_str().len() < 260 { plain } else { p.to_path_buf() }
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect()
}

/// Formats offered for `paths`: `CF_HDROP` (wide `DROPFILES`), "FileNameW" (single item),
/// `CF_UNICODETEXT` (one path per line) and, when the drag default is an explicit copy or
/// move, "Preferred DropEffect" (omitted for `auto` / `ask` so targets apply their own
/// same-volume rules, like a drag from Explorer).
pub fn build_data(paths: &[PathBuf], default: DragDefault) -> DragData {
    let ext: Vec<PathBuf> = paths.iter().map(|p| external_path(p)).collect();
    let mut formats = vec![(Format::Hdrop, crate::services::os_clipboard::build_dropfiles(&ext))];
    if let [one] = ext.as_slice() {
        formats.push((Format::Named("FileNameW".into()), utf16z(&one.to_string_lossy())));
    }
    let text = ext.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\r\n");
    formats.push((Format::UnicodeText, utf16z(&text)));
    let preferred = match default {
        DragDefault::Copy => Some(DROPEFFECT_COPY),
        DragDefault::Move => Some(DROPEFFECT_MOVE),
        DragDefault::Auto | DragDefault::Ask => None,
    };
    if let Some(e) = preferred {
        formats.push((Format::Named("Preferred DropEffect".into()), e.to_le_bytes().to_vec()));
    }
    DragData { formats }
}

/// What Soyuz does after the OS drag ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AfterDrop {
    /// Nothing changed on our side (copy, link, cancel).
    Nothing,
    /// The target moved the items itself: re-read the panes, no undo entry.
    Refresh,
    /// Unoptimized move confirmed by the target: trash the sources (undoable job).
    TrashSources(Vec<PathBuf>),
    /// The pointer came back: resume the internal drag.
    Resume,
}

/// Decide the follow-up from the target's report (conservative: never delete when unsure).
pub fn after_drop(report: &DragReport, paths: &[PathBuf], exists: impl Fn(&Path) -> bool) -> AfterDrop {
    match report.outcome {
        Outcome::Returned => return AfterDrop::Resume,
        Outcome::Cancelled => return AfterDrop::Nothing,
        Outcome::Dropped => {}
    }
    let moved_logically = report.logical_performed.is_some_and(|e| e & DROPEFFECT_MOVE != 0);
    if report.effect & DROPEFFECT_MOVE == 0 {
        // Copy / link / none. "None" with a logical move = optimized move.
        return if moved_logically { AfterDrop::Refresh } else { AfterDrop::Nothing };
    }
    // DoDragDrop said MOVE: per the docs the source deletes, unless the target tells us
    // it did something else.
    let contradicts = |v: Option<u32>| v.is_some_and(|e| e & DROPEFFECT_MOVE == 0);
    if contradicts(report.performed) || contradicts(report.paste_succeeded) {
        return AfterDrop::Refresh;
    }
    if paths.is_empty() || !paths.iter().all(|p| exists(p)) {
        // Some are gone already: the target moved them (or part of them). Hands off.
        return AfterDrop::Refresh;
    }
    AfterDrop::TrashSources(paths.to_vec())
}

fn reset_id() -> Id {
    Id::new("soyuz.dragout.reset")
}

/// End of frame (before the drag badge): hand an internal drag that left the window to
/// the OS. Windows only; a no-op elsewhere or with `[behaviour] drag_out = false`.
pub fn end_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    if !cfg!(windows) || !app.config.behaviour.drag_out {
        return;
    }
    let Some(payload) = dnd::payload(ctx) else { return };
    let (primary, pointer, screen) =
        ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos(), i.viewport_rect()));
    if !should_hand_off(true, primary, pointer, screen) {
        return;
    }
    egui::DragAndDrop::clear_payload(ctx);
    ctx.request_repaint();
    // The button may have been released outside the window without us seeing it
    // (capture lost): never start an OS drag then, it would drop at once.
    if !soyuz_fs::dragout::primary_button_down() {
        ctx.data_mut(|d| d.insert_temp(reset_id(), true));
        return;
    }
    let paths = payload.paths.clone();
    let data = build_data(&paths, payload.default);
    let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
    // The window the drag started in (main or an in-process secondary window): coming
    // back over it resumes the internal drag.
    let hwnd = crate::ui::native::hwnd_of(ctx, ctx.viewport_id()).unwrap_or_else(crate::ui::native_chrome::main_hwnd);
    let report = match soyuz_fs::dragout::do_drag(&data, allowed, hwnd) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("drag-out failed: {e}");
            ctx.data_mut(|d| d.insert_temp(reset_id(), true));
            return;
        }
    };
    log::debug!("drag-out of {} item(s): {report:?}", paths.len());
    let action = after_drop(&report, &paths, |p| p.symlink_metadata().is_ok());
    if action != AfterDrop::Resume {
        ctx.data_mut(|d| d.insert_temp(reset_id(), true));
    }
    match action {
        AfterDrop::Nothing => {}
        AfterDrop::Resume => egui::DragAndDrop::set_payload(ctx, DragPayload::clone(&payload)),
        // The target moved the files itself: relist the panes showing the sources.
        AfterDrop::Refresh => crate::ops::refresh_affected(app, &paths),
        AfterDrop::TrashSources(p) => {
            super::delete(app, p, soyuz_jobs::DeleteMode::Trash);
            crate::ops::refresh_affected(app, &paths);
        }
    }
}

/// `raw_input_hook`: after an OS drag, tell egui the button is up and the pointer left
/// (the real `WM_LBUTTONUP` was consumed by the OLE loop).
pub fn raw_input(ctx: &egui::Context, raw: &mut egui::RawInput) {
    if ctx.data_mut(|d| d.remove_temp::<bool>(reset_id())).is_none() {
        return;
    }
    let (pos, modifiers) = ctx.input(|i| (i.pointer.latest_pos().or(i.pointer.interact_pos()), i.modifiers));
    let pos = pos.unwrap_or(Pos2::ZERO);
    raw.events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    raw.events.push(egui::Event::PointerGone);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};
    use soyuz_fs::dragout::DROPEFFECT_NONE;

    fn screen() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))
    }

    #[test]
    fn hand_off_trigger() {
        let s = screen();
        assert!(!should_hand_off(false, true, None, s), "no internal drag");
        assert!(!should_hand_off(true, false, None, s), "button up: the drop already happened");
        assert!(!should_hand_off(true, true, Some(pos2(400.0, 300.0)), s), "inside: internal drag");
        assert!(!should_hand_off(true, true, Some(pos2(800.0, 600.0)), s), "edge pixel is inside");
        assert!(should_hand_off(true, true, Some(pos2(-5.0, 300.0)), s), "left of the window");
        assert!(should_hand_off(true, true, Some(pos2(400.0, 620.0)), s), "below the window");
        assert!(should_hand_off(true, true, None, s), "pointer gone (CursorLeft)");
    }

    fn report(outcome: Outcome, effect: u32) -> DragReport {
        DragReport::plain(outcome, effect)
    }

    #[test]
    fn after_drop_semantics() {
        let paths = vec![PathBuf::from(r"C:\a.txt"), PathBuf::from(r"C:\b.txt")];
        let all = |_: &Path| true;
        let none = |_: &Path| false;
        assert_eq!(after_drop(&report(Outcome::Cancelled, 0), &paths, all), AfterDrop::Nothing);
        assert_eq!(after_drop(&report(Outcome::Returned, 0), &paths, all), AfterDrop::Resume);
        assert_eq!(after_drop(&report(Outcome::Dropped, DROPEFFECT_COPY), &paths, all), AfterDrop::Nothing);
        assert_eq!(after_drop(&report(Outcome::Dropped, DROPEFFECT_LINK), &paths, all), AfterDrop::Nothing);
        // Optimized move: DROPEFFECT_NONE + logical move.
        let mut opt = report(Outcome::Dropped, DROPEFFECT_NONE);
        opt.logical_performed = Some(DROPEFFECT_MOVE);
        assert_eq!(after_drop(&opt, &paths, all), AfterDrop::Refresh);
        // MOVE but the sources are gone: the target moved them.
        assert_eq!(after_drop(&report(Outcome::Dropped, DROPEFFECT_MOVE), &paths, none), AfterDrop::Refresh);
        let partly = |p: &Path| p.ends_with("a.txt");
        assert_eq!(after_drop(&report(Outcome::Dropped, DROPEFFECT_MOVE), &paths, partly), AfterDrop::Refresh);
        // MOVE but the target reports it performed a copy / paste without move.
        let mut perf = report(Outcome::Dropped, DROPEFFECT_MOVE);
        perf.performed = Some(DROPEFFECT_COPY);
        assert_eq!(after_drop(&perf, &paths, all), AfterDrop::Refresh);
        let mut paste = report(Outcome::Dropped, DROPEFFECT_MOVE);
        paste.paste_succeeded = Some(DROPEFFECT_NONE);
        assert_eq!(after_drop(&paste, &paths, all), AfterDrop::Refresh);
        // Unoptimized move with everything still there: trash the sources.
        assert_eq!(
            after_drop(&report(Outcome::Dropped, DROPEFFECT_MOVE), &paths, all),
            AfterDrop::TrashSources(paths.clone())
        );
        let mut confirmed = report(Outcome::Dropped, DROPEFFECT_MOVE);
        confirmed.performed = Some(DROPEFFECT_MOVE);
        assert_eq!(after_drop(&confirmed, &paths, all), AfterDrop::TrashSources(paths.clone()));
        assert_eq!(after_drop(&report(Outcome::Dropped, DROPEFFECT_MOVE), &[], all), AfterDrop::Refresh);
    }

    fn named<'a>(d: &'a DragData, name: &str) -> Option<&'a Vec<u8>> {
        d.formats.iter().find(|(f, _)| *f == Format::Named(name.into())).map(|(_, b)| b)
    }

    #[test]
    fn format_list() {
        let one = vec![PathBuf::from(r"\\?\C:\dir\a.txt")];
        let d = build_data(&one, DragDefault::Auto);
        let kinds: Vec<&Format> = d.formats.iter().map(|(f, _)| f).collect();
        assert_eq!(kinds, [&Format::Hdrop, &Format::Named("FileNameW".into()), &Format::UnicodeText]);
        let parsed = crate::services::os_clipboard::parse_dropfiles(&d.formats[0].1).unwrap();
        assert_eq!(parsed, vec![PathBuf::from(r"C:\dir\a.txt")], "verbatim prefix stripped");
        assert_eq!(named(&d, "FileNameW").unwrap(), &utf16z(r"C:\dir\a.txt"));

        let two = vec![PathBuf::from(r"C:\a"), PathBuf::from(r"D:\b c\d.txt")];
        let d = build_data(&two, DragDefault::Move);
        assert!(named(&d, "FileNameW").is_none(), "FileNameW only for a single item");
        assert_eq!(named(&d, "Preferred DropEffect").unwrap(), &DROPEFFECT_MOVE.to_le_bytes().to_vec());
        let text = d.formats.iter().find(|(f, _)| *f == Format::UnicodeText).unwrap();
        assert_eq!(text.1, utf16z("C:\\a\r\nD:\\b c\\d.txt"));
        assert_eq!(crate::services::os_clipboard::parse_dropfiles(&d.formats[0].1).unwrap(), two);
        let d = build_data(&two, DragDefault::Copy);
        assert_eq!(named(&d, "Preferred DropEffect").unwrap(), &DROPEFFECT_COPY.to_le_bytes().to_vec());
        assert!(named(&build_data(&two, DragDefault::Ask), "Preferred DropEffect").is_none());
    }

    #[test]
    fn long_paths_stay_verbatim() {
        let long = format!(r"\\?\C:\{}\f.txt", "x".repeat(300));
        assert_eq!(external_path(Path::new(&long)), PathBuf::from(&long));
    }

    /// In-process OLE check: the data object serves every format through its COM vtable
    /// and the paths read back from `CF_HDROP` match.
    #[cfg(windows)]
    #[test]
    fn ole_data_object_serves_paths() {
        let paths = vec![PathBuf::from(r"C:\Windows\win.ini"), PathBuf::from(r"C:\Users\Public\ä ö.txt")];
        let data = build_data(&paths, DragDefault::Copy);
        let got = soyuz_fs::dragout::read_back(&data).unwrap();
        let kinds: Vec<Format> = got.iter().map(|(f, _)| f.clone()).collect();
        assert_eq!(
            kinds,
            [Format::Hdrop, Format::UnicodeText, Format::Named("Preferred DropEffect".into())],
            "EnumFormatEtc lists the offered formats in order"
        );
        let hdrop = &got[0].1;
        assert_eq!(crate::services::os_clipboard::parse_dropfiles(hdrop).unwrap(), paths);
        let text: Vec<u16> = got[1].1.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let text = String::from_utf16_lossy(&text[..text.iter().position(|u| *u == 0).unwrap()]);
        assert_eq!(text, "C:\\Windows\\win.ini\r\nC:\\Users\\Public\\ä ö.txt");
        assert_eq!(&got[2].1[..4], &DROPEFFECT_COPY.to_le_bytes());
    }
}
