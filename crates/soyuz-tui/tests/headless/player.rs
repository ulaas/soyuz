//! Headless tests of Phase V3b: the media player in the preview pane and Quick Look. Each
//! test runs in its own sandbox: `crate::common::relaunch(module_path!(), "<test name>",
//! |data| ...)`.
//!
//! The clips are generated (`soyuz_media::fixtures`: a Motion-JPEG AVI, a PCM WAV). Real
//! decoding needs the FFmpeg libraries: those tests run only with `SOYUZ_MEDIA_TESTS=1`
//! (and `SOYUZ_FFMPEG_DIR` pointing at a runtime, inherited by the sandboxed child) and
//! skip otherwise. `TogglePreviewPane` is Alt+P, `QuickLook` Space; the player keys are
//! the `Player` context's defaults (K, J / L, M, - / =, ← / →).

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_core::config::{Config, KeyContext, TuiColorMode, TuiImages};
use soyuz_media::fixtures;
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::input::TuiFocus;
use soyuz_tui::layout::HitTarget;
use soyuz_tui::player::Status;
use soyuz_tui::view::preview::Body;

use crate::common::{self, Run};

/// Real decoding is wanted (the `media` CI job).
fn media_tests() -> bool {
    std::env::var("SOYUZ_MEDIA_TESTS").is_ok_and(|v| v == "1")
}

/// A 3 s Motion-JPEG AVI (24 red frames at 8 fps) and a 2 s mono WAV.
fn files(data: &std::path::Path) {
    let jpeg = fixtures::jpeg_solid(64, 48, [200, 30, 30]);
    let frames: Vec<Vec<u8>> = (0..24).map(|_| jpeg.clone()).collect();
    std::fs::write(data.join("clip.avi"), fixtures::avi(64, 48, 8, fixtures::AviCodec::Mjpeg, &frames)).unwrap();
    let tone = fixtures::sine(8000, 440.0, 0.5, 16_000);
    std::fs::write(data.join("tone.wav"), fixtures::wav(8000, 1, &tone)).unwrap();
}

fn config() -> Config {
    let mut c = common::test_config();
    c.tui.images = TuiImages::Halfblocks;
    c.tui.color_mode = TuiColorMode::Truecolor;
    c.tui.preview_ratio = 0.5;
    c.tui.mouse = true;
    c.media.autoplay = true;
    c.media.loop_ = true;
    c.media.seek_step_secs = 1.0;
    c.media.volume = 80;
    c.media.muted = false;
    c
}

/// Run frames (sending `events` with the first) until `done` holds; panics after 30 s.
fn frames_until(
    app: &mut App,
    t: &mut Terminal<TestBackend>,
    io: &mut NullIo,
    events: &[Event],
    what: &str,
    done: impl Fn(&App, &Terminal<TestBackend>) -> bool,
) {
    let start = std::time::Instant::now();
    app.frame(t, events, io).unwrap();
    while !done(app, t) {
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}:\n{}", common::screen(t));
        std::thread::sleep(Duration::from_millis(10));
        app.frame(t, &[], io).unwrap();
    }
}

/// The preview pane's body when it is the player's.
fn preview_player(app: &App) -> Option<(&soyuz_tui::player::StatusVm, usize, u64)> {
    match &app.view()?.preview.body {
        Body::Player { status, pics, serial, .. } if app.view()?.preview.visible => Some((status, pics.len(), *serial)),
        _ => None,
    }
}

fn quicklook_player(app: &App) -> Option<&soyuz_tui::player::StatusVm> {
    match &app.view()?.quicklook.preview.body {
        Body::Player { status, .. } if app.view()?.quicklook.visible => Some(status),
        _ => None,
    }
}

/// The text of the pane's rows (`r`).
fn text_of(t: &Terminal<TestBackend>, r: ratatui::layout::Rect) -> String {
    let buf = t.backend().buffer();
    (r.top()..r.bottom())
        .map(|y| (r.left()..r.right()).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n")
        .collect()
}

fn block_cells(app: &App, t: &Terminal<TestBackend>) -> usize {
    let r = app.regions().preview;
    let buf = t.backend().buffer();
    (r.top()..r.bottom())
        .flat_map(|y| (r.left()..r.right()).map(move |x| (x, y)))
        .filter(|&p| matches!(buf[p].symbol(), "▀" | "▄" | "█"))
        .count()
}

fn press(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: crossterm::event::KeyModifiers::NONE,
    })
}

fn hit(app: &App, target: HitTarget) -> Option<ratatui::layout::Rect> {
    app.hits().iter().find(|(_, t)| *t == target).map(|(r, _)| *r)
}

/// The video plays in the preview pane as half-block frames with the status row; the
/// `Player` keys work while the pane has the keyboard (K, M, =, -, L, J), the mouse
/// clicks the mark and the bar, images off keeps the sound and the status, Quick Look
/// takes the same player (← seeks there, ↑↓ still step), the WAV plays sound only, and
/// hiding the pane closes the player. Screen-reader mode reads the state in words.
#[test]
fn video_plays_in_the_preview_pane_and_quick_look() {
    if !media_tests() {
        eprintln!("skipped: SOYUZ_MEDIA_TESTS is not 1");
        return;
    }
    match common::relaunch(module_path!(), "video_plays_in_the_preview_pane_and_quick_look", files) {
        Run::Child(data) => video_child(data),
        Run::Parent(_) => {}
    }
}

fn video_child(data: PathBuf) {
    let mut app = common::new_app(config(), &data);
    let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let mut io = NullIo::default();
    frames_until(&mut app, &mut t, &mut io, &[], "the listing", |a, _| {
        common::labels(a).iter().any(|l| l == "clip.avi")
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Home)], "the cursor on the clip", |a, _| {
        common::cursor(a).as_deref() == Some("clip.avi")
    });
    frames_until(&mut app, &mut t, &mut io, &[common::alt('p')], "the clip playing with frames", |a, t| {
        preview_player(a).is_some_and(|(s, pics, serial)| s.status == Status::Playing && pics == 1 && serial > 0)
            && block_cells(a, t) > 10
            && a.core.ui.player.is_open()
    });
    let pane = text_of(&t, app.regions().preview);
    assert!(pane.contains("/ 0:03"), "the duration in the status row:\n{pane}");
    assert!(pane.contains("80%") && pane.contains("loop"), "volume and loop:\n{pane}");
    assert!(pane.contains("o---"), "the bar (ASCII icons in the tests):\n{pane}");
    // The pane takes the keyboard on a click: the Player context.
    let r = app.regions().preview;
    frames_until(&mut app, &mut t, &mut io, &[press(r.x + 2, r.y + 2)], "the pane focused", |a, _| {
        a.focus() == TuiFocus::Preview && a.key_context() == KeyContext::Player
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Char('k'))], "paused", |a, _| {
        preview_player(a).is_some_and(|(s, ..)| s.status == Status::Paused)
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Char('m'))], "muted", |a, _| {
        preview_player(a).is_some_and(|(s, ..)| s.muted) && a.core.config.media.muted
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Char('='))], "louder", |a, _| {
        preview_player(a).is_some_and(|(s, ..)| s.volume == 85) && a.core.config.media.volume == 85
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Char('-'))], "quieter", |a, _| {
        preview_player(a).is_some_and(|(s, ..)| s.volume == 80)
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Char('m'))], "unmuted", |a, _| {
        preview_player(a).is_some_and(|(s, ..)| !s.muted)
    });
    // Seeks while paused: one step (1 s) forward, then back to the start.
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Char('l'))], "a step forward", |a, _| {
        preview_player(a).is_some_and(|(s, ..)| s.status == Status::Paused && s.pos_ds >= 9)
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Char('j'))], "a step back", |a, _| {
        preview_player(a).is_some_and(|(s, ..)| s.pos_ds < 9)
    });
    // The mouse: three quarters along the bar seeks there (the very end would loop back
    // to the start), the mark plays.
    let bar = hit(&app, HitTarget::PlayerSeek).expect("the bar target");
    let x = bar.x + (bar.width - 1) * 3 / 4;
    frames_until(&mut app, &mut t, &mut io, &[press(x, bar.y)], "seeked to three quarters", |a, _| {
        preview_player(a).is_some_and(|(s, ..)| (15..30).contains(&s.pos_ds))
    });
    let mark = hit(&app, HitTarget::PlayerPlay).expect("the mark target");
    frames_until(&mut app, &mut t, &mut io, &[press(mark.x, mark.y)], "playing again", |a, _| {
        preview_player(a).is_some_and(|(s, ..)| s.status == Status::Playing)
    });
    // Screen-reader mode: the state in words on the status row.
    app.core.config.tui.screen_reader = true;
    frames_until(&mut app, &mut t, &mut io, &[], "the words", |a, t| {
        a.screen_reader() && text_of(t, a.regions().status).contains("Playing, 0:0")
    });
    let status = text_of(&t, app.regions().status);
    assert!(status.contains("volume 80 %") && status.contains("loop"), "{status}");
    app.core.config.tui.screen_reader = false;
    // Images off: the sound plays on, a text line stands in for the frames.
    app.core.config.tui.images = TuiImages::Off;
    frames_until(&mut app, &mut t, &mut io, &[], "images off", |a, t| {
        let pane = text_of(t, a.regions().preview);
        pane.contains("Pictures are off") && pane.contains("/ 0:03") && block_cells(a, t) == 0
    });
    assert!(app.core.ui.player.is_open(), "the player stays open without pictures");
    app.core.config.tui.images = TuiImages::Halfblocks;
    // Quick Look takes the same player: Left seeks (the Player context), Down steps.
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Esc)], "the list has the keyboard", |a, _| {
        a.focus() == TuiFocus::List
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Char(' '))], "Quick Look on the clip", |a, _| {
        a.focus() == TuiFocus::QuickLook
            && a.key_context() == KeyContext::Player
            && quicklook_player(a).is_some_and(|s| s.status == Status::Playing)
            && a.view().is_some_and(|v| v.quicklook.hints.contains("play / pause"))
    });
    assert!(app.core.ui.player.is_open());
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Char('k'))], "paused in Quick Look", |a, _| {
        quicklook_player(a).is_some_and(|s| s.status == Status::Paused)
    });
    let before = quicklook_player(&app).map(|s| s.pos_ds).unwrap();
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Right)], "a seek in Quick Look", |a, _| {
        quicklook_player(a).is_some_and(|s| s.pos_ds != before) && common::cursor(a).as_deref() == Some("clip.avi")
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Down)], "the WAV in Quick Look", |a, _| {
        common::cursor(a).as_deref() == Some("tone.wav")
            && quicklook_player(a).is_some_and(|s| s.status == Status::Playing && !s.video)
            && a.view().is_some_and(|v| v.quicklook.body_text().iter().any(|l| l == "tone.wav"))
    });
    frames_until(
        &mut app,
        &mut t,
        &mut io,
        &[common::key(KeyCode::Esc)],
        "Quick Look closed, the pane plays the WAV",
        |a, _| {
            a.focus() == TuiFocus::List
                && preview_player(a).is_some_and(|(s, pics, _)| s.status == Status::Playing && pics == 0)
        },
    );
    let pane = text_of(&t, app.regions().preview);
    assert!(pane.contains("/ 0:02"), "the WAV's duration:\n{pane}");
    // Hiding the pane closes the player.
    frames_until(&mut app, &mut t, &mut io, &[common::alt('p')], "the pane hidden", |a, _| {
        a.regions().preview.is_empty() && !a.core.ui.player.is_open()
    });
    app.finish();
    common::ok();
}

/// `[media] player = false`: the clip previews as before (poster frame or details), no
/// player opens. Runs with or without the libraries.
#[test]
fn player_off_previews_without_playing() {
    match common::relaunch(module_path!(), "player_off_previews_without_playing", files) {
        Run::Child(data) => off_child(data),
        Run::Parent(_) => {}
    }
}

fn off_child(data: PathBuf) {
    let mut c = config();
    c.media.player = false;
    let mut app = common::new_app(c, &data);
    let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let mut io = NullIo::default();
    frames_until(&mut app, &mut t, &mut io, &[], "the listing", |a, _| {
        common::labels(a).iter().any(|l| l == "clip.avi")
    });
    frames_until(&mut app, &mut t, &mut io, &[common::key(KeyCode::Home)], "the cursor on the clip", |a, _| {
        common::cursor(a).as_deref() == Some("clip.avi")
    });
    frames_until(&mut app, &mut t, &mut io, &[common::alt('p')], "the clip's preview", |a, _| {
        a.view().is_some_and(|v| {
            v.preview.visible
                && !matches!(v.preview.body, Body::Loading | Body::Nothing)
                && v.preview.info.iter().any(|(k, _)| k == "Type")
        })
    });
    // A few more frames: nothing opens.
    for _ in 0..5 {
        app.frame(&mut t, &[], &mut io).unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(preview_player(&app).is_none(), "{:?}", app.view().map(|v| v.preview.body_text()));
    assert!(!app.core.ui.player.is_open());
    app.finish();
    common::ok();
}
