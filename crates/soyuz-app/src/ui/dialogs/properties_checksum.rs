//! "Checksums" tab: MD5 / SHA-1 / SHA-256 / BLAKE3 / CRC-32 computed on worker threads
//! with progress (`soyuz_fs::hash_file`, pause-free but cancellable), copy buttons, and a
//! compare field that accepts a pasted hash (or a `sha256sum` line) and shows a match /
//! mismatch badge, starting the matching algorithm automatically. The hashing and the
//! verdict are the core's (`models::checksums`, shared with the terminal UI).

use std::path::PathBuf;

use egui::{Align, CornerRadius, FontId, Layout, Margin, RichText, Sense, TextEdit, Ui, vec2};
use soyuz_core::HashAlgo;

use super::properties_widgets::{badge, caption, copy_button, small_icon_button};
use crate::cmd::Cmd;
pub use crate::models::checksums::{ALGOS, Verdict, candidates, normalize_hash, verdict};
use crate::models::checksums::{ChecksumSet, HashState};
use crate::theme::{icons::ph, palette};

pub struct Checksums {
    set: ChecksumSet,
    pub compare: String,
    pub focus_compare: bool,
}

impl Checksums {
    pub fn new(path: PathBuf) -> Self {
        Checksums { set: ChecksumSet::new(path), compare: String::new(), focus_compare: false }
    }

    pub fn start(&mut self, algo: HashAlgo, ctx: &egui::Context) {
        self.set.start(algo, &crate::host::egui_wake(ctx));
    }

    pub fn start_all(&mut self, ctx: &egui::Context) {
        self.set.start_all(&crate::host::egui_wake(ctx));
    }

    pub fn cancel_all(&mut self) {
        self.set.cancel_all();
    }

    fn computed(&self) -> Vec<(HashAlgo, Option<String>)> {
        self.set.computed()
    }

    pub fn show(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let p = palette(ui.ctx());
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            caption(ui, tr!("dialogs-props-tab-checksums"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let any_idle = self.set.any_idle();
                if any_idle
                    && ui.add(egui::Button::new(RichText::new(tr!("dialogs-hash-compute-all")).size(12.0))).clicked()
                {
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
        for algo in ALGOS {
            let state = self.set.state(algo);
            let hit = matched == Some(algo);
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
                                RichText::new(algo.label()).font(FontId::proportional(12.0)).color(p.text_secondary),
                            ),
                        );
                        match &state {
                            HashState::Idle => {
                                ui.label(RichText::new(tr!("dialogs-hash-not-computed")).color(p.text_tertiary));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if small_icon_button(ui, ph::PLAY, tr!("dialogs-hash-compute")).clicked() {
                                        start = Some(algo);
                                    }
                                });
                            }
                            HashState::Running { done, total } => {
                                let frac = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if small_icon_button(ui, ph::X, tr!("common-cancel")).clicked() {
                                        cancel = Some(algo);
                                    }
                                    ui.label(
                                        RichText::new(tr!(
                                            "dialogs-hash-percent",
                                            percent = format!("{:.0}", frac * 100.0)
                                        ))
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
                                    if small_icon_button(ui, ph::ARROW_CLOCKWISE, tr!("common-retry")).clicked() {
                                        start = Some(algo);
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
        if let Some(a) = cancel {
            self.set.cancel(a);
        }

        caption(ui, tr!("dialogs-hash-compare"));
        let r = ui.add(
            TextEdit::singleline(&mut self.compare)
                .hint_text(tr!("dialogs-hash-compare-hint"))
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
                badge(ui, Some(ph::WARNING), tr!("dialogs-hash-not-hex"), p.warning);
            }
            Verdict::UnknownLength => {
                badge(ui, Some(ph::WARNING), tr!("dialogs-hash-bad-length"), p.warning);
            }
            Verdict::Pending => {
                badge(ui, Some(ph::HOURGLASS), tr!("dialogs-hash-computing"), p.info);
            }
            Verdict::Match(a) => {
                badge(ui, Some(ph::CHECK_CIRCLE), &tr!("dialogs-hash-match", algo = a.label()), p.success);
            }
            Verdict::Mismatch => {
                badge(ui, Some(ph::X_CIRCLE), tr!("dialogs-hash-mismatch"), p.danger);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

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
