//! Saved snapshots: `<dir>/snapshots/<id>.json`, with a sidecar `<id>.meta.json` so that listing
//! does not parse big files. Command lines are masked or dropped per `SnapshotCommands` when a
//! snapshot is captured and when processes are exported; a process's environment is never part
//! of a snapshot. Everything here blocks: use a `Loader`.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use soyuz_core::config::monitor::{CpuScale, ProcColumn, SnapshotCommands};
use soyuz_fs::sysmon::{Connection, ProcPriority, SystemInfo};

use crate::FORMAT_VERSION;
use crate::frame::{Frame, ProcRow, state_id};
use crate::redact;
use crate::sanitize;
use crate::table::{CellValue, cell};

/// What a list of snapshots shows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapshotMeta {
    /// Format version on disk.
    pub v: u32,
    /// The file stem, from the time taken.
    pub id: String,
    pub label: String,
    pub taken_ms: i64,
    /// Taken automatically (when an alert fired): may be removed to stay within the limit.
    pub auto: bool,
    pub hostname: String,
    pub procs: u32,
    pub cpu_percent: f32,
    pub mem_percent: f32,
}

/// One saved moment.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    /// Format version on disk.
    pub v: u32,
    pub meta: SnapshotMeta,
    pub info: SystemInfo,
    pub frame: Frame,
    pub connections: Vec<Connection>,
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

fn snapshots_dir(dir: &Path) -> PathBuf {
    dir.join("snapshots")
}

/// Ids name files: only what [`Snapshot::capture`] makes may reach a path.
fn checked(id: &str) -> io::Result<&str> {
    if !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        Ok(id)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "not a snapshot id"))
    }
}

fn full_path(dir: &Path, id: &str) -> io::Result<PathBuf> {
    Ok(snapshots_dir(dir).join(format!("{}.json", checked(id)?)))
}

fn meta_path(dir: &Path, id: &str) -> io::Result<PathBuf> {
    Ok(snapshots_dir(dir).join(format!("{}.meta.json", checked(id)?)))
}

fn id_for(t_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(t_ms)
        .map_or_else(|| format!("t{}", t_ms.max(0)), |d| d.format("%Y%m%d-%H%M%S-%3f").to_string())
}

impl Snapshot {
    /// Builds a snapshot of a frame, with command lines as `commands` says.
    pub fn capture(
        frame: &Frame,
        info: &SystemInfo,
        connections: Vec<Connection>,
        label: &str,
        auto: bool,
        commands: SnapshotCommands,
    ) -> Snapshot {
        let rows: Vec<ProcRow> = frame.procs.iter().map(|r| redact::apply_to_row(r, commands)).collect();
        let mut frame = frame.clone();
        frame.procs = Arc::from(rows);
        let meta = SnapshotMeta {
            v: FORMAT_VERSION,
            id: id_for(frame.at_ms),
            label: sanitize::sanitize_text(label).into_owned(),
            taken_ms: frame.at_ms,
            auto,
            hostname: info.hostname.clone(),
            procs: frame.counts.processes,
            cpu_percent: frame.cpu.percent,
            mem_percent: frame.memory.percent,
        };
        Snapshot { v: FORMAT_VERSION, meta, info: info.clone(), frame, connections }
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    soyuz_fs::write_atomic(path, bytes).map_err(other)
}

/// Saves a snapshot; a taken id gets a suffix. Returns its meta with the final id.
pub fn save(dir: &Path, snapshot: &Snapshot) -> io::Result<SnapshotMeta> {
    fs::create_dir_all(snapshots_dir(dir))?;
    let base = checked(&snapshot.meta.id)?.to_string();
    let mut id = base.clone();
    let mut n = 1;
    while full_path(dir, &id)?.exists() {
        n += 1;
        id = format!("{base}-{n}");
    }
    let meta = SnapshotMeta { id: id.clone(), v: FORMAT_VERSION, ..snapshot.meta.clone() };
    let rest = snapshot.body();
    let body = serde_json::to_vec(&SnapshotRef { v: FORMAT_VERSION, meta: &meta, rest: &rest }).map_err(other)?;
    write_atomic(&full_path(dir, &id)?, &body)?;
    write_atomic(&meta_path(dir, &id)?, &serde_json::to_vec(&meta).map_err(other)?)?;
    Ok(meta)
}

/// Serializes a snapshot with another meta, without copying its big frame.
#[derive(Serialize)]
struct SnapshotRef<'a> {
    v: u32,
    meta: &'a SnapshotMeta,
    #[serde(flatten)]
    rest: &'a SnapshotBody<'a>,
}

#[derive(Serialize)]
struct SnapshotBody<'a> {
    info: &'a SystemInfo,
    frame: &'a Frame,
    connections: &'a [Connection],
}

impl Snapshot {
    fn body(&self) -> SnapshotBody<'_> {
        SnapshotBody { info: &self.info, frame: &self.frame, connections: &self.connections }
    }
}

/// All snapshots, newest first. Unreadable entries are skipped; a snapshot whose sidecar was
/// lost (a crash between the two writes) gets it back.
pub fn list(dir: &Path) -> io::Result<Vec<SnapshotMeta>> {
    let read = match fs::read_dir(snapshots_dir(dir)) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut metas = Vec::new();
    let mut bodies = Vec::new();
    for entry in read {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if let Some(id) = name.strip_suffix(".meta.json") {
            if checked(id).is_err() || !full_path(dir, id)?.exists() {
                continue;
            }
            let parsed =
                fs::read(&path).map_err(other).and_then(|b| serde_json::from_slice::<SnapshotMeta>(&b).map_err(other));
            if let Ok(meta) = parsed {
                metas.push(SnapshotMeta { id: id.to_string(), ..meta });
            }
        } else if let Some(id) = name.strip_suffix(".json") {
            bodies.push(id.to_string());
        }
    }
    for id in bodies {
        if checked(&id).is_ok()
            && !meta_path(dir, &id)?.exists()
            && let Ok(s) = load(dir, &id)
        {
            let _ = write_atomic(&meta_path(dir, &id)?, &serde_json::to_vec(&s.meta).map_err(other)?);
            metas.push(s.meta);
        }
    }
    metas.sort_by(|a, b| b.taken_ms.cmp(&a.taken_ms).then_with(|| b.id.cmp(&a.id)));
    Ok(metas)
}

/// Reads a snapshot. The sidecar's label wins; texts are cleaned again, since a file can come
/// from anywhere.
pub fn load(dir: &Path, id: &str) -> io::Result<Snapshot> {
    let bytes = fs::read(full_path(dir, id)?)?;
    let mut s: Snapshot = serde_json::from_slice(&bytes).map_err(other)?;
    s.meta.id = id.to_string();
    if let Ok(side) = fs::read(meta_path(dir, id)?)
        && let Ok(meta) = serde_json::from_slice::<SnapshotMeta>(&side)
    {
        s.meta.label = meta.label;
    }
    s.meta.label = sanitize::sanitize_text(&s.meta.label).into_owned();
    s.meta.hostname = sanitize::sanitize_text(&s.meta.hostname).into_owned();
    let rows: Vec<ProcRow> = s
        .frame
        .procs
        .iter()
        .map(|r| {
            let mut r = r.clone();
            sanitize::sanitize_raw(&mut r.raw);
            r
        })
        .collect();
    s.frame.procs = Arc::from(rows);
    sanitize::sanitize_connections(&mut s.connections);
    Ok(s)
}

/// Changes a snapshot's label. Only the sidecar is rewritten, not the big file.
pub fn rename(dir: &Path, id: &str, label: &str) -> io::Result<()> {
    let path = meta_path(dir, id)?;
    let mut meta: SnapshotMeta = serde_json::from_slice(&fs::read(&path)?).map_err(other)?;
    meta.label = sanitize::sanitize_text(label).into_owned();
    write_atomic(&path, &serde_json::to_vec(&meta).map_err(other)?)
}

/// Deletes a snapshot. Deleting one that is already gone is fine.
pub fn delete(dir: &Path, id: &str) -> io::Result<()> {
    for path in [full_path(dir, id)?, meta_path(dir, id)?] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Removes the oldest automatic snapshots until at most `keep` snapshots remain in all. Manual
/// ones are never removed. Returns how many went.
pub fn prune_auto(dir: &Path, keep: u32) -> io::Result<usize> {
    let all = list(dir)?;
    let mut excess = all.len().saturating_sub(keep as usize);
    let mut removed = 0;
    for meta in all.iter().rev() {
        if excess == 0 {
            break;
        }
        if meta.auto {
            delete(dir, &meta.id)?;
            excess -= 1;
            removed += 1;
        }
    }
    Ok(removed)
}

/// The whole snapshot as readable JSON (already masked at capture).
pub fn export_json(snapshot: &Snapshot, writer: &mut dyn Write) -> io::Result<()> {
    serde_json::to_writer_pretty(writer, snapshot).map_err(other)
}

/// Options of [`export_procs_csv`].
#[derive(Clone, Copy, Debug)]
pub struct CsvOpts {
    pub ncpu: u32,
    pub scale: CpuScale,
    pub now_ms: i64,
    /// Applied to the Command column.
    pub commands: SnapshotCommands,
}

fn priority_id(p: ProcPriority) -> &'static str {
    match p {
        ProcPriority::Idle => "idle",
        ProcPriority::BelowNormal => "below_normal",
        ProcPriority::Normal => "normal",
        ProcPriority::AboveNormal => "above_normal",
        ProcPriority::High => "high",
        ProcPriority::Realtime => "realtime",
    }
}

fn csv_value(v: CellValue) -> (String, bool) {
    match v {
        CellValue::Text(s) => (s, true),
        CellValue::Count(n) => (n.to_string(), false),
        CellValue::Bytes(n) => (n.to_string(), false),
        CellValue::Rate(r) => (format!("{r:.0}"), false),
        CellValue::Percent(p) => (format!("{p:.1}"), false),
        CellValue::Millis(n) => (n.to_string(), false),
        CellValue::Seconds(n) => (n.to_string(), false),
        CellValue::UnixMs(t) => (
            DateTime::<Utc>::from_timestamp_millis(t)
                .map_or_else(String::new, |d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
            false,
        ),
        CellValue::State(s) => (state_id(s).to_string(), false),
        CellValue::Priority(p) => (priority_id(p).to_string(), false),
        CellValue::Bool(b) => (b.to_string(), false),
        CellValue::Int(n) => (n.to_string(), false),
        CellValue::Missing => (String::new(), false),
    }
}

/// A cell a spreadsheet would run as a formula gets a leading `'`: one starting with `= + - @`,
/// also behind leading whitespace, or with a tab or line break. Plain numbers (`-5`) stay numbers.
fn formula_guard(s: &str) -> bool {
    let trimmed = s.trim_start();
    if s.starts_with(['\t', '\r']) {
        return true;
    }
    trimmed.starts_with(['=', '+', '-', '@']) && trimmed.parse::<f64>().is_err()
}

fn write_cell(out: &mut String, text: &str, guard: bool) {
    let guarded = guard && formula_guard(text);
    if guarded || text.contains([',', '"', '\n', '\r']) {
        out.push('"');
        if guarded {
            out.push('\'');
        }
        for c in text.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
    } else {
        out.push_str(text);
    }
}

/// Writes processes as RFC 4180 CSV (CRLF line ends, header of column ids), raw numbers, no units.
pub fn export_procs_csv(
    rows: &[ProcRow],
    columns: &[ProcColumn],
    opts: CsvOpts,
    writer: &mut dyn Write,
) -> io::Result<()> {
    let mut line = String::with_capacity(512);
    for (i, col) in columns.iter().enumerate() {
        if i > 0 {
            line.push(',');
        }
        line.push_str(col.id());
    }
    line.push_str("\r\n");
    writer.write_all(line.as_bytes())?;
    for row in rows {
        line.clear();
        let masked;
        let row = if columns.contains(&ProcColumn::Command) && opts.commands != SnapshotCommands::Full {
            masked = redact::apply_to_row(row, opts.commands);
            &masked
        } else {
            row
        };
        for (i, col) in columns.iter().enumerate() {
            if i > 0 {
                line.push(',');
            }
            let (text, is_text) = csv_value(cell(*col, row, opts.now_ms, opts.ncpu, opts.scale));
            write_cell(&mut line, &text, is_text);
        }
        line.push_str("\r\n");
        writer.write_all(line.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{frame_of, row};
    use soyuz_fs::sysmon::Proto;

    fn info() -> SystemInfo {
        SystemInfo { hostname: "host1".into(), logical_cores: 4, ..Default::default() }
    }

    fn frame(t: i64) -> Frame {
        let mut r = row(10, Some(1), "app", 12.5, 4096);
        r.raw.cmdline = Some(Arc::from("app --password hunter2 file.txt"));
        let mut f = frame_of(vec![r, row(11, None, "b", 0.0, 1)]);
        f.at_ms = t;
        f.cpu.percent = 33.0;
        f.memory.percent = 44.0;
        f.counts.processes = 2;
        f
    }

    fn snap(t: i64, auto: bool) -> Snapshot {
        Snapshot::capture(&frame(t), &info(), Vec::new(), "my label", auto, SnapshotCommands::Redacted)
    }

    #[test]
    fn capture_masks_command_lines_per_mode() {
        let s = snap(1000, false);
        assert_eq!(s.frame.procs[0].raw.cmdline.as_deref(), Some("app --password *** file.txt"));
        assert_eq!(s.meta.procs, 2);
        assert_eq!(s.meta.cpu_percent, 33.0);
        assert_eq!(s.meta.hostname, "host1");
        let none = Snapshot::capture(&frame(1), &info(), Vec::new(), "", false, SnapshotCommands::None);
        assert_eq!(none.frame.procs[0].raw.cmdline, None);
        let full = Snapshot::capture(&frame(1), &info(), Vec::new(), "", false, SnapshotCommands::Full);
        assert!(full.frame.procs[0].raw.cmdline.as_deref().unwrap().contains("hunter2"));
    }

    #[test]
    fn round_trip_with_secrets_gone_from_the_file() {
        let t = tempfile::tempdir().unwrap();
        let mut s = snap(1_700_000_000_123, false);
        s.connections = vec![Connection {
            proto: Proto::Tcp,
            local_addr: "127.0.0.1".into(),
            local_port: 80,
            ..Default::default()
        }];
        let meta = save(t.path(), &s).unwrap();
        assert_eq!(meta.id, "20231114-221320-123");
        assert_eq!(meta.v, FORMAT_VERSION);
        let back = load(t.path(), &meta.id).unwrap();
        assert_eq!(back.meta.label, "my label");
        assert_eq!(back.v, FORMAT_VERSION);
        assert_eq!(back.info, info());
        assert_eq!(back.frame.procs, s.frame.procs);
        assert_eq!(back.connections, s.connections);
        let file = fs::read_to_string(t.path().join(format!("snapshots/{}.json", meta.id))).unwrap();
        assert!(!file.contains("hunter2"));
    }

    #[test]
    fn the_same_moment_twice_gets_two_files() {
        let t = tempfile::tempdir().unwrap();
        let a = save(t.path(), &snap(5000, false)).unwrap();
        let b = save(t.path(), &snap(5000, false)).unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(list(t.path()).unwrap().len(), 2);
        assert_eq!(load(t.path(), &b.id).unwrap().meta.id, b.id);
    }

    #[test]
    fn list_is_newest_first_and_tolerates_junk() {
        let t = tempfile::tempdir().unwrap();
        assert!(list(t.path()).unwrap().is_empty(), "no folder yet");
        save(t.path(), &snap(1000, false)).unwrap();
        let newest = save(t.path(), &snap(3000, true)).unwrap();
        save(t.path(), &snap(2000, false)).unwrap();
        let folder = t.path().join("snapshots");
        fs::write(folder.join("junk.txt"), "x").unwrap();
        fs::write(folder.join("broken.meta.json"), "not json").unwrap();
        fs::write(folder.join("orphan.meta.json"), "{}").unwrap();
        fs::write(folder.join("alsobroken.json"), "not json").unwrap();
        let l = list(t.path()).unwrap();
        assert_eq!(l.iter().map(|m| m.taken_ms).collect::<Vec<_>>(), [3000, 2000, 1000]);
        assert_eq!(l[0].id, newest.id);
        assert!(l[0].auto);
    }

    #[test]
    fn a_lost_sidecar_comes_back() {
        let t = tempfile::tempdir().unwrap();
        let m = save(t.path(), &snap(1000, false)).unwrap();
        let side = t.path().join(format!("snapshots/{}.meta.json", m.id));
        fs::remove_file(&side).unwrap();
        let l = list(t.path()).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].label, "my label");
        assert!(side.exists());
    }

    #[test]
    fn rename_and_delete() {
        let t = tempfile::tempdir().unwrap();
        let m = save(t.path(), &snap(1000, false)).unwrap();
        rename(t.path(), &m.id, "new\u{1B}[0m name").unwrap();
        assert_eq!(list(t.path()).unwrap()[0].label, "new name");
        assert_eq!(load(t.path(), &m.id).unwrap().meta.label, "new name");
        delete(t.path(), &m.id).unwrap();
        assert!(list(t.path()).unwrap().is_empty());
        delete(t.path(), &m.id).unwrap();
        assert!(load(t.path(), &m.id).is_err());
    }

    #[test]
    fn ids_cannot_leave_the_folder() {
        let t = tempfile::tempdir().unwrap();
        for bad in ["../evil", "a/b", r"a\b", "", "..", "x.json"] {
            assert!(load(t.path(), bad).is_err(), "{bad}");
            assert!(delete(t.path(), bad).is_err(), "{bad}");
            assert!(rename(t.path(), bad, "x").is_err(), "{bad}");
        }
    }

    #[test]
    fn prune_removes_only_automatic_ones_oldest_first() {
        let t = tempfile::tempdir().unwrap();
        save(t.path(), &snap(1000, false)).unwrap();
        save(t.path(), &snap(2000, true)).unwrap();
        save(t.path(), &snap(3000, true)).unwrap();
        save(t.path(), &snap(4000, false)).unwrap();
        save(t.path(), &snap(5000, true)).unwrap();
        assert_eq!(prune_auto(t.path(), 3).unwrap(), 2);
        let times: Vec<_> = list(t.path()).unwrap().iter().map(|m| m.taken_ms).collect();
        assert_eq!(times, [5000, 4000, 1000]);
        assert_eq!(prune_auto(t.path(), 0).unwrap(), 1, "manual ones stay, whatever the limit");
        assert_eq!(list(t.path()).unwrap().len(), 2);
    }

    #[test]
    fn loading_cleans_untrusted_texts() {
        let t = tempfile::tempdir().unwrap();
        let mut f = frame(1000);
        let mut r = row(1, None, "evil\u{1B}[2J", 0.0, 0);
        r.raw.user = Some(Arc::from("bo\u{202E}b"));
        f.procs = Arc::from(vec![r]);
        let s = Snapshot::capture(&f, &info(), Vec::new(), "x", false, SnapshotCommands::Full);
        let m = save(t.path(), &s).unwrap();
        let back = load(t.path(), &m.id).unwrap();
        assert_eq!(&*back.frame.procs[0].raw.name, "evil");
        assert_eq!(back.frame.procs[0].raw.user.as_deref(), Some("bob"));
    }

    #[test]
    fn export_json_parses_back() {
        let s = snap(1000, false);
        let mut out = Vec::new();
        export_json(&s, &mut out).unwrap();
        let back: Snapshot = serde_json::from_slice(&out).unwrap();
        assert_eq!(back.frame.procs, s.frame.procs);
        assert!(!String::from_utf8(out).unwrap().contains("hunter2"));
    }

    fn csv(rows: &[ProcRow], columns: &[ProcColumn], commands: SnapshotCommands) -> String {
        let mut out = Vec::new();
        let opts = CsvOpts { ncpu: 2, scale: CpuScale::Machine, now_ms: 0, commands };
        export_procs_csv(rows, columns, opts, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn csv_quotes_per_rfc_4180() {
        let r = row(1, None, "a,b \"c\"\nd", 50.0, 1024);
        let text = csv(&[r], &[ProcColumn::Name, ProcColumn::Cpu, ProcColumn::Memory], SnapshotCommands::Full);
        assert_eq!(text, "name,cpu,memory\r\n\"a,b \"\"c\"\"\nd\",25.0,1024\r\n");
    }

    #[test]
    fn csv_guards_formulas_but_not_numbers() {
        for (name, expected) in [
            ("=cmd()", "\"'=cmd()\""),
            ("+x", "\"'+x\""),
            ("-x", "\"'-x\""),
            ("@x", "\"'@x\""),
            ("  =x", "\"'  =x\""),
            ("\tx", "\"'\tx\""),
            ("\rx", "\"'\rx\""),
            ("-5", "-5"),
            ("+1.5", "+1.5"),
            ("plain", "plain"),
        ] {
            let r = row(1, None, name, 0.0, 0);
            assert_eq!(
                csv(&[r], &[ProcColumn::Name], SnapshotCommands::Full),
                format!("name\r\n{expected}\r\n"),
                "{name}"
            );
        }
        let mut r = row(1, None, "x", 0.0, 0);
        r.raw.nice = Some(-5);
        assert!(csv(&[r], &[ProcColumn::Nice], SnapshotCommands::Full).contains("\r\n-5\r\n"));
    }

    #[test]
    fn csv_applies_the_command_setting_and_leaves_missing_empty() {
        let mut r = row(1, None, "x", 0.0, 0);
        r.raw.cmdline = Some(Arc::from("x --token abc"));
        let cols = [ProcColumn::Command, ProcColumn::Gpu];
        assert!(csv(std::slice::from_ref(&r), &cols, SnapshotCommands::Redacted).contains("x --token ***,\r\n"));
        assert!(csv(std::slice::from_ref(&r), &cols, SnapshotCommands::None).ends_with("\r\n,\r\n"));
        assert!(csv(&[r], &cols, SnapshotCommands::Full).contains("--token abc"));
    }
}
