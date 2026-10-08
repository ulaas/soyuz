//! Pure listing maintenance: incremental sorted merge of streamed chunks, in-place
//! watcher patches, filtering and grouping into the visible order.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use soyuz_core::{Attributes, EntryInfo, GroupBy, NamePattern, SortSpec, compare_entries};

/// Merge `chunk` (any order) into the already sorted `entries`.
pub fn merge_sorted(entries: &mut Vec<EntryInfo>, mut chunk: Vec<EntryInfo>, spec: &SortSpec) {
    if chunk.is_empty() {
        return;
    }
    chunk.sort_by(|a, b| compare_entries(a, b, spec));
    if entries.is_empty() {
        *entries = chunk;
        return;
    }
    // Fast path: the whole chunk sorts after the existing entries.
    if let (Some(last), Some(first)) = (entries.last(), chunk.first())
        && compare_entries(last, first, spec) != Ordering::Greater
    {
        entries.extend(chunk);
        return;
    }
    let old = std::mem::take(entries);
    let mut out = Vec::with_capacity(old.len() + chunk.len());
    let mut a = old.into_iter().peekable();
    let mut b = chunk.into_iter().peekable();
    loop {
        match (a.peek(), b.peek()) {
            (Some(x), Some(y)) => {
                if compare_entries(x, y, spec) != Ordering::Greater {
                    out.push(a.next().expect("peeked"));
                } else {
                    out.push(b.next().expect("peeked"));
                }
            }
            (Some(_), None) => out.extend(a.by_ref()),
            (None, Some(_)) => out.extend(b.by_ref()),
            (None, None) => break,
        }
    }
    *entries = out;
}

/// Apply a watcher patch: entries named in `removed` disappear, `updated` entries
/// replace existing ones with the same name (or are inserted) at their sorted position.
pub fn apply_patch(entries: &mut Vec<EntryInfo>, updated: Vec<EntryInfo>, removed: &[String], spec: &SortSpec) {
    let mut drop: HashSet<&str> = removed.iter().map(String::as_str).collect();
    for u in &updated {
        drop.insert(u.name.as_str());
    }
    entries.retain(|e| !drop.contains(e.name.as_str()));
    merge_sorted(entries, updated, spec);
}

/// One run of consecutive visible items sharing a group label.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupSpan {
    pub label: String,
    /// Index into the visible list where the group starts.
    pub start: usize,
    /// Items shown (0 when the group is collapsed).
    pub len: usize,
    /// Items in the group regardless of collapsing.
    pub count: usize,
}

/// Client-side filter applied on top of the listing.
pub struct ViewFilter<'a> {
    pub show_hidden: bool,
    pub show_system: bool,
    pub pattern: Option<&'a NamePattern>,
}

impl ViewFilter<'_> {
    pub fn accepts(&self, e: &EntryInfo) -> bool {
        if !self.show_hidden && e.is_hidden() {
            return false;
        }
        if !self.show_system && e.attrs.contains(Attributes::SYSTEM) && e.attrs.contains(Attributes::HIDDEN) {
            return false;
        }
        self.pattern.is_none_or(|p| p.is_match(&e.name))
    }
}

/// Indices of `entries` to show (in sorted order, regrouped when grouping) plus group spans.
pub fn compute_visible(
    entries: &[EntryInfo],
    filter: &ViewFilter<'_>,
    group: GroupBy,
    now: SystemTime,
) -> (Vec<usize>, Vec<GroupSpan>) {
    let mut vis: Vec<usize> = (0..entries.len()).filter(|&i| filter.accepts(&entries[i])).collect();
    if group == GroupBy::None {
        return (vis, Vec::new());
    }
    let keys: Vec<(u32, String)> = entries.iter().map(|e| group_key(e, group, now)).collect();
    // Stable: keeps the sort order inside each group.
    vis.sort_by(|&a, &b| keys[a].0.cmp(&keys[b].0).then_with(|| keys[a].1.cmp(&keys[b].1)));
    let mut spans: Vec<GroupSpan> = Vec::new();
    for (pos, &i) in vis.iter().enumerate() {
        let label = &keys[i].1;
        match spans.last_mut() {
            Some(s) if &s.label == label => {
                s.len += 1;
                s.count += 1;
            }
            _ => spans.push(GroupSpan { label: label.clone(), start: pos, len: 1, count: 1 }),
        }
    }
    (vis, spans)
}

/// `(order, label)` of an entry's group.
pub fn group_key(e: &EntryInfo, group: GroupBy, now: SystemTime) -> (u32, String) {
    match group {
        GroupBy::None => (0, String::new()),
        GroupBy::Kind => {
            if e.is_dir_like() {
                (0, "Folders".into())
            } else {
                (1, kind_label(e))
            }
        }
        GroupBy::Extension => match e.extension() {
            _ if e.is_dir_like() => (0, "Folders".into()),
            Some(x) => (1, format!(".{}", x.to_lowercase())),
            None => (2, "No extension".into()),
        },
        GroupBy::FirstLetter => {
            let c = e.name.chars().next().map(|c| c.to_uppercase().next().unwrap_or(c)).unwrap_or('#');
            if c.is_alphabetic() { (1, c.to_string()) } else { (0, "#".into()) }
        }
        GroupBy::Size => {
            if e.is_dir_like() {
                return (0, "Folders".into());
            }
            const MIB: u64 = 1024 * 1024;
            let s = e.size;
            match s {
                0 => (1, "Empty".into()),
                1..16_384 => (2, "Tiny (< 16 KiB)".into()),
                16_384..1_048_576 => (3, "Small (< 1 MiB)".into()),
                1_048_576..134_217_728 => (4, "Medium (< 128 MiB)".into()),
                _ if s < 1024 * MIB => (5, "Large (< 1 GiB)".into()),
                _ => (6, "Huge (≥ 1 GiB)".into()),
            }
        }
        GroupBy::Date => match e.modified {
            None => (9, "Unknown date".into()),
            Some(t) => date_bucket(t, now),
        },
        GroupBy::Tag => (0, "No tags".into()),
    }
}

fn date_bucket(t: SystemTime, now: SystemTime) -> (u32, String) {
    let age = now.duration_since(t).unwrap_or(Duration::ZERO);
    let day = 86_400;
    let secs = age.as_secs();
    if t > now {
        (0, "In the future".into())
    } else if secs < day {
        (1, "Today".into())
    } else if secs < 2 * day {
        (2, "Yesterday".into())
    } else if secs < 7 * day {
        (3, "Earlier this week".into())
    } else if secs < 31 * day {
        (4, "Earlier this month".into())
    } else if secs < 365 * day {
        (5, "Earlier this year".into())
    } else {
        (6, "A long time ago".into())
    }
}

/// Human type description ("Folder", "PNG image", "Rust source", "ZIP archive", ...).
pub fn kind_label(e: &EntryInfo) -> String {
    use soyuz_core::EntryKind;
    match e.kind {
        EntryKind::Dir => return "Folder".into(),
        EntryKind::Junction => return "Junction".into(),
        EntryKind::Symlink { target: soyuz_core::LinkTarget::Dir } => return "Folder link".into(),
        EntryKind::Symlink { target: soyuz_core::LinkTarget::Missing } => return "Broken link".into(),
        EntryKind::Device => return "Device".into(),
        EntryKind::Fifo => return "Pipe".into(),
        EntryKind::Socket => return "Socket".into(),
        _ => {}
    }
    let Some(ext) = e.extension() else { return "File".into() };
    let lower = ext.to_ascii_lowercase();
    let known = match lower.as_str() {
        "png" => "PNG image",
        "jpg" | "jpeg" => "JPEG image",
        "gif" => "GIF image",
        "webp" => "WebP image",
        "bmp" => "Bitmap image",
        "svg" => "SVG image",
        "ico" => "Icon",
        "tif" | "tiff" => "TIFF image",
        "heic" | "heif" => "HEIC image",
        "mp4" | "m4v" => "MP4 video",
        "mkv" => "Matroska video",
        "mov" => "QuickTime video",
        "avi" => "AVI video",
        "webm" => "WebM video",
        "mp3" => "MP3 audio",
        "flac" => "FLAC audio",
        "wav" => "WAV audio",
        "ogg" | "opus" => "Ogg audio",
        "m4a" | "aac" => "AAC audio",
        "zip" => "ZIP archive",
        "7z" => "7-Zip archive",
        "rar" => "RAR archive",
        "tar" => "Tar archive",
        "gz" | "tgz" => "Gzip archive",
        "xz" | "bz2" | "zst" => "Compressed file",
        "iso" => "Disc image",
        "pdf" => "PDF document",
        "doc" | "docx" => "Word document",
        "xls" | "xlsx" => "Excel workbook",
        "ppt" | "pptx" => "PowerPoint deck",
        "odt" => "Text document",
        "txt" => "Text file",
        "md" => "Markdown",
        "rtf" => "Rich text",
        "csv" => "CSV table",
        "json" => "JSON",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "xml" => "XML",
        "html" | "htm" => "HTML page",
        "css" => "Stylesheet",
        "rs" => "Rust source",
        "py" => "Python script",
        "js" | "mjs" | "cjs" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "c" | "h" => "C source",
        "cpp" | "cc" | "hpp" | "cxx" => "C++ source",
        "cs" => "C# source",
        "go" => "Go source",
        "java" => "Java source",
        "kt" => "Kotlin source",
        "swift" => "Swift source",
        "sh" | "bash" | "zsh" => "Shell script",
        "ps1" => "PowerShell script",
        "bat" | "cmd" => "Batch file",
        "exe" => "Application",
        "msi" => "Installer",
        "dll" | "so" | "dylib" => "Library",
        "lnk" => "Shortcut",
        "ttf" | "otf" | "woff" | "woff2" => "Font",
        "log" => "Log file",
        "ini" | "cfg" | "conf" => "Settings",
        "lock" => "Lock file",
        _ => "",
    };
    if known.is_empty() { format!("{} file", ext.to_uppercase()) } else { known.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::{Column, EntryKind, NamePatternSpec, PatternKind, SortDir, sort_entries};
    use std::path::PathBuf;

    fn file(name: &str, size: u64) -> EntryInfo {
        let mut e = EntryInfo::minimal(PathBuf::from("/d").join(name), EntryKind::File);
        e.size = size;
        e
    }

    fn dir(name: &str) -> EntryInfo {
        EntryInfo::minimal(PathBuf::from("/d").join(name), EntryKind::Dir)
    }

    fn names(v: &[EntryInfo]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn merge_equals_full_sort() {
        let spec = SortSpec::default();
        let all = vec![
            file("file10.txt", 1),
            file("file2.txt", 1),
            dir("zeta"),
            file("Alpha.md", 1),
            dir("beta"),
            file("file1.txt", 1),
            file("b.rs", 1),
        ];
        let mut merged = Vec::new();
        for chunk in all.chunks(2) {
            merge_sorted(&mut merged, chunk.to_vec(), &spec);
        }
        let mut full = all.clone();
        sort_entries(&mut full, &spec);
        assert_eq!(names(&merged), names(&full));
        assert_eq!(names(&merged), ["beta", "zeta", "Alpha.md", "b.rs", "file1.txt", "file2.txt", "file10.txt"]);
    }

    #[test]
    fn merge_with_size_desc() {
        let spec = SortSpec { column: Column::Size, dir: SortDir::Desc, ..SortSpec::default() };
        let mut v = Vec::new();
        merge_sorted(&mut v, vec![file("a", 5), file("b", 50)], &spec);
        merge_sorted(&mut v, vec![file("c", 10), dir("d")], &spec);
        assert_eq!(names(&v), ["d", "b", "c", "a"]);
    }

    #[test]
    fn patch_updates_in_place() {
        let spec = SortSpec::default();
        let mut v = Vec::new();
        merge_sorted(&mut v, vec![file("a", 1), file("b", 1), file("c", 1)], &spec);
        apply_patch(&mut v, vec![file("b", 99), file("aa", 1)], &["c".to_string()], &spec);
        assert_eq!(names(&v), ["a", "aa", "b"]);
        assert_eq!(v[2].size, 99);
        apply_patch(&mut v, vec![], &["missing".to_string()], &spec);
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn filtering() {
        let spec = SortSpec::default();
        let mut v = Vec::new();
        merge_sorted(&mut v, vec![file(".hidden", 1), file("readme.md", 1), file("main.rs", 1)], &spec);
        let now = SystemTime::now();
        let f = ViewFilter { show_hidden: false, show_system: false, pattern: None };
        let (vis, groups) = compute_visible(&v, &f, GroupBy::None, now);
        assert_eq!(vis.len(), 2);
        assert!(groups.is_empty());
        let pat = NamePatternSpec::new(PatternKind::Substring, "MAIN").compile().unwrap();
        let f = ViewFilter { show_hidden: true, show_system: false, pattern: Some(&pat) };
        let (vis, _) = compute_visible(&v, &f, GroupBy::None, now);
        assert_eq!(vis.iter().map(|&i| v[i].name.as_str()).collect::<Vec<_>>(), ["main.rs"]);
    }

    #[test]
    fn grouping_by_kind_keeps_sort_inside_groups() {
        let spec = SortSpec { folders_first: false, ..SortSpec::default() };
        let mut v = Vec::new();
        merge_sorted(&mut v, vec![file("b.png", 1), dir("a"), file("c.rs", 1), file("a.png", 1), dir("z")], &spec);
        let f = ViewFilter { show_hidden: true, show_system: false, pattern: None };
        let (vis, groups) = compute_visible(&v, &f, GroupBy::Kind, SystemTime::now());
        let ordered: Vec<&str> = vis.iter().map(|&i| v[i].name.as_str()).collect();
        assert_eq!(ordered, ["a", "z", "a.png", "b.png", "c.rs"]);
        let labels: Vec<(&str, usize, usize)> = groups.iter().map(|g| (g.label.as_str(), g.start, g.len)).collect();
        assert_eq!(labels, [("Folders", 0, 2), ("PNG image", 2, 2), ("Rust source", 4, 1)]);
    }

    #[test]
    fn date_buckets() {
        let now = SystemTime::now();
        let day = Duration::from_secs(86_400);
        assert_eq!(date_bucket(now - Duration::from_secs(60), now).1, "Today");
        assert_eq!(date_bucket(now - day - Duration::from_secs(60), now).1, "Yesterday");
        assert_eq!(date_bucket(now - day * 400, now).1, "A long time ago");
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_label(&file("x.PNG", 1)), "PNG image");
        assert_eq!(kind_label(&file("x.weird", 1)), "WEIRD file");
        assert_eq!(kind_label(&file("Makefile", 1)), "File");
        assert_eq!(kind_label(&dir("d")), "Folder");
    }
}
