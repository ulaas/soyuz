//! Headless tests of T4: images (`[tui] images`) and the preview pane's pictures. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;
use soyuz_appcore::services::preview_gfx::{Pixels, Tex};
use soyuz_core::config::{TuiColorMode, TuiImages};
use soyuz_tui::app::NullIo;
use soyuz_tui::images::{self, Drawn, ImageState, ProtocolType};
use soyuz_tui::view::preview::Body;

use crate::common::{self, Run};

/// The app hands the core the terminal UI's uploader: textures the core makes land in the
/// image store and draw as half-blocks; `[tui] images` / `color_mode` changes apply on
/// the next frame; a dropped texture frees its pixels.
#[test]
fn tui_images_store_follows_the_core_and_config() {
    match common::relaunch(module_path!(), "tui_images_store_follows_the_core_and_config", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
    }) {
        Run::Child(data) => store_child(data),
        Run::Parent(_) => {}
    }
}

fn store_child(data: PathBuf) {
    let mut c = common::test_config();
    c.tui.images = TuiImages::Halfblocks;
    c.tui.color_mode = TuiColorMode::Truecolor;
    let mut app = common::new_app(c, &data);
    let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
    let mut io = NullIo::default();
    app.frame(&mut t, &[], &mut io).unwrap();
    assert_eq!(images::active(), Some(ProtocolType::Halfblocks));

    // A texture made through the core's uploader (as the preview / thumbnail workers do).
    let rgba: Vec<u8> = (0..64 * 32).flat_map(|_| [0u8, 200, 0, 255]).collect();
    let tex =
        Tex::upload(&app.core.uploader, "green".into(), Pixels { size: [64, 32], rgba: &rgba, premultiplied: false });
    assert_eq!(images::stats().stored, (1, 64 * 32 * 4));

    let mut img = Terminal::new(TestBackend::new(20, 8)).unwrap();
    let mut st = ImageState { upscale: true, ..ImageState::default() };
    let mut r = Drawn::Off;
    img.draw(|f| {
        r = images::draw(f, f.area(), &tex, &mut st);
        images::end_frame(f);
    })
    .unwrap();
    assert_eq!(r, Drawn::Image);
    // 64 x 32 px upscaled into 200 x 160 px (cells 10 x 20): 200 x 100 px = 20 x 5 cells.
    assert_eq!(st.drawn, Some(Rect::new(0, 1, 20, 5)));
    assert_eq!(img.backend().buffer()[(10, 3)].fg, Color::Rgb(0, 200, 0));

    // `color_mode = "256"` applies with the next frame.
    app.core.config.tui.color_mode = TuiColorMode::Ansi256;
    app.frame(&mut t, &[], &mut io).unwrap();
    img.draw(|f| r = images::draw(f, f.area(), &tex, &mut st)).unwrap();
    assert!(matches!(img.backend().buffer()[(10, 3)].fg, Color::Indexed(_)));

    // A forced protocol and off.
    app.core.config.tui.images = TuiImages::Sixel;
    app.frame(&mut t, &[], &mut io).unwrap();
    assert_eq!(images::active(), Some(ProtocolType::Sixel));
    img.draw(|f| {
        r = images::draw(f, f.area(), &tex, &mut st);
        images::end_frame(f);
    })
    .unwrap();
    assert_eq!(r, Drawn::Pending);
    assert!(images::wait_encoded(Duration::from_secs(30)));
    // (No app frame in between: placements are tracked per screen, and the app's frame
    // has none.)
    img.draw(|f| {
        r = images::draw(f, f.area(), &tex, &mut st);
        images::end_frame(f);
    })
    .unwrap();
    assert_eq!(r, Drawn::Image);
    app.core.config.tui.images = TuiImages::Off;
    app.frame(&mut t, &[], &mut io).unwrap();
    assert_eq!(images::active(), None);
    img.draw(|f| r = images::draw(f, f.area(), &tex, &mut st)).unwrap();
    assert_eq!(r, Drawn::Off);

    drop(tex);
    assert_eq!(images::stats().stored, (0, 0), "the last clone freed the pixels");
    app.finish();
    common::ok();
}

// ---- the preview pane's pictures (T4.4) ---------------------------------------------

/// Run frames (sending `events` with the first) until `done` holds on the app and the
/// screen; panics after 30 s.
fn frames_until(
    app: &mut soyuz_tui::app::App,
    t: &mut Terminal<TestBackend>,
    io: &mut NullIo,
    events: &[crossterm::event::Event],
    what: &str,
    done: impl Fn(&soyuz_tui::app::App, &Terminal<TestBackend>) -> bool,
) {
    let start = std::time::Instant::now();
    app.frame(t, events, io).unwrap();
    while !done(app, t) {
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}:\n{}", common::screen(t));
        std::thread::sleep(Duration::from_millis(10));
        app.frame(t, &[], io).unwrap();
    }
}

/// Half-block cells inside the preview pane.
fn block_cells(app: &soyuz_tui::app::App, t: &Terminal<TestBackend>) -> usize {
    let r = app.regions().preview;
    let buf = t.backend().buffer();
    (r.top()..r.bottom())
        .flat_map(|y| (r.left()..r.right()).map(move |x| (x, y)))
        .filter(|&p| matches!(buf[p].symbol(), "▀" | "▄" | "█"))
        .count()
}

/// The preview pane's text.
fn pane_text(app: &soyuz_tui::app::App, t: &Terminal<TestBackend>) -> String {
    let r = app.regions().preview;
    let buf = t.backend().buffer();
    (r.top()..r.bottom())
        .map(|y| (r.left()..r.right()).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n")
        .collect()
}

fn picture_shown(app: &soyuz_tui::app::App) -> bool {
    app.view().is_some_and(|v| v.preview.visible && matches!(v.preview.body, Body::Picture { .. }))
}

fn preview_config(images: TuiImages, color: TuiColorMode) -> soyuz_core::config::Config {
    let mut c = common::test_config();
    c.tui.images = images;
    c.tui.color_mode = color;
    c.tui.preview_ratio = 0.5;
    c
}

/// A PNG previews as half-block cells with its dimensions under it; `[tui] images = "off"`
/// shows a text line instead; mono colour mode a `[W×H]` placeholder.
#[test]
fn tui_preview_draws_pictures() {
    match common::relaunch(module_path!(), "tui_preview_draws_pictures", |data| {
        image::RgbaImage::from_fn(200, 150, |x, _| image::Rgba([if x < 100 { 255 } else { 0 }, 0, 200, 255]))
            .save(data.join("pic.png"))
            .unwrap();
    }) {
        Run::Child(data) => pictures_child(data),
        Run::Parent(_) => {}
    }
}

fn pictures_child(data: PathBuf) {
    let mut app = common::new_app(preview_config(TuiImages::Halfblocks, TuiColorMode::Truecolor), &data);
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut io = NullIo::default();
    frames_until(&mut app, &mut t, &mut io, &[], "the listing", |a, _| {
        common::labels(a).iter().any(|l| l == "pic.png")
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Home)], "the cursor", |a, _| {
        common::cursor(a).as_deref() == Some("pic.png")
    });
    frames_until(&mut app, &mut t, &mut io, &[common::alt('p')], "the picture", |a, t| {
        picture_shown(a) && block_cells(a, t) > 20
    });
    let text = pane_text(&app, &t);
    assert!(text.contains("200 × 150 px"), "the dimensions under the picture:\n{text}");
    let body = app.view().unwrap().preview.body_text();
    assert!(body.iter().any(|l| l.contains("PNG")), "{body:?}");
    // Colours of the picture (truecolor): red on the left half.
    let r = app.regions().preview;
    let buf = t.backend().buffer();
    let red = (r.top()..r.bottom())
        .flat_map(|y| (r.left()..r.right()).map(move |x| (x, y)))
        .any(|p| matches!(buf[p].symbol(), "▀" | "▄" | "█") && buf[p].fg == Color::Rgb(255, 0, 200));
    assert!(red, "picture colours reach the cells");

    // Off: a text line instead, no block cells.
    app.core.config.tui.images = TuiImages::Off;
    frames_until(&mut app, &mut t, &mut io, &[], "the off line", |a, t| {
        pane_text(a, t).contains("Pictures are off") && block_cells(a, t) == 0
    });
    // Mono: the `[W×H]` placeholder.
    app.core.config.tui.images = TuiImages::Halfblocks;
    app.core.config.tui.color_mode = TuiColorMode::Mono;
    frames_until(&mut app, &mut t, &mut io, &[], "the placeholder", |a, t| pane_text(a, t).contains("[200x150]"));

    // Hidden: nothing loads, the texture is let go.
    app.core.config.tui.color_mode = TuiColorMode::Truecolor;
    frames_until(&mut app, &mut t, &mut io, &[common::alt('p')], "the pane hidden", |a, _| {
        a.regions().preview.is_empty() && soyuz_tui::input::preview::target(a).is_none()
    });
    app.finish();
    common::ok();
}

/// An SVG is rasterized by the core and drawn like any picture.
#[test]
fn tui_preview_draws_svg() {
    match common::relaunch(module_path!(), "tui_preview_draws_svg", |data| {
        std::fs::write(
            data.join("logo.svg"),
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#00c000"/></svg>"##,
        )
        .unwrap();
    }) {
        Run::Child(data) => svg_child(data),
        Run::Parent(_) => {}
    }
}

fn svg_child(data: PathBuf) {
    let mut app = common::new_app(preview_config(TuiImages::Halfblocks, TuiColorMode::Truecolor), &data);
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut io = NullIo::default();
    frames_until(&mut app, &mut t, &mut io, &[], "the listing", |a, _| {
        common::labels(a).iter().any(|l| l == "logo.svg")
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Home)], "the cursor", |a, _| {
        common::cursor(a).as_deref() == Some("logo.svg")
    });
    frames_until(&mut app, &mut t, &mut io, &[common::alt('p')], "the SVG", |a, t| {
        picture_shown(a) && block_cells(a, t) > 20
    });
    let r = app.regions().preview;
    let buf = t.backend().buffer();
    let green = (r.top()..r.bottom())
        .flat_map(|y| (r.left()..r.right()).map(move |x| (x, y)))
        .any(|p| buf[p].fg == Color::Rgb(0, 192, 0));
    assert!(green, "the SVG's colour:\n{}", common::screen(&t));
    assert!(pane_text(&app, &t).contains("40 × 20"), "{}", pane_text(&app, &t));
    app.finish();
    common::ok();
}
