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

/// What a pane shows when nothing is visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyKind {
    /// Something is visible (or there are group headers to draw): show the view.
    NotEmpty,
    /// The type-to-filter pattern hides every shown entry.
    NoMatches,
    /// Every entry is hidden and "show hidden" (Ctrl+H) would reveal at least one.
    OnlyHidden,
    /// Nothing to show: an empty folder, or one holding only protected operating-system
    /// files (`desktop.ini`, `Thumbs.db`, `$RECYCLE.BIN`… — hidden *and* system, kept
    /// out of sight by `show_system` like Explorer does) that Ctrl+H cannot reveal.
    Empty,
}

/// Hidden + system: a protected operating-system file (only `show_system` shows it).
fn is_protected(e: &EntryInfo) -> bool {
    e.attrs.contains(Attributes::HIDDEN) && e.attrs.contains(Attributes::SYSTEM)
}

/// Classify an empty visible list. `visible`/`groups` are the counts the view would draw.
pub fn classify_empty(
    entries: &[EntryInfo],
    visible: usize,
    groups: usize,
    filter_active: bool,
    show_hidden: bool,
    show_system: bool,
) -> EmptyKind {
    if visible > 0 || groups > 0 {
        return EmptyKind::NotEmpty;
    }
    // Entries the hidden / system rules let through (only the name filter can hide them).
    let shown = |e: &EntryInfo| (show_hidden || !e.is_hidden()) && (show_system || !is_protected(e));
    if filter_active && entries.iter().any(shown) {
        return EmptyKind::NoMatches;
    }
    let revealable = |e: &EntryInfo| e.is_hidden() && (show_system || !is_protected(e));
    if !show_hidden && !entries.is_empty() && entries.iter().all(EntryInfo::is_hidden) && entries.iter().any(revealable)
    {
        return EmptyKind::OnlyHidden;
    }
    EmptyKind::Empty
}

/// `(order, label)` of an entry's group.
pub fn group_key(e: &EntryInfo, group: GroupBy, now: SystemTime) -> (u32, String) {
    match group {
        GroupBy::None => (0, String::new()),
        GroupBy::Kind => {
            if e.is_dir_like() {
                (0, tr!("views-group-folders").into())
            } else {
                (1, kind_label(e))
            }
        }
        GroupBy::Extension => match e.extension() {
            _ if e.is_dir_like() => (0, tr!("views-group-folders").into()),
            Some(x) => (1, format!(".{}", x.to_lowercase())),
            None => (2, tr!("views-group-no-extension").into()),
        },
        GroupBy::FirstLetter => {
            let c = e.name.chars().next().map(|c| c.to_uppercase().next().unwrap_or(c)).unwrap_or('#');
            if c.is_alphabetic() { (1, c.to_string()) } else { (0, "#".into()) }
        }
        GroupBy::Size => {
            if e.is_dir_like() {
                return (0, tr!("views-group-folders").into());
            }
            const MIB: u64 = 1024 * 1024;
            let s = e.size;
            match s {
                0 => (1, tr!("views-group-size-empty").into()),
                1..16_384 => (2, tr!("views-group-size-tiny").into()),
                16_384..1_048_576 => (3, tr!("views-group-size-small").into()),
                1_048_576..134_217_728 => (4, tr!("views-group-size-medium").into()),
                _ if s < 1024 * MIB => (5, tr!("views-group-size-large").into()),
                _ => (6, tr!("views-group-size-huge").into()),
            }
        }
        GroupBy::Date => match e.modified {
            None => (9, tr!("views-group-date-unknown").into()),
            Some(t) => date_bucket(t, now),
        },
        GroupBy::Tag => (0, tr!("views-group-no-tags").into()),
    }
}

fn date_bucket(t: SystemTime, now: SystemTime) -> (u32, String) {
    let age = now.duration_since(t).unwrap_or(Duration::ZERO);
    let day = 86_400;
    let secs = age.as_secs();
    if t > now {
        (0, tr!("views-group-date-future").into())
    } else if secs < day {
        (1, tr!("views-group-date-today").into())
    } else if secs < 2 * day {
        (2, tr!("views-group-date-yesterday").into())
    } else if secs < 7 * day {
        (3, tr!("views-group-date-week").into())
    } else if secs < 31 * day {
        (4, tr!("views-group-date-month").into())
    } else if secs < 365 * day {
        (5, tr!("views-group-date-year").into())
    } else {
        (6, tr!("views-group-date-older").into())
    }
}

/// Human type description ("Folder", "PNG image", "Rust source", "ZIP archive", ...).
pub fn kind_label(e: &EntryInfo) -> String {
    use soyuz_core::EntryKind;
    match e.kind {
        EntryKind::Dir => return tr!("views-kind-folder").into(),
        EntryKind::Junction => return tr!("views-kind-junction").into(),
        EntryKind::Symlink { target: soyuz_core::LinkTarget::Dir } => return tr!("views-kind-folder-link").into(),
        EntryKind::Symlink { target: soyuz_core::LinkTarget::Missing } => return tr!("views-kind-broken-link").into(),
        EntryKind::Device => return tr!("views-kind-device").into(),
        EntryKind::Fifo => return tr!("views-kind-pipe").into(),
        EntryKind::Socket => return tr!("views-kind-socket").into(),
        _ => {}
    }
    let Some(ext) = e.extension() else { return tr!("views-kind-file").into() };
    let lower = ext.to_ascii_lowercase();
    let known = match lower.as_str() {
        "png" => tr!("views-kind-png-image"),
        "jpg" | "jpeg" => tr!("views-kind-jpeg-image"),
        "gif" => tr!("views-kind-gif-image"),
        "webp" => tr!("views-kind-webp-image"),
        "bmp" => tr!("views-kind-bitmap-image"),
        "svg" => tr!("views-kind-svg-image"),
        "ico" => tr!("views-kind-icon"),
        "tif" | "tiff" => tr!("views-kind-tiff-image"),
        "heic" | "heif" => tr!("views-kind-heic-image"),
        "mp4" | "m4v" => tr!("views-kind-mp4-video"),
        "mkv" => tr!("views-kind-matroska-video"),
        "mov" => tr!("views-kind-quicktime-video"),
        "avi" => tr!("views-kind-avi-video"),
        "webm" => tr!("views-kind-webm-video"),
        "mp3" => tr!("views-kind-mp3-audio"),
        "flac" => tr!("views-kind-flac-audio"),
        "wav" => tr!("views-kind-wav-audio"),
        "ogg" | "opus" => tr!("views-kind-ogg-audio"),
        "m4a" | "aac" => tr!("views-kind-aac-audio"),
        "zip" => tr!("views-kind-zip-archive"),
        "7z" => tr!("views-kind-7-zip-archive"),
        "rar" => tr!("views-kind-rar-archive"),
        "tar" => tr!("views-kind-tar-archive"),
        "gz" | "tgz" => tr!("views-kind-gzip-archive"),
        "xz" | "bz2" | "zst" => tr!("views-kind-compressed-file"),
        "iso" => tr!("views-kind-disc-image"),
        "pdf" => tr!("views-kind-pdf-document"),
        "doc" | "docx" => tr!("views-kind-word-document"),
        "xls" | "xlsx" => tr!("views-kind-excel-workbook"),
        "ppt" | "pptx" => tr!("views-kind-powerpoint-deck"),
        "odt" => tr!("views-kind-text-document"),
        "txt" => tr!("views-kind-text-file"),
        "md" => "Markdown",
        "rtf" => tr!("views-kind-rich-text"),
        "csv" => tr!("views-kind-csv-table"),
        "json" => "JSON",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "xml" => "XML",
        "html" | "htm" => tr!("views-kind-html-page"),
        "css" => tr!("views-kind-stylesheet"),
        "rs" => tr!("views-kind-rust-source"),
        "py" => tr!("views-kind-python-script"),
        "js" | "mjs" | "cjs" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "c" | "h" => tr!("views-kind-c-source"),
        "cpp" | "cc" | "hpp" | "cxx" => tr!("views-kind-cpp-source"),
        "cs" => tr!("views-kind-csharp-source"),
        "go" => tr!("views-kind-go-source"),
        "java" => tr!("views-kind-java-source"),
        "kt" => tr!("views-kind-kotlin-source"),
        "swift" => tr!("views-kind-swift-source"),
        "sh" | "bash" | "zsh" => tr!("views-kind-shell-script"),
        "ps1" => tr!("views-kind-powershell-script"),
        "bat" | "cmd" => tr!("views-kind-batch-file"),
        "exe" => tr!("views-kind-application"),
        "msi" => tr!("views-kind-installer"),
        "dll" | "so" | "dylib" => tr!("views-kind-library"),
        "lnk" => tr!("views-kind-shortcut"),
        "ttf" | "otf" | "woff" | "woff2" => tr!("views-kind-font"),
        "log" => tr!("views-kind-log-file"),
        "ini" | "cfg" | "conf" => tr!("views-kind-settings"),
        "lock" => tr!("views-kind-lock-file"),
        _ => "",
    };
    if known.is_empty() { tr!("views-kind-ext-file", ext = ext.to_uppercase()) } else { known.into() }
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

    fn with_attrs(mut e: EntryInfo, attrs: Attributes) -> EntryInfo {
        e.attrs = attrs;
        e
    }

    #[test]
    fn empty_state_classifier() {
        use EmptyKind::*;
        let plain = file("a.txt", 1);
        let dot = file(".env", 1);
        let hidden = with_attrs(file("h.txt", 1), Attributes::HIDDEN);
        let desktop_ini = with_attrs(file("desktop.ini", 1), Attributes::HIDDEN | Attributes::SYSTEM);
        let c =
            |e: &[EntryInfo], vis: usize, filter: bool, sh: bool, ss: bool| classify_empty(e, vis, 0, filter, sh, ss);

        // A truly empty folder is empty, whatever the flags.
        for (sh, ss) in [(false, false), (true, false), (false, true), (true, true)] {
            assert_eq!(c(&[], 0, false, sh, ss), Empty);
            assert_eq!(c(&[], 0, true, sh, ss), Empty);
        }
        // Something visible, or group headers to draw (all groups collapsed).
        assert_eq!(c(std::slice::from_ref(&plain), 1, false, false, false), NotEmpty);
        assert_eq!(classify_empty(std::slice::from_ref(&plain), 0, 1, false, false, false), NotEmpty);
        // Hidden files that Ctrl+H would reveal.
        assert_eq!(c(&[hidden.clone(), dot.clone()], 0, false, false, false), OnlyHidden);
        assert_eq!(c(&[hidden.clone(), desktop_ini.clone()], 0, false, false, false), OnlyHidden);
        // Only protected OS files: Ctrl+H cannot show them, so the folder reads as empty
        // (the reported bug: "Only hidden items" in a folder that looks empty).
        assert_eq!(c(std::slice::from_ref(&desktop_ini), 0, false, false, false), Empty);
        assert_eq!(c(std::slice::from_ref(&desktop_ini), 0, false, true, false), Empty);
        // ...unless system files are shown too.
        assert_eq!(c(std::slice::from_ref(&desktop_ini), 0, false, false, true), OnlyHidden);
        // Hidden items are already shown: never "only hidden".
        assert_eq!(c(std::slice::from_ref(&hidden), 0, false, true, false), Empty);
        // Entries that are not hidden can never make an "only hidden" state.
        assert_eq!(c(&[plain.clone(), hidden.clone()], 0, false, false, false), Empty);
        // The name filter hides shown entries: its own state.
        assert_eq!(c(std::slice::from_ref(&plain), 0, true, false, false), NoMatches);
        assert_eq!(c(std::slice::from_ref(&hidden), 0, true, true, false), NoMatches);
        // ...but a filter over hidden-only entries is still about hidden items.
        assert_eq!(c(&[hidden], 0, true, false, false), OnlyHidden);
        assert_eq!(c(&[desktop_ini], 0, true, false, false), Empty);
    }
}
