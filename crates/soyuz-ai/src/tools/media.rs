//! Media tools (Phase V): `media_info` reads a video / audio file through the FFmpeg
//! libraries (`soyuz-media`), and `media_convert` / `media_extract_audio` / `media_trim` /
//! `media_frame` plan a `JobKind::Media` job, the same job the media tools dialog submits:
//! output named like the dialog does (`<stem>.<ext>` beside the input, made unique), the
//! dialog's conflict choices, low priority, journaled, undo through the job's
//! identity-stamped `Copied` record.
//!
//! Planning opens the input (`soyuz_media::probe` + `soyuz_media::plan`), so it needs the
//! libraries; it never writes. [`MediaAccess`] carries the `[media]` settings the tools
//! obey: `ffmpeg = false` refuses every media tool, `tools = false` refuses the write tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use soyuz_core::{ConflictResolution, EntryKind, SizeStyle, format_size};
use soyuz_jobs::{JobKind, JobOptions, JobSpec, Priority};
use soyuz_media::{AudioCodecChoice, ImageFormat, MediaError, MediaInfo, MediaOp, PresetId, Runtime, StreamKind};

use super::args::*;
use super::plan::{Plan, PlanAction, PlanConflict, PlanItem, PlanOp, check_new_name, exists_conflict};
use super::scope::Scope;
use super::{Risk, ToolError, ToolInput};

/// Error text when no libraries are loaded (the model and the user read it).
pub const NOT_LOADED: &str = "FFmpeg libraries not loaded: media tools need the FFmpeg runtime \
     (Settings → Preview → Media shows where Soyuz looks for it)";

/// Tags / chapters handed to the model (untrusted text, capped).
const MAX_TAGS: usize = 64;
const MAX_CHAPTERS: usize = 100;
const MAX_TAG_CHARS: usize = 500;

/// The `[media]` settings the media tools follow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaAccess {
    /// `[media] ffmpeg`: off refuses every media tool.
    pub ffmpeg: bool,
    /// `[media] tools`: off refuses the write tools (`media_info` keeps working).
    pub tools: bool,
    /// Folders to load the libraries from when this process has not loaded them yet
    /// (`soyuz_media::search_dirs`). Empty: only an already loaded runtime is used (the
    /// app loads it itself, `services::media`).
    pub dirs: Vec<PathBuf>,
}

impl Default for MediaAccess {
    fn default() -> Self {
        MediaAccess { ffmpeg: true, tools: true, dirs: Vec::new() }
    }
}

impl MediaAccess {
    /// From `[media]`; `load` adds the library search folders (headless hosts that have
    /// no other loader).
    pub fn from_config(m: &soyuz_core::config::Media, load: bool) -> Self {
        let dirs = if load && m.ffmpeg {
            soyuz_media::search_dirs(&m.ffmpeg_dir, &soyuz_core::config::config_dir())
        } else {
            Vec::new()
        };
        MediaAccess { ffmpeg: m.ffmpeg, tools: m.tools, dirs }
    }

    /// The runtime, or why the media tools cannot run.
    pub fn runtime(&self) -> Result<Arc<Runtime>, ToolError> {
        if !self.ffmpeg {
            return Err(ToolError::Failed(
                "the FFmpeg libraries are turned off in the settings ([media] ffmpeg = false)".into(),
            ));
        }
        if let Some(rt) = Runtime::loaded() {
            return Ok(rt);
        }
        if self.dirs.is_empty() {
            return Err(ToolError::Failed(NOT_LOADED.into()));
        }
        Runtime::get(&self.dirs).map_err(|e| ToolError::Failed(format!("{NOT_LOADED} ({e})")))
    }

    /// The runtime for a write tool (also checks `[media] tools`).
    pub fn runtime_for_tools(&self) -> Result<Arc<Runtime>, ToolError> {
        if !self.tools {
            return Err(ToolError::Failed(
                "the media tools are turned off in the settings ([media] tools = false)".into(),
            ));
        }
        self.runtime()
    }
}

/// The media tools' inputs (`media_info` and the four write tools).
pub fn is_media_tool(input: &ToolInput) -> bool {
    media_source(input).is_some()
}

/// The file a media tool opens.
pub fn media_source(input: &ToolInput) -> Option<&Path> {
    Some(match input {
        ToolInput::MediaInfo(a) => &a.path,
        ToolInput::MediaConvert(a) => &a.path,
        ToolInput::MediaExtractAudio(a) => &a.path,
        ToolInput::MediaTrim(a) => &a.path,
        ToolInput::MediaFrame(a) => &a.path,
        _ => return None,
    })
}

fn media_err(e: MediaError) -> ToolError {
    match e {
        MediaError::NotLoaded => ToolError::Failed(NOT_LOADED.into()),
        e => ToolError::Failed(e.to_string()),
    }
}

fn input_file(path: &Path) -> Result<u64, ToolError> {
    let e = soyuz_fs::stat(path, true).map_err(|e| ToolError::Failed(e.to_string()))?;
    if e.kind != EntryKind::File {
        return Err(ToolError::Failed(format!("not a file: {}", path.display())));
    }
    Ok(e.size)
}

fn capped(s: &str) -> String {
    if s.chars().count() > MAX_TAG_CHARS {
        let mut out: String = s.chars().take(MAX_TAG_CHARS).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

fn tags_json(tags: &[(String, String)]) -> Value {
    tags.iter().take(MAX_TAGS).map(|(k, v)| json!({"key": capped(k), "value": capped(v)})).collect::<Vec<_>>().into()
}

/// Model-facing facts of a probe (no cover bytes; tags and chapters capped).
pub fn info_json(path: &Path, info: &MediaInfo, presets: &[&str]) -> Value {
    let streams: Vec<Value> = info
        .streams
        .iter()
        .map(|s| {
            let mut v = json!({
                "index": s.index,
                "kind": s.kind.map(|k| k.to_string()).unwrap_or_else(|| "other".into()),
                "codec": s.codec,
                "codec_long": s.codec_long,
                "default": s.default,
            });
            let o = v.as_object_mut().expect("object");
            let mut put = |k: &str, val: Value| {
                if !val.is_null() {
                    o.insert(k.into(), val);
                }
            };
            put("profile", json!(s.profile));
            put("bit_rate", json!(s.bit_rate));
            put("language", json!(s.language));
            put("title", json!(s.title.as_deref().map(capped)));
            match s.kind {
                Some(StreamKind::Video) => {
                    put("width", json!(s.width));
                    put("height", json!(s.height));
                    put("rotation", json!(s.rotation));
                    put("fps", json!(s.fps));
                    put("pix_fmt", json!(s.pix_fmt));
                    put("hdr", json!(s.hdr));
                    put("cover_art", json!(s.attached_pic));
                }
                Some(StreamKind::Audio) => {
                    put("sample_rate", json!(s.sample_rate));
                    put("channels", json!(s.channels));
                    if !s.channel_layout.is_empty() {
                        put("channel_layout", json!(s.channel_layout));
                    }
                }
                _ => {}
            }
            if s.forced {
                put("forced", json!(true));
            }
            if !s.tags.is_empty() {
                put("tags", tags_json(&s.tags));
            }
            v
        })
        .collect();
    let chapters: Vec<Value> = info
        .chapters
        .iter()
        .take(MAX_CHAPTERS)
        .map(|c| json!({"start": c.start, "end": c.end, "title": c.title.as_deref().map(capped)}))
        .collect();
    json!({
        "path": path,
        "container": info.container,
        "container_long": info.container_long,
        "duration": info.duration,
        "start": info.start,
        "bit_rate": info.bit_rate,
        "size": info.size,
        "rotation": info.video().map(|v| v.rotation).unwrap_or(0),
        "streams": streams,
        "chapters": chapters,
        "chapters_truncated": info.chapters.len() > MAX_CHAPTERS,
        "tags": tags_json(&info.tags),
        "has_cover_art": info.cover.is_some(),
        "presets": presets,
    })
}

/// `media_info`.
pub fn info(access: &MediaAccess, path: &Path) -> Result<Value, ToolError> {
    let rt = access.runtime()?;
    input_file(path)?;
    let info = soyuz_media::probe(&rt, &soyuz_media::Source::File(path.to_path_buf())).map_err(media_err)?;
    let presets: Vec<&str> =
        if access.tools { soyuz_media::presets(&rt).iter().map(|p| p.id.name()).collect() } else { Vec::new() };
    Ok(info_json(path, &info, &presets))
}

/// Seconds → `"1:05.5"` (the media tools dialog's format).
pub fn format_time(secs: f64) -> String {
    let secs = secs.max(0.0);
    let h = (secs / 3600.0).floor() as u64;
    let m = ((secs % 3600.0) / 60.0).floor() as u64;
    let s = secs % 60.0;
    if h > 0 { format!("{h}:{m:02}:{s:04.1}") } else { format!("{m}:{s:04.1}") }
}

fn preset_id(p: MediaPresetArg) -> Result<PresetId, ToolError> {
    PresetId::parse(&p.name()).ok_or_else(|| ToolError::InvalidArgs(vec![format!("unknown preset “{}”", p.name())]))
}

fn audio_codec(c: MediaAudioCodecArg) -> AudioCodecChoice {
    match c {
        MediaAudioCodecArg::Copy => AudioCodecChoice::Copy,
        MediaAudioCodecArg::Flac => AudioCodecChoice::Flac,
        MediaAudioCodecArg::Aac => AudioCodecChoice::Aac,
        MediaAudioCodecArg::Opus => AudioCodecChoice::Opus,
        MediaAudioCodecArg::Wav => AudioCodecChoice::Wav,
    }
}

fn conflict_parts(c: MediaConflictArg) -> (&'static str, Option<ConflictResolution>) {
    match c {
        MediaConflictArg::Ask => ("ask", None),
        MediaConflictArg::KeepBoth => ("keep_both", Some(ConflictResolution::KeepBoth)),
        MediaConflictArg::Overwrite => ("overwrite", Some(ConflictResolution::Overwrite)),
        MediaConflictArg::Skip => ("skip", Some(ConflictResolution::Skip)),
    }
}

fn time_ok(t: f64) -> bool {
    t.is_finite() && t >= 0.0
}

/// A write tool's request, validated without opening the file: the tool name, the input,
/// the operation, the explicit output and the conflict choice.
pub struct MediaRequest {
    pub tool: &'static str,
    pub op_kind: PlanOp,
    pub src: PathBuf,
    pub op: MediaOp,
    pub dest: Option<PathBuf>,
    pub conflict: MediaConflictArg,
}

impl MediaRequest {
    /// The request of a media write tool (`None` for other tools); argument errors
    /// (trim window, frame time, picture size) are reported here.
    pub fn of(input: &ToolInput) -> Option<Result<MediaRequest, ToolError>> {
        let bad = |m: String| Some(Err(ToolError::InvalidArgs(vec![m])));
        let (tool, op_kind, src, op, dest, conflict) = match input {
            ToolInput::MediaConvert(a) => {
                let preset = match preset_id(a.preset) {
                    Ok(p) => p,
                    Err(e) => return Some(Err(e)),
                };
                let op = MediaOp::Convert { preset, video_stream: None, audio_stream: None };
                ("media_convert", PlanOp::MediaConvert, &a.path, op, &a.dest, a.conflict)
            }
            ToolInput::MediaExtractAudio(a) => {
                let op = MediaOp::ExtractAudio { codec: audio_codec(a.codec) };
                ("media_extract_audio", PlanOp::MediaExtractAudio, &a.path, op, &a.dest, a.conflict)
            }
            ToolInput::MediaTrim(a) => {
                if !time_ok(a.start) || !a.end.is_finite() {
                    return bad(format!("start must be ≥ 0 seconds (got {})", a.start));
                }
                if a.end <= a.start {
                    return bad(format!("end ({}) must be after start ({})", a.end, a.start));
                }
                let op = MediaOp::Trim { start: a.start, end: a.end, stream_copy: a.stream_copy };
                ("media_trim", PlanOp::MediaTrim, &a.path, op, &a.dest, a.conflict)
            }
            ToolInput::MediaFrame(a) => {
                if !time_ok(a.at) {
                    return bad(format!("at must be ≥ 0 seconds (got {})", a.at));
                }
                if a.max_px.is_some_and(|p| p < 16) {
                    return bad("max_px must be at least 16".into());
                }
                let format = match a.format {
                    MediaImageFormatArg::Png => ImageFormat::Png,
                    MediaImageFormatArg::Jpeg => ImageFormat::Jpeg,
                };
                let op = MediaOp::Frame { at: a.at, format, max_px: a.max_px };
                ("media_frame", PlanOp::MediaFrame, &a.path, op, &a.dest, a.conflict)
            }
            _ => return None,
        };
        Some(Ok(MediaRequest { tool, op_kind, src: src.clone(), op, dest: dest.clone(), conflict }))
    }

    /// One line for the plan card.
    fn summary(&self, dest: &Path) -> String {
        let src = soyuz_core::entry::display_name(&self.src);
        let out = soyuz_core::entry::display_name(dest);
        match &self.op {
            MediaOp::Convert { preset, .. } => format!("Convert {src} to {out} ({})", preset.name()),
            MediaOp::Remux { container } => format!("Remux {src} to {out} ({container})"),
            MediaOp::ExtractAudio { .. } => format!("Extract the audio of {src} to {out}"),
            MediaOp::Trim { start, end, stream_copy } => format!(
                "Trim {src} {}–{} into {out}{}",
                format_time(*start),
                format_time(*end),
                if *stream_copy { "" } else { " (re-encoded)" }
            ),
            MediaOp::Frame { at, .. } => format!("Save the frame at {} of {src} as {out}", format_time(*at)),
        }
    }
}

/// Plan a media write tool: probe the input, check the operation against it, dry-run the
/// transcoder for the output extension, stream actions and size, name the output and
/// build the job.
pub fn plan(scope: &Scope, rt: &Runtime, req: MediaRequest) -> Result<Plan, ToolError> {
    let size = input_file(&req.src)?;
    let info = soyuz_media::probe(rt, &soyuz_media::Source::File(req.src.clone())).map_err(media_err)?;
    let duration = info.duration.filter(|d| *d > 0.0);
    match &req.op {
        MediaOp::Convert { preset, .. } => {
            let available = soyuz_media::presets(rt);
            if !available.iter().any(|p| p.id == *preset) {
                let names: Vec<&str> = available.iter().map(|p| p.id.name()).collect();
                return Err(ToolError::InvalidArgs(vec![format!(
                    "preset “{}” cannot be encoded by this FFmpeg build; available: {}",
                    preset.name(),
                    names.join(", ")
                )]));
            }
        }
        MediaOp::ExtractAudio { .. } if info.audio().is_none() => {
            return Err(ToolError::Failed("the file has no audio stream".into()));
        }
        MediaOp::Trim { start, .. } => {
            if let Some(d) = duration
                && *start >= d
            {
                return Err(ToolError::InvalidArgs(vec![format!(
                    "start {} is beyond the duration {}",
                    format_time(*start),
                    format_time(d)
                )]));
            }
        }
        MediaOp::Frame { at, .. } => {
            if info.video().is_none() {
                return Err(ToolError::Failed("the file has no video stream".into()));
            }
            if let Some(d) = duration
                && *at > d
            {
                return Err(ToolError::InvalidArgs(vec![format!(
                    "at {} is beyond the duration {}",
                    format_time(*at),
                    format_time(d)
                )]));
            }
        }
        _ => {}
    }
    let planned = soyuz_media::plan(rt, &req.src, &req.op).map_err(media_err)?;
    let ext = if planned.ext.is_empty() {
        soyuz_jobs::kinds::media::expected_ext(&req.src, &req.op)
    } else {
        planned.ext.clone()
    };
    let mut warnings = planned.warnings.clone();
    let dest = match &req.dest {
        Some(d) => {
            let got = d.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if !ext.is_empty() && got != ext {
                warnings.push(format!("the output's extension “.{got}” is not the usual “.{ext}” of this container"));
            }
            d.clone()
        }
        None => soyuz_jobs::kinds::media::default_dest(&req.src, &ext),
    };
    scope.check_write(&dest)?;
    if let Some(parent) = dest.parent()
        && !soyuz_fs::stat(parent, true).is_ok_and(|e| e.is_dir_like())
    {
        return Err(ToolError::Failed(format!("the output folder does not exist: {}", parent.display())));
    }
    let (label, resolution) = conflict_parts(req.conflict);
    let mut item = PlanItem::new(req.op_kind, Some(req.src.clone()), Some(dest.clone()));
    item.kind = "file".into();
    item.bytes = size;
    item.entries = 1;
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let same = soyuz_fs::normalize(&dest) == soyuz_fs::normalize(&req.src)
        || (cfg!(any(windows, target_os = "macos"))
            && dest.to_string_lossy().to_lowercase() == req.src.to_string_lossy().to_lowercase());
    item.conflict = if same {
        Some(PlanConflict::InvalidName { reason: "the output cannot be the input file".into() })
    } else {
        check_new_name(&name).or_else(|| exists_conflict(&dest, label))
    };
    // What happens to each stream, and the expected size, after the main line.
    let mut facts: Vec<String> = planned.streams.iter().map(|(i, a)| format!("stream {i}: {a}")).collect();
    if let Some(b) = planned.estimated_bytes {
        facts.push(format!("≈ {}", format_size(b, SizeStyle::Binary)));
    }
    let line = req.summary(&dest);
    let mut summary = line.clone();
    if !facts.is_empty() {
        summary.push_str(&format!(" — {}", facts.join(", ")));
    }
    let spec = JobSpec {
        kind: JobKind::Media { src: req.src.clone(), dest, op: req.op.clone() },
        options: JobOptions {
            priority: Priority::Low,
            journal: true,
            conflict: resolution,
            label: Some(format!("AI: {line}")),
            ..JobOptions::default()
        },
    };
    let mut plan = Plan::finish(req.tool, Risk::Write, summary, vec![item], PlanAction::Job(spec));
    plan.warnings = warnings;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(name: &str, args: Value) -> ToolInput {
        ToolInput::parse(name, args).unwrap()
    }

    #[test]
    fn preset_args_match_the_media_presets() {
        let all = [
            MediaPresetArg::MkvCopy,
            MediaPresetArg::Mp4Copy,
            MediaPresetArg::Mp4H264Hw,
            MediaPresetArg::Mp4HevcHw,
            MediaPresetArg::WebmVp9,
            MediaPresetArg::WebmVp8,
            MediaPresetArg::MovProres,
            MediaPresetArg::Mp4Mpeg4,
            MediaPresetArg::AviMpeg4,
            MediaPresetArg::AudioFlac,
            MediaPresetArg::AudioAac,
            MediaPresetArg::AudioOpus,
            MediaPresetArg::AudioWav,
            MediaPresetArg::AudioAlac,
        ];
        for p in all {
            let id = preset_id(p).unwrap();
            assert_eq!(id.name(), p.name());
        }
        assert_eq!(MediaPresetArg::Mp4H264Hw.name(), "mp4-h264-hw");
        assert!(ToolInput::parse("media_convert", json!({"path": "/v/a.mkv", "preset": "x264"})).is_err());
    }

    #[test]
    fn requests_validate_times() {
        let req = |name: &str, args: Value| MediaRequest::of(&input(name, args)).unwrap();
        let t = req("media_trim", json!({"path": "/v/a.mkv", "start": 1.5, "end": 4}));
        let t = t.unwrap();
        assert_eq!(t.op, MediaOp::Trim { start: 1.5, end: 4.0, stream_copy: true });
        assert_eq!(t.tool, "media_trim");
        assert!(matches!(
            req("media_trim", json!({"path": "/v/a.mkv", "start": 5, "end": 4})),
            Err(ToolError::InvalidArgs(_))
        ));
        assert!(matches!(
            req("media_trim", json!({"path": "/v/a.mkv", "start": -1, "end": 4})),
            Err(ToolError::InvalidArgs(_))
        ));
        assert!(matches!(req("media_frame", json!({"path": "/v/a.mkv", "at": -0.5})), Err(ToolError::InvalidArgs(_))));
        assert!(matches!(
            req("media_frame", json!({"path": "/v/a.mkv", "at": 1, "max_px": 4})),
            Err(ToolError::InvalidArgs(_))
        ));
        let f = req("media_frame", json!({"path": "/v/a.mkv", "at": 2, "format": "jpeg"})).unwrap();
        assert_eq!(f.op, MediaOp::Frame { at: 2.0, format: ImageFormat::Jpeg, max_px: None });
        let x =
            req("media_extract_audio", json!({"path": "/v/a.mkv", "codec": "aac", "conflict": "overwrite"})).unwrap();
        assert_eq!(x.op, MediaOp::ExtractAudio { codec: AudioCodecChoice::Aac });
        assert_eq!(conflict_parts(x.conflict), ("overwrite", Some(ConflictResolution::Overwrite)));
        assert!(MediaRequest::of(&input("stat", json!({"path": "/v/a.mkv"}))).is_none());
        assert_eq!(media_source(&input("media_info", json!({"path": "/v/a.mkv"}))), Some(Path::new("/v/a.mkv")));
    }

    #[test]
    fn settings_gate_the_tools() {
        let off = MediaAccess { ffmpeg: false, ..MediaAccess::default() };
        let e = off.runtime().unwrap_err().to_string();
        assert!(e.contains("[media] ffmpeg = false"), "{e}");
        let no_tools = MediaAccess { tools: false, ..MediaAccess::default() };
        let e = no_tools.runtime_for_tools().unwrap_err().to_string();
        assert!(e.contains("[media] tools = false"), "{e}");
        // Nothing loaded and nowhere to load from: a readable error, not a panic.
        if Runtime::loaded().is_none() {
            let e = MediaAccess::default().runtime().unwrap_err().to_string();
            assert!(e.contains("FFmpeg libraries not loaded"), "{e}");
        }
    }

    #[test]
    fn info_json_is_capped_and_has_no_cover_bytes() {
        use soyuz_media::{Chapter, StreamInfo};
        let v = StreamInfo {
            index: 0,
            kind: Some(StreamKind::Video),
            codec: "h264".into(),
            width: 1080,
            height: 1920,
            rotation: 90,
            fps: Some(30.0),
            ..StreamInfo::default()
        };
        let a = StreamInfo {
            index: 1,
            kind: Some(StreamKind::Audio),
            codec: "aac".into(),
            sample_rate: 48_000,
            channels: 2,
            channel_layout: "stereo".into(),
            ..StreamInfo::default()
        };
        let info = MediaInfo {
            container: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            container_long: "QuickTime / MOV".into(),
            duration: Some(12.5),
            size: 1000,
            streams: vec![v, a],
            chapters: (0..150).map(|i| Chapter { start: i as f64, end: i as f64 + 1.0, title: None }).collect(),
            tags: vec![("comment".into(), "x".repeat(2000))],
            cover: Some(vec![1, 2, 3]),
            ..MediaInfo::default()
        };
        let j = info_json(Path::new("/v/a.mp4"), &info, &["mkv-copy"]);
        assert_eq!(j["rotation"], 90);
        assert_eq!(j["streams"][0]["width"], 1080);
        assert!(j["streams"][0].get("sample_rate").is_none());
        assert_eq!(j["streams"][1]["channel_layout"], "stereo");
        assert_eq!(j["chapters"].as_array().unwrap().len(), MAX_CHAPTERS);
        assert_eq!(j["chapters_truncated"], true);
        assert!(j["tags"][0]["value"].as_str().unwrap().chars().count() <= MAX_TAG_CHARS + 1);
        assert_eq!(j["has_cover_art"], true);
        assert!(!j.to_string().contains("[1,2,3]"));
        assert_eq!(j["presets"][0], "mkv-copy");
    }

    #[test]
    fn times_format_like_the_dialog() {
        assert_eq!(format_time(65.5), "1:05.5");
        assert_eq!(format_time(3600.0), "1:00:00.0");
    }
}
