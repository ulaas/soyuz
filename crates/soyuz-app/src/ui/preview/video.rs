//! The video player of the preview pane and Quick Look (Phase V3a): the poster first,
//! a play overlay, then the presented frame filling the view, with a controls bar
//! (play / pause, scrubber with hover time and the buffered range, elapsed / total,
//! volume, mute, loop). The engine is [`soyuz_appcore::models::player::PlayerModel`]
//! (`core.ui.player`): opened on a worker thread, followed with `sync_target`, closed
//! when the preview moves on or Quick Look closes. With `[media] gpu_yuv` and the wgpu
//! renderer, frames arrive as YUV planes and are converted on the GPU
//! ([`super::video_gpu`]); otherwise as RGBA textures.
//!
//! Keyboard: `KeyContext::Player` (K / J / L, ← / →, - / =, M by default) is active while
//! Quick Look shows a video or audio file, or while the pane's player surface or one of
//! its controls has focus ([`owns_keyboard`]). The audio card (`rich.rs`) shares the
//! volume / mute cluster and sends its changes through [`push_audio_cmd`].

use std::cell::{Cell, RefCell};
use std::time::Duration;

use egui::{
    Align2, Color32, CornerRadius, EventFilter, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, WidgetInfo,
    pos2, vec2,
};
use soyuz_appcore::models::player::{self as player, MediaSource, Picture};
use soyuz_core::EntryInfo;
use soyuz_core::config::{Config, KeyContext};

use crate::app::SoyuzApp;
use crate::core::AppCore;
use crate::services::media::MediaInfo;
use crate::services::preview::{Content, Loaded};
use crate::services::preview_media::format_duration;
use crate::services::preview_rich::{Rich, Tex};
use crate::theme::icons::{self, ph};
use crate::theme::tokens::{motion, radius};
use crate::theme::{Palette, palette};
use crate::ui::gfx::TexExt;
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{focus_ring, icon_toggle, spinner, trunc_galley};

/// The preview's identity for a file: what the player is opened under.
pub fn key(entry: &EntryInfo) -> String {
    let modified = entry.modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    format!("{}|{}|{modified}", entry.path.display(), entry.size)
}

/// Whether the player (not the poster card) shows this loaded content: a video, with
/// `[media] player` on and the libraries loaded.
pub fn takes(loaded: &Loaded, cfg: &Config) -> bool {
    cfg.media.player
        && matches!(&loaded.content, Content::Rich(Rich::Video { .. }))
        && crate::services::media::runtime().is_some()
}

/// The video content the player draws over.
pub struct Media<'a> {
    pub entry: &'a EntryInfo,
    pub poster: Option<&'a Tex>,
    /// Why there is no poster.
    pub note: Option<&'a str>,
    pub info: Option<&'a MediaInfo>,
}

impl<'a> Media<'a> {
    /// The video of `loaded` (`None` for anything else).
    pub fn of(entry: &'a EntryInfo, loaded: &'a Loaded) -> Option<Media<'a>> {
        match &loaded.content {
            Content::Rich(Rich::Video { poster, note, info }) => {
                Some(Media { entry, poster: poster.as_ref(), note: note.as_deref(), info: info.as_deref() })
            }
            _ => None,
        }
    }
}

/// Per-view state in egui's temporary memory (one per consumer: pane / Quick Look).
#[derive(Clone, Default)]
struct View {
    /// The key the view last drew (the first frame of a new key starts autoplay).
    key: String,
    /// Longest edge (pixels) last asked of the engine, and the size the view wants now
    /// with when it started wanting it (`SetOutputSize` is debounced).
    sent_px: u32,
    wanted_px: u32,
    wanted_since: f64,
    /// Volume while the slider is dragged (written back on release).
    volume_drag: Option<u8>,
}

fn view_id(big: bool) -> Id {
    Id::new(("soyuz.player.view", big))
}

/// Settle time of a view-size change before the engine is asked for that size.
const SIZE_DEBOUNCE: f64 = 0.15;
/// Repaint cadence while playing when the stream's frame rate is unknown.
const FALLBACK_FRAME: Duration = Duration::from_millis(33);

thread_local! {
    /// The ids of the player's focusable widgets (keyboard ownership), with the pass
    /// they were registered in (the video view and the audio card both register).
    static FOCUS_IDS: RefCell<(u64, Vec<Id>)> = const { RefCell::new((0, Vec::new())) };
    /// Volume / mute changes the audio card asked for (applied by the pane / Quick
    /// Look, which hold the core).
    static AUDIO_CMDS: RefCell<Vec<AudioCmd>> = const { RefCell::new(Vec::new()) };
    /// A play action with nothing open, handed to the audio card.
    static AUDIO_PLAY_REQUEST: Cell<bool> = const { Cell::new(false) };
}

/// What the audio card (no core access) asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCmd {
    Volume(u8),
    Muted(bool),
}

pub fn push_audio_cmd(c: AudioCmd) {
    AUDIO_CMDS.with_borrow_mut(|v| v.push(c));
}

/// Apply what the audio card queued (volume / mute are persisted by the model).
pub fn apply_audio_cmds(core: &mut AppCore, host: &dyn soyuz_appcore::host::Host) {
    let cmds = AUDIO_CMDS.with_borrow_mut(std::mem::take);
    for c in cmds {
        match c {
            AudioCmd::Volume(v) => player::set_volume(core, host, v),
            AudioCmd::Muted(m) => player::set_muted(core, host, m),
        }
    }
}

/// Hand a play request (K with nothing open) to the audio card drawn this frame.
pub fn set_audio_play_request(on: bool) {
    if on {
        AUDIO_PLAY_REQUEST.set(true);
    }
}

/// The audio card takes the play request (once).
pub fn take_audio_play_request() -> bool {
    AUDIO_PLAY_REQUEST.replace(false)
}

/// Register the player's focusable widgets drawn this pass (the video view and the
/// audio card): while one of them has focus the player owns the keyboard.
pub fn register_focus_ids(ctx: &egui::Context, ids: impl IntoIterator<Item = Id>) {
    let pass = ctx.cumulative_pass_nr();
    FOCUS_IDS.with_borrow_mut(|(p, v)| {
        if *p != pass {
            *p = pass;
            v.clear();
        }
        v.extend(ids);
    });
}

/// The pane's player has the keyboard: its surface or one of its controls has egui
/// focus (Quick Look dispatches the player keys itself while it shows media).
pub fn owns_keyboard(app: &SoyuzApp, ctx: &egui::Context) -> bool {
    if !app.core.ui.preview.open {
        return false;
    }
    let pass = ctx.cumulative_pass_nr();
    ctx.memory(|m| m.focused())
        .is_some_and(|f| FOCUS_IDS.with_borrow(|(p, ids)| pass.saturating_sub(*p) <= 1 && ids.contains(&f)))
}

/// Run this frame's `KeyContext::Player` chords (only the player's own actions, no
/// global fallback) and remove them from egui's input. For Quick Look, which owns the
/// keyboard while open.
pub fn dispatch_player_keys(app: &mut SoyuzApp, ctx: &egui::Context) {
    let cmd_is_ctrl = app.core.config.keys.cmd_is_ctrl;
    let mut frame = crate::ui::input::frame(ctx, cmd_is_ctrl);
    let mut actions = Vec::new();
    for i in 0..frame.events.len() {
        if let soyuz_appcore::keys::input::InputEvent::Chord(c) = &frame.events[i]
            && let Some(a) = app.core.keymap.lookup(KeyContext::Player, *c)
            && a.is_player()
        {
            actions.push(a.clone());
            frame.consumed[i] = true;
        }
    }
    crate::ui::input::strip(ctx, &frame, cmd_is_ctrl);
    let host = crate::host::EguiHost::new(ctx);
    for a in actions {
        crate::actions::execute(&mut app.core, a, &host);
    }
}

/// Draw the player for `m` into `rect`; `big` is Quick Look.
pub fn show(ui: &mut Ui, rect: Rect, core: &mut AppCore, m: Media<'_>, big: bool) {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let host = crate::host::EguiHost::new(&ctx);
    let ppp = ctx.pixels_per_point();
    let now = ui.input(|i| i.time);
    let id = view_id(big);
    let mut view = ui.data(|d| d.get_temp::<View>(id)).unwrap_or_default();
    let key = key(m.entry);
    let first_seen = view.key != key;
    if first_seen {
        view.key = key.clone();
        view.sent_px = 0;
        view.wanted_px = 0;
        view.volume_drag = None;
    }
    let cfg = core.config.media.clone();
    let model = &mut core.ui.player;
    model.sync_target(Some(&key));
    model.set_yuv_output(if cfg.gpu_yuv { super::video_gpu::max_side() } else { 0 });
    let mut want_open: Option<bool> = None; // Some(play now)
    if model.take_play_request() {
        want_open = Some(true);
    } else if first_seen && cfg.autoplay {
        want_open = Some(false);
    }
    let bar_h = if big { 52.0 } else { 44.0 };
    let video_r = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - bar_h));
    let bar_r = Rect::from_min_max(video_r.left_bottom(), rect.max);

    // ---- the picture -------------------------------------------------------------
    ui.painter().rect_filled(video_r, 0.0, Color32::from_gray(12));
    let state = model.state();
    let opened = model.key() == Some(key.as_str());
    let pending = model.pending_key() == Some(key.as_str());
    let open_error = model.open_error(&key).map(str::to_string);
    let frame = if opened { model.frame() } else { None };
    let surface = ui.interact(video_r, id.with("surface"), Sense::click());
    let surface_name = tr!("views-player-surface", name = m.entry.name.clone());
    crate::ui::a11y::button(&surface, &surface_name);
    if surface.clicked() {
        surface.request_focus();
        if opened {
            model.toggle();
        } else if !pending {
            want_open = Some(true);
        }
    }
    if surface.has_focus() {
        ui.memory_mut(|mem| {
            mem.set_focus_lock_filter(
                surface.id,
                EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: false, escape: false },
            );
        });
    }
    let fit = |w: f32, h: f32| -> Rect {
        let natural = vec2(w, h) / ppp;
        let s = (video_r.width() / natural.x).min(video_r.height() / natural.y).max(0.0001);
        Rect::from_center_size(video_r.center(), natural * s)
    };
    let painter = ui.painter().with_clip_rect(video_r.intersect(ui.clip_rect()));
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    match (&frame, m.poster) {
        (Some(f), _) => {
            let [w, h] = f.picture.size();
            let r = fit(w as f32, h as f32);
            match &f.picture {
                Picture::Rgba(t) => {
                    painter.image(t.id(), r, uv, Color32::WHITE);
                }
                Picture::Yuv(y) => super::video_gpu::paint(&painter, r, id.value(), y.clone()),
            }
        }
        (None, Some(t)) => {
            painter.image(t.id(), fit(t.size[0] as f32, t.size[1] as f32), uv, Color32::WHITE);
        }
        (None, None) => {
            let c = video_r.center() - vec2(0.0, if big { 40.0 } else { 24.0 });
            let disc = Rect::from_center_size(c, vec2(96.0, 96.0));
            painter.rect_filled(disc, CornerRadius::same(22), p.glyph_video.gamma_multiply(0.14));
            painter.text(c, Align2::CENTER_CENTER, ph::FILM_SLATE, icons::font_fill(46.0), p.glyph_video);
            let tg =
                trunc_galley(&painter, &m.entry.name, semibold(14.0), Color32::from_gray(225), video_r.width() - 32.0);
            painter.galley(
                pos2(video_r.center().x - tg.size().x / 2.0, disc.bottom() + 14.0),
                tg,
                Color32::from_gray(225),
            );
            if let Some(n) = m.note.filter(|_| !opened && !pending) {
                let g = painter.layout(
                    n.to_string(),
                    FontId::proportional(11.5),
                    Color32::from_gray(170),
                    (video_r.width() - 40.0).clamp(80.0, 380.0),
                );
                painter.galley(
                    pos2(video_r.center().x - g.size().x / 2.0, disc.bottom() + 40.0),
                    g,
                    Color32::from_gray(170),
                );
            }
        }
    }
    let playing = state.as_ref().is_some_and(|s| s.playing);
    let engine_error = state.as_ref().and_then(|s| s.error.clone());
    let error = open_error.or(engine_error);
    if let Some(e) = &error {
        let c = video_r.center();
        painter.rect_filled(video_r, 0.0, Color32::from_black_alpha(120));
        painter.text(c - vec2(0.0, 22.0), Align2::CENTER_CENTER, ph::WARNING_CIRCLE, icons::font_fill(30.0), p.danger);
        let g = painter.layout(
            e.clone(),
            FontId::proportional(12.0),
            Color32::from_gray(230),
            (video_r.width() - 40.0).clamp(80.0, 420.0),
        );
        painter.galley(pos2(c.x - g.size().x / 2.0, c.y + 2.0), g, Color32::from_gray(230));
    } else if pending {
        spinner(ui, video_r.center(), 13.0, p.accent);
        painter.text(
            video_r.center() + vec2(0.0, 30.0),
            Align2::CENTER_CENTER,
            tr!("views-player-opening"),
            FontId::proportional(11.5),
            Color32::from_gray(200),
        );
        ctx.request_repaint_after(Duration::from_millis(60));
    } else if !playing {
        // Play overlay: a big disc, lit on hover.
        let hover =
            ctx.animate_bool_with_time(id.with("ov"), surface.hovered(), crate::theme::anim(&ctx, motion::HOVER));
        let r = if big { 34.0 } else { 26.0 };
        let c = video_r.center();
        painter.circle_filled(c, r + 2.0, Color32::from_black_alpha(90));
        painter.circle_filled(c, r, p.accent.gamma_multiply(0.85 + 0.15 * hover));
        painter.text(
            c + vec2(r * 0.08, 0.0),
            Align2::CENTER_CENTER,
            ph::PLAY,
            icons::font_fill(r * 0.9),
            p.on_accent(),
        );
    }
    if surface.has_focus() {
        painter.rect_stroke(video_r.shrink(1.5), 0.0, Stroke::new(2.0, p.accent), StrokeKind::Inside);
    }
    if error.is_none() && !pending {
        surface.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
    }

    // ---- output size (debounced) ---------------------------------------------------
    let wanted = ((video_r.width().max(video_r.height())) * ppp).round().max(16.0) as u32;
    if wanted != view.wanted_px {
        view.wanted_px = wanted;
        view.wanted_since = now;
    }
    if opened {
        let differs = view.sent_px == 0 || (wanted.abs_diff(view.sent_px) as f32) > view.sent_px as f32 * 0.08;
        if differs && (view.sent_px == 0 || now - view.wanted_since >= SIZE_DEBOUNCE) {
            model.set_output_size(wanted);
            view.sent_px = wanted;
        } else if differs {
            ctx.request_repaint_after(Duration::from_secs_f64(SIZE_DEBOUNCE));
        }
    }

    // ---- controls bar --------------------------------------------------------------
    ui.painter().rect_filled(bar_r, 0.0, p.bg_surface1);
    ui.painter().line_segment([bar_r.left_top(), bar_r.right_top()], Stroke::new(1.0, p.border_subtle));
    let (pos, dur) = match &state {
        Some(s) if opened => (s.pos, s.duration),
        _ => (0.0, m.info.and_then(|i| i.duration)),
    };
    let buffered = state.as_ref().filter(|_| opened).map_or(0.0, |s| s.buffered);
    let loop_on = model.loop_enabled();
    let mut ids = Vec::with_capacity(6);
    ids.push(surface.id);
    let inner = bar_r.shrink2(vec2(if big { 14.0 } else { 8.0 }, 0.0));
    let mut bar =
        ui.new_child(UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    bar.spacing_mut().item_spacing.x = if big { 8.0 } else { 4.0 };
    let play_label = if playing { tr!("views-rich-pause") } else { tr!("views-rich-play") };
    let play = icon_toggle(&mut bar, if playing { ph::PAUSE } else { ph::PLAY }, play_label, true, false);
    ids.push(play.id);
    if play.clicked() {
        if opened {
            model.toggle();
        } else if !pending {
            want_open = Some(true);
        }
    }
    let mono = FontId::monospace(if big { 12.0 } else { 11.0 });
    let time_w = bar.painter().layout_no_wrap("00:00:00".into(), mono.clone(), p.text_secondary).size().x;
    let total = dur.map_or_else(|| "—".to_string(), format_duration);
    let elapsed_w = if dur.is_some_and(|d| d >= 3600.0) { time_w } else { time_w * 0.66 };
    let (er, _) = bar.allocate_exact_size(vec2(elapsed_w, 20.0), Sense::hover());
    bar.painter().text(er.right_center(), Align2::RIGHT_CENTER, format_duration(pos), mono.clone(), p.text_secondary);

    // Right cluster widths (volume cluster hides on a narrow bar).
    let btn = (bar.spacing().interact_size.y + 6.0).max(30.0);
    let wide = inner.width() > (if big { 420.0 } else { 330.0 });
    let slider_w = if big { 90.0 } else { 64.0 };
    let right_w = elapsed_w
        + btn * 2.0
        + if wide { slider_w + bar.spacing().item_spacing.x } else { 0.0 }
        + bar.spacing().item_spacing.x * 4.0;
    let scrub_w = (bar.available_width() - right_w).max(40.0);
    let scrub = scrubber(&mut bar, scrub_w, pos, dur, buffered, &p, opened);
    ids.push(scrub.id);
    if let Some(t) = scrub.seek {
        model.seek(t);
    }
    let (tr_, _) = bar.allocate_exact_size(vec2(elapsed_w, 20.0), Sense::hover());
    bar.painter().text(tr_.left_center(), Align2::LEFT_CENTER, total, mono, p.text_secondary);

    let muted = cfg.muted;
    let volume = view.volume_drag.unwrap_or(cfg.volume.min(100));
    let (mute_id, vol_change, mute_change, vol_id) =
        volume_cluster(&mut bar, volume, muted, if wide { Some(slider_w) } else { None }, &mut view.volume_drag);
    ids.push(mute_id);
    if let Some(i) = vol_id {
        ids.push(i);
    }
    let loop_resp = icon_toggle(&mut bar, ph::REPEAT, tr!("views-player-loop"), true, loop_on);
    ids.push(loop_resp.id);
    if loop_resp.clicked() {
        model.set_loop(!loop_on);
    }
    if let Some(v) = vol_change {
        player::set_volume(core, &host, v);
    }
    if let Some(mu) = mute_change {
        player::set_muted(core, &host, mu);
    }

    // ---- open on demand (autoplay, a play request, the overlay or the button) ------
    if let Some(play) = want_open
        && !opened
        && !pending
    {
        let up = crate::ui::gfx::egui_uploader(&ctx);
        let wake = crate::host::egui_wake(&ctx);
        let src = MediaSource::File(m.entry.path.clone());
        if let Err(e) = core.ui.player.open_in_background(&key, src, &cfg, &up, &wake, play) {
            log::warn!("player: {e}");
        }
    }

    // ---- repaint while playing -----------------------------------------------------
    if playing {
        let per_frame = m
            .info
            .and_then(MediaInfo::video)
            .and_then(|v| v.fps)
            .filter(|f| f.is_finite() && *f > 1.0)
            .map_or(FALLBACK_FRAME, |f| Duration::from_secs_f64((1.0 / f).clamp(1.0 / 60.0, 0.5)));
        ctx.request_repaint_after(per_frame);
    }
    register_focus_ids(&ctx, ids);
    ui.data_mut(|d| d.insert_temp(id, view));
}

/// What the scrubber did this frame.
struct Scrub {
    id: Id,
    /// Seconds to seek to (click / drag).
    seek: Option<f64>,
}

/// The position slider: played range in the accent, the buffered range lighter, a knob
/// while hovered / focused, and the hovered time above the pointer.
fn scrubber(ui: &mut Ui, width: f32, pos: f64, dur: Option<f64>, buffered: f64, p: &Palette, live: bool) -> Scrub {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click_and_drag());
    let d = dur.unwrap_or(0.0);
    let frac = if d > 0.0 { (pos / d).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let label = tr!("views-player-position");
    let value = f64::from(frac * 100.0);
    resp.widget_info(|| WidgetInfo::slider(live && d > 0.0, value, label));
    crate::ui::a11y::with(ui.ctx(), resp.id, |n| {
        n.set_value(format!("{} / {}", format_duration(pos), dur.map_or_else(|| "—".into(), format_duration)));
    });
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 8.0, 4.0));
    ui.painter().rect_filled(track, CornerRadius::same(2), p.border_strong);
    if d > 0.0 {
        let buf_to = ((pos + buffered) / d).clamp(0.0, 1.0) as f32;
        if buf_to > frac {
            let mut b = track;
            b.min.x = track.left() + track.width() * frac;
            b.max.x = track.left() + track.width() * buf_to;
            ui.painter().rect_filled(b, CornerRadius::same(2), p.text_tertiary.gamma_multiply(0.55));
        }
        let mut played = track;
        played.set_width(track.width() * frac);
        ui.painter().rect_filled(played, CornerRadius::same(2), p.accent);
    }
    let hot = resp.hovered() || resp.dragged() || resp.has_focus();
    if hot && d > 0.0 {
        let knob = pos2(track.left() + track.width() * frac, track.center().y);
        ui.painter().circle_filled(knob, 6.0, p.text_primary);
        ui.painter().circle_stroke(knob, 6.0, Stroke::new(1.0, p.border_strong));
    }
    let mut seek = None;
    let at = |x: f32| -> f64 { (((x - track.left()) / track.width()).clamp(0.0, 1.0) as f64) * d };
    if live && d > 0.0 {
        if let Some(ptr) = resp.hover_pos().filter(|_| resp.hovered() || resp.dragged()) {
            let t = at(ptr.x);
            let text = format_duration(t);
            let g = ui.painter().layout_no_wrap(text, FontId::monospace(11.0), p.text_primary);
            let pill = Rect::from_center_size(
                pos2(ptr.x.clamp(rect.left() + 24.0, rect.right() - 24.0), rect.top() - 12.0),
                g.size() + vec2(12.0, 6.0),
            );
            let painter =
                ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("soyuz.player.hover")));
            painter.rect_filled(pill, CornerRadius::same(radius::CONTROL), p.bg_surface3);
            painter.rect_stroke(
                pill,
                CornerRadius::same(radius::CONTROL),
                Stroke::new(1.0, p.border_subtle),
                StrokeKind::Inside,
            );
            painter.galley(pill.min + vec2(6.0, 3.0), g, p.text_primary);
            ui.painter().line_segment(
                [pos2(ptr.x, track.top() - 2.0), pos2(ptr.x, track.bottom() + 2.0)],
                Stroke::new(1.0, p.text_secondary),
            );
            resp.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        if (resp.clicked() || resp.dragged())
            && let Some(ptr) = resp.interact_pointer_pos()
        {
            seek = Some(at(ptr.x));
        }
    }
    if resp.has_focus() {
        focus_ring(ui, rect.shrink2(vec2(0.0, 4.0)));
    }
    Scrub { id: resp.id, seek }
}

/// Mute button plus (when `slider` is `Some(width)`) the volume slider. Returns the mute
/// button's id, a new volume (on release), a new mute, and the slider's id. Shared with
/// the audio card.
pub(super) fn volume_cluster(
    ui: &mut Ui,
    volume: u8,
    muted: bool,
    slider: Option<f32>,
    drag: &mut Option<u8>,
) -> (Id, Option<u8>, Option<bool>, Option<Id>) {
    let p = palette(ui.ctx());
    let glyph = if muted || volume == 0 {
        ph::SPEAKER_X
    } else if volume < 50 {
        ph::SPEAKER_LOW
    } else {
        ph::SPEAKER_HIGH
    };
    let label = if muted { tr!("views-player-unmute") } else { tr!("views-player-mute") };
    let mute = icon_toggle(ui, glyph, label, true, muted);
    let mute_change = mute.clicked().then_some(!muted);
    let Some(width) = slider else { return (mute.id, None, mute_change, None) };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click_and_drag());
    let mut shown = drag.unwrap_or(volume);
    let mut change = None;
    if (resp.dragged() || resp.clicked())
        && let Some(ptr) = resp.interact_pointer_pos()
    {
        let t = ((ptr.x - rect.left() - 6.0) / (rect.width() - 12.0)).clamp(0.0, 1.0);
        shown = (t * 100.0).round() as u8;
        *drag = Some(shown);
    }
    if resp.drag_stopped() || resp.clicked() {
        change = drag.take().or(Some(shown));
    }
    if resp.has_focus() {
        // Arrows nudge by the player's step (the keymap's - / = also work anywhere).
        let (l, r) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp),
            )
        });
        let step = player::VOLUME_STEP;
        if l {
            change = Some((i16::from(shown) - step).clamp(0, 100) as u8);
        }
        if r {
            change = Some((i16::from(shown) + step).clamp(0, 100) as u8);
        }
        ui.memory_mut(|mem| {
            mem.set_focus_lock_filter(
                resp.id,
                EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: false },
            );
        });
    }
    let vlabel = tr!("views-player-volume");
    let v = f64::from(shown);
    resp.widget_info(|| WidgetInfo::slider(true, v, vlabel));
    crate::ui::a11y::with(ui.ctx(), resp.id, |n| n.set_value(format!("{shown}%")));
    let t = f32::from(shown) / 100.0;
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 12.0, 3.0));
    ui.painter().rect_filled(track, CornerRadius::same(2), p.border_strong);
    let mut filled = track;
    filled.set_width(track.width() * t);
    ui.painter().rect_filled(filled, CornerRadius::same(2), if muted { p.text_tertiary } else { p.accent });
    let knob = pos2(track.left() + track.width() * t, rect.center().y);
    let hot = resp.hovered() || resp.dragged();
    ui.painter().circle_filled(knob, if hot { 6.0 } else { 5.0 }, p.text_primary);
    ui.painter().circle_stroke(knob, if hot { 6.0 } else { 5.0 }, Stroke::new(1.0, p.border_strong));
    if resp.has_focus() {
        focus_ring(ui, rect);
    }
    let id = resp.id;
    resp.on_hover_text(format!("{shown}%"));
    (mute.id, change, mute_change, Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::EntryKind;

    #[test]
    fn keys_follow_path_size_and_mtime() {
        let mut a = EntryInfo::minimal("v.mp4".into(), EntryKind::File);
        a.size = 10;
        let mut b = a.clone();
        assert_eq!(key(&a), key(&b));
        b.size = 11;
        assert_ne!(key(&a), key(&b));
        b.size = 10;
        b.modified = Some(std::time::UNIX_EPOCH + Duration::from_secs(5));
        assert_ne!(key(&a), key(&b));
        assert!(key(&a).starts_with("v.mp4|10|"));
    }

    #[test]
    fn audio_commands_and_play_requests_queue_once() {
        push_audio_cmd(AudioCmd::Volume(40));
        push_audio_cmd(AudioCmd::Muted(true));
        let got = AUDIO_CMDS.with_borrow_mut(std::mem::take);
        assert_eq!(got, vec![AudioCmd::Volume(40), AudioCmd::Muted(true)]);
        assert!(!take_audio_play_request());
        set_audio_play_request(false);
        assert!(!take_audio_play_request());
        set_audio_play_request(true);
        assert!(take_audio_play_request());
        assert!(!take_audio_play_request());
    }
}
