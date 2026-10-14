//! The media player model (Phase V2b): at most one open [`soyuz_media::Player`], the
//! file the preview pane or Quick Look shows. Frontend-agnostic: the GUI (V3a) and the
//! terminal UI (V3b) draw [`PlayerModel::state`] and [`PlayerModel::frame`] and call the
//! commands; the player's threads call the core's wake-up when either changes.
//!
//! The model opens the player with the `[media]` options (`player`, `autoplay`, `loop`,
//! `volume`, `muted`, `seek_step_secs`, `max_px`, `threads`, `hw_decode`), the rodio
//! output ([`media_player::RodioOutput`]) and the core's texture uploader (every presented
//! picture becomes a [`Tex`] under one stable name per player, so a frontend can update
//! the texture in place). Volume and mute changes go through [`set_volume`] /
//! [`set_muted`], which write `[media] volume` / `muted` back to `config.toml` and apply
//! them to the open player and to the audio preview's player.

use std::sync::Arc;

use parking_lot::Mutex;
use soyuz_core::config::{HwDecode, Media as MediaCfg};
use soyuz_media::{FramePixels, FrameSink, HwPref, PlayerCmd, PlayerOptions, PlayerState, Source};

use crate::core::AppCore;
use crate::host::{Host, Wake};
use crate::services::media;
use crate::services::media_player::RodioOutput;
use crate::services::preview_gfx::{Pixels, Tex, Uploader};

/// The latest presented picture (cheap to clone).
#[derive(Clone)]
pub struct FrameSlot {
    pub tex: Tex,
    /// Presentation time (seconds).
    pub pts: f64,
    /// Counts up with every presented frame (a frontend skips unchanged ones).
    pub serial: u64,
}

/// The engine behind the model: [`soyuz_media::Player`], or a fake in tests.
pub trait Engine: Send {
    fn send(&self, cmd: PlayerCmd);
    fn state(&self) -> PlayerState;
}

impl Engine for soyuz_media::Player {
    fn send(&self, cmd: PlayerCmd) {
        soyuz_media::Player::send(self, cmd);
    }

    fn state(&self) -> PlayerState {
        soyuz_media::Player::state(self)
    }
}

/// Where presented frames land: uploaded through the core's uploader on the video
/// thread, then stored for the frontend.
struct CoreFrameSink {
    upload: Uploader,
    name: String,
    slot: Arc<Mutex<Option<FrameSlot>>>,
    serial: u64,
}

impl FrameSink for CoreFrameSink {
    fn present(&mut self, frame: FramePixels<'_>) {
        let (w, h) = (frame.width as usize, frame.height as usize);
        if w == 0 || h == 0 || frame.rgba.len() < w * h * 4 {
            return;
        }
        let pixels = Pixels { size: [w, h], rgba: &frame.rgba[..w * h * 4], premultiplied: false };
        let tex = Tex::upload(&self.upload, self.name.clone(), pixels);
        self.serial += 1;
        *self.slot.lock() = Some(FrameSlot { tex, pts: frame.pts, serial: self.serial });
    }
}

struct Open {
    key: String,
    engine: Box<dyn Engine>,
    slot: Arc<Mutex<Option<FrameSlot>>>,
}

/// The open player, if any, and what it was opened with.
#[derive(Default)]
pub struct PlayerModel {
    open: Option<Open>,
    /// Seconds one seek step moves (`[media] seek_step_secs`).
    seek_step: f64,
    /// `(volume, muted)` last pushed into the engine, to forward config changes.
    applied: Option<(u8, bool)>,
    /// Players opened so far (texture names stay unique).
    opened: u64,
}

impl std::fmt::Debug for PlayerModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerModel")
            .field("key", &self.key())
            .field("seek_step", &self.seek_step)
            .finish_non_exhaustive()
    }
}

/// [`PlayerOptions`] from the `[media]` keys.
pub fn options(cfg: &MediaCfg) -> PlayerOptions {
    PlayerOptions {
        max_px: cfg.max_px.clamp(64, 8192),
        fps_cap: 60.0,
        threads: cfg.threads,
        hw: match cfg.hw_decode {
            HwDecode::Auto => HwPref::Auto,
            HwDecode::Off => HwPref::Off,
            HwDecode::D3d11va => HwPref::D3d11va,
            HwDecode::Vaapi => HwPref::Vaapi,
            HwDecode::Videotoolbox => HwPref::Videotoolbox,
        },
        autoplay: cfg.autoplay,
        loop_: cfg.loop_,
        volume: f32::from(cfg.volume.min(100)) / 100.0,
        muted: cfg.muted,
    }
}

impl PlayerModel {
    /// Open `src` (identified by `key`, the preview's identity for the file) with the
    /// loaded FFmpeg libraries. Already open for the same key: nothing happens. Errors:
    /// the player is off (`[media] player`), no libraries, or the file cannot be opened.
    pub fn open(
        &mut self,
        key: &str,
        src: Source,
        cfg: &MediaCfg,
        uploader: &Uploader,
        wake: &Wake,
    ) -> Result<(), String> {
        if self.key() == Some(key) {
            return Ok(());
        }
        if !cfg.player {
            return Err(tr!("views-player-off").into());
        }
        let Some(rt) = media::runtime() else { return Err(tr!("services-audio-no-ffmpeg").into()) };
        self.close();
        self.opened += 1;
        let slot = Arc::new(Mutex::new(None));
        let sink = CoreFrameSink {
            upload: uploader.clone(),
            name: format!("player:{}", self.opened),
            slot: Arc::clone(&slot),
            serial: 0,
        };
        let player =
            soyuz_media::Player::open(rt, src, options(cfg), Box::new(sink), Some(Arc::new(RodioOutput)), wake.clone())
                .map_err(|e| tr!("views-player-cant-open", error = e.to_string()))?;
        self.install(key, Box::new(player), slot, cfg);
        Ok(())
    }

    /// Open with a ready engine (tests, and frontends with their own engine).
    pub fn open_with(&mut self, key: &str, engine: Box<dyn Engine>, cfg: &MediaCfg) {
        self.close();
        self.install(key, engine, Arc::new(Mutex::new(None)), cfg);
    }

    fn install(&mut self, key: &str, engine: Box<dyn Engine>, slot: Arc<Mutex<Option<FrameSlot>>>, cfg: &MediaCfg) {
        self.seek_step = f64::from(cfg.seek_step_secs.max(0.1));
        self.applied = Some((cfg.volume.min(100), cfg.muted));
        self.open = Some(Open { key: key.to_string(), engine, slot });
    }

    /// Close the player (stops the threads and frees the frame).
    pub fn close(&mut self) {
        if let Some(o) = self.open.take() {
            o.engine.send(PlayerCmd::Stop);
            // Dropping the player joins its threads (bounded, `soyuz_media::JOIN_TIMEOUT`):
            // not on the UI thread.
            let engine = o.engine;
            let _ = std::thread::Builder::new().name("soyuz-player-close".into()).spawn(move || drop(engine));
        }
    }

    /// Close unless `key` is what is open (the preview moved to another file).
    pub fn sync_target(&mut self, key: Option<&str>) {
        if self.open.is_some() && self.key() != key {
            self.close();
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The key the player was opened with.
    pub fn key(&self) -> Option<&str> {
        self.open.as_ref().map(|o| o.key.as_str())
    }

    /// A snapshot of the open player.
    pub fn state(&self) -> Option<PlayerState> {
        self.open.as_ref().map(|o| o.engine.state())
    }

    /// The latest presented picture of the open player.
    pub fn frame(&self) -> Option<FrameSlot> {
        self.open.as_ref().and_then(|o| o.slot.lock().clone())
    }

    /// Seconds one seek step moves.
    pub fn seek_step(&self) -> f64 {
        self.seek_step
    }

    /// Queue a command on the open player (ignored when none is open).
    pub fn send(&self, cmd: PlayerCmd) {
        if let Some(o) = &self.open {
            o.engine.send(cmd);
        }
    }

    pub fn play(&self) {
        self.send(PlayerCmd::Play);
    }

    pub fn pause(&self) {
        self.send(PlayerCmd::Pause);
    }

    pub fn toggle(&self) {
        self.send(PlayerCmd::TogglePlay);
    }

    /// Seconds from the start, clamped to the duration when known.
    pub fn seek(&self, secs: f64) {
        let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        let secs = match self.state().and_then(|s| s.duration) {
            Some(d) if d > 0.0 => secs.min(d),
            _ => secs,
        };
        self.send(PlayerCmd::Seek(secs));
    }

    /// Relative seconds (negative = back).
    pub fn seek_by(&self, delta: f64) {
        if delta.is_finite() && delta != 0.0 {
            self.send(PlayerCmd::SeekBy(delta));
        }
    }

    /// One step back (`forward = false`) or forward (`[media] seek_step_secs`).
    pub fn seek_step_by(&self, forward: bool) {
        self.seek_by(if forward { self.seek_step } else { -self.seek_step });
    }

    /// The frames wanted from now on: the pane's longest edge in pixels.
    pub fn set_output_size(&self, px: u32) {
        self.send(PlayerCmd::SetOutputSize(px.max(16)));
    }

    /// Push `[media] volume` / `muted` into the open player when they changed.
    pub fn apply_config(&mut self, cfg: &MediaCfg) {
        if !cfg.player {
            self.close();
            return;
        }
        self.seek_step = f64::from(cfg.seek_step_secs.max(0.1));
        let now = (cfg.volume.min(100), cfg.muted);
        if self.open.is_some() && self.applied != Some(now) {
            self.send(PlayerCmd::SetVolume(f32::from(now.0) / 100.0));
            self.send(PlayerCmd::Mute(now.1));
        }
        self.applied = Some(now);
    }
}

/// Every frame: forward `[media]` changes (Settings, a config reload) to the open
/// player, close it when the player is turned off.
pub fn pump(app: &mut AppCore, _host: &dyn Host) {
    let cfg = &app.config.media;
    if app.ui.player.applied != Some((cfg.volume.min(100), cfg.muted))
        || (!cfg.player && app.ui.player.is_open())
        || app.ui.player.seek_step != f64::from(cfg.seek_step_secs.max(0.1))
    {
        let cfg = cfg.clone();
        app.ui.player.apply_config(&cfg);
    }
}

/// Set the player volume (0–100): applies live and writes `[media] volume` back.
pub fn set_volume(app: &mut AppCore, host: &dyn Host, percent: u8) {
    let percent = percent.min(100);
    crate::models::settings::update_config(app, host, |c| c.media.volume = percent);
    pump(app, host);
}

/// Nudge the volume by `delta` percentage points.
pub fn adjust_volume(app: &mut AppCore, host: &dyn Host, delta: i16) {
    let now = i16::from(app.config.media.volume.min(100));
    set_volume(app, host, (now + delta).clamp(0, 100) as u8);
}

/// Mute / unmute: applies live and writes `[media] muted` back.
pub fn set_muted(app: &mut AppCore, host: &dyn Host, muted: bool) {
    crate::models::settings::update_config(app, host, |c| c.media.muted = muted);
    pump(app, host);
}

pub fn toggle_mute(app: &mut AppCore, host: &dyn Host) {
    let muted = !app.config.media.muted;
    set_muted(app, host, muted);
}

/// Volume step of the player's volume actions (percentage points).
pub const VOLUME_STEP: i16 = 5;

/// Run a player action ([`Action::is_player`](soyuz_core::config::Action::is_player)) on
/// the open player; without one, only the volume / mute settings change.
pub fn execute(app: &mut AppCore, action: soyuz_core::config::Action, host: &dyn Host) {
    use soyuz_core::config::Action as A;
    match action {
        A::PlayerPlayPause => app.ui.player.toggle(),
        A::PlayerSeekBack => app.ui.player.seek_step_by(false),
        A::PlayerSeekForward => app.ui.player.seek_step_by(true),
        A::PlayerVolumeUp => adjust_volume(app, host, VOLUME_STEP),
        A::PlayerVolumeDown => adjust_volume(app, host, -VOLUME_STEP),
        A::PlayerMute => toggle_mute(app, host),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Records commands and keeps a snapshot the way the V0 stub does.
    struct Fake {
        cmds: Arc<StdMutex<Vec<PlayerCmd>>>,
        state: StdMutex<PlayerState>,
    }

    fn fake(duration: Option<f64>) -> (Box<dyn Engine>, Arc<StdMutex<Vec<PlayerCmd>>>) {
        let cmds = Arc::new(StdMutex::new(Vec::new()));
        let f = Fake { cmds: Arc::clone(&cmds), state: StdMutex::new(PlayerState { duration, ..Default::default() }) };
        (Box::new(f), cmds)
    }

    impl Engine for Fake {
        fn send(&self, cmd: PlayerCmd) {
            self.cmds.lock().unwrap().push(cmd);
            let mut s = self.state.lock().unwrap();
            match cmd {
                PlayerCmd::Play => s.playing = true,
                PlayerCmd::Pause => s.playing = false,
                PlayerCmd::TogglePlay => s.playing = !s.playing,
                PlayerCmd::Seek(at) => s.pos = at,
                PlayerCmd::SeekBy(by) => s.pos = (s.pos + by).max(0.0),
                PlayerCmd::SetVolume(v) => s.volume = v,
                PlayerCmd::Mute(on) => s.muted = on,
                PlayerCmd::SetOutputSize(_) => {}
                PlayerCmd::Stop => {
                    s.playing = false;
                    s.ended = true;
                }
            }
        }

        fn state(&self) -> PlayerState {
            self.state.lock().unwrap().clone()
        }
    }

    #[test]
    fn options_come_from_the_media_keys() {
        let cfg = MediaCfg {
            volume: 250,
            muted: true,
            autoplay: true,
            loop_: true,
            hw_decode: HwDecode::Off,
            ..Default::default()
        };
        let o = options(&cfg);
        assert_eq!(o.volume, 1.0, "clamped to 100 %");
        assert!(o.muted && o.autoplay && o.loop_);
        assert_eq!(o.hw, HwPref::Off);
        assert_eq!(o.max_px, cfg.max_px);
        assert_eq!(options(&MediaCfg::default()).hw, HwPref::Auto);
    }

    #[test]
    fn commands_reach_the_engine_and_seeks_are_clamped() {
        let mut m = PlayerModel::default();
        assert!(!m.is_open() && m.state().is_none() && m.frame().is_none());
        m.play(); // nothing open: ignored
        let (e, cmds) = fake(Some(10.0));
        let cfg = MediaCfg { seek_step_secs: 2.5, ..Default::default() };
        m.open_with("a.mp4", e, &cfg);
        assert_eq!(m.key(), Some("a.mp4"));
        assert_eq!(m.seek_step(), 2.5);
        m.toggle();
        m.seek(42.0);
        assert_eq!(m.state().unwrap().pos, 10.0, "clamped to the duration");
        m.seek(-3.0);
        assert_eq!(m.state().unwrap().pos, 0.0);
        m.seek(f64::NAN);
        m.seek_step_by(true);
        m.seek_step_by(false);
        m.seek_by(0.0);
        m.seek_by(f64::INFINITY);
        m.set_output_size(4);
        m.pause();
        m.play();
        let got = cmds.lock().unwrap().clone();
        assert_eq!(
            got,
            vec![
                PlayerCmd::TogglePlay,
                PlayerCmd::Seek(10.0),
                PlayerCmd::Seek(0.0),
                PlayerCmd::Seek(0.0),
                PlayerCmd::SeekBy(2.5),
                PlayerCmd::SeekBy(-2.5),
                PlayerCmd::SetOutputSize(16),
                PlayerCmd::Pause,
                PlayerCmd::Play,
            ]
        );
        assert!(format!("{m:?}").contains("a.mp4"));
    }

    #[test]
    fn target_changes_close_and_config_changes_forward() {
        let mut m = PlayerModel::default();
        let (e, cmds) = fake(None);
        let cfg = MediaCfg::default();
        m.open_with("x", e, &cfg);
        m.sync_target(Some("x"));
        assert!(m.is_open(), "same target stays open");
        // Same volume: nothing sent.
        m.apply_config(&cfg);
        assert!(cmds.lock().unwrap().is_empty());
        let louder = MediaCfg { volume: 50, muted: true, seek_step_secs: 1.0, ..Default::default() };
        m.apply_config(&louder);
        assert_eq!(cmds.lock().unwrap().clone(), vec![PlayerCmd::SetVolume(0.5), PlayerCmd::Mute(true)]);
        assert_eq!(m.seek_step(), 1.0);
        let s = m.state().unwrap();
        assert!(s.muted && (s.volume - 0.5).abs() < 1e-6);
        m.sync_target(Some("y"));
        assert!(!m.is_open(), "another target closes");
        assert_eq!(cmds.lock().unwrap().last(), Some(&PlayerCmd::Stop));
        // Turning the player off closes it too.
        let (e, cmds) = fake(None);
        m.open_with("z", e, &cfg);
        m.apply_config(&MediaCfg { player: false, ..Default::default() });
        assert!(!m.is_open());
        assert_eq!(cmds.lock().unwrap().clone(), vec![PlayerCmd::Stop]);
        m.sync_target(None);
        m.close();
    }

    #[test]
    fn frames_are_uploaded_under_one_name() {
        let names = Arc::new(StdMutex::new(Vec::new()));
        let n2 = Arc::clone(&names);
        let up: Uploader = Arc::new(move |name, px: Pixels<'_>| {
            n2.lock().unwrap().push((name, px.size));
            Arc::new(())
        });
        let slot = Arc::new(Mutex::new(None));
        let mut sink = CoreFrameSink { upload: up, name: "player:1".into(), slot: Arc::clone(&slot), serial: 0 };
        let rgba = vec![0u8; 2 * 3 * 4];
        sink.present(FramePixels { width: 2, height: 3, rgba: &rgba, pts: 0.5 });
        sink.present(FramePixels { width: 2, height: 3, rgba: &rgba, pts: 1.0 });
        sink.present(FramePixels { width: 9, height: 9, rgba: &rgba, pts: 2.0 }); // short buffer: ignored
        let f = slot.lock().clone().unwrap();
        assert_eq!((f.pts, f.serial, f.tex.size), (1.0, 2, [2, 3]));
        assert_eq!(
            names.lock().unwrap().clone(),
            vec![("player:1".to_string(), [2, 3]), ("player:1".to_string(), [2, 3])]
        );
    }

    #[test]
    fn open_needs_the_player_setting_and_the_libraries() {
        let mut m = PlayerModel::default();
        let up = crate::services::preview_gfx::no_upload();
        let wake = crate::host::no_wake();
        let off = MediaCfg { player: false, ..Default::default() };
        let err = m.open("v.mp4", Source::File("v.mp4".into()), &off, &up, &wake).unwrap_err();
        assert_eq!(err, tr!("views-player-off"));
        if media::runtime().is_none() {
            let err = m.open("v.mp4", Source::File("v.mp4".into()), &MediaCfg::default(), &up, &wake).unwrap_err();
            assert_eq!(err, tr!("services-audio-no-ffmpeg"));
        }
        assert!(!m.is_open());
    }
}
