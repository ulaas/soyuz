//! Entry sorting.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::columns::Column;
use crate::entry::EntryInfo;
use crate::naming::natural_cmp;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

/// How a pane is sorted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SortSpec {
    pub column: Column,
    pub dir: SortDir,
    pub folders_first: bool,
    pub natural: bool,
    pub case_insensitive: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        SortSpec { column: Column::Name, dir: SortDir::Asc, folders_first: true, natural: true, case_insensitive: true }
    }
}

/// Sort `v` by `spec`. [`compare_entries`] is a total order (ties fall back to the path),
/// so an unstable sort gives the same result as a stable one, about 4× faster with the
/// natural comparator (`cargo bench -p soyuz-core --bench sort`).
pub fn sort_entries(v: &mut [EntryInfo], spec: &SortSpec) {
    v.sort_unstable_by(|a, b| compare_entries(a, b, spec));
}

/// Total order used by [`sort_entries`] and by the app's incremental merge.
///
/// `folders_first` groups directory-like entries first regardless of direction; the
/// primary column honours `dir`; ties fall back to name (ascending) and then path.
pub fn compare_entries(a: &EntryInfo, b: &EntryInfo, spec: &SortSpec) -> Ordering {
    if spec.folders_first {
        let ord = b.is_dir_like().cmp(&a.is_dir_like());
        if ord != Ordering::Equal {
            return ord;
        }
    }
    let name = |a: &EntryInfo, b: &EntryInfo| cmp_names(&a.name, &b.name, spec);
    let primary = match spec.column {
        Column::Name => name(a, b),
        Column::Size => a.size.cmp(&b.size),
        Column::Kind => kind_key(a).cmp(&kind_key(b)),
        Column::Modified => a.modified.cmp(&b.modified),
        Column::Created => a.created.cmp(&b.created),
        Column::Accessed => a.accessed.cmp(&b.accessed),
        Column::Attributes => a.attrs.bits().cmp(&b.attrs.bits()),
        Column::Mode => a.mode.cmp(&b.mode),
        Column::Owner => a.owner.cmp(&b.owner),
        Column::Extension => ext_key(a).cmp(&ext_key(b)),
        Column::Path => cmp_names(&a.path.to_string_lossy(), &b.path.to_string_lossy(), spec),
        // Columns whose data lives outside `EntryInfo` (app services) sort by name here.
        Column::Tags
        | Column::GitStatus
        | Column::Dimensions
        | Column::Duration
        | Column::LinkTarget
        | Column::Checksum
        | Column::ItemCount
        | Column::Script1
        | Column::Script2
        | Column::Script3
        | Column::Script4 => name(a, b),
    };
    let primary = match spec.dir {
        SortDir::Asc => primary,
        SortDir::Desc => primary.reverse(),
    };
    primary.then_with(|| name(a, b)).then_with(|| a.path.cmp(&b.path))
}

/// Sort `v` by `spec` with the primary key from outside [`EntryInfo`] (script columns):
/// `key` gives each entry's text (`None` or empty: no value yet), compared like names
/// (numbers in natural order). Entries without a value come last in both directions;
/// folders-first and the name / path tie-breaks are as in [`compare_entries`].
pub fn sort_entries_keyed(v: &mut Vec<EntryInfo>, spec: &SortSpec, key: impl Fn(&EntryInfo) -> Option<String>) {
    let mut keyed: Vec<(Option<String>, EntryInfo)> =
        v.drain(..).map(|e| (key(&e).filter(|k| !k.is_empty()), e)).collect();
    keyed.sort_unstable_by(|(ka, a), (kb, b)| {
        if spec.folders_first {
            let ord = b.is_dir_like().cmp(&a.is_dir_like());
            if ord != Ordering::Equal {
                return ord;
            }
        }
        let primary = match (ka, kb) {
            (Some(x), Some(y)) => match spec.dir {
                SortDir::Asc => cmp_names(x, y, spec),
                SortDir::Desc => cmp_names(x, y, spec).reverse(),
            },
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        primary.then_with(|| cmp_names(&a.name, &b.name, spec)).then_with(|| a.path.cmp(&b.path))
    });
    v.extend(keyed.into_iter().map(|(_, e)| e));
}

fn cmp_names(a: &str, b: &str, spec: &SortSpec) -> Ordering {
    if spec.natural {
        natural_cmp(a, b, spec.case_insensitive)
    } else if spec.case_insensitive {
        a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b))
    } else {
        a.cmp(b)
    }
}

fn ext_key(e: &EntryInfo) -> String {
    e.extension().map(str::to_lowercase).unwrap_or_default()
}

/// Directories first, then by lower-cased extension.
fn kind_key(e: &EntryInfo) -> (u8, String) {
    if e.is_dir_like() { (0, String::new()) } else { (1, ext_key(e)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::EntryKind;
    use std::path::PathBuf;

    fn e(name: &str, kind: EntryKind, size: u64) -> EntryInfo {
        let mut x = EntryInfo::minimal(PathBuf::from("/r").join(name), kind);
        x.size = size;
        x
    }

    fn names(v: &[EntryInfo]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn name_sort_folders_first() {
        let mut v = vec![
            e("b10.txt", EntryKind::File, 1),
            e("Zdir", EntryKind::Dir, 0),
            e("b2.txt", EntryKind::File, 5),
            e("adir", EntryKind::Dir, 0),
        ];
        let spec = SortSpec::default();
        sort_entries(&mut v, &spec);
        assert_eq!(names(&v), ["adir", "Zdir", "b2.txt", "b10.txt"]);

        let desc = SortSpec { dir: SortDir::Desc, ..SortSpec::default() };
        sort_entries(&mut v, &desc);
        assert_eq!(names(&v), ["Zdir", "adir", "b10.txt", "b2.txt"]);
    }

    #[test]
    fn size_sort_ties_by_name() {
        let mut v = vec![e("c", EntryKind::File, 10), e("a", EntryKind::File, 10), e("b", EntryKind::File, 1)];
        let spec = SortSpec { column: Column::Size, folders_first: false, ..SortSpec::default() };
        sort_entries(&mut v, &spec);
        assert_eq!(names(&v), ["b", "a", "c"]);
        let spec = SortSpec { dir: SortDir::Desc, ..spec };
        sort_entries(&mut v, &spec);
        assert_eq!(names(&v), ["a", "c", "b"]);
    }

    #[test]
    fn kind_and_extension() {
        let mut v = vec![
            e("x.zip", EntryKind::File, 0),
            e("y.TXT", EntryKind::File, 0),
            e("d", EntryKind::Dir, 0),
            e("a.txt", EntryKind::File, 0),
        ];
        let spec = SortSpec { column: Column::Kind, folders_first: false, ..SortSpec::default() };
        sort_entries(&mut v, &spec);
        assert_eq!(names(&v), ["d", "a.txt", "y.TXT", "x.zip"]);
    }

    #[test]
    fn keyed_sort_puts_missing_values_last() {
        let mut v = vec![
            e("a", EntryKind::File, 0),
            e("b", EntryKind::File, 0),
            e("c", EntryKind::File, 0),
            e("d", EntryKind::File, 0),
        ];
        let key = |x: &EntryInfo| match x.name.as_str() {
            "a" => Some("10".to_string()),
            "b" => Some("9".to_string()),
            "c" => Some(String::new()),
            _ => None,
        };
        let spec = SortSpec { column: Column::Script1, ..SortSpec::default() };
        sort_entries_keyed(&mut v, &spec, key);
        assert_eq!(names(&v), ["b", "a", "c", "d"], "natural order, no value last");
        let spec = SortSpec { dir: SortDir::Desc, ..spec };
        sort_entries_keyed(&mut v, &spec, key);
        assert_eq!(names(&v), ["a", "b", "c", "d"], "still last when descending");
    }
}
