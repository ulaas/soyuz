//! Images in the terminal (`[tui] images`, `[tui] cell_pixels`, `[tui] color_mode`) through
//! `ratatui-image`: kitty, sixel, iTerm2 or half-blocks — Phase T4 (previews, thumbnails,
//! grid cells, Quick Look, editors).
//!
//! # Pieces
//! - [`install`] (from `App::new`) makes the terminal UI's texture [`Uploader`]
//!   ([`store`]): the core's workers hand it decoded pixels, it keeps them under unique ids
//!   in a bounded store (budget: `[preview] thumb_cache_mib + preview_cache_mib`, least
//!   recently drawn evicted first) and returns [`Tex`] handles; dropping the last clone of
//!   a `Tex` frees its pixels.
//! - [`startup`] (from `app::run`, after entering TUI mode and before the input reader
//!   starts) asks the terminal which graphics protocol it speaks, only for `auto` on unix
//!   outside tmux, with [`QUERY_TIMEOUT`]. [`choose`] is the pure decision.
//! - [`begin_frame`] (every frame, before drawing) follows `[tui]` changes and collects
//!   images the encoder thread finished; [`end_frame`] (in the draw closure, after
//!   everything else was drawn) repairs images that something was drawn over.
//! - [`draw`] / [`draw_in`] draw one texture fitted and centred in an area (aspect kept,
//!   never scaled up unless [`ImageState::upscale`]).
//!
//! # Protocols
//! - Half-blocks are plain coloured cells (`▀` / `▄`), scaled on the UI thread (cheap: a
//!   few thousand pixels) and cached per (texture, cell size); colours follow
//!   `[tui] color_mode` (truecolor, the 256 palette, the 16 ANSI colours) and `mono` draws a
//!   `[W×H]` placeholder instead.
//! - Sixel, kitty and iTerm2 images are encoded on a worker thread (`images::worker`),
//!   cached per (texture, cell size, protocol generation); [`Drawn::Pending`] until ready.
//!
//! # Repainting after overlays
//! Sixel and iTerm2 images are one escape sequence in the top-left cell; the other cells
//! are skipped by ratatui's diff, so text drawn over them (a dialog, menu or the palette)
//! would not reach the screen, and once the overlay closes its text would stay on the
//! image. [`end_frame`] handles both without help from the views: cells drawn over an
//! image this frame are un-skipped, and in the frame the image is uncovered again the
//! image's cells are blanked (erasing the overlay's leftovers and the old image) and the
//! image is drawn again in the next frame (the loop is woken). A new placement starts the
//! same way: one blank frame, then the image. Kitty images use placeholder characters,
//! which ratatui diffs like text, and half-blocks are text: neither needs this.
//!
//! [`invalidate`] forgets what is on the screen, so every graphics image is blanked and
//! redrawn: call it when something wrote to the terminal outside ratatui (a child process
//! in the terminal, a suspend / resume without `Terminal::clear`). Overlays drawn in the
//! same frame after the image need no call.

pub mod blocks;
pub mod store;
mod worker;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use image::RgbaImage;
use ratatui::Frame;
use ratatui::buffer::{Buffer, Cell, CellDiffOption};
use ratatui::layout::{Rect, Size};
use ratatui::style::Style;
use ratatui::widgets::Widget;
use ratatui_image::picker::Picker;
pub use ratatui_image::picker::ProtocolType;
use ratatui_image::protocol::Protocol;
use ratatui_image::{FontSize, Image};
use soyuz_appcore::host::{Wake, no_wake};
use soyuz_appcore::services::preview_gfx::{Tex, Uploader};
use soyuz_core::config::{Config, TuiColorMode, TuiImages, TuiSection};

use crate::theme::{ColorMode, ColorSetting};
use store::Store;

/// How images are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMode {
    /// No images.
    Off,
    /// Ask the terminal (half-blocks when it does not answer).
    Auto,
    /// This protocol, whatever the terminal says.
    Forced(ProtocolType),
}

/// The mode `[tui] images` asks for.
pub fn mode(tui: &TuiSection) -> ImageMode {
    match tui.images {
        TuiImages::Off => ImageMode::Off,
        TuiImages::Auto => ImageMode::Auto,
        TuiImages::Kitty => ImageMode::Forced(ProtocolType::Kitty),
        TuiImages::Sixel => ImageMode::Forced(ProtocolType::Sixel),
        TuiImages::Iterm2 => ImageMode::Forced(ProtocolType::Iterm2),
        TuiImages::Halfblocks => ImageMode::Forced(ProtocolType::Halfblocks),
    }
}

/// The cell size in pixels when the terminal does not report it (`[tui] cell_pixels`,
/// clamped).
pub fn cell_pixels(tui: &TuiSection) -> (u16, u16) {
    tui.cell_pixels_clamped()
}

// ---- protocol choice ----------------------------------------------------------------

/// What `[tui] images` comes to on this terminal ([`choose`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// No images.
    Off,
    /// Half-blocks, without asking the terminal.
    Halfblocks,
    /// Ask the terminal at startup ([`startup`]); half-blocks when it does not answer.
    Query,
    /// This graphics protocol, without asking.
    Protocol(ProtocolType),
}

/// Whether the environment says we run inside tmux or GNU screen.
pub fn in_multiplexer(env: &dyn Fn(&str) -> Option<String>) -> bool {
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    let term = env("TERM").unwrap_or_default();
    set("TMUX")
        || set("STY")
        || term.starts_with("tmux")
        || term.starts_with("screen")
        || env("TERM_PROGRAM").is_some_and(|p| p == "tmux")
}

/// The choice for `images` with environment lookup `env` (`std::env::var` in the app):
/// `auto` asks the terminal only on unix outside tmux / screen and outside the `dumb` and
/// Linux console terminals; Windows (the console does not reliably pass the answers on)
/// and multiplexers (pass-through is unreliable) get half-blocks. A forced protocol is
/// used as asked (inside tmux with pass-through).
pub fn choose(images: TuiImages, env: &dyn Fn(&str) -> Option<String>, windows: bool) -> Choice {
    match images {
        TuiImages::Off => Choice::Off,
        TuiImages::Halfblocks => Choice::Halfblocks,
        TuiImages::Kitty => Choice::Protocol(ProtocolType::Kitty),
        TuiImages::Sixel => Choice::Protocol(ProtocolType::Sixel),
        TuiImages::Iterm2 => Choice::Protocol(ProtocolType::Iterm2),
        TuiImages::Auto => {
            let term = env("TERM").unwrap_or_default();
            if windows || in_multiplexer(env) || term == "dumb" || term == "linux" {
                Choice::Halfblocks
            } else {
                Choice::Query
            }
        }
    }
}

/// How long [`startup`] waits for the terminal's answer.
pub const QUERY_TIMEOUT: Duration = Duration::from_millis(1500);

/// The startup query's answer ([`startup`]); `auto` uses it for the whole run.
static STARTUP: Mutex<Option<Picker>> = Mutex::new(None);

/// Ask the terminal for its graphics protocol and cell size when `[tui] images` is
/// `auto` and [`choose`] says so. Call once, in TUI mode, before any thread reads terminal
/// input (the answer arrives on the input stream). Returns the choice made.
pub fn startup(tui: &TuiSection) -> Choice {
    let choice = choose(tui.images, &|k| std::env::var(k).ok(), cfg!(windows));
    if choice == Choice::Query && cfg!(unix) {
        let started = Instant::now();
        let opts =
            ratatui_image::picker::cap_parser::QueryStdioOptions { timeout: QUERY_TIMEOUT, ..Default::default() };
        match Picker::from_query_stdio_with_options(opts) {
            Ok(p) => {
                log::info!(
                    "tui: images: terminal answered {:?}, cell {:?} ({} ms)",
                    p.protocol_type(),
                    p.font_size(),
                    started.elapsed().as_millis()
                );
                if let Ok(mut g) = STARTUP.lock() {
                    *g = Some(p);
                }
            }
            Err(e) => log::info!("tui: images: no answer from the terminal ({e}); half-blocks"),
        }
    } else {
        log::info!("tui: images: {choice:?}");
    }
    choice
}

/// What draws images now.
#[derive(Clone)]
enum Backend {
    Off,
    Blocks,
    Graphics(Picker),
}

/// The backend and cell size for `choice`; `queried` is the startup answer.
fn backend_for(choice: Choice, queried: Option<Picker>, cell: (u16, u16)) -> (Backend, (u16, u16)) {
    let size = |p: &Picker| {
        let f = p.font_size();
        (f.width.max(1), f.height.max(1))
    };
    match choice {
        Choice::Off => (Backend::Off, cell),
        Choice::Halfblocks => (Backend::Blocks, cell),
        Choice::Query => match queried {
            Some(p) if p.protocol_type() != ProtocolType::Halfblocks => {
                let c = size(&p);
                (Backend::Graphics(p), c)
            }
            _ => (Backend::Blocks, cell),
        },
        Choice::Protocol(ProtocolType::Halfblocks) => (Backend::Blocks, cell),
        Choice::Protocol(t) => {
            // The configured cell size; `from_fontsize` is what this needs (deprecated only
            // in favour of asking the terminal, which a forced protocol does not do).
            #[allow(deprecated)]
            let mut p = Picker::from_fontsize(FontSize::new(cell.0, cell.1));
            p.set_protocol_type(t);
            (Backend::Graphics(p), cell)
        }
    }
}

// ---- fitting ------------------------------------------------------------------------

/// Where an image of `img` pixels goes inside `area` with cells of `cell` pixels: the
/// centred cell rect and the pixel size it is scaled to (aspect kept; not larger than
/// the image unless `upscale`). `None` when nothing fits.
pub fn fit(img: [usize; 2], area: Rect, cell: (u16, u16), upscale: bool) -> Option<(Rect, (u32, u32))> {
    if area.is_empty() || img[0] == 0 || img[1] == 0 {
        return None;
    }
    let (cw, ch) = (f64::from(cell.0.max(1)), f64::from(cell.1.max(1)));
    let (iw, ih) = (img[0] as f64, img[1] as f64);
    let mut s = (f64::from(area.width) * cw / iw).min(f64::from(area.height) * ch / ih);
    if !upscale {
        s = s.min(1.0);
    }
    let tw = (iw * s).round().max(1.0);
    let th = (ih * s).round().max(1.0);
    let cols = ((tw / cw).ceil() as u16).clamp(1, area.width);
    let rows = ((th / ch).ceil() as u16).clamp(1, area.height);
    let rect = Rect::new(area.x + (area.width - cols) / 2, area.y + (area.height - rows) / 2, cols, rows);
    Some((rect, (tw as u32, th as u32)))
}

// ---- the engine (one per UI thread) -------------------------------------------------

/// (texture id, cols, rows, protocol generation).
pub(crate) type Key = (u64, u16, u16, u64);

/// A small LRU map.
struct Lru<V> {
    map: HashMap<Key, (V, u64)>,
    tick: u64,
    cap: usize,
}

impl<V> Lru<V> {
    fn new(cap: usize) -> Lru<V> {
        Lru { map: HashMap::new(), tick: 0, cap }
    }

    fn touch(&mut self, k: &Key) -> Option<&V> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(k).map(|e| {
            e.1 = tick;
            &e.0
        })
    }

    fn insert(&mut self, k: Key, v: V) {
        self.tick += 1;
        self.map.insert(k, (v, self.tick));
        while self.map.len() > self.cap {
            let Some(old) = self.map.iter().min_by_key(|(_, e)| e.1).map(|(k, _)| *k) else { break };
            self.map.remove(&old);
        }
    }
}

/// Encoded half-block images kept.
const BLOCKS_CACHE: usize = 256;
/// Encoded graphics-protocol images kept (they can be large).
const PROTO_CACHE: usize = 48;

/// What the screen shows at a graphics placement ((rect, texture id)) after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    /// The image, nothing over it.
    Clean,
    /// The image with something drawn over it.
    Covered,
    /// Blank cells, ready for the image.
    Blank,
}

struct Placement {
    rect: Rect,
    id: u64,
    hashes: Vec<u64>,
    image: bool,
    /// The image is encoded (a blank placement: draw it next frame).
    ready: bool,
}

struct Engine {
    synced: Option<(TuiImages, TuiColorMode, [u16; 2])>,
    backend: Backend,
    color: ColorMode,
    cell: (u16, u16),
    generation: u64,
    wake: Wake,
    store: Option<Arc<Store>>,
    blocks: Lru<Arc<RgbaImage>>,
    protos: Lru<Option<Protocol>>,
    pending: HashSet<Key>,
    worker: Option<worker::Worker>,
    placed: Vec<Placement>,
    shown: HashMap<(Rect, u64), Shown>,
    redraw: bool,
    frames: u64,
}

impl Engine {
    fn new() -> Engine {
        let mut e = Engine {
            synced: None,
            backend: Backend::Blocks,
            color: ColorMode::Truecolor,
            cell: TuiSection::default().cell_pixels_clamped(),
            generation: 0,
            wake: no_wake(),
            store: None,
            blocks: Lru::new(BLOCKS_CACHE),
            protos: Lru::new(PROTO_CACHE),
            pending: HashSet::new(),
            worker: None,
            placed: Vec::new(),
            shown: HashMap::new(),
            redraw: false,
            frames: 0,
        };
        e.sync(&TuiSection::default());
        e
    }

    fn sync(&mut self, tui: &TuiSection) {
        let k = (tui.images, tui.color_mode, tui.cell_pixels);
        if self.synced == Some(k) {
            return;
        }
        let env = |k: &str| std::env::var(k).ok();
        self.color = ColorMode::resolve(ColorSetting::from_config(tui), &env);
        let images_changed = self.synced.is_none_or(|(i, _, c)| i != tui.images || c != tui.cell_pixels);
        self.synced = Some(k);
        if images_changed {
            let queried = STARTUP.lock().ok().and_then(|g| g.clone());
            let choice = choose(tui.images, &env, cfg!(windows));
            (self.backend, self.cell) = backend_for(choice, queried, tui.cell_pixels_clamped());
            self.reset();
        }
    }

    /// Forget every encoding and what is on the screen.
    fn reset(&mut self) {
        self.generation += 1;
        self.blocks.map.clear();
        self.protos.map.clear();
        self.pending.clear();
        self.shown.clear();
    }

    fn collect(&mut self) {
        let Some(w) = &self.worker else { return };
        for d in w.drain() {
            if self.pending.remove(&d.key) {
                self.protos.insert(d.key, d.proto);
            }
        }
    }

    fn request_redraw(&mut self) {
        self.redraw = true;
        (self.wake)();
    }

    fn submit(&mut self, job: worker::Job) {
        if self.worker.is_none() {
            self.worker = worker::Worker::spawn(self.wake.clone());
        }
        let key = job.key;
        let job = match &self.worker {
            Some(w) => match w.send(job) {
                Ok(()) => {
                    self.pending.insert(key);
                    return;
                }
                Err(job) => job,
            },
            None => job,
        };
        // No thread: encode here (slow but correct).
        let p = worker::encode(&job);
        self.protos.insert(key, p);
    }

    fn draw(&mut self, buf: &mut Buffer, area: Rect, tex: &Tex, st: &mut ImageState) -> Drawn {
        st.drawn = None;
        let area = area.intersection(buf.area);
        if area.is_empty() {
            return Drawn::NoRoom;
        }
        if matches!(self.backend, Backend::Off) {
            return Drawn::Off;
        }
        let Some((id, img)) = store::lookup(tex) else { return Drawn::Missing };
        let cell = self.cell;
        let Some((rect, px)) = fit(tex.size, area, cell, st.upscale) else { return Drawn::NoRoom };
        st.drawn = Some(rect);
        let key = (id, rect.width, rect.height, self.generation);
        let picker = match &self.backend {
            Backend::Off => return Drawn::Off,
            Backend::Blocks if self.color == ColorMode::Mono => {
                placeholder(buf, rect, tex.size);
                return Drawn::Placeholder;
            }
            Backend::Blocks => {
                let enc = match self.blocks.touch(&key) {
                    Some(e) => e.clone(),
                    None => {
                        let w = ((f64::from(px.0) / f64::from(cell.0)).round() as u32).clamp(1, u32::from(rect.width));
                        let h = ((f64::from(px.1) * 2.0 / f64::from(cell.1)).round() as u32)
                            .clamp(1, u32::from(rect.height) * 2);
                        let e = Arc::new(blocks::encode(&img, w, h));
                        self.blocks.insert(key, e.clone());
                        e
                    }
                };
                blocks::render(&enc, rect, buf, self.color);
                return Drawn::Image;
            }
            Backend::Graphics(p) => p.clone(),
        };
        self.collect();
        // Sixel and iTerm2 skip all but the first cell: track them (see the module docs).
        let tracked = matches!(picker.protocol_type(), ProtocolType::Sixel | ProtocolType::Iterm2);
        let state = self.protos.touch(&key).map(Option::is_some);
        match state {
            Some(false) => Drawn::Missing,
            Some(true) => {
                if tracked && !self.shown.contains_key(&(rect, id)) {
                    blank(buf, rect);
                    self.place(buf, rect, id, false, true);
                    return Drawn::Pending;
                }
                if let Some(Some(proto)) = self.protos.map.get(&key).map(|e| &e.0) {
                    Image::new(proto).allow_clipping(true).render(rect, buf);
                }
                if tracked {
                    self.place(buf, rect, id, true, true);
                }
                Drawn::Image
            }
            None => {
                if !self.pending.contains(&key) {
                    self.submit(worker::Job { key, img, px, cells: Size::new(rect.width, rect.height), picker });
                }
                if tracked {
                    blank(buf, rect);
                    let ready = self.protos.map.contains_key(&key);
                    self.place(buf, rect, id, false, ready);
                }
                Drawn::Pending
            }
        }
    }

    fn place(&mut self, buf: &Buffer, rect: Rect, id: u64, image: bool, ready: bool) {
        let hashes = cells(rect).map(|p| buf.cell(p).map_or(0, hash_cell)).collect();
        self.placed.push(Placement { rect, id, hashes, image, ready });
    }

    fn end_frame(&mut self, buf: &mut Buffer) {
        let placed = std::mem::take(&mut self.placed);
        let mut shown = HashMap::new();
        let mut again = false;
        for p in placed {
            let mut covered = false;
            for (i, pos) in cells(p.rect).enumerate() {
                let Some(cell) = buf.cell_mut(pos) else {
                    covered = true;
                    continue;
                };
                if p.hashes.get(i) != Some(&hash_cell(cell)) {
                    covered = true;
                    // Drawn over the image: make sure it reaches the screen.
                    if cell.diff_option == CellDiffOption::Skip {
                        cell.set_diff_option(CellDiffOption::None);
                    }
                }
            }
            let key = (p.rect, p.id);
            let before = self.shown.get(&key).copied();
            let now = match (p.image, covered) {
                (true, true) => Some(Shown::Covered),
                (true, false) if before == Some(Shown::Covered) => {
                    // Uncovered: wipe the overlay's leftovers; the image follows.
                    blank(buf, p.rect);
                    again = true;
                    Some(Shown::Blank)
                }
                (true, false) => Some(Shown::Clean),
                (false, true) => None,
                (false, false) => {
                    again |= p.ready;
                    Some(Shown::Blank)
                }
            };
            if let Some(s) = now {
                shown.insert(key, s);
            }
        }
        self.shown = shown;
        if again {
            self.request_redraw();
        }
        self.frames += 1;
    }

    fn begin_frame(&mut self, config: &Config) {
        self.sync(&config.tui);
        if let Some(s) = &self.store {
            s.set_cap(store::cap_for(config));
        }
        self.collect();
        self.placed.clear();
        // Now and then drop encodings of freed textures.
        if self.frames.is_multiple_of(64)
            && let Some(s) = self.store.clone()
        {
            self.blocks.map.retain(|k, _| s.contains(k.0));
            self.protos.map.retain(|k, _| s.contains(k.0));
        }
    }
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::new());
}

fn hash_cell(c: &Cell) -> u64 {
    let mut h = DefaultHasher::new();
    c.hash(&mut h);
    h.finish()
}

fn cells(r: Rect) -> impl Iterator<Item = (u16, u16)> {
    (r.top()..r.bottom()).flat_map(move |y| (r.left()..r.right()).map(move |x| (x, y)))
}

/// Blank cells (keeping their style) that ratatui writes out.
fn blank(buf: &mut Buffer, rect: Rect) {
    for p in cells(rect.intersection(buf.area)) {
        if let Some(c) = buf.cell_mut(p) {
            c.set_symbol(" ").set_diff_option(CellDiffOption::None);
        }
    }
}

/// `[W×H]` in the middle of `rect` (colour mode `mono`).
fn placeholder(buf: &mut Buffer, rect: Rect, size: [usize; 2]) {
    let text = format!("[{}x{}]", size[0], size[1]);
    let w = (text.len() as u16).min(rect.width);
    let x = rect.x + (rect.width - w) / 2;
    let y = rect.y + rect.height / 2;
    buf.set_stringn(x, y, &text, usize::from(w), Style::default());
}

// ---- public API ---------------------------------------------------------------------

/// What [`draw`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drawn {
    /// The image is on screen.
    Image,
    /// Being encoded (or blanked for one frame first); a frame follows by itself.
    Pending,
    /// `[W×H]` instead of the image (`[tui] color_mode = "mono"` with half-blocks).
    Placeholder,
    /// Not a terminal UI texture, freed or evicted, or the encoder failed: nothing drawn.
    Missing,
    /// `[tui] images = "off"`: nothing drawn.
    Off,
    /// The area is empty: nothing drawn.
    NoRoom,
}

/// Per call site: options and what the last [`draw`] did.
#[derive(Clone, Debug, Default)]
pub struct ImageState {
    /// Scale images up to fill the area (default: at most their own pixel size).
    pub upscale: bool,
    /// The cells the image took in the last draw.
    pub drawn: Option<Rect>,
    /// The last result.
    pub status: Option<Drawn>,
}

/// Draw `tex` fitted and centred in `area` (aspect kept).
pub fn draw(frame: &mut Frame<'_>, area: Rect, tex: &Tex, state: &mut ImageState) -> Drawn {
    draw_in(frame.buffer_mut(), area, tex, state)
}

/// [`draw`] into a buffer (widgets).
pub fn draw_in(buf: &mut Buffer, area: Rect, tex: &Tex, state: &mut ImageState) -> Drawn {
    let r = ENGINE.with_borrow_mut(|e| e.draw(buf, area, tex, state));
    state.status = Some(r);
    r
}

/// Make the terminal UI's uploader (pass it to `AppCore::new`) and remember `wake` for
/// the encoder thread. Call on the UI thread.
pub fn install(config: &Config, wake: Wake) -> Uploader {
    let store = Store::new(store::cap_for(config));
    let up = store::uploader(&store);
    ENGINE.with_borrow_mut(|e| {
        e.wake = wake;
        e.store = Some(store);
        e.worker = None;
        e.sync(&config.tui);
    });
    up
}

/// Every frame before drawing: follow `[tui]` and the budgets, collect encoded images.
pub fn begin_frame(config: &Config) {
    ENGINE.with_borrow_mut(|e| e.begin_frame(config));
}

/// Every frame after everything was drawn (inside `Terminal::draw`): repair images that
/// were drawn over (see the module docs).
pub fn end_frame(frame: &mut Frame<'_>) {
    end_frame_in(frame.buffer_mut());
}

/// [`end_frame`] on a buffer.
pub fn end_frame_in(buf: &mut Buffer) {
    ENGINE.with_borrow_mut(|e| e.end_frame(buf));
}

/// Forget what the screen shows: graphics images are blanked and drawn again.
pub fn invalidate() {
    ENGINE.with_borrow_mut(|e| {
        e.shown.clear();
        e.request_redraw();
    });
}

/// Whether images asked for another frame since the last call (tests; the loop is woken
/// anyway).
pub fn take_redraw() -> bool {
    ENGINE.with_borrow_mut(|e| std::mem::take(&mut e.redraw))
}

/// The protocol images are drawn with now (`None`: off).
pub fn active() -> Option<ProtocolType> {
    ENGINE.with_borrow(|e| match &e.backend {
        Backend::Off => None,
        Backend::Blocks => Some(ProtocolType::Halfblocks),
        Backend::Graphics(p) => Some(p.protocol_type()),
    })
}

/// The colour mode half-block images use now.
pub fn color_mode() -> ColorMode {
    ENGINE.with_borrow(|e| e.color)
}

/// Counters for tests and diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Images in the store and their bytes.
    pub stored: (usize, u64),
    /// Cached half-block encodings.
    pub blocks: usize,
    /// Cached graphics encodings.
    pub protos: usize,
    /// Encodings in flight.
    pub pending: usize,
}

/// See [`Stats`].
pub fn stats() -> Stats {
    ENGINE.with_borrow(|e| Stats {
        stored: e.store.as_ref().map_or((0, 0), |s| s.stats()),
        blocks: e.blocks.map.len(),
        protos: e.protos.map.len(),
        pending: e.pending.len(),
    })
}

/// Wait up to `timeout` until no encoding is in flight (tests). Returns whether all
/// finished.
pub fn wait_encoded(timeout: Duration) -> bool {
    let until = Instant::now() + timeout;
    loop {
        let (left, done) = ENGINE.with_borrow_mut(|e| {
            e.collect();
            if e.pending.is_empty() {
                return (Duration::ZERO, true);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return (left, false);
            }
            if let Some(d) = e.worker.as_ref().and_then(|w| w.wait(left.min(Duration::from_millis(50))))
                && e.pending.remove(&d.key)
            {
                e.protos.insert(d.key, d.proto);
            }
            (left, e.pending.is_empty())
        });
        if done || left.is_zero() {
            return done;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use soyuz_appcore::services::preview_gfx::Pixels;

    #[test]
    fn settings_map_to_modes() {
        let t = |images| TuiSection { images, ..TuiSection::default() };
        assert_eq!(mode(&TuiSection::default()), ImageMode::Auto);
        assert_eq!(mode(&t(TuiImages::Off)), ImageMode::Off);
        assert_eq!(mode(&t(TuiImages::Sixel)), ImageMode::Forced(ProtocolType::Sixel));
        assert_eq!(mode(&t(TuiImages::Halfblocks)), ImageMode::Forced(ProtocolType::Halfblocks));
        assert_eq!(cell_pixels(&TuiSection { cell_pixels: [0, 999], ..TuiSection::default() }), (1, 200));
    }

    fn env<'a>(v: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| v.iter().find(|(n, _)| *n == k).map(|(_, x)| x.to_string())
    }

    #[test]
    fn protocol_choice() {
        let xterm = [("TERM", "xterm-256color")];
        let c = |i, v: &[(&str, &str)], win| choose(i, &env(v), win);
        assert_eq!(c(TuiImages::Auto, &xterm, false), Choice::Query, "unix asks");
        assert_eq!(c(TuiImages::Auto, &xterm, true), Choice::Halfblocks, "Windows never asks");
        assert_eq!(c(TuiImages::Auto, &[("TERM", "tmux-256color")], false), Choice::Halfblocks);
        assert_eq!(c(TuiImages::Auto, &[("TERM", "xterm"), ("TMUX", "/tmp/t,1,0")], false), Choice::Halfblocks);
        assert_eq!(c(TuiImages::Auto, &[("TERM", "screen.xterm")], false), Choice::Halfblocks);
        assert_eq!(c(TuiImages::Auto, &[("TERM", "xterm"), ("TERM_PROGRAM", "tmux")], false), Choice::Halfblocks);
        assert_eq!(c(TuiImages::Auto, &[("TERM", "xterm"), ("TMUX", "")], false), Choice::Query, "empty TMUX");
        assert_eq!(c(TuiImages::Auto, &[("TERM", "linux")], false), Choice::Halfblocks);
        assert_eq!(c(TuiImages::Auto, &[("TERM", "dumb")], false), Choice::Halfblocks);
        assert_eq!(c(TuiImages::Auto, &[], false), Choice::Query);
        for win in [false, true] {
            let tmux = [("TMUX", "x")];
            assert_eq!(c(TuiImages::Sixel, &tmux, win), Choice::Protocol(ProtocolType::Sixel), "forced");
            assert_eq!(c(TuiImages::Kitty, &xterm, win), Choice::Protocol(ProtocolType::Kitty));
            assert_eq!(c(TuiImages::Iterm2, &xterm, win), Choice::Protocol(ProtocolType::Iterm2));
            assert_eq!(c(TuiImages::Halfblocks, &xterm, win), Choice::Halfblocks);
            assert_eq!(c(TuiImages::Off, &xterm, win), Choice::Off);
        }
    }

    #[test]
    fn backends_for_choices() {
        let kind = |b: &Backend| match b {
            Backend::Off => None,
            Backend::Blocks => Some(ProtocolType::Halfblocks),
            Backend::Graphics(p) => Some(p.protocol_type()),
        };
        let (b, cell) = backend_for(Choice::Protocol(ProtocolType::Sixel), None, (8, 16));
        assert_eq!((kind(&b), cell), (Some(ProtocolType::Sixel), (8, 16)));
        let (b, _) = backend_for(Choice::Query, None, (8, 16));
        assert_eq!(kind(&b), Some(ProtocolType::Halfblocks), "no answer: half-blocks");
        #[allow(deprecated)]
        let mut answered = Picker::from_fontsize(FontSize::new(9, 18));
        answered.set_protocol_type(ProtocolType::Kitty);
        let (b, cell) = backend_for(Choice::Query, Some(answered), (8, 16));
        assert_eq!((kind(&b), cell), (Some(ProtocolType::Kitty), (9, 18)), "the terminal's cell size");
        assert_eq!(kind(&backend_for(Choice::Off, None, (8, 16)).0), None);
        assert_eq!(kind(&backend_for(Choice::Halfblocks, None, (8, 16)).0), Some(ProtocolType::Halfblocks));
    }

    #[test]
    fn fit_keeps_aspect_and_centres() {
        let a = Rect::new(2, 1, 20, 10);
        // 400 x 200 px in 200 x 200 px (cells of 10 x 20): 200 x 100 px = 20 x 5 cells.
        assert_eq!(fit([400, 200], a, (10, 20), false), Some((Rect::new(2, 3, 20, 5), (200, 100))));
        // Tall: 100 x 400 into 200 x 200 → 50 x 200 = 5 x 10 cells.
        assert_eq!(fit([100, 400], a, (10, 20), false), Some((Rect::new(9, 1, 5, 10), (50, 200))));
        // Small images are not scaled up …
        assert_eq!(fit([20, 20], a, (10, 20), false), Some((Rect::new(11, 5, 2, 1), (20, 20))));
        // … unless asked.
        assert_eq!(fit([20, 20], a, (10, 20), true).map(|f| f.1), Some((200, 200)));
        assert_eq!(fit([20, 20], Rect::new(0, 0, 0, 5), (10, 20), false), None);
        assert_eq!(fit([0, 20], a, (10, 20), false), None);
        // Tiny areas never panic.
        for w in 0..=20 {
            for h in 0..=6 {
                if let Some((r, _)) = fit([1000, 3], Rect::new(0, 0, w, h), (1, 200), true) {
                    assert!(r.width >= 1 && r.width <= w && r.height >= 1 && r.height <= h);
                }
            }
        }
    }

    fn red_tex(up: &Uploader, w: usize, h: usize) -> Tex {
        let rgba: Vec<u8> = (0..w * h).flat_map(|_| [255, 0, 0, 255]).collect();
        Tex::upload(up, "red".into(), Pixels { size: [w, h], rgba: &rgba, premultiplied: false })
    }

    fn config(images: TuiImages, color: TuiColorMode) -> Config {
        let mut c = Config::default();
        c.tui.images = images;
        c.tui.color_mode = color;
        c
    }

    #[test]
    fn halfblocks_draw_with_test_backend() {
        let c = config(TuiImages::Halfblocks, TuiColorMode::Truecolor);
        let up = install(&c, no_wake());
        begin_frame(&c);
        assert_eq!(active(), Some(ProtocolType::Halfblocks));
        // 200 x 100 px, cells 10 x 20: 20 x 5 cells, 20 x 10 half-block pixels.
        let tex = red_tex(&up, 200, 100);
        let mut term = Terminal::new(TestBackend::new(30, 9)).unwrap();
        let mut st = ImageState::default();
        let mut r = Drawn::Off;
        term.draw(|f| {
            r = draw(f, Rect::new(5, 2, 20, 5), &tex, &mut st);
            end_frame(f);
        })
        .unwrap();
        assert_eq!(r, Drawn::Image);
        assert_eq!(st.drawn, Some(Rect::new(5, 2, 20, 5)));
        let buf = term.backend().buffer();
        assert_eq!(buf[(5, 2)].symbol(), "█");
        assert_eq!(buf[(5, 2)].fg, ratatui::style::Color::Rgb(255, 0, 0));
        assert_eq!(buf[(24, 6)].symbol(), "█");
        assert_eq!(buf[(4, 2)].symbol(), " ", "outside");
        assert_eq!(buf[(25, 2)].symbol(), " ", "outside");
        assert_eq!(stats().blocks, 1);
        // Redraw: cached.
        term.draw(|f| {
            draw(f, Rect::new(5, 2, 20, 5), &tex, &mut st);
        })
        .unwrap();
        assert_eq!(stats().blocks, 1, "no new encoding");

        // 256 colours, 16 colours, mono.
        for (mode, want) in [
            (TuiColorMode::Ansi256, Some(ratatui::style::Color::Indexed(196))),
            (TuiColorMode::Ansi16, Some(ratatui::style::Color::LightRed)),
            (TuiColorMode::Mono, None),
        ] {
            let c = config(TuiImages::Halfblocks, mode);
            begin_frame(&c);
            let mut term = Terminal::new(TestBackend::new(30, 9)).unwrap();
            term.draw(|f| {
                r = draw(f, Rect::new(5, 2, 20, 5), &tex, &mut st);
            })
            .unwrap();
            let cell = &term.backend().buffer()[(5, 2)];
            match want {
                Some(col) => assert_eq!((r, cell.fg), (Drawn::Image, col), "{mode:?}"),
                None => {
                    assert_eq!(r, Drawn::Placeholder);
                    let text: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
                    assert!(text.contains("[200x100]"), "{text}");
                }
            }
        }
        // Off.
        let c = config(TuiImages::Off, TuiColorMode::Truecolor);
        begin_frame(&c);
        assert_eq!(active(), None);
        let mut term = Terminal::new(TestBackend::new(30, 9)).unwrap();
        term.draw(|f| r = draw(f, f.area(), &tex, &mut st)).unwrap();
        assert_eq!(r, Drawn::Off);
        // Freed textures are missing.
        let c = config(TuiImages::Halfblocks, TuiColorMode::Truecolor);
        begin_frame(&c);
        let other = tex.clone();
        drop(tex);
        term.draw(|f| r = draw(f, f.area(), &other, &mut st)).unwrap();
        assert_eq!(r, Drawn::Image, "a clone keeps it");
        let foreign = red_tex(&soyuz_appcore::services::preview_gfx::no_upload(), 4, 4);
        term.draw(|f| r = draw(f, f.area(), &foreign, &mut st)).unwrap();
        assert_eq!(r, Drawn::Missing);
    }

    #[test]
    fn tiny_areas_never_panic() {
        let c = config(TuiImages::Halfblocks, TuiColorMode::Truecolor);
        let up = install(&c, no_wake());
        begin_frame(&c);
        let tex = red_tex(&up, 7, 300);
        for w in 0..=20 {
            for h in 0..=6 {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                let mut st = ImageState { upscale: w % 2 == 0, ..ImageState::default() };
                term.draw(|f| {
                    draw(f, f.area(), &tex, &mut st);
                    draw(f, Rect::new(1, 1, w, h), &tex, &mut st);
                    end_frame(f);
                })
                .unwrap();
            }
        }
    }

    /// Sixel: blank first, the image next frame, un-skipped overlay cells, blank again
    /// when the overlay goes away, then the image.
    #[test]
    fn sixel_repaints_after_overlays() {
        let c = config(TuiImages::Sixel, TuiColorMode::Truecolor);
        let up = install(&c, no_wake());
        begin_frame(&c);
        assert_eq!(active(), Some(ProtocolType::Sixel));
        let tex = red_tex(&up, 40, 40);
        let area = Rect::new(0, 0, 10, 4);
        let mut term = Terminal::new(TestBackend::new(12, 6)).unwrap();
        let mut st = ImageState::default();
        let frame = |term: &mut Terminal<TestBackend>, st: &mut ImageState, overlay: bool| {
            begin_frame(&c);
            let mut r = Drawn::Off;
            term.draw(|f| {
                r = draw(f, area, &tex, st);
                if overlay {
                    f.buffer_mut().set_string(2, 2, "dlg", Style::default());
                }
                end_frame(f);
            })
            .unwrap();
            r
        };
        assert_eq!(frame(&mut term, &mut st, false), Drawn::Pending, "encoding");
        assert!(wait_encoded(Duration::from_secs(20)), "the encoder finished");
        assert_eq!(stats().protos, 1);
        assert_eq!(frame(&mut term, &mut st, false), Drawn::Image, "the blank frame was settled");
        let rect = st.drawn.unwrap();
        let anchor = term.backend().buffer()[(rect.x, rect.y)].symbol().to_string();
        assert!(anchor.contains("\x1bP"), "a sixel sequence in the first cell: {anchor:?}");

        // An overlay over it: its cells are written out.
        assert_eq!(frame(&mut term, &mut st, true), Drawn::Image);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 6));
        begin_frame(&c);
        draw_in(&mut buf, area, &tex, &mut st);
        buf.set_string(2, 2, "dlg", Style::default());
        end_frame_in(&mut buf);
        assert_eq!(buf[(2, 2)].diff_option, CellDiffOption::None, "overlay text is not skipped");
        // Gone: blanked this frame, redraw asked for, then the image again.
        let _ = take_redraw();
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 6));
        begin_frame(&c);
        assert_eq!(draw_in(&mut buf, area, &tex, &mut st), Drawn::Image);
        end_frame_in(&mut buf);
        assert!(take_redraw(), "another frame follows");
        assert_eq!(buf[(rect.x, rect.y)].symbol(), " ", "blanked");
        assert_eq!(buf[(rect.x + 1, rect.y)].diff_option, CellDiffOption::None);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 6));
        begin_frame(&c);
        assert_eq!(draw_in(&mut buf, area, &tex, &mut st), Drawn::Image);
        end_frame_in(&mut buf);
        assert!(buf[(rect.x, rect.y)].symbol().contains("\x1bP"));
        assert!(!take_redraw());

        // invalidate(): blank, then the image.
        invalidate();
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 6));
        begin_frame(&c);
        assert_eq!(draw_in(&mut buf, area, &tex, &mut st), Drawn::Pending);
        end_frame_in(&mut buf);
        assert_eq!(buf[(rect.x, rect.y)].symbol(), " ");
    }
}
