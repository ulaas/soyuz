//! History on disk: `<dir>/history/YYYY-MM-DD.jsonl` (UTC days), one coarse point per line,
//! each line carrying its format version. A torn or unparseable line is skipped, never the file.
//! Everything here blocks: sampler thread or a `Loader` only.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};

use crate::history::{AggPoint, Decimator, Metric, Pt};

const DAY_MS: i64 = 86_400_000;

fn history_dir(dir: &Path) -> PathBuf {
    dir.join("history")
}

fn day_name(t_ms: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(t_ms).map(|d| d.format("%Y-%m-%d").to_string())
}

fn day_start_ms(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0).map_or(0, |d| d.and_utc().timestamp_millis())
}

/// The history files with their day, oldest first. No directory: none.
fn day_files(dir: &Path) -> io::Result<Vec<(NaiveDate, PathBuf)>> {
    let read = match fs::read_dir(history_dir(dir)) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut files = Vec::new();
    for entry in read {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        let date =
            path.file_stem().and_then(|s| s.to_str()).and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
        if let Some(date) = date {
            files.push((date, path));
        }
    }
    files.sort();
    Ok(files)
}

/// Appends coarse points, one day file at a time.
pub struct Writer {
    dir: PathBuf,
    day: String,
    file: Option<File>,
}

impl Writer {
    /// `dir` is the monitor's data folder; `history/` below it is created when needed.
    pub fn new(dir: &Path) -> Self {
        Writer { dir: dir.to_path_buf(), day: String::new(), file: None }
    }

    /// Writes one point and flushes it.
    pub fn append(&mut self, point: &AggPoint) -> io::Result<()> {
        let day = day_name(point.t).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "time out of range"))?;
        if self.file.is_none() || self.day != day {
            let folder = history_dir(&self.dir);
            fs::create_dir_all(&folder)?;
            let path = folder.join(format!("{day}.jsonl"));
            let mut file = OpenOptions::new().create(true).read(true).append(true).open(path)?;
            finish_torn_line(&mut file)?;
            self.file = Some(file);
            self.day = day;
        }
        let mut line = serde_json::to_vec(point)?;
        line.push(b'\n');
        // One buffer in one write, so that a crash tears at most this line.
        if let Some(file) = &mut self.file {
            file.write_all(&line)?;
            file.flush()?;
        }
        Ok(())
    }
}

/// A crash can leave the last line unfinished; the next line must not join it.
fn finish_torn_line(file: &mut File) -> io::Result<()> {
    let len = file.seek(SeekFrom::End(0))?;
    if len == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    if last[0] != b'\n' {
        file.write_all(b"\n")?;
    }
    Ok(())
}

fn each_point(path: &Path, mut f: impl FnMut(AggPoint)) -> io::Result<()> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let reader = BufReader::with_capacity(64 * 1024, file);
    let mut last_t = i64::MIN;
    for line in reader.split(b'\n') {
        let line = line?;
        // Skips torn lines and whatever else does not parse.
        let Ok(point) = serde_json::from_slice::<AggPoint>(&line) else { continue };
        // Clock steps can leave a file out of order; time order is what matters.
        if point.t > last_t {
            last_t = point.t;
            f(point);
        }
    }
    Ok(())
}

/// Every point of the last `days` days, oldest first.
pub fn load(dir: &Path, days: u32, now_ms: i64) -> io::Result<Vec<AggPoint>> {
    load_since(dir, now_ms - i64::from(days) * DAY_MS)
}

/// Every point newer than `from_ms`, oldest first.
pub fn load_since(dir: &Path, from_ms: i64) -> io::Result<Vec<AggPoint>> {
    let mut out: Vec<AggPoint> = Vec::new();
    for (date, path) in day_files(dir)? {
        if day_start_ms(date) + DAY_MS <= from_ms {
            continue;
        }
        each_point(&path, |p| {
            if p.t >= from_ms && out.last().is_none_or(|l| p.t > l.t) {
                out.push(p);
            }
        })?;
    }
    Ok(out)
}

/// One metric between two times from the files, decimated while reading: older days never
/// enter memory whole. For a `Loader`.
pub fn load_series(dir: &Path, metric: &Metric, from_ms: i64, to_ms: i64, max_points: usize) -> io::Result<Vec<Pt>> {
    let mut d = Decimator::new(from_ms, to_ms, max_points);
    let mut last_t = i64::MIN;
    for (date, path) in day_files(dir)? {
        let start = day_start_ms(date);
        if start + DAY_MS <= from_ms || start > to_ms {
            continue;
        }
        each_point(&path, |p| {
            if p.t >= from_ms && p.t <= to_ms && p.t > last_t {
                last_t = p.t;
                d.add(p.t, p.value(metric).map(|v| (v.avg, v.max)), p.brk);
            }
        })?;
    }
    Ok(d.finish())
}

/// The start of the oldest day on disk.
pub fn oldest(dir: &Path) -> io::Result<Option<i64>> {
    Ok(day_files(dir)?.first().map(|(date, _)| day_start_ms(*date)))
}

/// Deletes day files older than `days`. Other files are left alone. Returns how many went.
pub fn prune(dir: &Path, days: u32, now_ms: i64) -> io::Result<usize> {
    let cutoff = now_ms - i64::from(days) * DAY_MS;
    let mut removed = 0;
    for (date, path) in day_files(dir)? {
        if day_start_ms(date) + DAY_MS <= cutoff {
            fs::remove_file(path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FORMAT_VERSION;
    use crate::history::Agg;

    /// 2026-11-06 00:00 UTC.
    const DAY0: i64 = 1_793_923_200_000;

    fn point(t: i64, cpu: f32) -> AggPoint {
        AggPoint { v: FORMAT_VERSION, t, cpu: Agg { avg: cpu, max: cpu + 1.0 }, ..Default::default() }
    }

    fn write_all(dir: &Path, points: &[AggPoint]) {
        let mut w = Writer::new(dir);
        for p in points {
            w.append(p).unwrap();
        }
    }

    #[test]
    fn missing_dir_is_empty_not_an_error() {
        let t = tempfile::tempdir().unwrap();
        let none = t.path().join("nope");
        assert!(load(&none, 7, DAY0).unwrap().is_empty());
        assert!(load_series(&none, &Metric::CpuTotal, 0, DAY0, 10).unwrap().is_empty());
        assert_eq!(oldest(&none).unwrap(), None);
        assert_eq!(prune(&none, 7, DAY0).unwrap(), 0);
    }

    #[test]
    fn points_split_by_utc_day_and_load_in_order() {
        let t = tempfile::tempdir().unwrap();
        write_all(t.path(), &[point(DAY0 - 60_000, 1.0), point(DAY0, 2.0), point(DAY0 + 60_000, 3.0)]);
        assert!(t.path().join("history/2026-11-05.jsonl").exists());
        assert!(t.path().join("history/2026-11-06.jsonl").exists());
        let back = load(t.path(), 7, DAY0 + 60_000).unwrap();
        assert_eq!(back.iter().map(|p| p.cpu.avg).collect::<Vec<_>>(), [1.0, 2.0, 3.0]);
        assert_eq!(back[0], point(DAY0 - 60_000, 1.0));
        assert_eq!(oldest(t.path()).unwrap(), Some(DAY0 - 86_400_000));
    }

    #[test]
    fn a_torn_last_line_is_skipped_and_the_next_append_stays_intact() {
        let t = tempfile::tempdir().unwrap();
        write_all(t.path(), &[point(DAY0, 1.0)]);
        let path = t.path().join("history/2026-11-06.jsonl");
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"v\":1,\"t\":17939").unwrap();
        drop(f);
        assert_eq!(load(t.path(), 7, DAY0).unwrap().len(), 1);
        write_all(t.path(), &[point(DAY0 + 60_000, 2.0)]);
        let back = load(t.path(), 7, DAY0 + 60_000).unwrap();
        assert_eq!(back.len(), 2, "the new line did not join the torn one");
        assert_eq!(back[1].cpu.avg, 2.0);
    }

    #[test]
    fn bad_lines_in_the_middle_do_not_lose_the_file() {
        let t = tempfile::tempdir().unwrap();
        let folder = t.path().join("history");
        fs::create_dir_all(&folder).unwrap();
        let good = serde_json::to_string(&point(DAY0, 1.0)).unwrap();
        let good2 = serde_json::to_string(&point(DAY0 + 60_000, 2.0)).unwrap();
        fs::write(folder.join("2026-11-06.jsonl"), format!("{good}\nnot json\n\u{0}\u{0}\n{good2}\n")).unwrap();
        fs::write(folder.join("notes.txt"), "keep").unwrap();
        fs::write(folder.join("whatever.jsonl"), "keep").unwrap();
        assert_eq!(load(t.path(), 7, DAY0 + 60_000).unwrap().len(), 2);
    }

    #[test]
    fn out_of_order_lines_are_dropped_on_load() {
        let t = tempfile::tempdir().unwrap();
        write_all(t.path(), &[point(DAY0 + 120_000, 1.0), point(DAY0 + 60_000, 2.0), point(DAY0 + 180_000, 3.0)]);
        let back = load(t.path(), 7, DAY0 + 180_000).unwrap();
        assert_eq!(back.iter().map(|p| p.cpu.avg).collect::<Vec<_>>(), [1.0, 3.0]);
    }

    #[test]
    fn load_respects_the_range() {
        let t = tempfile::tempdir().unwrap();
        write_all(t.path(), &[point(DAY0 - 3 * 86_400_000, 1.0), point(DAY0, 2.0)]);
        assert_eq!(load(t.path(), 1, DAY0 + 1000).unwrap().len(), 1);
        assert_eq!(load(t.path(), 7, DAY0 + 1000).unwrap().len(), 2);
    }

    #[test]
    fn load_series_decimates_while_reading_and_keeps_peaks() {
        let t = tempfile::tempdir().unwrap();
        let points: Vec<AggPoint> =
            (0..100).map(|i| point(DAY0 + i * 60_000, if i == 42 { 90.0 } else { 10.0 })).collect();
        write_all(t.path(), &points);
        let s = load_series(t.path(), &Metric::CpuTotal, DAY0, DAY0 + 99 * 60_000, 10).unwrap();
        assert!(s.len() <= 12);
        assert!(s.iter().any(|p| p.max == 91.0));
        let inner = load_series(t.path(), &Metric::CpuTotal, DAY0 + 60_000, DAY0 + 120_000, 0).unwrap();
        assert_eq!(inner.len(), 2);
    }

    #[test]
    fn saved_breaks_come_back_as_breaks() {
        let t = tempfile::tempdir().unwrap();
        let mut b = point(DAY0 + 600_000, 2.0);
        b.brk = true;
        write_all(t.path(), &[point(DAY0, 1.0), b]);
        let s = load_series(t.path(), &Metric::CpuTotal, DAY0, DAY0 + 600_000, 0).unwrap();
        assert_eq!(s.len(), 3);
        assert!(s[1].is_break());
    }

    #[test]
    fn prune_removes_old_days_only() {
        let t = tempfile::tempdir().unwrap();
        write_all(t.path(), &[point(DAY0 - 10 * 86_400_000, 1.0), point(DAY0, 2.0)]);
        let folder = t.path().join("history");
        fs::write(folder.join("notes.txt"), "keep").unwrap();
        assert_eq!(prune(t.path(), 7, DAY0).unwrap(), 1);
        assert_eq!(load(t.path(), 30, DAY0).unwrap().len(), 1);
        assert!(folder.join("notes.txt").exists());
    }
}
