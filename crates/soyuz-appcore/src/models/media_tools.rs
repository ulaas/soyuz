//! Media tools (Phase V, V3c): the model behind "Convert media…", "Extract audio…",
//! "Trim media…" and "Save video frame…". Each is a [`MediaToolsModel`] dialog
//! ([`crate::models::dialogs::DialogModel::MediaTools`]) that turns the selection into
//! `JobKind::Media` jobs (`soyuz-jobs`, `soyuz_media::transcode::run`): a dry-run plan
//! first, the user confirms, the job runs with the usual conflict prompts and undo.
//!
//! Nothing here touches the libraries on the UI thread: the input summary comes from
//! `soyuz_media::probe` and the plan card from `soyuz_media::plan`, both on a worker
//! thread that stores its result in a shared slot and wakes the frontend; [`sync`]
//! (called by the frontends before drawing) moves the results into the model. Every
//! change of the operation asks for a new plan (debounced, stale results dropped).
//!
//! Multiple selected files: Convert and Extract audio run one job per file (the first file
//! is the one probed and planned, the others get the same operation and the same output
//! extension beside themselves); Trim and Save frame act on the first file only.
//!
//! [`sync`]: MediaToolsModel::sync

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use soyuz_core::config::Action;
use soyuz_core::{ConflictResolution, Location, SizeStyle, format_size};
use soyuz_jobs::{JobKind, JobOptions, JobSpec, Priority};
use soyuz_media::transcode::{MediaOp, PlanInfo};
use soyuz_media::{MediaInfo, Preset, PresetId, Runtime, StreamKind};
// Re-exported for the frontends (`soyuz-tui` does not depend on `soyuz-media`).
pub use soyuz_media::transcode::{AudioCodecChoice, ImageFormat};

use crate::cmd::Cmd;
use crate::core::AppCore;
use crate::host::Wake;
use crate::models::toasts::Toast;
use crate::ops::fileops::names;
use crate::services::jobs::{JobMeta, OpKind};
use crate::services::media;

/// Which tool the dialog runs (also the sub-kind of `OpKind::Media` on job cards).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaTool {
    Convert,
    ExtractAudio,
    Trim,
    SaveFrame,
}

impl MediaTool {
    /// In the dialog's switcher order.
    pub const ALL: [MediaTool; 4] =
        [MediaTool::Convert, MediaTool::ExtractAudio, MediaTool::Trim, MediaTool::SaveFrame];

    pub fn of_action(a: &Action) -> Option<MediaTool> {
        match a {
            Action::MediaConvert => Some(MediaTool::Convert),
            Action::MediaExtractAudio => Some(MediaTool::ExtractAudio),
            Action::MediaTrim => Some(MediaTool::Trim),
            Action::MediaSaveFrame => Some(MediaTool::SaveFrame),
            _ => None,
        }
    }

    /// The tool a job's operation belongs to (job cards, history rows).
    pub fn of_op(op: &MediaOp) -> MediaTool {
        match op {
            MediaOp::Convert { .. } | MediaOp::Remux { .. } => MediaTool::Convert,
            MediaOp::ExtractAudio { .. } => MediaTool::ExtractAudio,
            MediaOp::Trim { .. } => MediaTool::Trim,
            MediaOp::Frame { .. } => MediaTool::SaveFrame,
        }
    }

    /// Dialog title ("Convert media").
    pub fn title(self) -> &'static str {
        match self {
            MediaTool::Convert => tr!("dialogs-media-title-convert"),
            MediaTool::ExtractAudio => tr!("dialogs-media-title-extract"),
            MediaTool::Trim => tr!("dialogs-media-title-trim"),
            MediaTool::SaveFrame => tr!("dialogs-media-title-frame"),
        }
    }

    /// Short name for the tool switcher.
    pub fn label(self) -> &'static str {
        match self {
            MediaTool::Convert => tr!("dialogs-media-tool-convert"),
            MediaTool::ExtractAudio => tr!("dialogs-media-tool-extract"),
            MediaTool::Trim => tr!("dialogs-media-tool-trim"),
            MediaTool::SaveFrame => tr!("dialogs-media-tool-frame"),
        }
    }

    /// The primary button ("Convert", "Extract", "Trim", "Save").
    pub fn go_label(self) -> &'static str {
        match self {
            MediaTool::Convert => tr!("dialogs-media-go-convert"),
            MediaTool::ExtractAudio => tr!("dialogs-media-go-extract"),
            MediaTool::Trim => tr!("dialogs-media-go-trim"),
            MediaTool::SaveFrame => tr!("dialogs-media-go-frame"),
        }
    }

    /// Whether the tool runs on every selected file (else the first only).
    pub fn per_file(self) -> bool {
        matches!(self, MediaTool::Convert | MediaTool::ExtractAudio)
    }
}

/// What to do when the output name is taken (`JobOptions::conflict`; Ask = the job's
/// usual prompt).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    Ask,
    KeepBoth,
    Overwrite,
    Skip,
}

impl ConflictChoice {
    pub const ALL: [ConflictChoice; 4] =
        [ConflictChoice::Ask, ConflictChoice::KeepBoth, ConflictChoice::Overwrite, ConflictChoice::Skip];

    pub fn label(self) -> &'static str {
        match self {
            ConflictChoice::Ask => tr!("dialogs-media-conflict-ask"),
            ConflictChoice::KeepBoth => tr!("dialogs-media-conflict-keep-both"),
            ConflictChoice::Overwrite => tr!("dialogs-media-conflict-overwrite"),
            ConflictChoice::Skip => tr!("dialogs-media-conflict-skip"),
        }
    }

    fn resolution(self) -> Option<ConflictResolution> {
        match self {
            ConflictChoice::Ask => None,
            ConflictChoice::KeepBoth => Some(ConflictResolution::KeepBoth),
            ConflictChoice::Overwrite => Some(ConflictResolution::Overwrite),
            ConflictChoice::Skip => Some(ConflictResolution::Skip),
        }
    }
}

/// The audio codecs of Extract audio, in the dialog's order.
pub const AUDIO_CODECS: [AudioCodecChoice; 5] = [
    AudioCodecChoice::Copy,
    AudioCodecChoice::Flac,
    AudioCodecChoice::Aac,
    AudioCodecChoice::Opus,
    AudioCodecChoice::Wav,
];

/// Label of an Extract audio codec.
pub fn codec_label(c: AudioCodecChoice) -> &'static str {
    match c {
        AudioCodecChoice::Copy => tr!("dialogs-media-codec-copy"),
        AudioCodecChoice::Flac => "FLAC",
        AudioCodecChoice::Aac => "AAC (M4A)",
        AudioCodecChoice::Opus => "Opus",
        AudioCodecChoice::Wav => "WAV",
    }
}

/// The preset's extension when the audio codec is not a copy.
fn codec_ext(c: AudioCodecChoice) -> Option<&'static str> {
    match c {
        AudioCodecChoice::Copy => None,
        AudioCodecChoice::Flac => Some(PresetId::AudioFlac.preset().ext),
        AudioCodecChoice::Aac => Some(PresetId::AudioAac.preset().ext),
        AudioCodecChoice::Opus => Some(PresetId::AudioOpus.preset().ext),
        AudioCodecChoice::Wav => Some(PresetId::AudioWav.preset().ext),
    }
}

/// Translated label of a built-in preset (`Preset::label` is the English one).
pub fn preset_label(id: PresetId) -> &'static str {
    match id {
        PresetId::MkvCopy => tr!("dialogs-media-preset-mkv-copy"),
        PresetId::Mp4Copy => tr!("dialogs-media-preset-mp4-copy"),
        PresetId::Mp4H264Hw => tr!("dialogs-media-preset-mp4-h264-hw"),
        PresetId::Mp4HevcHw => tr!("dialogs-media-preset-mp4-hevc-hw"),
        PresetId::WebmVp9 => tr!("dialogs-media-preset-webm-vp9"),
        PresetId::WebmVp8 => tr!("dialogs-media-preset-webm-vp8"),
        PresetId::MovProres => tr!("dialogs-media-preset-mov-prores"),
        PresetId::Mp4Mpeg4 => tr!("dialogs-media-preset-mp4-mpeg4"),
        PresetId::AviMpeg4 => tr!("dialogs-media-preset-avi-mpeg4"),
        PresetId::AudioFlac => tr!("dialogs-media-preset-audio-flac"),
        PresetId::AudioAac => tr!("dialogs-media-preset-audio-aac"),
        PresetId::AudioOpus => tr!("dialogs-media-preset-audio-opus"),
        PresetId::AudioWav => tr!("dialogs-media-preset-audio-wav"),
        PresetId::AudioAlac => tr!("dialogs-media-preset-audio-alac"),
    }
}

/// What the probe found out about the first file (the parts the dialog shows).
#[derive(Clone, Debug, PartialEq)]
pub struct InputSummary {
    pub container: String,
    pub container_long: String,
    pub duration: Option<f64>,
    pub size: u64,
    pub has_video: bool,
    pub has_audio: bool,
    /// The default audio stream's codec (for the copy extension of Extract audio).
    pub audio_codec: Option<String>,
    /// One line per stream: "video h264 1920×1080 25 fps", "audio aac stereo 48 kHz".
    pub streams: Vec<(usize, String)>,
}

impl InputSummary {
    fn of(info: &MediaInfo) -> InputSummary {
        let streams = info
            .streams
            .iter()
            .map(|s| {
                let kind = s.kind.map(|k| k.to_string()).unwrap_or_else(|| "other".into());
                let mut line = format!("{kind} {}", s.codec);
                match s.kind {
                    Some(StreamKind::Video) if s.width > 0 => {
                        line.push_str(&format!(" {}\u{d7}{}", s.width, s.height));
                        if let Some(fps) = s.fps.filter(|f| *f > 0.0) {
                            line.push_str(&format!(" {fps:.3} fps").replace(".000", ""));
                        }
                    }
                    Some(StreamKind::Audio) => {
                        if !s.channel_layout.is_empty() {
                            line.push_str(&format!(" {}", s.channel_layout));
                        } else if s.channels > 0 {
                            line.push_str(&format!(" {} ch", s.channels));
                        }
                        if s.sample_rate > 0 {
                            line.push_str(&format!(" {} Hz", s.sample_rate));
                        }
                    }
                    _ => {}
                }
                if let Some(l) = &s.language {
                    line.push_str(&format!(" [{l}]"));
                }
                (s.index, line)
            })
            .collect();
        InputSummary {
            container: info.container.clone(),
            container_long: info.container_long.clone(),
            duration: info.duration,
            size: info.size,
            has_video: info.video().is_some(),
            has_audio: info.audio().is_some(),
            audio_codec: info.audio().map(|a| a.codec.clone()),
            streams,
        }
    }

    /// "Matroska · 1:23.4 · 12.3 MiB".
    pub fn line(&self) -> String {
        let mut parts =
            vec![if self.container_long.is_empty() { self.container.clone() } else { self.container_long.clone() }];
        if let Some(d) = self.duration {
            parts.push(format_time(d));
        }
        parts.push(format_size(self.size, SizeStyle::Binary));
        parts.join(" \u{b7} ")
    }
}

/// The probe of the first file.
#[derive(Clone, Debug, PartialEq)]
pub enum InputState {
    Pending,
    Ready(InputSummary),
    Failed(String),
}

/// The dry run of the current operation.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanState {
    /// Nothing asked yet (the operation is invalid).
    Idle,
    Pending,
    Ready(PlanInfo),
    Failed(String),
}

/// A row of the plan card and how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanTone {
    Text,
    Muted,
    Warning,
    Danger,
}

/// Results the worker threads leave for [`MediaToolsModel::sync`].
#[derive(Default)]
struct Shared {
    probe: Option<Result<InputSummary, String>>,
    /// The generation a plan was asked for last (older results are dropped).
    wanted: u64,
    plan: Option<(u64, Result<PlanInfo, String>)>,
}

/// Debounce before a plan runs (typing in a time field asks for one per keystroke).
const PLAN_DEBOUNCE: Duration = Duration::from_millis(120);

/// The media tools dialog (see the module docs).
pub struct MediaToolsModel {
    pub tool: MediaTool,
    /// The selected files; the first one is probed and planned.
    pub sources: Vec<PathBuf>,
    /// The built-in presets this runtime can encode, in menu order.
    pub presets: Vec<Preset>,
    /// Index into `presets`.
    pub preset: usize,
    pub codec: AudioCodecChoice,
    /// Trim window, as typed (`"1:05.5"`, `"65.5"`).
    pub start: String,
    pub end: String,
    pub stream_copy: bool,
    /// Save frame: the time, as typed.
    pub at: String,
    pub format: ImageFormat,
    /// The output file name (beside the first source).
    pub name: String,
    pub conflict: ConflictChoice,
    /// Ask the frontend to focus the name field once.
    pub focus: bool,
    pub input: InputState,
    pub plan: PlanState,
    /// Names in the source folder (the pane listing): no I/O for "exists" checks.
    existing: Vec<String>,
    /// The name was typed by the user (the automatic one stops following the extension).
    name_edited: bool,
    /// The end time was typed (else it follows the probed duration).
    end_edited: bool,
    rt: Arc<Runtime>,
    wake: Wake,
    shared: Arc<Mutex<Shared>>,
}

impl MediaToolsModel {
    pub fn new(tool: MediaTool, sources: Vec<PathBuf>, existing: Vec<String>, rt: Arc<Runtime>, wake: Wake) -> Self {
        let presets = soyuz_media::presets(&rt);
        let mut m = MediaToolsModel {
            tool,
            sources,
            presets,
            preset: 0,
            codec: AudioCodecChoice::Copy,
            start: "0".into(),
            end: String::new(),
            stream_copy: true,
            at: "0".into(),
            format: ImageFormat::Png,
            name: String::new(),
            conflict: ConflictChoice::Ask,
            focus: true,
            input: InputState::Pending,
            plan: PlanState::Idle,
            existing,
            name_edited: false,
            end_edited: false,
            rt,
            wake,
            shared: Arc::new(Mutex::new(Shared::default())),
        };
        m.start_probe();
        m.name = m.default_name();
        m.refresh_plan();
        m
    }

    pub fn title(&self) -> &'static str {
        self.tool.title()
    }

    pub fn source(&self) -> &Path {
        &self.sources[0]
    }

    fn source_name(&self) -> String {
        soyuz_core::entry::display_name(self.source())
    }

    /// "clip.mp4", or "3 files" with the probed one named.
    pub fn source_line(&self) -> String {
        let name = soyuz_core::naming::reveal_controls(&self.source_name()).into_owned();
        if self.sources.len() > 1 && self.tool.per_file() {
            tr!("dialogs-media-sources-many", count = self.sources.len(), first = name)
        } else {
            name
        }
    }

    /// Several files get one job each (Convert / Extract audio).
    pub fn multi(&self) -> bool {
        self.sources.len() > 1 && self.tool.per_file()
    }

    // ---- workers -------------------------------------------------------------------

    fn start_probe(&self) {
        let rt = Arc::clone(&self.rt);
        let src = self.source().to_path_buf();
        let shared = Arc::clone(&self.shared);
        let wake = self.wake.clone();
        let _ = std::thread::Builder::new().name("soyuz-media-probe".into()).spawn(move || {
            let r = soyuz_media::probe(&rt, &soyuz_media::Source::File(src))
                .map(|info| InputSummary::of(&info))
                .map_err(|e| e.to_string());
            shared.lock().probe = Some(r);
            wake();
        });
    }

    /// Ask for the dry run of the current operation (the previous one becomes stale).
    fn refresh_plan(&mut self) {
        let generation = {
            let mut s = self.shared.lock();
            s.wanted += 1;
            s.wanted
        };
        let Some(op) = self.op() else {
            self.plan = PlanState::Idle;
            return;
        };
        self.plan = PlanState::Pending;
        let rt = Arc::clone(&self.rt);
        let src = self.source().to_path_buf();
        let shared = Arc::clone(&self.shared);
        let wake = self.wake.clone();
        let _ = std::thread::Builder::new().name("soyuz-media-plan".into()).spawn(move || {
            std::thread::sleep(PLAN_DEBOUNCE);
            if shared.lock().wanted != generation {
                return;
            }
            let r = soyuz_media::plan(&rt, &src, &op).map_err(|e| e.to_string());
            let mut s = shared.lock();
            if s.wanted == generation {
                s.plan = Some((generation, r));
                drop(s);
                wake();
            }
        });
    }

    /// Move worker results into the model (the frontends call it before drawing).
    /// Returns `true` when something changed.
    pub fn sync(&mut self) -> bool {
        let (probe, plan) = {
            let mut s = self.shared.lock();
            let plan = s.plan.take().filter(|(g, _)| *g == s.wanted).map(|(_, r)| r);
            (s.probe.take(), plan)
        };
        let mut changed = false;
        if let Some(r) = probe {
            self.input = match r {
                Ok(i) => InputState::Ready(i),
                Err(e) => InputState::Failed(e),
            };
            if !self.end_edited && self.end.is_empty() {
                self.end = self.duration().map(format_time).unwrap_or_default();
            }
            // An operation that needed the probe (a trim window ending at the duration)
            // can be planned now.
            if matches!(self.plan, PlanState::Idle) {
                self.refresh_plan();
            }
            if !self.name_edited {
                self.name = self.default_name();
            }
            changed = true;
        }
        if let Some(r) = plan {
            self.plan = match r {
                Ok(p) => PlanState::Ready(p),
                Err(e) => PlanState::Failed(e),
            };
            if !self.name_edited {
                self.name = self.default_name();
            }
            changed = true;
        }
        changed
    }

    // ---- edits ---------------------------------------------------------------------

    pub fn set_tool(&mut self, tool: MediaTool) {
        if self.tool != tool {
            self.tool = tool;
            self.after_op_change();
        }
    }

    pub fn set_preset(&mut self, i: usize) {
        if i < self.presets.len() && i != self.preset {
            self.preset = i;
            self.after_op_change();
        }
    }

    pub fn set_codec(&mut self, c: AudioCodecChoice) {
        if self.codec != c {
            self.codec = c;
            self.after_op_change();
        }
    }

    pub fn set_format(&mut self, f: ImageFormat) {
        if self.format != f {
            self.format = f;
            self.after_op_change();
        }
    }

    pub fn set_stream_copy(&mut self, on: bool) {
        if self.stream_copy != on {
            self.stream_copy = on;
            self.after_op_change();
        }
    }

    pub fn set_start(&mut self, s: &str) {
        if self.start != s {
            self.start = s.to_string();
            self.after_op_change();
        }
    }

    pub fn set_end(&mut self, s: &str) {
        if self.end != s {
            self.end = s.to_string();
            self.end_edited = true;
            self.after_op_change();
        }
    }

    pub fn set_at(&mut self, s: &str) {
        if self.at != s {
            self.at = s.to_string();
            self.after_op_change();
        }
    }

    /// The user typed an output name: it stops following the operation's extension.
    pub fn set_name(&mut self, s: &str) {
        if self.name != s {
            self.name = s.to_string();
            self.name_edited = true;
        }
    }

    pub fn set_conflict(&mut self, c: ConflictChoice) {
        self.conflict = c;
    }

    fn after_op_change(&mut self) {
        // The plan first: a stale plan would still name the previous extension.
        self.refresh_plan();
        if !self.name_edited {
            self.name = self.default_name();
        }
    }

    // ---- derived -------------------------------------------------------------------

    fn summary(&self) -> Option<&InputSummary> {
        match &self.input {
            InputState::Ready(i) => Some(i),
            _ => None,
        }
    }

    pub fn duration(&self) -> Option<f64> {
        self.summary().and_then(|i| i.duration).filter(|d| *d > 0.0)
    }

    /// The operation as set up, `None` while a time field does not parse.
    pub fn op(&self) -> Option<MediaOp> {
        Some(match self.tool {
            MediaTool::Convert => {
                MediaOp::Convert { preset: self.presets.get(self.preset)?.id, video_stream: None, audio_stream: None }
            }
            MediaTool::ExtractAudio => MediaOp::ExtractAudio { codec: self.codec },
            MediaTool::Trim => {
                let start = parse_time(&self.start)?;
                let end = parse_time(&self.end)?;
                (end > start).then_some(MediaOp::Trim { start, end, stream_copy: self.stream_copy })?
            }
            MediaTool::SaveFrame => MediaOp::Frame { at: parse_time(&self.at)?, format: self.format, max_px: None },
        })
    }

    /// The extension the output gets: the plan's once it arrived, else what the operation
    /// implies (preset, codec, picture format, the input's container for a trim).
    pub fn expected_ext(&self) -> String {
        if let PlanState::Ready(p) = &self.plan
            && !p.ext.is_empty()
        {
            return p.ext.clone();
        }
        match self.tool {
            MediaTool::Convert => self.presets.get(self.preset).map(|p| p.ext.to_string()).unwrap_or_default(),
            MediaTool::ExtractAudio => match codec_ext(self.codec) {
                Some(e) => e.to_string(),
                None => self
                    .summary()
                    .and_then(|i| i.audio_codec.as_deref())
                    .map(|c| soyuz_media::audio_copy_container(c).1.to_string())
                    .unwrap_or_else(|| "mka".into()),
            },
            MediaTool::Trim => {
                let from_input = self
                    .summary()
                    .and_then(|i| soyuz_media::mux::container_for_trim(&i.container, None))
                    .map(|c| soyuz_media::ext_for_container(c).to_string());
                from_input.unwrap_or_else(|| {
                    self.source().extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
                })
            }
            MediaTool::SaveFrame => self.format.ext().to_string(),
        }
    }

    fn stem(&self) -> String {
        let name = self.source_name();
        match name.rfind('.') {
            Some(i) if i > 0 => name[..i].to_string(),
            _ => name,
        }
    }

    /// `<stem>.<ext>`, made unique against the folder's names (the source included).
    fn default_name(&self) -> String {
        let ext = self.expected_ext();
        let base = if ext.is_empty() { self.stem() } else { format!("{}.{ext}", self.stem()) };
        let src = self.source_name();
        let taken: Vec<&str> = self.existing.iter().map(String::as_str).chain(std::iter::once(src.as_str())).collect();
        names::unique_in(&taken, &base, names::case_insensitive_fs())
    }

    fn same_name(a: &str, b: &str) -> bool {
        if names::case_insensitive_fs() { a.eq_ignore_ascii_case(b) } else { a == b }
    }

    /// The typed name is taken in the folder (the job's conflict policy applies).
    pub fn name_exists(&self) -> bool {
        let n = self.name.trim();
        self.existing.iter().any(|e| Self::same_name(e, n))
    }

    /// Why the operation cannot start yet.
    pub fn error(&self) -> Option<String> {
        if let InputState::Failed(e) = &self.input {
            return Some(tr!("dialogs-media-input-failed", error = e.as_str()));
        }
        let name = self.name.trim();
        if name.is_empty() {
            return Some(tr!("dialogs-media-name-empty").to_string());
        }
        if soyuz_core::naming::validate_name_with(name, cfg!(windows)).is_err() {
            return Some(tr!("dialogs-media-name-invalid").to_string());
        }
        if Self::same_name(name, &self.source_name()) {
            return Some(tr!("dialogs-media-name-is-source").to_string());
        }
        let dur = self.duration();
        match self.tool {
            MediaTool::Trim => {
                let (Some(start), Some(end)) = (parse_time(&self.start), parse_time(&self.end)) else {
                    return Some(tr!("dialogs-media-time-invalid").to_string());
                };
                if let Some(d) = dur
                    && start >= d
                {
                    return Some(tr!("dialogs-media-time-beyond", duration = format_time(d)));
                }
                if end <= start {
                    return Some(tr!("dialogs-media-trim-order").to_string());
                }
            }
            MediaTool::SaveFrame => {
                let Some(at) = parse_time(&self.at) else {
                    return Some(tr!("dialogs-media-time-invalid").to_string());
                };
                if let Some(d) = dur
                    && at > d
                {
                    return Some(tr!("dialogs-media-time-beyond", duration = format_time(d)));
                }
                if self.summary().is_some_and(|i| !i.has_video) {
                    return Some(tr!("dialogs-media-no-video").to_string());
                }
            }
            MediaTool::ExtractAudio => {
                if self.summary().is_some_and(|i| !i.has_audio) {
                    return Some(tr!("dialogs-media-no-audio").to_string());
                }
            }
            MediaTool::Convert => {
                if self.presets.is_empty() {
                    return Some(tr!("dialogs-media-no-presets").to_string());
                }
            }
        }
        if let PlanState::Failed(e) = &self.plan {
            return Some(tr!("dialogs-media-plan-failed", error = e.as_str()));
        }
        None
    }

    /// The input card: the summary line and one line per stream, or the state.
    pub fn input_lines(&self) -> Vec<(PlanTone, String)> {
        match &self.input {
            InputState::Pending => vec![(PlanTone::Muted, tr!("dialogs-media-reading").to_string())],
            InputState::Failed(e) => vec![(PlanTone::Danger, e.clone())],
            InputState::Ready(i) => {
                let mut v = vec![(PlanTone::Text, i.line())];
                v.extend(i.streams.iter().map(|(_, s)| (PlanTone::Muted, s.clone())));
                v
            }
        }
    }

    /// The plan card: what happens to each stream, the estimated size, the warnings.
    pub fn plan_lines(&self) -> Vec<(PlanTone, String)> {
        match &self.plan {
            PlanState::Idle => Vec::new(),
            PlanState::Pending => vec![(PlanTone::Muted, tr!("dialogs-media-planning").to_string())],
            PlanState::Failed(e) => vec![(PlanTone::Danger, e.clone())],
            PlanState::Ready(p) => {
                let mut v = Vec::new();
                let label = |idx: usize| -> String {
                    self.summary()
                        .and_then(|i| i.streams.iter().find(|(i, _)| *i == idx))
                        .map(|(_, s)| s.clone())
                        .unwrap_or_else(|| tr!("dialogs-media-stream-n", n = idx))
                };
                for (idx, action) in &p.streams {
                    let tone = if action.starts_with("drop") { PlanTone::Muted } else { PlanTone::Text };
                    v.push((tone, format!("{}: {}", label(*idx), describe_action(action))));
                }
                let mut facts = vec![p.container.clone()];
                if let Some(d) = p.duration {
                    facts.push(format_time(d));
                }
                if let Some(b) = p.estimated_bytes {
                    facts.push(tr!("dialogs-media-estimated", size = format_size(b, SizeStyle::Binary)));
                }
                v.push((PlanTone::Muted, facts.join(" \u{b7} ")));
                v.extend(p.warnings.iter().map(|w| (PlanTone::Warning, w.clone())));
                if self.multi() {
                    v.push((PlanTone::Muted, tr!("dialogs-media-per-file", count = self.sources.len())));
                }
                v
            }
        }
    }

    // ---- submit --------------------------------------------------------------------

    /// The job specs (one per file for Convert / Extract audio), or why not.
    pub fn specs(&self) -> Result<Vec<JobSpec>, String> {
        if let Some(e) = self.error() {
            return Err(e);
        }
        let op = self.op().ok_or_else(|| tr!("dialogs-media-time-invalid").to_string())?;
        let name = self.name.trim();
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_string()).filter(|e| !e.is_empty());
        let files: &[PathBuf] = if self.tool.per_file() { &self.sources } else { &self.sources[..1] };
        let mut out = Vec::new();
        for (i, src) in files.iter().enumerate() {
            let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
            let dest = if i == 0 {
                dir.join(name)
            } else {
                // The other files get the same extension beside themselves.
                let n = soyuz_core::entry::display_name(src);
                let stem = match n.rfind('.') {
                    Some(k) if k > 0 => n[..k].to_string(),
                    _ => n.clone(),
                };
                let candidate = match &ext {
                    Some(e) => format!("{stem}.{e}"),
                    None => stem,
                };
                let unique = names::unique_in(&[n.as_str()], &candidate, names::case_insensitive_fs());
                dir.join(unique)
            };
            out.push(JobSpec {
                kind: JobKind::Media { src: src.clone(), dest, op: op.clone() },
                options: JobOptions {
                    priority: Priority::Low,
                    journal: true,
                    conflict: self.conflict.resolution(),
                    ..JobOptions::default()
                },
            });
        }
        Ok(out)
    }

    /// Confirmed: the command that submits the jobs (`Err` keeps the dialog open).
    pub fn submit(&self) -> Result<Cmd, String> {
        let mut cmds: Vec<Cmd> = self.specs()?.into_iter().map(|s| Cmd::SubmitJob(Box::new(s))).collect();
        Ok(if cmds.len() == 1 { cmds.remove(0) } else { Cmd::Batch(cmds) })
    }
}

/// `"encode h264_nvenc"` → "encode with h264_nvenc" style text, translated.
fn describe_action(action: &str) -> String {
    if action == "copy" {
        return tr!("dialogs-media-action-copy").to_string();
    }
    if let Some(enc) = action.strip_prefix("encode ") {
        return tr!("dialogs-media-action-encode", encoder = enc);
    }
    if let Some(why) = action.strip_prefix("drop: ") {
        return tr!("dialogs-media-action-drop", why = why);
    }
    action.to_string()
}

/// `"65.5"`, `"1:05.5"`, `"0:01:05"` → seconds.
pub fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut secs = 0.0;
    for part in s.split(':') {
        let v: f64 = part.trim().replace(',', ".").parse().ok()?;
        if !v.is_finite() || v < 0.0 {
            return None;
        }
        secs = secs * 60.0 + v;
    }
    Some(secs)
}

/// Seconds → `"1:05.5"` (hours only when needed, tenths always).
pub fn format_time(secs: f64) -> String {
    let secs = secs.max(0.0);
    let h = (secs / 3600.0).floor() as u64;
    let m = ((secs % 3600.0) / 60.0).floor() as u64;
    let s = secs % 60.0;
    if h > 0 { format!("{h}:{m:02}:{s:04.1}") } else { format!("{m}:{s:04.1}") }
}

/// The job card's facts for a media job (`Cmd::SubmitJob` lands in
/// `models::archive::submit_job`, which asks here for `JobKind::Media`).
pub fn job_meta(kind: &JobKind) -> Option<JobMeta> {
    let JobKind::Media { src, dest, op } = kind else { return None };
    let tool = MediaTool::of_op(op);
    Some(JobMeta::new(OpKind::Media(tool), std::slice::from_ref(src), Some(dest.clone())))
}

// ---- actions ---------------------------------------------------------------------------

/// The four media actions.
pub fn handles(a: &Action) -> bool {
    MediaTool::of_action(a).is_some()
}

/// `[media] tools = false` hides the actions from the palette.
pub fn palette_shows(app: &AppCore, a: &Action) -> bool {
    !handles(a) || app.config.media.tools
}

/// `ext` (lower-case, no dot) is a file the tools accept (video or audio).
pub fn is_media_ext(ext: &str) -> bool {
    let ffmpeg = media::runtime().is_some();
    crate::file_class::is_video_ext(ext, ffmpeg) || crate::file_class::is_audio_ext(ext, ffmpeg)
}

/// `ext` is a video container (Extract audio / Save frame make sense).
pub fn is_video_ext(ext: &str) -> bool {
    crate::file_class::is_video_ext(ext, media::runtime().is_some())
}

/// The media files among the focused pane's selection (plain folders only).
fn selected_media(app: &AppCore) -> Vec<PathBuf> {
    let pane = app.ws.pane();
    if !matches!(pane.location, Location::Dir(_)) {
        return Vec::new();
    }
    pane.selected_entries()
        .iter()
        .filter(|e| !e.is_dir_like() && e.extension().is_some_and(|x| is_media_ext(&x.to_ascii_lowercase())))
        .map(|e| e.path.clone())
        .collect()
}

/// Open the dialog for the action (toasts when the tools are off, the libraries are
/// missing or nothing suitable is selected).
pub fn execute(app: &mut AppCore, a: Action) {
    let Some(tool) = MediaTool::of_action(&a) else { return };
    if !app.config.media.tools {
        app.ui.toasts.push(Toast::info(tr!("dialogs-media-tools-off")));
        return;
    }
    let Some(rt) = media::runtime() else {
        let dir = soyuz_fs::strip_verbatim(&soyuz_core::config::config_dir().join("ffmpeg"));
        let body =
            tr!("services-preview-no-ffmpeg-libs", tag = media::runtime_release_tag(), dir = dir.display().to_string());
        app.ui.toasts.push(Toast::warning(tr!("dialogs-media-no-runtime")).body(body));
        return;
    };
    let sources = selected_media(app);
    if sources.is_empty() {
        app.ui.toasts.push(Toast::info(tr!("dialogs-media-select-file")));
        return;
    }
    let existing: Vec<String> = app.ws.pane().entries.iter().map(|e| e.name.clone()).collect();
    let wake = app.wake.clone();
    app.dialogs.push(MediaToolsModel::new(tool, sources, existing, rt, wake));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_parse_and_format() {
        assert_eq!(parse_time("65.5"), Some(65.5));
        assert_eq!(parse_time("1:05.5"), Some(65.5));
        assert_eq!(parse_time("0:01:05"), Some(65.0));
        assert_eq!(parse_time(" 2 "), Some(2.0));
        assert_eq!(parse_time("1,5"), Some(1.5));
        assert_eq!(parse_time(""), None);
        assert_eq!(parse_time("-1"), None);
        assert_eq!(parse_time("a:b"), None);
        assert_eq!(format_time(65.5), "1:05.5");
        assert_eq!(format_time(3600.0), "1:00:00.0");
        assert_eq!(format_time(0.0), "0:00.0");
        assert_eq!(parse_time(&format_time(1234.5)), Some(1234.5));
    }

    #[test]
    fn tools_map_to_actions_and_ops() {
        assert_eq!(MediaTool::of_action(&Action::MediaTrim), Some(MediaTool::Trim));
        assert_eq!(MediaTool::of_action(&Action::Copy), None);
        assert!(handles(&Action::MediaSaveFrame) && !handles(&Action::Paste));
        assert_eq!(MediaTool::of_op(&MediaOp::Remux { container: "mp4".into() }), MediaTool::Convert);
        assert_eq!(MediaTool::of_op(&MediaOp::ExtractAudio { codec: AudioCodecChoice::Aac }), MediaTool::ExtractAudio);
        assert!(MediaTool::Convert.per_file() && !MediaTool::Trim.per_file());
        assert_eq!(describe_action("copy"), "copy");
        assert_eq!(describe_action("encode mpeg4"), "encode with mpeg4");
        assert_eq!(describe_action("drop: not kept"), "dropped (not kept)");
        let meta = job_meta(&JobKind::Media {
            src: "/v/a.mp4".into(),
            dest: "/v/a.png".into(),
            op: MediaOp::Frame { at: 0.0, format: ImageFormat::Jpeg, max_px: None },
        })
        .unwrap();
        assert_eq!(meta.op, OpKind::Media(MediaTool::SaveFrame));
        assert_eq!(meta.dest.as_deref(), Some(Path::new("/v/a.png")));
        assert!(job_meta(&JobKind::EmptyTrash).is_none());
        assert_eq!(ConflictChoice::Overwrite.resolution(), Some(ConflictResolution::Overwrite));
        assert_eq!(ConflictChoice::Ask.resolution(), None);
        assert_eq!(codec_ext(AudioCodecChoice::Aac), Some("m4a"));
        assert!(is_media_ext("mp4") && is_media_ext("flac") && !is_media_ext("txt"));
        assert!(is_video_ext("mkv") && !is_video_ext("mp3"));
    }

    #[test]
    fn summary_lines_from_probe_info() {
        use soyuz_media::StreamInfo;
        let v = StreamInfo {
            index: 0,
            kind: Some(StreamKind::Video),
            codec: "h264".into(),
            width: 1920,
            height: 1080,
            fps: Some(25.0),
            ..StreamInfo::default()
        };
        let a = StreamInfo {
            index: 1,
            kind: Some(StreamKind::Audio),
            codec: "aac".into(),
            channel_layout: "stereo".into(),
            sample_rate: 48_000,
            language: Some("eng".into()),
            ..StreamInfo::default()
        };
        let info = MediaInfo {
            container: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            container_long: "QuickTime / MOV".into(),
            duration: Some(83.4),
            start: 0.0,
            bit_rate: None,
            size: 12_900_000,
            streams: vec![v, a],
            chapters: Vec::new(),
            tags: Vec::new(),
            cover: None,
        };
        let s = InputSummary::of(&info);
        assert!(s.has_video && s.has_audio);
        assert_eq!(s.audio_codec.as_deref(), Some("aac"));
        assert_eq!(s.streams[0].1, "video h264 1920\u{d7}1080 25 fps");
        assert_eq!(s.streams[1].1, "audio aac stereo 48000 Hz [eng]");
        assert!(s.line().starts_with("QuickTime / MOV \u{b7} 1:23.4 \u{b7} 12.3 MiB"), "{}", s.line());
    }

    // The model itself needs a loaded runtime (the with-libraries suite in
    // `tests/media_tools.rs` covers validation, the plan refresh and preset filtering).
}
