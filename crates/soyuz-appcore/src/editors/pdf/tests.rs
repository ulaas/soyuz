//! The viewer model against real (lopdf-built) documents, with textures that go nowhere
//! (`no_upload`).

use std::time::{Duration, Instant};

use super::*;
use crate::host::{Host, NullHost, no_wake};
use crate::services::pdf::fixtures;
use crate::services::preview_gfx::no_upload;

fn cfg() -> ViewerCfg {
    let mut c = ViewerCfg::from_config(&soyuz_core::config::Pdf::default(), 64);
    c.session.threads = 2;
    c
}

/// `bytes` written to a temp file and opened in an 800 × 600 view.
fn open_with(bytes: Vec<u8>, cfg: ViewerCfg) -> (tempfile::TempDir, PdfDoc) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.pdf");
    std::fs::write(&path, bytes).unwrap();
    let mut d = PdfDoc::new(path, cfg, no_wake(), no_upload(), None);
    d.set_viewport([800.0, 600.0], 1.0);
    settle(&mut d, "opening", |d| d.status() != &Status::Opening);
    (dir, d)
}

fn open(bytes: Vec<u8>) -> (tempfile::TempDir, PdfDoc) {
    open_with(bytes, cfg())
}

/// Pump until `done` (or fail after 30 s).
fn settle(d: &mut PdfDoc, what: &str, done: impl Fn(&PdfDoc) -> bool) {
    let t = Instant::now();
    loop {
        d.pump();
        if done(d) {
            return;
        }
        assert!(t.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn opens_at_fit_width_and_follows_the_view() {
    let (_t, mut d) = open(fixtures::text_pages());
    assert_eq!(d.status(), &Status::Ready);
    assert!(!d.is_dirty());
    assert_eq!(d.page_count(), 3);
    assert_eq!(d.zoom(), Zoom::FitWidth);
    let gap = soyuz_core::config::Pdf::PAGE_GAP_DEFAULT;
    assert!(close(d.scale(), (800.0 - 2.0 * gap) / 612.0), "{}", d.scale());
    let r = d.layout().rects[0];
    assert!(close(r[2] - r[0], 800.0 - 2.0 * gap));
    assert_eq!(d.scroll(), [0.0, 0.0]);
    assert_eq!(d.page_label(0), "Page 1 of 3");
    assert_eq!(d.info().map(|i| i.pages), Some(3));
    assert!(d.page_size_mm(0).is_some_and(|(w, h)| close(w, 215.9) && close(h, 279.4)));
    // Fit width follows the view; a fixed zoom does not.
    d.set_viewport([500.0, 600.0], 1.0);
    assert!(close(d.scale(), (500.0 - 2.0 * gap) / 612.0));
    d.command(&Action::PdfZoomActual);
    assert!(close(d.percent(), 100.0));
    assert_eq!(d.zoom_label(), "100 %");
    d.set_viewport([900.0, 600.0], 1.0);
    assert!(close(d.percent(), 100.0));
    d.command(&Action::PdfFitPage);
    assert!(close(d.scale(), (600.0 - 2.0 * gap) / 792.0), "the page height fits");
    // Steps through the presets.
    d.command(&Action::PdfZoomActual);
    d.command(&Action::PdfZoomIn);
    assert!(close(d.percent(), 110.0));
    d.command(&Action::PdfZoomOut);
    d.command(&Action::PdfZoomOut);
    assert!(close(d.percent(), 90.0));
}

#[test]
fn zoom_keeps_the_point_under_the_pointer() {
    let (_t, mut d) = open(fixtures::many_pages(5));
    d.set_scroll([0.0, 1500.0]);
    let anchor = [300.0, 250.0];
    let (page, before) = d.view_to_page(anchor, false).unwrap();
    d.zoom_to(Zoom::percent(300.0), Some(anchor));
    assert!(close(d.percent(), 300.0));
    let (page2, after) = d.view_to_page(anchor, false).unwrap();
    assert_eq!(page, page2);
    assert!(close(before[0], after[0]) && close(before[1], after[1]), "{before:?} → {after:?}");
    d.zoom_by(0.5, anchor);
    assert!(close(d.percent(), 150.0));
    let (_, again) = d.view_to_page(anchor, false).unwrap();
    assert!(close(before[0], again[0]) && close(before[1], again[1]), "{before:?} → {again:?}");
    assert_eq!(d.page(), page, "the current page follows the scroll");
}

#[test]
fn pages_clamp_and_go_to() {
    let (_t, mut d) = open(fixtures::many_pages(5));
    let gap = soyuz_core::config::Pdf::PAGE_GAP_DEFAULT;
    d.go_to_page(999);
    assert_eq!(d.page(), 4, "clamped to the last page");
    d.command(&Action::PdfFirstPage);
    assert_eq!((d.page(), d.scroll()[1]), (0, 0.0));
    d.command(&Action::PdfNextPage);
    d.command(&Action::PdfNextPage);
    assert_eq!(d.page(), 2);
    assert!(close(d.scroll()[1], d.layout().rects[2][1] - gap));
    d.command(&Action::PdfPrevPage);
    assert_eq!(d.page(), 1);
    d.command(&Action::PdfLastPage);
    assert_eq!(d.page(), 4);
    // The go-to field (1-based).
    d.command(&Action::EditorGoTo);
    assert_eq!(d.goto.as_deref(), Some(""));
    assert!(d.goto_focus);
    assert!(!d.submit_goto("abc"));
    assert!(d.goto.is_some(), "stays open on a bad number");
    assert!(d.submit_goto("2"));
    assert_eq!((d.page(), d.goto.as_deref()), (1, None));
    assert!(d.submit_goto("0"), "page 0 is the first page");
    assert_eq!(d.page(), 0);
    // Scrolling moves the current page; keys scroll.
    d.set_scroll([0.0, d.layout().rects[3][1] + 10.0]);
    assert_eq!(d.page(), 3);
    d.key(Some(KeyName::Home), KeyMods::default());
    assert_eq!((d.page(), d.scroll()[1]), (0, 0.0));
    d.key(Some(KeyName::PageDown), KeyMods::default());
    assert!(close(d.scroll()[1], 540.0));
    d.key(Some(KeyName::End), KeyMods::default());
    assert_eq!(d.page(), 4);
    // Esc closes nothing here.
    assert!(!d.close_bars());
}

#[test]
fn find_wraps_and_scrolls() {
    let (_t, mut d) = open(fixtures::many_pages(6));
    d.command(&Action::EditorFind);
    assert!(d.find.open && d.find.focus_find);
    d.find.query.text = "sweep".into();
    d.find_changed();
    settle(&mut d, "the find sweep", |d| d.find.done());
    assert_eq!(d.find.count(), 6);
    assert_eq!(d.find.current, Some((0, 0)), "the first match from the current page");
    assert_eq!(d.find.ordinal(), Some(1));
    let top = d.scroll()[1];
    for p in 1..6 {
        d.command(&Action::EditorFindNext);
        assert_eq!(d.find.current, Some((p, 0)));
    }
    assert!(d.scroll()[1] > top + 1000.0, "scrolled to the last page's match");
    assert_eq!(d.page(), 5);
    let v = d.visible_pages();
    let cur: Vec<Rect> = v.iter().flat_map(|p| p.current.clone()).collect();
    assert_eq!(cur.len(), 1, "the current match is highlighted");
    assert!(cur[0][1] >= 0.0 && cur[0][3] <= 600.0, "and in view: {cur:?}");
    d.command(&Action::EditorFindNext);
    assert_eq!(d.find.current, Some((0, 0)), "wraps to the first");
    assert_eq!(d.page(), 0);
    d.command(&Action::EditorFindPrev);
    assert_eq!(d.find.current, Some((5, 0)), "and back");
    assert_eq!(d.find.ordinal(), Some(6));
    // Whole word / case re-run the sweep.
    d.command(&Action::EditorMatchCase);
    assert!(d.find.query.case);
    settle(&mut d, "the case sweep", |d| d.find.done());
    assert_eq!(d.find.count(), 6);
    d.find.query.text = "SWEEP".into();
    d.find_changed();
    settle(&mut d, "no matches", |d| d.find.done());
    assert_eq!(d.find.count(), 0);
    assert!(d.close_bars(), "Esc closes the find bar");
    assert!(!d.find.open && !d.find.active());
}

#[test]
fn select_all_and_copy() {
    let (_t, mut d) = open(fixtures::text_pages());
    assert_eq!(d.input(EditInput::Copy), None, "nothing selected");
    d.key(Some(KeyName::A), KeyMods { command: true, ..Default::default() });
    let host = NullHost::default();
    if let Some(t) = d.input(EditInput::Copy) {
        host.copy_text(t);
    }
    settle(&mut d, "the copied text", |d| d.copy_out.is_some() || !d.pending_copy);
    if let Some(t) = d.take_copy() {
        host.copy_text(t);
    }
    let copied = host.copied.lock().unwrap().clone();
    assert_eq!(copied.len(), 1, "{copied:?}");
    let t = &copied[0];
    let (a, b, c) = (t.find("Hello world").unwrap(), t.find("Second page").unwrap(), t.find("Third page").unwrap());
    assert!(a < b && b < c, "{t:?}");
    assert!(d.selection_len() > 30);
    // Now every page's text is there: copying again is immediate.
    assert!(d.input(EditInput::Copy).is_some());
    assert!(d.close_bars(), "Esc drops the selection");
    assert!(d.selection().is_none());
}

#[test]
fn pointer_selects_words_and_drags() {
    let (_t, mut d) = open(fixtures::text_pages());
    settle(&mut d, "page 0 text", |d| d.page_text(0).is_some());
    let t = d.texts[&0].clone();
    let center = |i: usize, d: &PdfDoc| {
        let r = d.page_rect_to_view(0, t.chars[i].rect()).unwrap();
        [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0]
    };
    // Double click selects the word.
    d.pointer_down(center(1, &d), 2, false);
    d.pointer_up();
    assert_eq!(d.selected_text().as_deref(), Some("Hello"));
    assert_eq!(d.hover(center(1, &d)), Hover::Text);
    // A plain click clears it; a drag selects chars.
    d.pointer_down(center(0, &d), 1, false);
    d.pointer_up();
    assert!(d.selection().is_none());
    let start = d.page_rect_to_view(0, t.chars[0].rect()).unwrap();
    d.pointer_down([start[0] + 0.5, (start[1] + start[3]) / 2.0], 1, false);
    d.pointer_drag(center(4, &d));
    d.pointer_drag(center(7, &d));
    d.pointer_up();
    assert_eq!(d.selected_text().as_deref(), Some("Hello w"), "up to the middle of the pointed char");
    // Shift+click extends; triple click takes the line.
    let end = d.page_rect_to_view(0, t.chars[10].rect()).unwrap();
    d.pointer_down([end[2] - 0.5, (end[1] + end[3]) / 2.0], 1, true);
    d.pointer_up();
    assert_eq!(d.selected_text().as_deref(), Some("Hello world"));
    d.pointer_down(center(3, &d), 3, false);
    assert_eq!(d.selected_text().as_deref(), Some("Hello world"));
    let v = d.visible_pages();
    assert!(!v[0].selection.is_empty(), "the selection is highlighted");
}

#[test]
fn rotation_turns_the_layout() {
    let (_t, mut d) = open(fixtures::text_pages());
    let gap = soyuz_core::config::Pdf::PAGE_GAP_DEFAULT;
    d.go_to_page(1);
    d.command(&Action::PdfRotateCw);
    assert_eq!(d.rotation(), 1);
    assert_eq!(d.page(), 1, "the page stays");
    // Fit width now fits the page's height.
    assert!(close(d.scale(), (800.0 - 2.0 * gap) / 792.0));
    let r = d.layout().rects[1];
    assert!(close((r[2] - r[0]) / (r[3] - r[1]), 792.0 / 612.0), "landscape now");
    // Text boxes land inside the turned page.
    settle(&mut d, "page 1 text", |d| d.page_text(1).is_some());
    let b = d.texts[&1].chars[0].rect();
    let v = d.page_rect_to_view(1, b).unwrap();
    let pr = d.visible_pages().into_iter().find(|p| p.page == 1).unwrap().rect;
    assert!(v[0] >= pr[0] && v[2] <= pr[2] && v[1] >= pr[1] && v[3] <= pr[3], "{v:?} in {pr:?}");
    assert!(v[2] > pr[2] - (pr[2] - pr[0]) * 0.3, "text near the top edge moved to the right edge: {v:?}");
    d.command(&Action::PdfRotateCcw);
    d.command(&Action::PdfRotateCcw);
    assert_eq!(d.rotation(), 3);
    for _ in 0..3 {
        d.command(&Action::PdfRotateCw);
    }
    assert_eq!(d.rotation(), 2);
}

#[test]
fn pump_renders_bases_and_sharp_tiles() {
    let (_t, mut d) = open(fixtures::text_pages());
    settle(&mut d, "the first page's base", |d| {
        d.tex.contains(&TexKey::Base(PageKey { page: 0, rot: 0, invert: false }))
    });
    let v = d.visible_pages();
    assert!(v[0].base.is_some());
    assert!(v[0].tiles.is_empty(), "fit width: the base layer is sharp enough");
    d.zoom_to(Zoom::percent(400.0), Some([0.0, 0.0]));
    settle(&mut d, "sharp tiles", |d| sharp_tiles(d) > 0);
    assert!(d.visible_pages().iter().any(|p| !p.tiles.is_empty()));
    // Dark pages: new keys, the old textures go.
    d.command(&Action::PdfToggleDarkPages);
    assert!(d.invert());
    d.pump();
    assert!(d.visible_pages().iter().all(|p| p.base.is_none() && p.tiles.is_empty()));
    settle(&mut d, "inverted base", |d| d.tex.contains(&TexKey::Base(PageKey { page: 0, rot: 0, invert: true })));
    assert!(d.tex.bytes() <= d.cfg.cache_bytes);
    // The sidebar's thumbnails.
    d.command(&Action::PdfToggleSidebar);
    assert_eq!(d.sidebar, PdfSidebar::Thumbnails);
    d.want_thumbs(0..3);
    settle(&mut d, "thumbnails", |d| {
        (0..3).all(|p| d.tex.contains(&TexKey::Thumb(PageKey { page: p, rot: 0, invert: true })))
    });
    assert!(d.thumb(2).is_some());
    d.command(&Action::PdfToggleSidebar);
    assert_eq!(d.sidebar, PdfSidebar::None);
    d.command(&Action::PdfToggleSidebar);
    assert_eq!(d.sidebar, PdfSidebar::Thumbnails, "back as it was");
}

/// Sharp tiles of the visible pages already rendered.
fn sharp_tiles(d: &PdfDoc) -> usize {
    let key = d.current_key();
    d.layout
        .visible(d.view_rect())
        .flat_map(|i| d.tiles_for(i, key))
        .filter(|t| d.tex.contains(&TexKey::Tile(*t)))
        .count()
}

#[test]
fn links_jump_or_come_back_per_policy() {
    let (_t, mut d) = open(fixtures::with_nav());
    assert_eq!(d.outline().len(), 3);
    let at = |d: &PdfDoc, x: f32, y: f32| {
        let r = d.page_rect_to_view(0, [x, y, x, y]).unwrap();
        [r[0], r[1]]
    };
    let uri = at(&d, 172.0, 77.0);
    assert_eq!(d.hover(uri), Hover::Link);
    assert_eq!(d.click_link(uri), Some(LinkAction::Ask("https://example.com/docs".into())));
    // Launch actions are not links at all.
    assert_eq!(d.link_at(at(&d, 172.0, 377.0)), None);
    // An internal link scrolls to its page.
    assert_eq!(d.click_link(at(&d, 172.0, 177.0)), Some(LinkAction::Jumped));
    assert_eq!(d.page(), 2);
    // `open` and `off`.
    let mut c = cfg();
    c.external_links = PdfExternalLinks::Open;
    let (_t2, mut d) = open_with(fixtures::with_nav(), c.clone());
    let uri = at(&d, 172.0, 77.0);
    assert_eq!(d.click_link(uri), Some(LinkAction::Open("https://example.com/docs".into())));
    c.external_links = PdfExternalLinks::Off;
    d.set_config(&c);
    assert_eq!(d.click_link(uri), None);
    // Outline destinations.
    let dest = d.outline()[2 - 1].dest.unwrap();
    d.go_to_dest(dest);
    assert_eq!(d.page(), 1);
}

#[test]
fn locked_documents_reopen_after_the_password() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("locked.pdf");
    std::fs::write(&path, fixtures::encrypted("secret")).unwrap();
    let mut d = PdfDoc::new(path.clone(), cfg(), no_wake(), no_upload(), None);
    settle(&mut d, "opening", |d| d.status() != &Status::Opening);
    assert_eq!(d.locked(), Some(false));
    preview_pdf::set_password(&path, "secret".into());
    settle(&mut d, "the reopened document", |d| d.is_ready());
    assert_eq!(d.page_count(), 2);
    assert_eq!(d.locked(), None);
}

#[test]
fn reloads_and_remembered_positions_keep_the_page() {
    let (_t, mut d) = open(fixtures::many_pages(4));
    d.command(&Action::PdfZoomActual);
    d.go_to_page(2);
    d.command(&Action::PdfRotateCw);
    d.reload();
    assert_eq!(d.status(), &Status::Opening);
    settle(&mut d, "the reload", |d| d.is_ready());
    assert_eq!((d.page(), d.rotation()), (2, 1));
    assert!(close(d.percent(), 100.0));
    let pos = d.position().unwrap();
    // A new document from a remembered position.
    let mut e = PdfDoc::new(d.path.clone(), cfg(), no_wake(), no_upload(), Some(pos));
    assert_eq!(e.position(), Some(pos), "known before it opens");
    e.set_viewport([800.0, 600.0], 1.0);
    settle(&mut e, "opening", |d| d.is_ready());
    assert_eq!((e.page(), e.rotation(), e.zoom()), (2, 1, pos.zoom));
    assert!(close(e.scroll()[1], d.scroll()[1]));
}

#[test]
fn devscript_verbs_wait_for_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.pdf");
    std::fs::write(&path, fixtures::many_pages(4)).unwrap();
    let mut d = PdfDoc::new(path, cfg(), no_wake(), no_upload(), None);
    d.set_viewport([800.0, 600.0], 1.0);
    for v in ["page=3", "zoom=150", "sidebar=outline", "find=Page 1", "dark"] {
        d.devscript(v);
    }
    settle(&mut d, "opening", |d| d.is_ready());
    assert_eq!(d.page(), 2);
    assert!(close(d.percent(), 150.0));
    assert_eq!(d.sidebar, PdfSidebar::Outline);
    assert!(d.invert());
    settle(&mut d, "the sweep", |d| d.find.done());
    assert_eq!(d.find.count(), 1);
    assert_eq!(d.find.current, Some((1, 0)));
    d.devscript("fit=page");
    assert_eq!(d.zoom(), Zoom::FitPage);
    d.devscript("fit=width");
    assert_eq!(d.zoom(), Zoom::FitWidth);
    d.devscript("rotate=cw");
    assert_eq!(d.rotation(), 1);
    d.devscript("rotate=ccw");
    assert_eq!(d.rotation(), 0);
    d.devscript("next");
    assert_eq!(d.find.current, Some((1, 0)), "one match: next stays on it");
    d.devscript("select=all");
    d.devscript("copy");
    settle(&mut d, "the copy", |d| d.copy_out.is_some());
    assert!(d.take_copy().unwrap().contains("Page 3 of the sweep"));
    d.devscript("sidebar=none");
    assert_eq!(d.sidebar, PdfSidebar::None);
}

#[test]
fn bad_files_fail() {
    let (_t, d) = open(b"%PDF-1.4 garbage".to_vec());
    assert_eq!(d.status(), &Status::Failed(PdfError::Invalid));
    assert_eq!(d.locked(), None);
    assert!(d.position().is_none());
}
