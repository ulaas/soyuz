//! "Checksums" tab: MD5 / SHA-1 / SHA-256 / BLAKE3 / CRC-32 computed on worker threads
//! with progress (`soyuz_fs::hash_file`, pause-free but cancellable), copy buttons, and a
//! compare field that accepts a pasted hash (or a `sha256sum` line) and shows a match /
//! mismatch badge, starting the matching algorithm automatically.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align, CornerRadius, FontId, Layout, Margin, RichText, Sense, TextEdit, Ui, vec2};
use parking_lot::Mutex;
use soyuz_core::{ByteProgress, HashAlgo, OpControl};

use super::properties_widgets::{badge, caption, copy_button, small_icon_button};
use crate::cmd::Cmd;
use crate::theme::{icons::ph, palette};

/// Display order.
pub const ALGOS: [HashAlgo; 5] = [HashAlgo::Md5, HashAlgo::Sha1, HashAlgo::Sha256, HashAlgo::Blake3, HashAlgo::Crc32];

#[derive(Clone, Debug, PartialEq)]
enum HashState {
    Idle,
    Running { done: u64, total: u64 },
    Done(String),
    Failed(String),
}

struct Row {
    algo: HashAlgo,
    state: Arc<Mutex<HashState>>,
    ctl: Option<Arc<OpControl>>,
}

pub struct Checksums {
    path: PathBuf,
    rows: Vec<Row>,
    pub compare: String,
    pub focus_compare: bool,
}

/// Lower-case hex without whitespace; a `<hash>  <file>` line keeps only the hash.
pub fn normalize_hash(s: &str) -> String {
    let first = s.split_whitespace().next().unwrap_or("");
    let first = first.trim_start_matches('\\');
    first.trim_start_matches("0x").to_ascii_lowercase()
}

/// Algorithms whose digest has this many hex digits.
pub fn candidates(hex_len: usize) -> Vec<HashAlgo> {
    match hex_len {
        8 => vec![HashAlgo::Crc32],
        32 => vec![HashAlgo::Md5],
        40 => vec![HashAlgo::Sha1],
        64 => vec![HashAlgo::Sha256, HashAlgo::Blake3],
        _ => Vec::new(),
    }
}

/// Outcome of comparing a pasted hash with the computed ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Empty,
    NotHex,
    UnknownLength,
    Pending,
    Match(HashAlgo),
    Mismatch,
}

pub fn verdict(input: &str, computed: &[(HashAlgo, Option<String>)]) -> Verdict {
    let h = normalize_hash(input);
    if h.is_empty() {
        return Verdict::Empty;
    }
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Verdict::NotHex;
    }
    let cands = candidates(h.len());
    if cands.is_empty() {
        return Verdict::UnknownLength;
    }
    let mut pending = false;
    for a in cands {
        match computed.iter().find(|(x, _)| *x == a).and_then(|(_, v)| v.as_ref()) {
            Some(v) if v.eq_ignore_ascii_case(&h) => return Verdict::Match(a),
            Some(_) => {}
            None => pending = true,
        }
    }
    if pending { Verdict::Pending } else { Verdict::Mismatch }
}

impl Checksums {
    pub fn new(path: PathBuf) -> Self {
        let rows =
            ALGOS.iter().map(|a| Row { algo: *a, state: Arc::new(Mutex::new(HashState::Idle)), ctl: None }).collect();
        Checksums { path, rows, compare: String::new(), focus_compare: false }
    }

    pub fn start(&mut self, algo: HashAlgo, ctx: &egui::Context) {
        let Some(row) = self.rows.iter_mut().find(|r| r.algo == algo) else { return };
        if matches!(*row.state.lock(), HashState::Running { .. } | HashState::Done(_)) {
            return;
        }
        let ctl = OpControl::new();
        row.ctl = Some(ctl.clone());
        *row.state.lock() = HashState::Running { done: 0, total: 0 };
        let state = row.state.clone();
        let path = self.path.clone();
        let ctx = ctx.clone();
        let spawn = std::thread::Builder::new().name("soyuz-hash".into()).spawn(move || {
            let mut last = Instant::now();
            let progress_state = state.clone();
            let progress_ctx = ctx.clone();
            let mut sink = move |p: ByteProgress| {
                *progress_state.lock() = HashState::Running { done: p.done, total: p.total };
                if last.elapsed() > Duration::from_millis(80) {
                    last = Instant::now();
                    progress_ctx.request_repaint();
                }
            };
            let r = soyuz_fs::hash_file(&path, algo, &ctl, &mut sink);
            *state.lock() = match r {
                Ok(h) => HashState::Done(h),
                Err(e) if e.is_cancel() => HashState::Idle,
                Err(e) => HashState::Failed(e.to_string()),
            };
            ctx.request_repaint();
        });
        if spawn.is_err() {
            *row.state.lock() = HashState::Failed("cannot start a worker thread".into());
        }
    }

    pub fn start_all(&mut self, ctx: &egui::Context) {
        for a in ALGOS {
            self.start(a, ctx);
        }
    }

    pub fn cancel_all(&mut self) {
        for r in &mut self.rows {
            if let Some(c) = r.ctl.take() {
                c.cancel();
            }
        }
    }

    fn computed(&self) -> Vec<(HashAlgo, Option<String>)> {
        self.rows
            .iter()
            .map(|r| {
                (
                    r.algo,
                    match &*r.state.lock() {
                        HashState::Done(h) => Some(h.clone()),
                        _ => None,
                    },
                )
            })
            .collect()
    }

    pub fn show(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let p = palette(ui.ctx());
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            caption(ui, "Checksums");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let any_idle =
                    self.rows.iter().any(|r| matches!(*r.state.lock(), HashState::Idle | HashState::Failed(_)));
                if any_idle && ui.add(egui::Button::new(RichText::new("Compute all").size(12.0))).clicked() {
                    self.start_all(&ctx);
                }
            });
        });
        let mut start = None;
        let mut cancel = None;
        let matched = match verdict(&self.compare, &self.computed()) {
            Verdict::Match(a) => Some(a),
            _ => None,
        };
        for row in &self.rows {
            let state = row.state.lock().clone();
            let hit = matched == Some(row.algo);
            egui::Frame::new()
                .fill(if hit { p.success.gamma_multiply(0.12) } else { p.bg_surface2 })
                .stroke(egui::Stroke::new(1.0, if hit { p.success } else { egui::Color32::TRANSPARENT }))
                .corner_radius(CornerRadius::same(crate::theme::tokens::radius::ROW))
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            vec2(62.0, 20.0),
                            egui::Label::new(
                                RichText::new(row.algo.label())
                                    .font(FontId::proportional(12.0))
                                    .color(p.text_secondary),
                            ),
                        );
                        match &state {
                            HashState::Idle => {
                                ui.label(RichText::new("Not computed").color(p.text_tertiary));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if small_icon_button(ui, ph::PLAY, "Compute").clicked() {
                                        start = Some(row.algo);
                                    }
                                });
                            }
                            HashState::Running { done, total } => {
                                let frac = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if small_icon_button(ui, ph::X, "Cancel").clicked() {
                                        cancel = Some(row.algo);
                                    }
                                    ui.label(
                                        RichText::new(format!("{:.0}%", frac * 100.0))
                                            .font(FontId::proportional(11.0))
                                            .color(p.text_tertiary),
                                    );
                                    let (r, _) =
                                        ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
                                    ui.painter().rect_filled(r, CornerRadius::same(3), p.bg_surface3);
                                    let mut f = r;
                                    f.set_width((r.width() * frac).max(4.0));
                                    ui.painter().rect_filled(f, CornerRadius::same(3), p.accent);
                                });
                            }
                            HashState::Done(h) => {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(h).font(FontId::monospace(11.5)).color(p.text_primary),
                                    )
                                    .truncate()
                                    .selectable(true),
                                )
                                .on_hover_text(h);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    copy_button(ui, h, cmds);
                                });
                            }
                            HashState::Failed(e) => {
                                ui.label(RichText::new(e).color(p.danger));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if small_icon_button(ui, ph::ARROW_CLOCKWISE, "Retry").clicked() {
                                        start = Some(row.algo);
                                    }
                                });
                            }
                        }
                    });
                });
            ui.add_space(4.0);
        }
        if let Some(a) = start {
            self.start(a, &ctx);
        }
        if let Some(a) = cancel
            && let Some(r) = self.rows.iter_mut().find(|r| r.algo == a)
            && let Some(c) = r.ctl.take()
        {
            c.cancel();
        }

        caption(ui, "Compare");
        let r = ui.add(
            TextEdit::singleline(&mut self.compare)
                .hint_text("Paste a checksum to verify (MD5, SHA-1, SHA-256, BLAKE3 or CRC-32)")
                .font(FontId::monospace(12.0))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 6)),
        );
        if self.focus_compare {
            r.request_focus();
            self.focus_compare = false;
        }
        let v = verdict(&self.compare, &self.computed());
        if v == Verdict::Pending {
            // Start whatever could match.
            for a in candidates(normalize_hash(&self.compare).len()) {
                self.start(a, &ctx);
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| match v {
            Verdict::Empty => {}
            Verdict::NotHex => {
                badge(ui, Some(ph::WARNING), "Not a hexadecimal checksum", p.warning);
            }
            Verdict::UnknownLength => {
                badge(ui, Some(ph::WARNING), "Unrecognized checksum length", p.warning);
            }
            Verdict::Pending => {
                badge(ui, Some(ph::HOURGLASS), "Computing\u{2026}", p.info);
            }
            Verdict::Match(a) => {
                badge(ui, Some(ph::CHECK_CIRCLE), &format!("Match \u{00b7} {}", a.label()), p.success);
            }
            Verdict::Mismatch => {
                badge(ui, Some(ph::X_CIRCLE), "Mismatch \u{00b7} the file differs", p.danger);
            }
        });
    }
}

impl Drop for Checksums {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_and_verdict() {
        assert_eq!(normalize_hash("  ABCD  file.txt\n"), "abcd");
        assert_eq!(normalize_hash("0xDEADBEEF"), "deadbeef");
        assert_eq!(normalize_hash("\\abc  f"), "abc");
        let md5 = "d41d8cd98f00b204e9800998ecf8427e".to_string();
        let computed = vec![(HashAlgo::Md5, Some(md5.clone())), (HashAlgo::Sha256, None), (HashAlgo::Blake3, None)];
        assert_eq!(verdict("", &computed), Verdict::Empty);
        assert_eq!(verdict("xyz", &computed), Verdict::NotHex);
        assert_eq!(verdict("abc", &computed), Verdict::UnknownLength);
        assert_eq!(verdict(&md5.to_uppercase(), &computed), Verdict::Match(HashAlgo::Md5));
        assert_eq!(verdict("00000000000000000000000000000000", &computed), Verdict::Mismatch);
        assert_eq!(verdict(&"a".repeat(64), &computed), Verdict::Pending);
        assert_eq!(candidates(64), vec![HashAlgo::Sha256, HashAlgo::Blake3]);
    }

    #[test]
    fn computes_real_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.txt");
        std::fs::write(&f, b"abc").unwrap();
        let ctx = egui::Context::default();
        let mut c = Checksums::new(f);
        c.start(HashAlgo::Md5, &ctx);
        c.start(HashAlgo::Sha256, &ctx);
        let deadline = Instant::now() + Duration::from_secs(10);
        while c.computed().iter().filter(|(_, v)| v.is_some()).count() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let got = c.computed();
        assert_eq!(got[0].1.as_deref(), Some("900150983cd24fb0d6963f7d28e17f72"));
        assert_eq!(got[2].1.as_deref(), Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    }
}
