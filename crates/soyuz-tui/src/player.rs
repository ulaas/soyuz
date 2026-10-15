//! The terminal UI's side of the media player (Phase V3b): the preview pane and Quick
//! Look play the video or audio file they show through the core's
//! [`PlayerModel`](soyuz_appcore::models::player::PlayerModel) (`core.ui.player`, one
//! player at a time, the shared `Player*` actions).
//!
//! What this module adds on top of the core's model:
//!
//! - **Opening off the UI thread.** `soyuz_media::Player::open` reads the file, so a worker
//!   thread opens it ([`State::pending`]) and the frame shows "Opening…" meanwhile; the
//!   engine is then handed to the model with
//!   [`PlayerModel::open_with`](soyuz_appcore::models::player::PlayerModel::open_with).
//!   It is opened with `fps_cap = [media] tui_fps`: the decoder presents at most that many
//!   frames per second, each one a terminal image.
//! - **Frames as terminal images.** The engine's frame sink ([`Sink`]) uploads every
//!   presented picture through the terminal UI's own texture store (`crate::images`), so
//!   the views draw it like any picture (kitty / sixel / iTerm2 / half-blocks; a text line
//!   when `[tui] images = "off"`, the sound still plays). The pane's pixel size is sent
//!   to the engine (`SetOutputSize`), so frames are decoded no bigger than the cells show.
//! - **Ownership.** Quick Look owns the player while it is open, else the preview pane
//!   ([`prepare`], every frame after the views polled their loaders). Moving to another
//!   file closes the player (`sync_target`); the same file in both views keeps it.
//! - **The status line** ([`StatusVm`]): the play / pause mark, elapsed / total, a text
//!   progress bar, the volume, `muted` and `loop`; words instead in screen-reader mode
//!   ([`StatusVm::words`]).
//!
//! `[media] player = false`: nothing opens (poster frame and details only); `autoplay`,
//! `volume`, `muted` and `seek_step_secs` are the engine's options through the model;
//! `loop` is the model's own toggle (`PlayerModel::loop_enabled`, seeded by the key), so
//! the engine is opened without its own loop. The keys are the core's `KeyContext::Player`
//! bindings (K / J / L, ← / →, - / =, M by default), active while Quick Look or the focused preview pane shows an open player
//! (`crate::input::preview`, `crate::input::quicklook`); the mouse clicks the mark and
//! the bar (`HitTarget::PlayerPlay` / `PlayerSeek`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::Receiver;
use parking_lot::Mutex;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Wake;
use soyuz_appcore::models::player::{PlayerModel, options};
use soyuz_appcore::services::media;
use soyuz_appcore::services::media_player::RodioOutput;
use soyuz_appcore::services::preview::Kind;
use soyuz_appcore::services::preview_gfx::{Pixels, Tex, Uploader};
use soyuz_appcore::tr;
use soyuz_media::{FramePixels, FrameSink, PlayerState, Source};

use crate::dialog::safe;
use crate::icons::IconSet;
use crate::input::InputState;

/// The latest presented picture (cheap to clone).
#[derive(Clone, Debug)]
pub struct Frame {
    pub tex: Tex,
    /// Presentation time (seconds).
    pub pts: f64,
    /// Counts up with every presented frame.
    pub serial: u64,
}

/// The engine's frame sink: uploads on the video thread, keeps the latest frame.
struct Sink {
    upload: Uploader,
    name: String,
    slot: Arc<Mutex<Option<Frame>>>,
    serial: u64,
}

impl FrameSink for Sink {
    fn present(&mut self, frame: FramePixels<'_>) {
        let (w, h) = (frame.width as usize, frame.height as usize);
        if w == 0 || h == 0 || frame.rgba.len() < w * h * 4 {
            return;
        }
        let px = Pixels { size: [w, h], rgba: &frame.rgba[..w * h * 4], premultiplied: false };
        let tex = Tex::upload(&self.upload, self.name.clone(), px);
        self.serial += 1;
        *self.slot.lock() = Some(Frame { tex, pts: frame.pts, serial: self.serial });
    }
}

/// A player being opened on a worker thread.
struct Pending {
    key: String,
    rx: Receiver<Result<soyuz_media::Player, String>>,
}

/// What the file a view shows asks of the player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    /// The identity of the file for the model (its path).
    pub key: String,
    pub path: PathBuf,
    /// It has a picture stream (an audio file plays sound only).
    pub video: bool,
    /// The longest edge of the cells the picture gets, in pixels (0: unknown yet).
    pub px: u32,
}

impl Want {
    /// What the loader's target asks, when it is an audio or video file whose details
    /// have loaded (the poster frame or cover shows first).
    pub fn of(loader: &soyuz_appcore::models::preview_load::PreviewLoader, px: u32) -> Option<Want> {
        let entry = loader.entry.as_ref()?;
        let kind = loader.loaded.as_ref()?.kind;
        if !matches!(kind, Kind::Audio | Kind::Video) || loader.force_hex || entry.is_dir_like() {
            return None;
        }
        Some(Want { key: key_of(&entry.path), path: entry.path.clone(), video: kind == Kind::Video, px })
    }
}

/// The model's key for a file.
pub fn key_of(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// What the terminal UI keeps about the player between frames.
#[derive(Default)]
pub struct State {
    pending: Option<Pending>,
    /// Frames of the open engine (shared with its video thread).
    slot: Arc<Mutex<Option<Frame>>>,
    /// The file that could not be opened and why (shown until the target changes).
    error: Option<(String, String)>,
    /// The output size last sent.
    px: Option<u32>,
    /// Engines opened so far (texture names stay unique).
    opened: u64,
    /// Who asked for the player this frame (the views read it back for their bodies).
    want: Option<Want>,
    /// What the views ask this frame (`view::preview::prepare`, `view::quicklook::prepare`
    /// fill it; [`prepare`] takes it).
    pub ask: Wants,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("player::State")
            .field("pending", &self.pending.as_ref().map(|p| &p.key))
            .field("error", &self.error)
            .field("want", &self.want)
            .finish_non_exhaustive()
    }
}

impl State {
    /// The file the player is for this frame.
    pub fn want(&self) -> Option<&Want> {
        self.want.as_ref()
    }

    /// The latest frame of the open engine.
    pub fn frame(&self) -> Option<Frame> {
        self.slot.lock().clone()
    }

    /// An open is in flight for `key`.
    pub fn opening(&self, key: &str) -> bool {
        self.pending.as_ref().is_some_and(|p| p.key == key)
    }

    /// Why `key` could not be opened.
    pub fn error(&self, key: &str) -> Option<&str> {
        self.error.as_ref().filter(|(k, _)| k == key).map(|(_, e)| e.as_str())
    }

    fn drop_pending(&mut self) {
        // The worker's result is dropped with the receiver: the engine stops itself.
        self.pending = None;
    }
}

/// What the preview pane asks this frame (`view::preview::prepare`) and what Quick Look
/// asks (`view::quicklook::prepare`), decided in [`prepare`]: Quick Look wins while it is
/// open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wants {
    pub preview: Option<Want>,
    pub quicklook: Option<Want>,
}

/// Every frame after the views polled their loaders (`view::prepare`): open, keep or
/// close the player for the file they show.
pub fn prepare(core: &mut AppCore, input: &mut InputState) {
    let st = &mut input.player;
    let wants = std::mem::take(&mut st.ask);
    let want = wants.quicklook.or(wants.preview);
    st.want = want.clone();
    let model: &mut PlayerModel = &mut core.ui.player;
    let Some(want) = want else {
        model.sync_target(None);
        st.drop_pending();
        st.px = None;
        *st.slot.lock() = None;
        return;
    };
    model.sync_target(Some(&want.key));
    if st.pending.as_ref().is_some_and(|p| p.key != want.key) {
        st.drop_pending();
    }
    if st.error.as_ref().is_some_and(|(k, _)| k != &want.key) {
        st.error = None;
    }
    let cfg = &core.config.media;
    if model.key() == Some(want.key.as_str()) {
        if want.px > 0 && st.px != Some(want.px) {
            st.px = Some(want.px);
            model.set_output_size(want.px);
        }
        return;
    }
    // Not open: take the worker's answer, or start one.
    if let Some(p) = &st.pending {
        match p.rx.try_recv() {
            Ok(Ok(player)) => {
                let key = p.key.clone();
                st.pending = None;
                model.open_with(&key, Box::new(player), cfg);
                st.px = None;
                if want.px > 0 {
                    st.px = Some(want.px);
                    model.set_output_size(want.px);
                }
                (core.wake)();
            }
            Ok(Err(e)) => {
                let key = p.key.clone();
                st.pending = None;
                st.error = Some((key, e));
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                let key = p.key.clone();
                st.pending = None;
                st.error = Some((key, tr!("tui-player-error", error = "worker gone")));
            }
        }
        return;
    }
    if !cfg.player || st.error.is_some() {
        return;
    }
    let Some(rt) = media::runtime() else { return };
    st.opened += 1;
    *st.slot.lock() = None;
    let sink = Sink {
        upload: core.uploader.clone(),
        name: format!("tui-player:{}", st.opened),
        slot: Arc::clone(&st.slot),
        serial: 0,
    };
    let mut opts = options(cfg);
    opts.fps_cap = f32::from(cfg.tui_fps.clamp(1, 30));
    // The model loops (`PlayerModel::loop_enabled`, restarted by its `poll`), not the
    // engine: the toggle in the model is the single source, as in the GUI.
    opts.loop_ = false;
    let wake: Wake = core.wake.clone();
    let (tx, rx) = crossbeam_channel::bounded(1);
    let src = Source::File(want.path.clone());
    let spawned = std::thread::Builder::new().name("soyuz-tui-player-open".into()).spawn(move || {
        let r = soyuz_media::Player::open(rt, src, opts, Box::new(sink), Some(Arc::new(RodioOutput)), wake.clone())
            .map_err(|e| safe(&e.to_string()));
        // Nobody waiting any more (the target moved on): the engine stops with the drop.
        let _ = tx.send(r);
        wake();
    });
    match spawned {
        Ok(_) => st.pending = Some(Pending { key: want.key.clone(), rx }),
        Err(e) => st.error = Some((want.key.clone(), safe(&e.to_string()))),
    }
}

/// Whether the open (or opening) player is the one `key` asks for: the views then draw
/// its body, and the `Player` key context is active with their focus.
pub fn shows(core: &AppCore, st: &State, key: &str) -> bool {
    core.ui.player.key() == Some(key) || st.opening(key) || st.error(key).is_some()
}

/// The `Player` key context applies: the file in front (Quick Look, else the preview
/// pane) has the open player.
pub fn active(core: &AppCore, st: &State) -> bool {
    st.want.as_ref().is_some_and(|w| core.ui.player.key() == Some(w.key.as_str()))
}

/// Seek to `fraction` (0 … 1) of the duration (nothing without a known duration).
pub fn seek_fraction(core: &AppCore, fraction: f64) {
    let Some(d) = core.ui.player.state().and_then(|s| s.duration).filter(|d| *d > 0.0) else { return };
    core.ui.player.seek(d * fraction.clamp(0.0, 1.0));
}

// ---- the status line ----------------------------------------------------------------

/// Where the player stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// The worker is opening the file.
    Opening,
    Playing,
    Paused,
    /// Reached the end (`[media] loop` off).
    Ended,
    /// Could not open or stopped with an error.
    Error(String),
}

/// The status line as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusVm {
    pub status: Status,
    /// Seconds, rounded to tenths (so equal frames compare equal).
    pub pos_ds: u64,
    pub duration_ds: Option<u64>,
    /// 0 … 100.
    pub volume: u8,
    pub muted: bool,
    pub loop_: bool,
    /// The file has a picture stream.
    pub video: bool,
}

impl StatusVm {
    /// From the engine's snapshot and the `[media]` options.
    pub fn from_state(s: &PlayerState, loop_: bool) -> StatusVm {
        let status = if let Some(e) = &s.error {
            Status::Error(safe(e))
        } else if s.playing {
            Status::Playing
        } else if s.ended {
            Status::Ended
        } else {
            Status::Paused
        };
        StatusVm {
            status,
            pos_ds: ds(s.pos),
            duration_ds: s.duration.filter(|d| d.is_finite() && *d > 0.0).map(ds),
            volume: (s.volume.clamp(0.0, 1.0) * 100.0).round() as u8,
            muted: s.muted,
            loop_,
            video: s.video.is_some(),
        }
    }

    /// Before the engine answered.
    pub fn opening(video: bool) -> StatusVm {
        StatusVm { status: Status::Opening, pos_ds: 0, duration_ds: None, volume: 0, muted: false, loop_: false, video }
    }

    /// An open that failed.
    pub fn failed(error: &str, video: bool) -> StatusVm {
        StatusVm { status: Status::Error(error.to_string()), ..StatusVm::opening(video) }
    }

    /// The state in words ("Playing", "Paused", …).
    pub fn state_words(&self) -> String {
        match &self.status {
            Status::Opening => tr!("tui-player-opening").into(),
            Status::Playing => tr!("tui-player-playing").into(),
            Status::Paused => tr!("tui-player-paused").into(),
            Status::Ended => tr!("tui-player-ended").into(),
            Status::Error(e) => tr!("tui-player-error", error = e.as_str()),
        }
    }

    /// Elapsed `/` total (`0:12 / 1:30`; `0:12` without a known duration).
    pub fn times(&self) -> String {
        match self.duration_ds {
            Some(d) => format!("{} / {}", clock(self.pos_ds, d), clock(d, d)),
            None => clock(self.pos_ds, self.pos_ds),
        }
    }

    /// Position as a fraction of the duration (`None` without one).
    pub fn fraction(&self) -> Option<f64> {
        let d = self.duration_ds?;
        (d > 0).then(|| (self.pos_ds as f64 / d as f64).clamp(0.0, 1.0))
    }

    /// The whole line in words for screen readers: "Playing, 0:12 of 1:30, volume 80 %,
    /// muted, loop".
    pub fn words(&self) -> String {
        let mut parts = vec![self.state_words()];
        if !matches!(self.status, Status::Opening | Status::Error(_)) {
            parts.push(match self.duration_ds {
                Some(d) => tr!("tui-sr-player-time", pos = clock(self.pos_ds, d), total = clock(d, d)),
                None => clock(self.pos_ds, self.pos_ds),
            });
            parts.push(tr!("tui-player-volume", percent = self.volume));
            if self.muted {
                parts.push(tr!("tui-player-muted").into());
            }
            if self.loop_ {
                parts.push(tr!("tui-player-loop").into());
            }
        }
        parts.join(", ")
    }

    /// The line's pieces for `width` cells: the mark, the times, the bar (its column and
    /// width in the line, for the click target) and the trailing flags. Pieces are dropped
    /// from the right when there is no room; the bar keeps at least [`BAR_MIN`] cells.
    pub fn layout(&self, width: usize, icons: IconSet) -> Line {
        let mark = match (&self.status, icons) {
            (Status::Playing, IconSet::Ascii) => "||",
            (Status::Playing, _) => "⏸",
            (Status::Error(_), IconSet::Ascii) => "!",
            (Status::Error(_), _) => "⚠",
            (_, IconSet::Ascii) => ">",
            (_, _) => "▶",
        };
        let mark_w = crate::view::text::width(mark);
        let times =
            if matches!(self.status, Status::Opening | Status::Error(_)) { self.state_words() } else { self.times() };
        let mut flags: Vec<String> = Vec::new();
        if !matches!(self.status, Status::Opening | Status::Error(_)) {
            flags.push(format!("{}%", self.volume));
            if self.muted {
                flags.push(tr!("tui-player-muted").into());
            }
            if self.loop_ {
                flags.push(tr!("tui-player-loop").into());
            }
        }
        let mut line = Line { mark: mark.to_string(), times: String::new(), bar: None, flags: String::new() };
        let mut used = mark_w;
        if used + 1 + crate::view::text::width(&times) <= width {
            used += 1 + crate::view::text::width(&times);
            line.times = times;
        } else {
            line.times = crate::view::text::truncate_end(&times, width.saturating_sub(used + 1));
            return line;
        }
        // The flags from the right, as many as fit while the bar keeps its minimum.
        let mut shown: Vec<String> = Vec::new();
        for f in &flags {
            let fw = crate::view::text::width(f) + 1;
            if used + fw + 1 + BAR_MIN <= width {
                used += fw;
                shown.push(f.clone());
            } else {
                break;
            }
        }
        line.flags = shown.join(" ");
        let room = width.saturating_sub(used + 1);
        if room >= BAR_MIN && self.fraction().is_some() {
            line.bar = Some((mark_w + 1 + crate::view::text::width(&line.times) + 1, room));
        }
        line
    }
}

/// The bar's minimum width in cells.
pub const BAR_MIN: usize = 4;

/// The status line's pieces ([`StatusVm::layout`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub mark: String,
    pub times: String,
    /// The bar's first column (from the line's start) and width.
    pub bar: Option<(usize, usize)>,
    pub flags: String,
}

/// The progress bar: `width` cells, the head at `fraction`.
pub fn bar(width: usize, fraction: f64, icons: IconSet) -> String {
    if width == 0 {
        return String::new();
    }
    let (done, head, rest) = match icons {
        IconSet::Ascii => ('=', 'o', '-'),
        _ => ('━', '●', '─'),
    };
    let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
    let at = ((width - 1) as f64 * f).round() as usize;
    (0..width)
        .map(|i| {
            if i < at {
                done
            } else if i == at {
                head
            } else {
                rest
            }
        })
        .collect()
}

/// Deciseconds of `secs`.
fn ds(secs: f64) -> u64 {
    if secs.is_finite() && secs > 0.0 { (secs * 10.0).round() as u64 } else { 0 }
}

/// `m:ss`, or `h:mm:ss` when `total` (deciseconds) reaches an hour.
pub fn clock(ds: u64, total_ds: u64) -> String {
    let s = ds / 10;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if total_ds >= 36_000 { format!("{h}:{m:02}:{s:02}") } else { format!("{}:{s:02}", h * 60 + m) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm(playing: bool, pos: f64, dur: Option<f64>) -> StatusVm {
        let s = PlayerState {
            playing,
            pos,
            duration: dur,
            volume: 0.8,
            video: Some(soyuz_media::StreamInfo::default()),
            ..PlayerState::default()
        };
        StatusVm::from_state(&s, false)
    }

    #[test]
    fn clock_and_bar() {
        assert_eq!(clock(0, 0), "0:00");
        assert_eq!(clock(125, 900), "0:12");
        assert_eq!(clock(36_000, 36_000), "1:00:00");
        assert_eq!(clock(615, 36_000), "0:01:01");
        assert_eq!(bar(5, 0.0, IconSet::Ascii), "o----");
        assert_eq!(bar(5, 0.5, IconSet::Ascii), "==o--");
        assert_eq!(bar(5, 1.0, IconSet::Ascii), "====o");
        assert_eq!(bar(5, f64::NAN, IconSet::Unicode), "●────");
        assert_eq!(bar(0, 0.5, IconSet::Ascii), "");
    }

    #[test]
    fn status_from_state_and_words() {
        let v = vm(true, 12.3, Some(90.0));
        assert_eq!(v.status, Status::Playing);
        assert_eq!(v.times(), "0:12 / 1:30");
        assert_eq!(v.volume, 80);
        assert!((v.fraction().unwrap() - 12.3 / 90.0).abs() < 1e-3);
        assert_eq!(v.words(), "Playing, 0:12 of 1:30, volume 80 %");
        let mut p = vm(false, 0.0, None);
        p.muted = true;
        p.loop_ = true;
        assert_eq!(p.status, Status::Paused);
        assert_eq!(p.times(), "0:00");
        assert_eq!(p.fraction(), None);
        assert_eq!(p.words(), "Paused, 0:00, volume 80 %, muted, loop");
        let ended = StatusVm::from_state(&PlayerState { ended: true, ..PlayerState::default() }, false);
        assert_eq!(ended.status, Status::Ended);
        let err =
            StatusVm::from_state(&PlayerState { error: Some("x\x1b[2J".into()), ..PlayerState::default() }, false);
        assert!(matches!(&err.status, Status::Error(e) if !e.contains('\x1b')));
        assert_eq!(StatusVm::opening(true).words(), "Opening the player…");
        assert_eq!(StatusVm::failed("bad", false).words(), "Playback failed: bad");
    }

    #[test]
    fn layout_drops_pieces_from_the_right() {
        let mut v = vm(true, 12.0, Some(90.0));
        v.muted = true;
        let l = v.layout(40, IconSet::Ascii);
        assert_eq!((l.mark.as_str(), l.times.as_str()), ("||", "0:12 / 1:30"));
        assert_eq!(l.flags, "80% muted");
        let (x, w) = l.bar.unwrap();
        assert_eq!(x, 2 + 1 + 11 + 1);
        assert_eq!(x + w + 1 + 9, 40);
        // Narrow: the flags go, then the bar, then the times are cut.
        let l = v.layout(24, IconSet::Ascii);
        assert_eq!(l.flags, "80%");
        assert!(l.bar.is_some_and(|(_, w)| w >= BAR_MIN));
        let l = v.layout(16, IconSet::Ascii);
        assert_eq!(l.flags, "");
        assert_eq!(l.bar, None);
        let l = v.layout(8, IconSet::Ascii);
        assert!(crate::view::text::width(&l.times) <= 5, "{l:?}");
        for w in 0..=30 {
            v.layout(w, IconSet::Unicode);
        }
        // Opening: the words take the times' place.
        let l = StatusVm::opening(true).layout(40, IconSet::Unicode);
        assert_eq!(
            (l.mark.as_str(), l.times.as_str(), l.bar, l.flags.as_str()),
            ("▶", "Opening the player…", None, "")
        );
    }

    #[test]
    fn want_of_media_targets_only() {
        use soyuz_appcore::models::preview_load::PreviewLoader;
        use soyuz_core::{EntryInfo, EntryKind};
        let mut l = PreviewLoader::new(
            soyuz_appcore::host::no_wake(),
            soyuz_appcore::services::preview_gfx::no_upload(),
            false,
            1.0,
        );
        assert!(Want::of(&l, 100).is_none());
        l.entry = Some(EntryInfo::minimal(PathBuf::from("clip.avi"), EntryKind::File));
        assert!(Want::of(&l, 100).is_none(), "nothing loaded yet");
        l.loaded = Some(soyuz_appcore::services::preview::Loaded {
            kind: Kind::Video,
            content: soyuz_appcore::services::preview::Content::Media { video: true },
            facts: Vec::new(),
            meta: None,
        });
        let w = Want::of(&l, 100).unwrap();
        assert_eq!((w.key.as_str(), w.video, w.px), ("clip.avi", true, 100));
        l.loaded.as_mut().unwrap().kind = Kind::Audio;
        assert!(!Want::of(&l, 1).unwrap().video);
        l.loaded.as_mut().unwrap().kind = Kind::Text;
        assert!(Want::of(&l, 1).is_none());
        l.loaded.as_mut().unwrap().kind = Kind::Video;
        l.force_hex = true;
        assert!(Want::of(&l, 1).is_none(), "the hex view has no player");
    }

    #[test]
    fn state_starts_empty() {
        let st = State::default();
        assert!(st.want().is_none() && st.frame().is_none() && !st.opening("x") && st.error("x").is_none());
        assert!(format!("{st:?}").contains("player::State"));
    }
}
