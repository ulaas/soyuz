//! Headless tests of T4: images (`[tui] images`). Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;
use soyuz_appcore::services::preview_gfx::{Pixels, Tex};
use soyuz_core::config::{TuiColorMode, TuiImages};
use soyuz_tui::app::NullIo;
use soyuz_tui::images::{self, Drawn, ImageState, ProtocolType};

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
