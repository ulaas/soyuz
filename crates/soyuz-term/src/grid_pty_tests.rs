//! [`GridSnapshot`] and the toolkit-free input API on a real PTY (ConPTY on Windows,
//! openpty on unix): SGR colours and decorations, wide characters, the cursor, the
//! selection, links and input modes. A machine where no PTY / shell can start skips
//! these with a message instead of failing.

use std::time::{Duration, Instant};

use super::*;

fn noop_wake() -> Wake {
    std::sync::Arc::new(|| {})
}

/// Run a script printing escape sequences, then idling: `sh -c` on unix,
/// `powershell -EncodedCommand` on Windows (the ESC byte is `$e` there).
fn spawn_script(unix: &str, windows: &str, id: u64) -> Option<Terminal> {
    let settings = if cfg!(windows) {
        let shell = if pty::which("pwsh").is_some() { "pwsh" } else { "powershell" };
        let script = format!(
            "$e = [char]27; [Console]::OutputEncoding = [Text.Encoding]::UTF8; {windows}; Start-Sleep -Seconds 20"
        );
        TermSettings {
            shell: Some(shell.into()),
            args: vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-EncodedCommand".into(),
                pty::encode_ps_command(&script),
            ],
            shell_integration: false,
            cwd: std::env::temp_dir(),
            ..Default::default()
        }
    } else {
        TermSettings {
            shell: Some("sh".into()),
            args: vec!["-c".into(), format!("{unix}; sleep 20")],
            shell_integration: false,
            cwd: std::env::temp_dir(),
            ..Default::default()
        }
    };
    match Terminal::spawn(settings, noop_wake(), id) {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("no PTY / shell here ({e}); skipping");
            None
        }
    }
}

/// Snapshot once `want` is on screen (and the output has settled).
fn wait_grid(term: &Terminal, want: &str, pal: &TermPalette) -> GridSnapshot {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        let g = term.snapshot(pal, GridStyle::default());
        if g.text().contains(want) && term.idle_for() > Duration::from_millis(200) {
            return g;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("`{want}` never appeared:\n{}", term.screen_text());
}

fn find(g: &GridSnapshot, needle: &str) -> (usize, usize) {
    for r in 0..g.rows {
        let cols: Vec<usize> = (0..g.cols).filter(|&c| !g.row(r)[c].spacer).collect();
        let text: String = cols.iter().map(|&c| g.row(r)[c].c).collect();
        if let Some(i) = text.find(needle) {
            let chars = text[..i].chars().count();
            return (r, cols[chars]);
        }
    }
    panic!("`{needle}` not in the grid:\n{}", g.text());
}

#[test]
fn grid_snapshot_sgr_wide_chars_cursor_and_selection() {
    let unix = r"printf '\033[1;31mRED\033[0m \033[4mUND\033[0m \033[7mINV\033[0m \346\274\242x end\n'; printf 'ab'";
    let windows = "[Console]::Write($e + '[1;31mRED' + $e + '[0m ' + $e + '[4mUND' + $e + '[0m ' + $e + '[7mINV' + $e + '[0m ' + [char]0x6F22 + 'x end' + [char]13 + [char]10 + 'ab')";
    let Some(term) = spawn_script(unix, windows, 101) else { return };
    term.resize(60, 10, 8, 16);
    let pal = TermPalette::dark();
    let g = wait_grid(&term, "x end", &pal);
    assert_eq!((g.cols, g.rows), (60, 10));
    assert_eq!(g.cells.len(), 600);

    // Bold red is drawn bright (bright_bold), with the bold flag.
    let (r, c) = find(&g, "RED");
    let red = &g.row(r)[c];
    assert_eq!(red.c, 'R');
    assert!(red.bold, "{red:?}");
    assert_eq!(red.fg, pal.ansi[9], "{red:?}");
    // Without bright_bold the same cell is plain red.
    let plain = term.snapshot(&pal, GridStyle { bright_bold: false });
    assert_eq!(plain.row(r)[c].fg, pal.ansi[1]);
    // Underline; inverse swaps the default colours.
    let (ur, uc) = find(&g, "UND");
    assert!(g.row(ur)[uc].underline && !g.row(ur)[uc].bold);
    let (ir, ic) = find(&g, "INV");
    assert_eq!((g.row(ir)[ic].fg, g.row(ir)[ic].bg), (pal.bg, pal.fg));
    // The space after RED has the default background.
    assert_eq!(g.row(r)[c + 3].bg, pal.bg);
    assert_eq!(g.default_bg, pal.bg);
    assert_eq!(g.cursor_color, pal.cursor);

    // A wide character covers two cells: the glyph, then a spacer.
    let (wr, wc) = find(&g, "\u{6F22}x");
    assert!(g.row(wr)[wc].wide && !g.row(wr)[wc].spacer);
    assert!(g.row(wr)[wc + 1].spacer);
    assert_eq!(g.row(wr)[wc + 2].c, 'x');
    assert!(g.row_text(wr).contains("\u{6F22}x end"), "{}", g.row_text(wr));

    // The cursor sits after `ab` on the next row.
    let (ar, ac) = find(&g, "ab");
    let cur = g.cursor.expect("cursor in the viewport");
    assert_eq!((cur.row, cur.col), (ar, ac + 2), "{}", g.text());
    assert_eq!(g.display_offset, 0);

    // Selection: a drag over RED, then a double-click (word) on UND.
    term.select_begin(r, c, false, 1);
    term.select_extend(r, c + 2, true);
    let s = term.snapshot(&pal, GridStyle::default());
    assert_eq!(s.selection, vec![(r, c, c + 2)]);
    assert!(s.is_selected(r, c + 1) && !s.is_selected(r, c + 3));
    assert_eq!(term.selected_text().as_deref(), Some("RED"));
    term.select_begin(ur, uc + 1, false, 2);
    assert_eq!(term.selected_text().as_deref(), Some("UND"));
    term.select_begin(ar, ac, false, 3);
    assert!(term.selected_text().is_some_and(|t| t.starts_with("ab")));
    // A click without a drag leaves an empty selection, dropped on release.
    term.select_begin(r, c, false, 1);
    term.select_finish();
    assert_eq!(term.selected_text(), None);
    term.select_begin(r, c, false, 1);
    term.select_extend(r, c + 1, true);
    term.select_clear();
    assert!(term.snapshot(&pal, GridStyle::default()).selection.is_empty());

    // Out-of-range cells are ignored.
    assert_eq!(term.link_at(99, 0), None);
    assert!(g.cell(0, 60).is_none());
    term.kill();
}

#[test]
fn links_in_the_grid() {
    let unix = r"printf 'see https://example.com/a_b ok\n'; printf '\033]8;;https://soyuz.test/x\033\\LINK\033]8;;\033\\ after\n'";
    let windows = "[Console]::Write('see https://example.com/a_b ok' + [char]13 + [char]10)";
    let Some(term) = spawn_script(unix, windows, 102) else { return };
    term.resize(60, 8, 8, 16);
    let pal = TermPalette::light();
    let g = wait_grid(&term, "ok", &pal);
    let (r, c) = find(&g, "example");
    let link = term.link_at(r, c + 2).expect("plain-text URL");
    assert_eq!(link.url, "https://example.com/a_b");
    let (_, start) = find(&g, "https");
    assert_eq!(link.spans, vec![(r, start, start + "https://example.com/a_b".len() - 1)]);
    assert_eq!(term.link_at(r, 0), None, "`see` is not a link");
    // OSC 8 hyperlinks (unix only: ConPTY does not forward them on every Windows build).
    if cfg!(unix) {
        let g = wait_grid(&term, "after", &pal);
        let (hr, hc) = find(&g, "LINK");
        assert_eq!(g.row(hr)[hc].hyperlink.as_deref(), Some("https://soyuz.test/x"));
        assert_eq!(g.row(hr)[hc + 5].hyperlink, None);
        let l = term.link_at(hr, hc + 1).expect("OSC 8 link");
        assert_eq!(l.url, "https://soyuz.test/x");
        assert_eq!(l.spans, vec![(hr, hc, hc + 3)]);
    }
    term.kill();
}

/// Input modes set by the program change what keys and the mouse send; focus reports
/// follow `set_focused`. Unix only: ConPTY filters private modes differently per build.
#[cfg(unix)]
#[test]
fn modes_mouse_and_focus_reports() {
    // DECCKM, bracketed paste, SGR click reporting, focus reports; then echo what the
    // program receives (raw, no echo) as hex.
    let script = r"printf '\033[?1h\033[?2004h\033[?1000h\033[?1006h\033[?1004hREADY\n'; stty raw -echo; dd bs=1 count=13 2>/dev/null | od -An -tx1";
    let Some(term) = spawn_script(script, "", 103) else { return };
    term.resize(80, 12, 8, 16);
    let pal = TermPalette::dark();
    wait_grid(&term, "READY", &pal);
    let m = term.modes();
    assert!(m.app_cursor && m.bracketed_paste && m.mouse_click && m.sgr_mouse && m.focus_in_out, "{m:?}");
    assert!(!m.alt_screen && !m.mouse_motion);
    assert_eq!(input::encode_key(TermKey::Up, TermMods::NONE, &m), Some(b"\x1bOA".to_vec()));

    // A click is reported; plain motion is not (no motion mode).
    assert!(term.report_mouse(MouseButton::Left, true, false, 2, 1, TermMods::NONE));
    assert!(term.report_mouse(MouseButton::None, false, true, 3, 1, TermMods::NONE));
    // The first set_focused only records the state; a change sends a report.
    term.set_focused(true);
    term.set_focused(false);
    term.input(b"Z\n".to_vec());
    // ESC [ < 0 ; 3 ; 2 M  then  ESC [ O  then  Z.
    let want = "1b 5b 3c 30 3b 33 3b 32 4d 1b 5b 4f 5a";
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) && !term.screen_text().contains(want) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(term.screen_text().contains(want), "{}", term.screen_text());
    term.kill();
}

#[test]
fn modes_without_a_program_request() {
    let Some(term) = spawn_script("printf 'READY\\n'", "[Console]::Write('READY')", 104) else { return };
    let pal = TermPalette::dark();
    wait_grid(&term, "READY", &pal);
    let m = term.modes();
    assert!(!m.mouse_any() && !m.alt_screen, "{m:?}");
    // No mouse mode: the front end handles the mouse itself.
    assert!(!term.report_mouse(MouseButton::Left, true, false, 0, 0, TermMods::NONE));
    // Not a shell prompt.
    assert!(!term.at_prompt());
    term.scroll_page(true);
    term.scroll_to_bottom();
    assert_eq!(term.snapshot(&pal, GridStyle::default()).display_offset, 0);
    term.kill();
}
