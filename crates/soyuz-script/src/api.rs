//! The Soyuz API visible to scripts.
//!
//! | Group | Functions |
//! |---|---|
//! | register (top level only) | `register_action(#{…})`, `register_menu_item(#{…})`, `register_column(#{…})`, `register_previewer(#{…})` |
//! | read (capped) | `list(dir)`, `stat(path)`, `exists(path)`, `is_dir(path)`, `read_text(path)`, `read_text(path, max_bytes)`, `search(dir, glob)` |
//! | write (planned jobs) | `copy(src, dest_dir)`, `move_to(src, dest_dir)`, `rename(path, new_name)`, `trash(paths)`, `mkdir(path)` |
//! | media (FFmpeg; actions and rules) | `media_info(path)`; planned jobs: `convert(src, preset)`, `extract_audio(src)`, `extract_audio(src, codec)`, `trim(src, start, end)`, `save_frame(src, secs)`, `save_frame(src, secs, format)` |
//! | UI | `navigate(path)`, `select(paths)`, `toast(msg)`, `copy_text(text)`, `tag(paths, color)` |
//! | helpers | `file_name`, `file_stem`, `extension`, `parent`, `join_path`, `now()`, `format_size(n)`, `format_date(secs, fmt)` |
//!
//! `src` / `paths` take a string or an array of strings. Entries are maps
//! `#{ name, path, is_dir, is_file, is_link, size, modified, ext, hidden }` (`modified`
//! in Unix seconds, `()` when unknown).
//!
//! The media functions need the FFmpeg libraries the app loaded (`[media] ffmpeg`); the
//! planned media jobs also need `[media] tools` ([`set_media_access`]). Each media job
//! writes a new file beside its input, named like the media tools dialog does
//! (`clip.mp4` → `clip.mkv`, `clip (2).mkv` when taken).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};

use rhai::{Array, Dynamic, Engine, FnPtr, ImmutableString, Map};
use soyuz_core::{EntryInfo, EntryKind};
use soyuz_jobs::{CreateKind, DeleteMode, JobKind};
use soyuz_media::{AudioCodecChoice, ImageFormat, MediaOp, PresetId, Runtime, StreamKind};

use crate::sandbox::{ApiResult, Mode, fail, require, state};
use crate::script::{ActionDef, ColumnDef, MenuContext, PreviewerDef};
use crate::{PlannedJob, UiOp};

pub(crate) fn register(e: &mut Engine) {
    // --- registration ---
    e.register_fn("register_action", |m: Map| register_action(m, false));
    e.register_fn("register_menu_item", |m: Map| register_action(m, true));
    e.register_fn("register_column", register_column);
    e.register_fn("register_previewer", register_previewer);
    // --- reads ---
    e.register_fn("list", list);
    e.register_fn("stat", stat);
    e.register_fn("exists", |p: &str| -> ApiResult<bool> {
        read_gate("exists")?;
        Ok(soyuz_fs::exists_nofollow(&abs(p)?))
    });
    e.register_fn("is_dir", |p: &str| -> ApiResult<bool> {
        read_gate("is_dir")?;
        Ok(soyuz_fs::stat(&abs(p)?, true).is_ok_and(|e| e.is_dir_like()))
    });
    e.register_fn("read_text", |p: &str| read_text(p, i64::MAX));
    e.register_fn("read_text", read_text);
    e.register_fn("search", search);
    // --- planned writes ---
    e.register_fn("copy", |src: Dynamic, dest: &str| plan_transfer(src, dest, false));
    e.register_fn("move_to", |src: Dynamic, dest: &str| plan_transfer(src, dest, true));
    e.register_fn("rename", plan_rename);
    e.register_fn("trash", plan_trash);
    e.register_fn("mkdir", plan_mkdir);
    // --- media ---
    e.register_fn("media_info", media_info);
    e.register_fn("convert", plan_convert);
    e.register_fn("extract_audio", |src: Dynamic| plan_extract_audio(src, "copy"));
    e.register_fn("extract_audio", plan_extract_audio);
    e.register_fn("trim", plan_trim);
    e.register_fn("save_frame", |src: Dynamic, secs: Dynamic| plan_save_frame(src, secs, "png"));
    e.register_fn("save_frame", plan_save_frame);
    // --- UI ---
    e.register_fn("navigate", |p: &str| ui_op("navigate", abs(p).map(UiOp::Navigate)));
    e.register_fn("select", |paths: Dynamic| ui_op("select", paths_of(paths).map(UiOp::Select)));
    e.register_fn("toast", |m: &str| ui_op("toast", Ok(UiOp::Toast(m.chars().take(300).collect()))));
    e.register_fn("copy_text", |t: &str| ui_op("copy_text", Ok(UiOp::CopyText(t.chars().take(1_000_000).collect()))));
    e.register_fn("tag", |paths: Dynamic, color: &str| {
        let color = color.to_string();
        ui_op("tag", paths_of(paths).map(|p| UiOp::Tag(p, color)))
    });
    // --- helpers (pure) ---
    e.register_fn("file_name", |p: &str| -> String {
        Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    });
    e.register_fn("file_stem", |p: &str| -> String {
        Path::new(p).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    });
    e.register_fn("extension", |p: &str| -> String {
        let name = Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        name_ext(&name).unwrap_or("").to_string()
    });
    e.register_fn("parent", |p: &str| -> String {
        Path::new(p).parent().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    });
    e.register_fn("join_path", |a: &str, b: &str| -> String { Path::new(a).join(b).to_string_lossy().into_owned() });
    e.register_fn("now", || -> i64 { unix(SystemTime::now()).unwrap_or(0) });
    e.register_fn("format_size", |n: i64| -> String {
        soyuz_core::format_size(n.max(0) as u64, soyuz_core::SizeStyle::Binary)
    });
    e.register_fn("format_date", |secs: i64, fmt: &str| -> String {
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|d| d.with_timezone(&chrono::Local).format(fmt).to_string())
            .unwrap_or_default()
    });
}

/// Extension of a file name (no dot; `None` for dot-files and names without one).
pub fn name_ext(name: &str) -> Option<&str> {
    let i = name.rfind('.')?;
    (i > 0 && i + 1 < name.len()).then(|| &name[i + 1..])
}

fn unix(t: SystemTime) -> Option<i64> {
    t.duration_since(SystemTime::UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

/// Entry map handed to scripts.
pub fn entry_map(e: &EntryInfo) -> Map {
    let mut m = Map::new();
    m.insert("name".into(), e.name.clone().into());
    m.insert("path".into(), soyuz_fs::strip_verbatim(&e.path).to_string_lossy().into_owned().into());
    m.insert("is_dir".into(), e.is_dir_like().into());
    m.insert("is_file".into(), matches!(e.kind, EntryKind::File).into());
    m.insert("is_link".into(), matches!(e.kind, EntryKind::Symlink { .. } | EntryKind::Junction).into());
    m.insert("size".into(), (e.size.min(i64::MAX as u64) as i64).into());
    m.insert("modified".into(), e.modified.and_then(unix).map(Dynamic::from).unwrap_or(Dynamic::UNIT));
    m.insert("ext".into(), e.extension().unwrap_or("").to_ascii_lowercase().into());
    m.insert("hidden".into(), e.is_hidden().into());
    m
}

fn abs(p: &str) -> ApiResult<PathBuf> {
    let path = PathBuf::from(p.trim());
    if p.trim().is_empty() {
        return fail("empty path");
    }
    if !path.is_absolute() {
        return fail(format!("“{p}” is not an absolute path"));
    }
    Ok(path)
}

/// A string or an array of strings → absolute paths.
fn paths_of(v: Dynamic) -> ApiResult<Vec<PathBuf>> {
    if v.is_string() {
        return Ok(vec![abs(&v.into_string().unwrap_or_default())?]);
    }
    if v.is_map() {
        // An entry map: use its path.
        let m = v.cast::<Map>();
        let p = m.get("path").map(|p| p.to_string()).unwrap_or_default();
        return Ok(vec![abs(&p)?]);
    }
    let Some(arr) = v.try_cast::<Array>() else {
        return fail("expected a path or an array of paths");
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        if item.is_map() {
            let m = item.cast::<Map>();
            out.push(abs(&m.get("path").map(|p| p.to_string()).unwrap_or_default())?);
        } else {
            out.push(abs(&item.to_string())?);
        }
    }
    Ok(out)
}

fn read_gate(what: &str) -> ApiResult<()> {
    require(what, Mode::Pure)?;
    if !state(|s| s.limits.allow_read).unwrap_or(false) {
        return fail(format!("{what}(): reading files is turned off ([scripting] allow_read = false)"));
    }
    Ok(())
}

fn deadline_passed() -> bool {
    state(|s| Instant::now() > s.deadline).unwrap_or(true)
}

fn list(dir: &str) -> ApiResult<Array> {
    read_gate("list")?;
    let dir = abs(dir)?;
    let cap = state(|s| s.limits.max_list_entries).unwrap_or(1000);
    let opts = soyuz_fs::ListOptions { include_hidden: true, ..Default::default() };
    let mut entries = match soyuz_fs::read_dir(&dir, &opts) {
        Ok(e) => e,
        Err(e) => return fail(format!("list({}): {e}", dir.display())),
    };
    entries.sort_by(|a, b| soyuz_core::natural_cmp(&a.name, &b.name, true));
    Ok(entries.iter().take(cap).map(|e| Dynamic::from_map(entry_map(e))).collect())
}

fn stat(p: &str) -> ApiResult<Dynamic> {
    read_gate("stat")?;
    Ok(match soyuz_fs::stat(&abs(p)?, true) {
        Ok(e) => Dynamic::from_map(entry_map(&e)),
        Err(_) => Dynamic::UNIT,
    })
}

fn read_text(p: &str, max: i64) -> ApiResult<String> {
    read_gate("read_text")?;
    let path = abs(p)?;
    let cap = state(|s| s.limits.max_read_bytes).unwrap_or(64 * 1024).min(max.max(0) as u64);
    let f = std::fs::File::open(&path).map_err(|e| format!("read_text({}): {e}", path.display()))?;
    let mut buf = Vec::new();
    f.take(cap).read_to_end(&mut buf).map_err(|e| format!("read_text({}): {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Recursive name search under `dir` (glob on the name, case-insensitive).
fn search(dir: &str, glob: &str) -> ApiResult<Array> {
    read_gate("search")?;
    let root = abs(dir)?;
    let matcher = globset::GlobBuilder::new(glob)
        .case_insensitive(true)
        .literal_separator(false)
        .build()
        .map_err(|e| format!("search: bad pattern “{glob}”: {e}"))?
        .compile_matcher();
    let cap = state(|s| s.limits.max_list_entries).unwrap_or(1000);
    let opts = soyuz_fs::ListOptions { include_hidden: true, ..Default::default() };
    let mut out = Array::new();
    let mut stack = vec![(root, 0usize)];
    while let Some((d, depth)) = stack.pop() {
        if deadline_passed() {
            return fail("search(): time limit");
        }
        let Ok(entries) = soyuz_fs::read_dir(&d, &opts) else { continue };
        for e in entries {
            if matcher.is_match(&e.name) {
                out.push(Dynamic::from_map(entry_map(&e)));
                if out.len() >= cap {
                    return Ok(out);
                }
            }
            if e.kind == EntryKind::Dir && depth < 32 {
                stack.push((e.path.clone(), depth + 1));
            }
        }
    }
    Ok(out)
}

fn push_job(kind: JobKind, summary: String) -> ApiResult<()> {
    require("write", Mode::Full)?;
    let full = state(|s| {
        if s.out.jobs.len() >= s.limits.max_jobs {
            return true;
        }
        s.out.jobs.push(PlannedJob { kind, summary });
        false
    })
    .unwrap_or(true);
    if full { fail("too many planned changes in one call") } else { Ok(()) }
}

fn quoted(p: &Path) -> String {
    format!("\u{201c}{}\u{201d}", soyuz_core::entry::display_name(p))
}

fn items(paths: &[PathBuf]) -> String {
    match paths {
        [one] => quoted(one),
        many => format!("{} items", many.len()),
    }
}

fn plan_transfer(src: Dynamic, dest: &str, mv: bool) -> ApiResult<()> {
    require(if mv { "move_to" } else { "copy" }, Mode::Full)?;
    let sources = paths_of(src)?;
    if sources.is_empty() {
        return Ok(());
    }
    let dest_dir = abs(dest)?;
    let summary = format!("{} {} to {}", if mv { "Move" } else { "Copy" }, items(&sources), quoted(&dest_dir));
    let kind = if mv {
        JobKind::Move { sources, dest_dir, rename_to: None }
    } else {
        JobKind::Copy { sources, dest_dir, rename_to: None }
    };
    push_job(kind, summary)
}

fn plan_rename(path: &str, new_name: &str) -> ApiResult<()> {
    require("rename", Mode::Full)?;
    let from = abs(path)?;
    if let Err(e) = soyuz_core::naming::validate_name(new_name) {
        return fail(format!("rename: “{new_name}” is not a valid name: {e}"));
    }
    let Some(parent) = from.parent() else { return fail("rename: no parent folder") };
    let to = parent.join(new_name);
    if to == from {
        return Ok(());
    }
    let summary = format!("Rename {} to {}", quoted(&from), quoted(&to));
    push_job(JobKind::Rename { pairs: vec![(from, to)] }, summary)
}

fn plan_trash(paths: Dynamic) -> ApiResult<()> {
    require("trash", Mode::Full)?;
    let paths = paths_of(paths)?;
    if paths.is_empty() {
        return Ok(());
    }
    let summary = format!("Move {} to trash", items(&paths));
    push_job(JobKind::Delete { paths, mode: DeleteMode::Trash }, summary)
}

fn plan_mkdir(path: &str) -> ApiResult<()> {
    require("mkdir", Mode::Full)?;
    let path = abs(path)?;
    let summary = format!("New folder {}", quoted(&path));
    push_job(JobKind::Create { path, kind: CreateKind::Dir }, summary)
}

// --- media ---

static MEDIA_FFMPEG: AtomicBool = AtomicBool::new(true);
static MEDIA_TOOLS: AtomicBool = AtomicBool::new(true);

/// `[media] ffmpeg` / `[media] tools` for the media functions (the app sets them from its
/// config; both default to on).
pub fn set_media_access(ffmpeg: bool, tools: bool) {
    MEDIA_FFMPEG.store(ffmpeg, Ordering::Relaxed);
    MEDIA_TOOLS.store(tools, Ordering::Relaxed);
}

/// The libraries this process loaded (scripts never load them), or why not.
fn media_runtime(what: &str) -> ApiResult<std::sync::Arc<Runtime>> {
    if !MEDIA_FFMPEG.load(Ordering::Relaxed) {
        return fail(format!("{what}(): the FFmpeg libraries are turned off ([media] ffmpeg = false)"));
    }
    match Runtime::loaded() {
        Some(rt) => Ok(rt),
        None => fail(format!("{what}(): FFmpeg libraries not loaded")),
    }
}

/// `media_info(path)`: container, duration, size, streams, tags and chapters (`()` for
/// unknown numbers).
fn media_info(p: &str) -> ApiResult<Map> {
    require("media_info", Mode::Full)?;
    read_gate("media_info")?;
    let path = abs(p)?;
    let rt = media_runtime("media_info")?;
    let info = soyuz_media::probe(&rt, &soyuz_media::Source::File(path.clone()))
        .map_err(|e| format!("media_info({}): {e}", path.display()))?;
    let opt_f = |v: Option<f64>| v.map(Dynamic::from_float).unwrap_or(Dynamic::UNIT);
    let int = |v: u64| Dynamic::from_int(v.min(i64::MAX as u64) as i64);
    let pairs = |tags: &[(String, String)]| {
        let mut m = Map::new();
        for (k, v) in tags.iter().take(64) {
            m.insert(k.as_str().into(), v.chars().take(500).collect::<String>().into());
        }
        m
    };
    let streams: Array = info
        .streams
        .iter()
        .map(|s| {
            let mut m = Map::new();
            m.insert("index".into(), int(s.index as u64));
            m.insert("kind".into(), s.kind.map(|k| k.to_string()).unwrap_or_else(|| "other".into()).into());
            m.insert("codec".into(), s.codec.clone().into());
            m.insert("language".into(), s.language.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT));
            match s.kind {
                Some(StreamKind::Video) => {
                    m.insert("width".into(), int(s.width.into()));
                    m.insert("height".into(), int(s.height.into()));
                    m.insert("rotation".into(), Dynamic::from_int(s.rotation.into()));
                    m.insert("fps".into(), opt_f(s.fps));
                    m.insert("cover_art".into(), s.attached_pic.into());
                }
                Some(StreamKind::Audio) => {
                    m.insert("sample_rate".into(), int(s.sample_rate.into()));
                    m.insert("channels".into(), int(s.channels.into()));
                }
                _ => {}
            }
            Dynamic::from_map(m)
        })
        .collect();
    let chapters: Array = info
        .chapters
        .iter()
        .take(200)
        .map(|c| {
            let mut m = Map::new();
            m.insert("start".into(), Dynamic::from_float(c.start));
            m.insert("end".into(), Dynamic::from_float(c.end));
            m.insert("title".into(), c.title.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT));
            Dynamic::from_map(m)
        })
        .collect();
    let mut m = Map::new();
    m.insert("container".into(), info.container.clone().into());
    m.insert("container_long".into(), info.container_long.clone().into());
    m.insert("duration".into(), opt_f(info.duration));
    m.insert("size".into(), int(info.size));
    m.insert("bit_rate".into(), info.bit_rate.map(int).unwrap_or(Dynamic::UNIT));
    m.insert("streams".into(), streams.into());
    m.insert("tags".into(), pairs(&info.tags).into());
    m.insert("chapters".into(), chapters.into());
    Ok(m)
}

/// A Rhai number (float or integer) → seconds.
fn secs_of(what: &str, name: &str, v: &Dynamic) -> ApiResult<f64> {
    let s = v.as_float().ok().or_else(|| v.as_int().ok().map(|i| i as f64));
    match s {
        Some(s) if s.is_finite() && s >= 0.0 => Ok(s),
        _ => fail(format!("{what}(): {name} must be a number of seconds ≥ 0")),
    }
}

/// Plan one media job per source; the output is named beside it (the dialog's naming).
/// With reads allowed, the dry run checks the operation against the file and gives the
/// exact extension.
fn plan_media(what: &str, src: Dynamic, op: MediaOp, verb: &str) -> ApiResult<()> {
    require(what, Mode::Full)?;
    if !MEDIA_TOOLS.load(Ordering::Relaxed) {
        return fail(format!("{what}(): the media tools are turned off ([media] tools = false)"));
    }
    let rt = media_runtime(what)?;
    if let MediaOp::Convert { preset, .. } = &op {
        let available = soyuz_media::presets(&rt);
        if !available.iter().any(|p| p.id == *preset) {
            let names: Vec<&str> = available.iter().map(|p| p.id.name()).collect();
            return fail(format!(
                "{what}(): “{}” cannot be encoded by this FFmpeg build (available: {})",
                preset.name(),
                names.join(", ")
            ));
        }
    }
    let read = state(|s| s.limits.allow_read).unwrap_or(false);
    for src in paths_of(src)? {
        let ext = if read {
            soyuz_media::plan(&rt, &src, &op).map_err(|e| format!("{what}({}): {e}", src.display()))?.ext
        } else {
            String::new()
        };
        let ext = if ext.is_empty() { soyuz_jobs::kinds::media::expected_ext(&src, &op) } else { ext };
        let dest = soyuz_jobs::kinds::media::default_dest(&src, &ext);
        let summary = format!("{verb} {} to {}", quoted(&src), quoted(&dest));
        push_job(JobKind::Media { src, dest, op: op.clone() }, summary)?;
    }
    Ok(())
}

fn plan_convert(src: Dynamic, preset: &str) -> ApiResult<()> {
    let Some(preset) = PresetId::parse(preset.trim()) else {
        return fail(format!("convert(): unknown preset “{preset}” (e.g. mkv-copy, mp4-copy, webm-vp9, audio-flac)"));
    };
    plan_media("convert", src, MediaOp::Convert { preset, video_stream: None, audio_stream: None }, "Convert")
}

fn plan_extract_audio(src: Dynamic, codec: &str) -> ApiResult<()> {
    let codec = match codec.trim().to_ascii_lowercase().as_str() {
        "" | "copy" => AudioCodecChoice::Copy,
        "flac" => AudioCodecChoice::Flac,
        "aac" | "m4a" => AudioCodecChoice::Aac,
        "opus" => AudioCodecChoice::Opus,
        "wav" => AudioCodecChoice::Wav,
        other => return fail(format!("extract_audio(): unknown codec “{other}” (copy, flac, aac, opus, wav)")),
    };
    plan_media("extract_audio", src, MediaOp::ExtractAudio { codec }, "Extract the audio of")
}

fn plan_trim(src: Dynamic, start: Dynamic, end: Dynamic) -> ApiResult<()> {
    let start = secs_of("trim", "start", &start)?;
    let end = secs_of("trim", "end", &end)?;
    if end <= start {
        return fail("trim(): end must be after start");
    }
    plan_media("trim", src, MediaOp::Trim { start, end, stream_copy: true }, "Trim")
}

fn plan_save_frame(src: Dynamic, secs: Dynamic, format: &str) -> ApiResult<()> {
    let at = secs_of("save_frame", "secs", &secs)?;
    let format = match format.trim().to_ascii_lowercase().as_str() {
        "" | "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        other => return fail(format!("save_frame(): unknown format “{other}” (png, jpeg)")),
    };
    plan_media("save_frame", src, MediaOp::Frame { at, format, max_px: None }, "Save a frame of")
}

fn ui_op(what: &str, op: ApiResult<UiOp>) -> ApiResult<()> {
    require(what, Mode::Full)?;
    let op = op?;
    state(|s| {
        if s.out.ui.len() < 1000 {
            s.out.ui.push(op)
        }
    });
    Ok(())
}

// --- registration ---

fn str_field(m: &Map, k: &str) -> String {
    m.get(k).filter(|v| !v.is_unit()).map(|v| v.to_string()).unwrap_or_default()
}

fn bool_field(m: &Map, k: &str, default: bool) -> bool {
    m.get(k).and_then(|v| v.as_bool().ok()).unwrap_or(default)
}

fn list_field(m: &Map, k: &str) -> Vec<String> {
    match m.get(k) {
        Some(v) if v.is_array() => v.clone().cast::<Array>().iter().map(|x| x.to_string()).collect(),
        Some(v) if v.is_string() => {
            v.to_string().split([',', ';', ' ']).filter(|s| !s.is_empty()).map(str::to_string).collect()
        }
        _ => Vec::new(),
    }
}

fn fn_field(m: &Map, k: &str) -> ApiResult<Option<FnPtr>> {
    match m.get(k) {
        None => Ok(None),
        Some(v) if v.is_unit() => Ok(None),
        Some(v) if v.is_fnptr() => Ok(v.clone().try_cast::<FnPtr>()),
        Some(v) if v.is_string() => {
            Ok(Some(FnPtr::new(v.clone().cast::<ImmutableString>()).map_err(|e| format!("“{k}”: {e}"))?))
        }
        Some(_) => fail(format!("“{k}” must be a function (|ctx| … or Fn(\"name\"))")),
    }
}

/// `"Copy names"` → `"copy-names"`.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

fn id_of(m: &Map, title: &str) -> ApiResult<String> {
    let id = str_field(m, "id");
    let id = if id.trim().is_empty() { slug(title) } else { slug(&id) };
    if id.is_empty() {
        return fail("needs an “id” or a “title”");
    }
    Ok(id)
}

fn exts(m: &Map) -> Vec<String> {
    list_field(m, "extensions").into_iter().map(|e| e.trim_start_matches('.').to_ascii_lowercase()).collect()
}

fn register_action(m: Map, menu_default: bool) -> ApiResult<()> {
    require(if menu_default { "register_menu_item" } else { "register_action" }, Mode::Load)?;
    let title = str_field(&m, "title");
    if title.trim().is_empty() {
        return fail("register_action: “title” is required");
    }
    let Some(run) = fn_field(&m, "run")? else {
        return fail("register_action: “run” (a function taking the context) is required");
    };
    let mut contexts = Vec::new();
    for c in list_field(&m, "contexts") {
        contexts.push(match c.to_ascii_lowercase().as_str() {
            "file" | "files" => MenuContext::File,
            "dir" | "dirs" | "folder" | "folders" => MenuContext::Dir,
            "background" => MenuContext::Background,
            other => return fail(format!("unknown context “{other}” (file, dir, background)")),
        });
    }
    let def = ActionDef {
        id: id_of(&m, &title)?,
        title,
        description: str_field(&m, "description"),
        hotkey: str_field(&m, "hotkey"),
        menu: bool_field(&m, "menu", menu_default),
        palette: bool_field(&m, "palette", true),
        contexts,
        extensions: exts(&m),
        when: fn_field(&m, "when")?,
        run,
    };
    state(|s| {
        if s.reg.actions.iter().any(|a| a.id == def.id) {
            Err(format!("duplicate action id “{}”", def.id))
        } else {
            s.reg.actions.push(def);
            Ok(())
        }
    })
    .unwrap_or(Ok(()))
    .map_err(Into::into)
}

fn register_column(m: Map) -> ApiResult<()> {
    require("register_column", Mode::Load)?;
    let title = str_field(&m, "title");
    if title.trim().is_empty() {
        return fail("register_column: “title” is required");
    }
    let Some(value) = fn_field(&m, "value")? else {
        return fail("register_column: “value” (a function taking an entry) is required");
    };
    let width = m.get("width").and_then(|w| w.as_int().ok()).unwrap_or(110).clamp(40, 600) as f32;
    let def = ColumnDef { id: id_of(&m, &title)?, title, width, extensions: exts(&m), value };
    state(|s| s.reg.columns.push(def));
    Ok(())
}

fn register_previewer(m: Map) -> ApiResult<()> {
    require("register_previewer", Mode::Load)?;
    let title = str_field(&m, "title");
    let Some(preview) = fn_field(&m, "preview")? else {
        return fail("register_previewer: “preview” (a function taking a path) is required");
    };
    let extensions = exts(&m);
    if extensions.is_empty() {
        return fail("register_previewer: “extensions” is required (e.g. [\"csv\"])");
    }
    let title = if title.trim().is_empty() { extensions.join(", ") } else { title };
    let def = PreviewerDef { id: id_of(&m, &title)?, title, extensions, preview };
    state(|s| s.reg.previewers.push(def));
    Ok(())
}
