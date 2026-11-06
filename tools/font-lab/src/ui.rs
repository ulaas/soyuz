//! The window: axes on the left, cards in the middle, inspector and picks on the right.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, mpsc};

use egui::emath::GuiRounding;
use egui::epaint::text::{FontTweak, HintingTarget, SmoothHinting};
use egui::{
    Align2, Color32, ColorImage, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Panel, Rect, RichText,
    ScrollArea, Sense, Stroke, TextureHandle, TextureOptions, pos2, vec2,
};

use crate::raster::{self, Gpu, PAD};
use crate::spec::*;

const MAX_CARDS: usize = 500_000;
const CACHE_CARDS: usize = 1500;

const SAMPLES: [(&str, &str); 8] = [
    (
        "File list",
        "Documents\nbudget_2026-Q3.xlsx  48.2 KB  Nov 3, 14:07\nIMG_20240817_193522.jpg  3.1 MB\nsoyuz-0.6.0-x86_64.tar.gz\nThe quick brown fox.txt  1,024 B",
    ),
    ("Sentence", "Keyboard-first, never blocks the window: pause, resume and undo every job."),
    ("Pangram", "Sphinx of black quartz, judge my vow. PACK MY BOX WITH FIVE DOZEN JUGS."),
    ("Hard letters", "Illegal 1lI|O0Q 8B6G rn m cl d vv w ffi fl tt ry .,:;'\" ()[]{}"),
    ("Numbers", "0123456789  12,345.67 MB  100%  2026-11-06  23:59:01"),
    ("Menu", "Open with...\nCopy path\nCompress to zip\nProperties    Alt+Enter"),
    ("Code", "fn main() {\n    let path = \"C:/Users/ann\"; // 0O 1lI\n    println!(\"{path:?} != {}\", 42);\n}"),
    ("Turkish and accents", "Şahin ığüşöç İĞÜŞÖÇ, àéîõü ñ ß Æ Ø"),
];

const PRESETS: [&str; 17] = [
    "Start here",
    "Your pick",
    "Soyuz now",
    "Hinting",
    "Curve, dark",
    "Curve, light",
    "All fonts",
    "Fonts and hinting",
    "Monospace",
    "Sizes",
    "Weights",
    "All techniques",
    "Darkening",
    "LCD and DirectWrite",
    "Light theme",
    "Display scale",
    "Broad sweep",
];

struct Job {
    key: Key,
    spec: Spec,
    text: Arc<str>,
    w: u32,
    h: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    spec: Spec,
    text: Arc<str>,
    w: u32,
    h: u32,
}

type Done = (Key, Result<ColorImage, String>);

struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    wake: Condvar,
}

/// Which values of each axis are on.
struct Axes {
    fonts: Vec<bool>,
    sizes: Vec<bool>,
    techs: Vec<bool>,
    hints: Vec<bool>,
    bins: [bool; 2],
    gammas: Vec<bool>,
    themes: Vec<bool>,
    weights: Vec<bool>,
    scales: Vec<bool>,
}

pub struct Lab {
    fonts: Arc<Vec<FontEntry>>,
    axes: Axes,
    native_scale: u16,
    list: Vec<Spec>,
    /// Line height of the largest sample in `list`, in pixels.
    tallest: u32,
    truncated: bool,
    dirty: bool,
    sample: usize,
    custom: String,
    card_w: f32,
    filter: String,
    only_picks: bool,
    queue: Arc<Queue>,
    rx: mpsc::Receiver<Done>,
    cache: HashMap<Key, (Result<TextureHandle, String>, u64)>,
    in_flight: HashSet<Key>,
    frame: u64,
    picks: BTreeSet<String>,
    picks_file: PathBuf,
    hovered: Option<Spec>,
    zoom: f32,
    renderer_info: String,
    preset_name: String,
    shot: Option<String>,
    shot_sent: bool,
    settled: u32,
    scroll: Option<f32>,
    /// Draw real window text beside the first card, to compare their pixels.
    verify: bool,
    /// `--hover`: the sample the inspector shows when there is no pointer to hover with.
    pinned_hover: Option<Spec>,
}

impl Lab {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        preset: Option<String>,
        shot: Option<String>,
        sample: Option<String>,
        scroll: Option<f32>,
        verify: bool,
        only_picks: bool,
        hover: Option<String>,
    ) -> Self {
        let fonts = Arc::new(fonts());
        install_chrome(&cc.egui_ctx, &fonts[0]);

        let state = cc.wgpu_render_state.as_ref().expect("the wgpu renderer");
        let info = state.adapter.get_info();
        let renderer_info = format!("{:?}, {} ({:?}), egui 0.37", info.backend, info.name, state.target_format);
        let queue = Arc::new(Queue { jobs: Mutex::new(VecDeque::new()), wake: Condvar::new() });
        let (tx, rx) = mpsc::channel();
        for n in 0..2 {
            let (queue, tx, fonts) = (queue.clone(), tx.clone(), fonts.clone());
            let (adapter, ctx) = (state.adapter.clone(), cc.egui_ctx.clone());
            std::thread::Builder::new()
                .name(format!("font-lab-{n}"))
                .spawn(move || worker(&queue, &tx, &fonts, &adapter, &ctx))
                .expect("a worker thread");
        }

        let native = cc.egui_ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0);
        let native_scale = ((native * 100.0).round() as u16).max(50);
        let picks_file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("picks.txt");
        let picks = std::fs::read_to_string(&picks_file)
            .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
            .unwrap_or_default();
        let axes = Axes {
            fonts: vec![false; fonts.len()],
            sizes: vec![false; SIZES.len()],
            techs: vec![false; TECHS.len()],
            hints: vec![false; HINTS.len()],
            bins: [false; 2],
            gammas: vec![false; GAMMAS.len()],
            themes: vec![false; THEMES.len()],
            weights: vec![false; WEIGHTS.len()],
            scales: vec![false; SCALES.len()],
        };
        let mut lab = Self {
            fonts,
            axes,
            native_scale,
            list: Vec::new(),
            tallest: 20,
            truncated: false,
            dirty: true,
            sample: 0,
            custom: "Type your own sample here".into(),
            card_w: 320.0,
            filter: String::new(),
            only_picks: false,
            queue,
            rx,
            cache: HashMap::new(),
            in_flight: HashSet::new(),
            frame: 0,
            picks,
            picks_file,
            hovered: None,
            zoom: 4.0,
            renderer_info,
            preset_name: String::new(),
            shot,
            shot_sent: false,
            settled: 0,
            scroll,
            verify,
            pinned_hover: None,
        };
        lab.apply_preset(preset.as_deref().unwrap_or("Start here"));
        lab.only_picks = only_picks;
        lab.pinned_hover = hover.and_then(|h| Spec::parse(&h, &lab.fonts));
        if let Some(s) = sample
            && let Some(i) = SAMPLES.iter().position(|(n, _)| n.eq_ignore_ascii_case(&s))
        {
            lab.sample = i;
        }
        lab
    }

    /// Soyuz's own look: one value on each axis.
    fn base(&mut self) {
        let a = &mut self.axes;
        for v in [
            &mut a.fonts,
            &mut a.sizes,
            &mut a.techs,
            &mut a.hints,
            &mut a.gammas,
            &mut a.themes,
            &mut a.weights,
            &mut a.scales,
        ] {
            v.fill(false);
        }
        a.fonts[0] = true;
        a.sizes[DEFAULT_SIZE] = true;
        a.techs[0] = true;
        a.hints[DEFAULT_HINT] = true;
        a.bins = [false, true];
        a.gammas[G45] = true;
        a.themes[0] = true;
        a.weights[0] = true;
        let scale = SCALES.iter().position(|s| *s == self.native_scale).unwrap_or(0);
        a.scales[scale] = true;
    }

    fn apply_preset(&mut self, name: &str) {
        self.preset_name = name.to_string();
        self.base();
        let fonts = self.fonts.clone();
        let code = |c: &str| fonts.iter().position(|f| f.code == c);
        let a = &mut self.axes;
        match name {
            // IN13-LCD5-NM-B1-G80-DS, and the same look through the real pipeline.
            "Your pick" => {
                a.techs.fill(false);
                for (on, t) in a.techs.iter_mut().zip(TECHS.iter()) {
                    *on = matches!(t.code, "EL" | "LCD5");
                }
                a.hints.fill(false);
                a.hints[3] = true;
                a.bins = [false, true];
                a.gammas.fill(false);
                a.gammas[G80] = true;
                a.themes.fill(false);
                a.themes[1] = true;
            }
            "Soyuz now" => {
                a.bins = [false, true];
                for s in [2, 6, 9, 11, 13] {
                    a.sizes[s] = true;
                }
                a.themes[1] = true;
                a.themes[3] = true;
                a.themes[4] = true;
                a.gammas[G80] = true;
            }
            "Hinting" => {
                a.hints.fill(true);
                a.bins = [true, true];
            }
            "Curve, dark" => a.gammas.fill(true),
            "Curve, light" => {
                a.gammas.fill(true);
                a.themes.fill(false);
                a.themes[3] = true;
            }
            "All fonts" => {
                for (on, f) in a.fonts.iter_mut().zip(fonts.iter()) {
                    *on = !f.mono;
                }
            }
            "Fonts and hinting" => {
                for c in ["PS", "NS", "SG", "SV", "TA", "VE", "AR", "UB"] {
                    if let Some(i) = code(c) {
                        a.fonts[i] = true;
                    }
                }
                for i in [3, 4, 7] {
                    a.hints[i] = true;
                }
            }
            "Monospace" => {
                for (on, f) in a.fonts.iter_mut().zip(fonts.iter()) {
                    *on = f.mono;
                }
                a.sizes[4] = true;
                a.sizes[8] = true;
                self.sample = 6;
            }
            "Sizes" => a.sizes.fill(true),
            "Weights" => {
                a.fonts.fill(false);
                for c in ["IV", "IX", "SV", "SX"] {
                    if let Some(i) = code(c) {
                        a.fonts[i] = true;
                    }
                }
                a.weights.fill(true);
            }
            "All techniques" => a.techs.fill(true),
            "Darkening" => {
                for (on, t) in a.techs.iter_mut().zip(TECHS.iter()) {
                    *on |= t.group == "Stem darkening" || t.group == "Sharpening";
                }
                a.gammas[7] = true;
                a.gammas[12] = true;
            }
            "LCD and DirectWrite" => {
                for (on, t) in a.techs.iter_mut().zip(TECHS.iter()) {
                    *on |= t.group == "LCD subpixel" || t.group == "DirectWrite";
                }
                for g in [5, 7, G80, 12] {
                    a.gammas[g] = true;
                }
            }
            "Light theme" => {
                a.techs[1] = true;
                a.themes.fill(false);
                a.themes[3] = true;
                a.themes[4] = true;
                a.gammas.fill(false);
                for g in [7, 9, G80, 11, 12, 13] {
                    a.gammas[g] = true;
                }
            }
            "Display scale" => a.scales.fill(true),
            // Every font and technique, at the values of the other axes that matter most.
            // (All of every axis is tens of millions: the All buttons still allow it.)
            "Broad sweep" => {
                a.fonts.fill(true);
                for s in [2, 4, 8, 10] {
                    a.sizes[s] = true;
                }
                a.techs.fill(true);
                for h in [3, 4, 7] {
                    a.hints[h] = true;
                }
                a.bins = [true, true];
                for g in [0, 5, 7, 12] {
                    a.gammas[g] = true;
                }
                a.themes[3] = true;
                self.sample = 1;
            }
            // "Start here": a bit of everything at Soyuz's own size.
            _ => {
                for (on, t) in a.techs.iter_mut().zip(TECHS.iter()) {
                    *on = matches!(
                        t.code,
                        "E" | "EM10"
                            | "EM20"
                            | "SH30"
                            | "GL"
                            | "LCD5"
                            | "LCD3"
                            | "LCD5L"
                            | "DWS"
                            | "DWSL"
                            | "DWC"
                            | "DWY"
                    );
                }
                for i in [3, 4, 7] {
                    a.hints[i] = true;
                }
            }
        }
        self.dirty = true;
    }

    fn rebuild(&mut self) {
        self.dirty = false;
        self.truncated = false;
        if self.only_picks {
            // Everything you picked, whichever preset it came from.
            self.list = self.picks.iter().filter_map(|p| Spec::parse(p, &self.fonts)).collect();
            self.tallest = self.list.iter().map(raster::line_height).max().unwrap_or(20);
            return;
        }
        let a = &self.axes;
        let on = |v: &[bool]| v.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).collect::<Vec<_>>();
        let (fonts, sizes, techs, hints) = (on(&a.fonts), on(&a.sizes), on(&a.techs), on(&a.hints));
        let (gammas, themes, weights, scales) = (on(&a.gammas), on(&a.themes), on(&a.weights), on(&a.scales));
        let bins: Vec<bool> = [true, false].into_iter().filter(|b| a.bins[usize::from(*b)]).collect();
        let mut seen = HashSet::new();
        let mut list = Vec::new();
        let mut truncated = false;
        let needle = self.filter.to_ascii_uppercase();
        'all: for &font in &fonts {
            for &size in &sizes {
                for &tech in &techs {
                    for &hint in &hints {
                        for &bin in &bins {
                            for &gamma in &gammas {
                                for &theme in &themes {
                                    for &w in &weights {
                                        for &s in &scales {
                                            let spec = Spec {
                                                font,
                                                size,
                                                tech,
                                                hint,
                                                bin,
                                                gamma,
                                                theme,
                                                wght: WEIGHTS[w],
                                                scale: SCALES[s],
                                            };
                                            if !spec.valid(&self.fonts) {
                                                continue;
                                            }
                                            let id = spec.id(&self.fonts);
                                            if !needle.is_empty() && !id.to_ascii_uppercase().contains(&needle) {
                                                continue;
                                            }
                                            if seen.insert(id) {
                                                list.push(spec);
                                                if list.len() >= MAX_CARDS {
                                                    truncated = true;
                                                    break 'all;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.tallest = list.iter().map(raster::line_height).max().unwrap_or(20);
        self.list = list;
        self.truncated = truncated;
    }

    fn text(&self) -> Arc<str> {
        if self.sample >= SAMPLES.len() { self.custom.as_str().into() } else { SAMPLES[self.sample].1.into() }
    }

    /// Card image size in physical pixels, the same for every card so rows line up.
    fn image_px(&self, ppp: f32) -> (u32, u32) {
        let lines = self.text().lines().count().max(1) as u32;
        ((self.card_w * ppp).round() as u32, self.tallest * lines + PAD * 2)
    }

    fn save_picks(&self) {
        let body: String = self.picks.iter().map(|p| format!("{p}\n")).collect();
        let _ = std::fs::write(&self.picks_file, body);
    }

    fn side(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(RichText::new("Presets").strong());
        ui.horizontal_wrapped(|ui| {
            for p in PRESETS {
                if ui.selectable_label(self.preset_name == p, p).clicked() {
                    self.apply_preset(p);
                }
            }
        });
        ui.separator();
        ScrollArea::vertical().id_salt("axes").show(ui, |ui| {
            let fonts = self.fonts.clone();
            let a = &mut self.axes;
            let mut changed = false;
            let labels: Vec<String> = fonts.iter().map(|f| format!("{}  {}", f.code, f.name)).collect();
            let help =
                |i: usize| if fonts[i].system { "A font on this machine" } else { "Embedded in Soyuz" }.to_string();
            changed |= axis(ui, "Font", &labels, &mut a.fonts, help, true);
            let labels: Vec<String> = SIZES.iter().map(f32::to_string).collect();
            changed |= axis(ui, "Size (pt)", &labels, &mut a.sizes, |_| String::new(), false);
            let labels: Vec<String> = TECHS.iter().map(|t| format!("{}  {}", t.code, t.label)).collect();
            changed |= axis(ui, "Technique", &labels, &mut a.techs, |i| TECHS[i].ship.to_string(), true);
            let labels: Vec<String> = HINTS.iter().map(|h| format!("{}  {}", h.code, h.label)).collect();
            changed |= axis(ui, "Hinting", &labels, &mut a.hints, |i| HINTS[i].help.to_string(), true);
            let mut bins = a.bins.to_vec();
            let labels = ["B0  whole pixels".to_string(), "B1  four bins".to_string()];
            changed |= axis(ui, "Subpixel positions", &labels, &mut bins, |_| String::new(), false);
            a.bins = [bins[0], bins[1]];
            let labels: Vec<String> = GAMMAS.iter().map(|g| g.0.to_string()).collect();
            let help = |i: usize| {
                GAMMAS[i].1.map_or("egui's dark curve, 2c - c²".to_string(), |g| {
                    format!("alpha = coverage^{g:.2}. Lower is heavier.")
                })
            };
            changed |= axis(ui, "Coverage curve", &labels, &mut a.gammas, help, false);
            let labels: Vec<String> = THEMES.iter().map(|t| format!("{}  {}", t.code, t.label)).collect();
            changed |= axis(ui, "Colours", &labels, &mut a.themes, |_| "Soyuz's Orbit palette".to_string(), true);
            let labels: Vec<String> =
                WEIGHTS.iter().map(|w| if *w == 0 { "default".to_string() } else { w.to_string() }).collect();
            changed |= axis(
                ui,
                "Weight (variable fonts)",
                &labels,
                &mut a.weights,
                |_| "Only fonts with a wght axis".to_string(),
                false,
            );
            let labels: Vec<String> = SCALES.iter().map(|s| format!("{s}%")).collect();
            let help = |_| "Text rasterized for this scale, shown 1:1 in your pixels".to_string();
            changed |= axis(ui, "Display scale", &labels, &mut a.scales, help, false);
            if changed {
                self.dirty = true;
                self.preset_name.clear();
            }
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Under the pointer").strong());
            ui.add(egui::Slider::new(&mut self.zoom, 2.0..=12.0).step_by(1.0).text("zoom"));
        });
        let (zoom_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::hover());
        ui.painter().rect_filled(zoom_rect, 4.0, Color32::BLACK);
        if let Some(spec) = self.hovered {
            let (w, h) = self.image_px(ppp);
            let key = Key { spec, text: self.text(), w, h };
            if let Some((Ok(tex), _)) = self.cache.get(&key) {
                let (tw, th) = (tex.size()[0] as f32, tex.size()[1] as f32);
                let z = self.zoom / ppp;
                let span = vec2((zoom_rect.width() / z).min(tw), (zoom_rect.height() / z).min(th));
                let uv = Rect::from_min_size(pos2(0.0, 0.0), vec2(span.x / tw, span.y / th));
                let rect = Rect::from_min_size(zoom_rect.min, span * z);
                ui.painter().image(tex.id(), rect, uv, Color32::WHITE);
            }
        }
        ScrollArea::vertical().id_salt("right").show(ui, |ui| {
            if let Some(spec) = self.hovered {
                let id = spec.id(&self.fonts);
                ui.label(RichText::new(&id).monospace().strong());
                egui::Grid::new("decode").num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
                    for (k, v) in spec.describe(&self.fonts) {
                        ui.label(RichText::new(k).weak());
                        ui.add(egui::Label::new(v).wrap());
                        ui.end_row();
                    }
                });
            } else {
                ui.label(RichText::new("Hover a card to read its settings and see its pixels magnified.").weak());
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Picks ({})", self.picks.len())).strong());
                if ui.button("Copy ids").clicked() {
                    ctx.copy_text(self.picks.iter().cloned().collect::<Vec<_>>().join("\n"));
                }
                if ui.button("Clear").clicked() {
                    self.picks.clear();
                    self.save_picks();
                    self.dirty |= self.only_picks;
                }
            });
            let mut remove = None;
            for p in &self.picks {
                ui.horizontal(|ui| {
                    if ui.small_button("x").clicked() {
                        remove = Some(p.clone());
                    }
                    let label = egui::Label::new(RichText::new(p).monospace()).sense(Sense::click());
                    if ui.add(label).on_hover_text("Click to copy").clicked() {
                        ctx.copy_text(p.clone());
                    }
                });
            }
            if let Some(p) = remove {
                self.picks.remove(&p);
                self.save_picks();
                self.dirty |= self.only_picks;
            }
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Saved to {}", self.picks_file.display())).small().weak());
        });
    }

    fn top(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Sample");
            let name = SAMPLES.get(self.sample).map_or("Your text", |s| s.0);
            egui::ComboBox::from_id_salt("sample").selected_text(name).show_ui(ui, |ui| {
                for (i, (n, _)) in SAMPLES.iter().enumerate() {
                    ui.selectable_value(&mut self.sample, i, *n);
                }
                ui.selectable_value(&mut self.sample, SAMPLES.len(), "Your text");
            });
            if self.sample >= SAMPLES.len() {
                ui.add(egui::TextEdit::multiline(&mut self.custom).desired_rows(1).desired_width(240.0));
            }
            ui.label("Card width");
            ui.add(egui::Slider::new(&mut self.card_w, 200.0..=1000.0).step_by(20.0).show_value(false));
            ui.label("Find id");
            let find = egui::TextEdit::singleline(&mut self.filter).desired_width(150.0).hint_text("LCD5, B0, IN13...");
            if ui.add(find).changed() {
                self.dirty = true;
            }
            if ui.checkbox(&mut self.only_picks, "Picks only").changed() {
                self.dirty = true;
            }
            let count = if self.truncated {
                let a = &self.axes;
                let on = |v: &[bool]| v.iter().filter(|b| **b).count() as u64;
                let total = [&a.fonts, &a.sizes, &a.techs, &a.hints, &a.gammas, &a.themes, &a.weights, &a.scales]
                    .iter()
                    .map(|v| on(v))
                    .product::<u64>()
                    * on(&a.bins);
                format!("first {} of up to {total} samples: narrow the axes", self.list.len())
            } else {
                format!("{} samples", self.list.len())
            };
            ui.label(RichText::new(count).strong());
        });
        let note =
            format!("{}. Click a card to pick it. Every sample is drawn 1:1 in screen pixels.", self.renderer_info);
        ui.label(RichText::new(note).small().weak());
    }

    fn grid(&mut self, ui: &mut egui::Ui, wanted: &mut Vec<Job>) {
        let ppp = ui.ctx().pixels_per_point();
        let (w_px, h_px) = self.image_px(ppp);
        let text = self.text();
        let img = vec2(w_px as f32 / ppp, h_px as f32 / ppp);
        let gap = 10.0;
        let (card_w, card_h) = (img.x + 2.0, img.y + 22.0);
        let cols = (((ui.available_width() + gap) / (card_w + gap)).floor() as usize).max(1);
        let rows = self.list.len().div_ceil(cols);
        let mut hovered = None;
        let mut area = ScrollArea::vertical().id_salt("grid").auto_shrink([false, false]);
        if let Some(y) = self.scroll.take() {
            area = area.vertical_scroll_offset(y);
        }
        let soyuz_now = self.soyuz_ids();
        area.show_rows(ui, card_h, rows, |ui, range| {
            ui.spacing_mut().item_spacing.y = gap;
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for col in 0..cols {
                        let Some(&spec) = self.list.get(row * cols + col) else { break };
                        let id = spec.id(&self.fonts);
                        let (rect, resp) = ui.allocate_exact_size(vec2(card_w, card_h), Sense::click());
                        let key = Key { spec, text: text.clone(), w: w_px, h: h_px };
                        self.card(ui, rect, &id, &key, soyuz_now.contains(&id), resp.hovered());
                        if resp.hovered() {
                            hovered = Some(spec);
                        }
                        if resp.clicked() {
                            if !self.picks.remove(&id) {
                                self.picks.insert(id);
                            }
                            self.save_picks();
                            self.dirty |= self.only_picks;
                        }
                        if !self.cache.contains_key(&key) && !self.in_flight.contains(&key) {
                            wanted.push(Job { key, spec, text: text.clone(), w: w_px, h: h_px });
                        }
                    }
                });
            }
        });
        self.hovered = hovered.or(self.pinned_hover).or(self.hovered);
        // The inspector magnifies this one even when it has scrolled off the grid.
        if let Some(spec) = self.hovered {
            let key = Key { spec, text: text.clone(), w: w_px, h: h_px };
            if let Some((_, used)) = self.cache.get_mut(&key) {
                *used = self.frame;
            } else if !self.in_flight.contains(&key) && !wanted.iter().any(|j| j.key == key) {
                wanted.push(Job { key, spec, text: text.clone(), w: w_px, h: h_px });
            }
        }
        if self.verify && cols > 1 {
            self.verify_text(ui, img, card_w + gap);
        }
    }

    /// Over the second card: the first card's sample, drawn by this window's own egui
    /// context (Inter, crisp, gamma 0.45, as Soyuz sets them), at the same pixel offset.
    fn verify_text(&self, ui: &egui::Ui, img: egui::Vec2, step: f32) {
        let Some(spec) = self.list.first() else { return };
        let ppp = ui.ctx().pixels_per_point();
        let t = &THEMES[spec.theme];
        let bg = Color32::from_rgb(t.bg[0], t.bg[1], t.bg[2]);
        let fg = Color32::from_rgb(t.fg[0], t.fg[1], t.fg[2]);
        let left = ui.max_rect().min + vec2(step + 1.0, 21.0);
        let rect = Rect::from_min_size(left.round_to_pixels(ppp), img);
        let lh = raster::line_height(spec) as f32 / ppp;
        let mut job = egui::text::LayoutJob::default();
        let format = egui::TextFormat {
            font_id: FontId::proportional(SIZES[spec.size]),
            color: fg,
            line_height: Some(lh),
            ..Default::default()
        };
        job.append(&self.text(), 0.0, format);
        let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
        let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("verify")));
        p.rect_filled(rect, 0.0, bg);
        p.galley(rect.min + vec2(PAD as f32 / ppp, PAD as f32 / ppp), galley, fg);
    }

    /// The looks Soyuz ships with, at this window's scale.
    fn soyuz_ids(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        let scale = self.native_scale;
        for (theme, gamma) in [(0, G45), (1, G45), (2, G45), (3, G80), (4, G80), (5, G80)] {
            for size in 0..SIZES.len() {
                let s = Spec { font: 0, size, tech: 0, hint: DEFAULT_HINT, bin: true, gamma, theme, wght: 0, scale };
                out.insert(s.id(&self.fonts));
            }
        }
        out
    }

    fn card(&mut self, ui: &egui::Ui, rect: Rect, id: &str, key: &Key, soyuz: bool, hovered: bool) {
        let ppp = ui.ctx().pixels_per_point();
        let picked = self.picks.contains(id);
        let accent = Color32::from_rgb(0x5B, 0x9D, 0xFF);
        let p = ui.painter();
        let border = if picked {
            Stroke::new(2.0, accent)
        } else if hovered {
            Stroke::new(1.0, Color32::from_gray(150))
        } else {
            Stroke::new(1.0, Color32::from_gray(52))
        };
        let label_color = if picked { accent } else { Color32::from_gray(205) };
        let top = rect.min + vec2(4.0, 3.0);
        let label = p.text(top, Align2::LEFT_TOP, id, FontId::monospace(11.5), label_color);
        if soyuz {
            let green = Color32::from_rgb(0x6F, 0xCF, 0x97);
            p.text(pos2(label.max.x + 8.0, top.y), Align2::LEFT_TOP, "SOYUZ TODAY", FontId::proportional(10.5), green);
        }
        // On whole physical pixels, or the 1:1 promise breaks.
        let min = (rect.min + vec2(1.0, 21.0)).round_to_pixels(ppp);
        let img = Rect::from_min_size(min, vec2(key.w as f32 / ppp, key.h as f32 / ppp));
        if let Some((res, used)) = self.cache.get_mut(key) {
            *used = self.frame;
            match res {
                Ok(tex) => {
                    p.image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                Err(e) => {
                    let red = Color32::from_rgb(0xE5, 0x73, 0x73);
                    p.text(img.min + vec2(6.0, 4.0), Align2::LEFT_TOP, e.as_str(), FontId::proportional(11.0), red);
                }
            }
        }
        p.rect_stroke(rect, CornerRadius::same(3), border, egui::StrokeKind::Inside);
    }

    fn pump(&mut self, ctx: &egui::Context, wanted: Vec<Job>) {
        for (key, res) in self.rx.try_iter() {
            self.in_flight.remove(&key);
            let res = res.map(|img| ctx.load_texture("sample", img, TextureOptions::NEAREST));
            self.cache.insert(key, (res, self.frame));
        }
        // Only what is on screen now: a long scroll must not leave a backlog.
        {
            let mut q = self.queue.jobs.lock().expect("queue");
            for j in q.drain(..) {
                self.in_flight.remove(&j.key);
            }
            for j in wanted {
                self.in_flight.insert(j.key.clone());
                q.push_back(j);
            }
            if !q.is_empty() {
                self.queue.wake.notify_all();
            }
        }
        if self.cache.len() > CACHE_CARDS {
            let mut by_age: Vec<(u64, Key)> = self.cache.iter().map(|(k, v)| (v.1, k.clone())).collect();
            by_age.sort_by_key(|e| e.0);
            for (_, k) in by_age.into_iter().take(self.cache.len() - CACHE_CARDS) {
                self.cache.remove(&k);
            }
        }
    }
}

fn axis(
    ui: &mut egui::Ui,
    title: &str,
    labels: &[String],
    on: &mut [bool],
    help: impl Fn(usize) -> String,
    column: bool,
) -> bool {
    let mut changed = false;
    let count = on.iter().filter(|b| **b).count();
    let header = format!("{title}  ({count}/{})", on.len());
    egui::CollapsingHeader::new(header).id_salt(title).default_open(true).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.small_button("All").clicked() {
                on.fill(true);
                changed = true;
            }
            if ui.small_button("None").clicked() {
                on.fill(false);
                changed = true;
            }
        });
        let mut one = |ui: &mut egui::Ui, i: usize, label: &String, on: &mut bool| {
            let mut r = ui.checkbox(on, label.as_str());
            let h = help(i);
            if !h.is_empty() {
                r = r.on_hover_text(h);
            }
            changed |= r.changed();
        };
        if column {
            for (i, (label, v)) in labels.iter().zip(on.iter_mut()).enumerate() {
                one(ui, i, label, v);
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                for (i, (label, v)) in labels.iter().zip(on.iter_mut()).enumerate() {
                    one(ui, i, label, v);
                }
            });
        }
    });
    changed
}

fn worker(
    queue: &Queue,
    tx: &mpsc::Sender<Done>,
    fonts: &[FontEntry],
    adapter: &eframe::wgpu::Adapter,
    ctx: &egui::Context,
) {
    let mut gpu = Gpu::new(adapter);
    loop {
        let job = {
            let mut q = queue.jobs.lock().expect("queue");
            loop {
                if let Some(j) = q.pop_front() {
                    break j;
                }
                q = queue.wake.wait(q).expect("queue");
            }
        };
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &mut gpu {
            Ok(gpu) => raster::render(gpu, &job.spec, fonts, &job.text, job.w, job.h),
            Err(e) => Err(format!("no GPU device: {e}")),
        }))
        .unwrap_or_else(|_| Err("this combination panicked".to_string()));
        if tx.send((job.key, res)).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// The lab's own interface looks like Soyuz: Inter, crisp hinting, gamma 0.45.
fn install_chrome(ctx: &egui::Context, inter: &FontEntry) {
    let tweak = FontTweak {
        hinting: Some(true),
        hinting_target: HintingTarget::Smooth(SmoothHinting {
            light: false,
            symmetric_rendering: false,
            preserve_linear_metrics: false,
        }),
        ..FontTweak::default()
    };
    let mut defs = FontDefinitions::default();
    defs.font_data.insert("Inter".into(), Arc::new(FontData::from_static(inter.bytes).tweak(tweak)));
    defs.families.entry(FontFamily::Proportional).or_default().insert(0, "Inter".into());
    ctx.set_fonts(defs);
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.visuals.text_options.color_transfer_function = egui::epaint::FontColorTransferFunction::Gamma(0.45);
    });
}

impl eframe::App for Lab {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame += 1;
        if self.dirty {
            self.rebuild();
        }
        // Scripted screenshot: once the pictures have stopped arriving.
        if let Some(path) = self.shot.clone() {
            let idle = self.in_flight.is_empty() && self.frame > 5;
            self.settled = if idle { self.settled + 1 } else { 0 };
            if self.settled > 5 && !self.shot_sent {
                self.shot_sent = true;
                let c = ctx.clone();
                ctx.request_screenshot(move |img| {
                    save_png(&path, &img);
                    c.send_viewport_cmd(egui::ViewportCommand::Close);
                    c.request_repaint();
                });
            }
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut wanted = Vec::new();
        Panel::left("axes").resizable(true).default_size(300.0).show(ui, |ui| self.side(ui));
        Panel::right("right").resizable(true).default_size(340.0).show(ui, |ui| self.inspector(ui));
        Panel::top("top").show(ui, |ui| self.top(ui));
        egui::CentralPanel::default().show(ui, |ui| self.grid(ui, &mut wanted));
        self.pump(&ctx, wanted);
    }
}

fn save_png(path: &str, img: &ColorImage) {
    let Ok(file) = std::fs::File::create(path) else { return };
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.width() as u32, img.height() as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    if let Ok(mut w) = enc.write_header() {
        let _ = w.write_image_data(&bytes);
    }
}
