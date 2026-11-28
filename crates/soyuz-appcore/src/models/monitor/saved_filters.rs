//! Saved process filters (`[[monitor.saved_filters]]`): named queries the Processes page's
//! filter menu applies in one step. Both frontends edit them through [`super::MonitorCmd`];
//! the list operations here are pure so they can be checked without a host.

use soyuz_core::config::monitor::SavedProcFilter;
use soyuz_sysmon::Filter;

/// Longest name kept (characters); longer names are cut.
pub const NAME_MAX: usize = 60;

/// Why a filter cannot be saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    EmptyName,
    EmptyQuery,
    /// Another saved filter already has this name.
    NameTaken,
    /// The query does not parse; the parser's message.
    BadQuery(String),
}

impl SaveError {
    /// The message shown under the editor or in a toast.
    pub fn message(&self) -> String {
        match self {
            SaveError::EmptyName => tr!("monitor-saved-filter-empty-name").to_string(),
            SaveError::EmptyQuery => tr!("monitor-saved-filter-empty-query").to_string(),
            SaveError::NameTaken => tr!("monitor-saved-filter-name-taken").to_string(),
            SaveError::BadQuery(m) => tr!("monitor-gui-filter-error", message = m.as_str()),
        }
    }
}

/// A name as stored: trimmed, inner whitespace runs as one space, at most [`NAME_MAX`] characters.
pub fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(NAME_MAX).collect()
}

/// Index of the entry named `name` (without case).
pub fn find(list: &[SavedProcFilter], name: &str) -> Option<usize> {
    let name = clean_name(name);
    list.iter().position(|f| f.name.to_lowercase() == name.to_lowercase())
}

/// The saved filter that is in force: the same query and mode as the filter box.
pub fn active<'a>(list: &'a [SavedProcFilter], query: &str, regex: bool) -> Option<&'a SavedProcFilter> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    list.iter().find(|f| f.regex == regex && f.query.trim() == q)
}

/// Check an entry that would replace `old` (by name; `None` adds a new one).
pub fn validate(list: &[SavedProcFilter], old: Option<&str>, entry: &SavedProcFilter) -> Result<(), SaveError> {
    let name = clean_name(&entry.name);
    if name.is_empty() {
        return Err(SaveError::EmptyName);
    }
    if entry.query.trim().is_empty() {
        return Err(SaveError::EmptyQuery);
    }
    if let Err(e) = Filter::parse(entry.query.trim(), entry.regex) {
        return Err(SaveError::BadQuery(e.message));
    }
    let own = old.and_then(|o| find(list, o));
    match find(list, &name) {
        Some(i) if Some(i) != own => Err(SaveError::NameTaken),
        _ => Ok(()),
    }
}

/// Add `entry`, or replace the one named `old` in place. Returns the entry's index.
pub fn save(list: &mut Vec<SavedProcFilter>, old: Option<&str>, entry: SavedProcFilter) -> Result<usize, SaveError> {
    validate(list, old, &entry)?;
    let entry = SavedProcFilter { name: clean_name(&entry.name), query: entry.query.trim().to_string(), ..entry };
    match old.and_then(|o| find(list, o)) {
        Some(i) => {
            list[i] = entry;
            Ok(i)
        }
        None => {
            list.push(entry);
            Ok(list.len() - 1)
        }
    }
}

/// Remove the entry named `name`; `false` when there is none.
pub fn remove(list: &mut Vec<SavedProcFilter>, name: &str) -> bool {
    match find(list, name) {
        Some(i) => {
            list.remove(i);
            true
        }
        None => false,
    }
}

/// Move the entry named `name` to position `to` (clamped).
pub fn move_to(list: &mut Vec<SavedProcFilter>, name: &str, to: usize) {
    if let Some(i) = find(list, name) {
        let item = list.remove(i);
        let to = to.min(list.len());
        list.insert(to, item);
    }
}

/// A name for a new entry: `base` (or the query's first words), made unique with ` 2`, ` 3`, ….
pub fn suggest_name(list: &[SavedProcFilter], base: &str) -> String {
    let mut base = clean_name(base);
    if base.is_empty() {
        base = tr!("monitor-saved-filter-default-name").to_string();
    }
    let base: String = base.chars().take(NAME_MAX - 4).collect();
    if find(list, &base).is_none() {
        return base;
    }
    (2..).map(|n| format!("{base} {n}")).find(|n| find(list, n).is_none()).unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, query: &str) -> SavedProcFilter {
        SavedProcFilter { name: name.into(), query: query.into(), regex: false }
    }

    #[test]
    fn save_adds_replaces_and_renames() {
        let mut l = Vec::new();
        assert_eq!(save(&mut l, None, f("  Busy   ones ", " cpu>5 ")), Ok(0));
        assert_eq!(l[0], f("Busy ones", "cpu>5"));
        assert_eq!(save(&mut l, None, f("Big", "mem>1G")), Ok(1));
        // Same name, other case: taken for a new entry, fine for the entry itself.
        assert_eq!(save(&mut l, None, f("busy ONES", "x")), Err(SaveError::NameTaken));
        assert_eq!(save(&mut l, Some("Busy ones"), f("busy ONES", "cpu>10")), Ok(0));
        assert_eq!(l[0], f("busy ONES", "cpu>10"));
        // Renaming onto another entry's name is refused.
        assert_eq!(save(&mut l, Some("Big"), f("Busy Ones", "x")), Err(SaveError::NameTaken));
        assert_eq!(save(&mut l, Some("Big"), f("Huge", "mem>4G")), Ok(1));
        assert_eq!(l.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["busy ONES", "Huge"]);
    }

    #[test]
    fn save_refuses_empty_and_bad_queries() {
        let mut l = Vec::new();
        assert_eq!(save(&mut l, None, f(" ", "cpu>5")), Err(SaveError::EmptyName));
        assert_eq!(save(&mut l, None, f("a", "  ")), Err(SaveError::EmptyQuery));
        assert!(matches!(save(&mut l, None, f("a", "cpu>abc")), Err(SaveError::BadQuery(_))));
        let bad_re = SavedProcFilter { name: "r".into(), query: "(".into(), regex: true };
        assert!(matches!(save(&mut l, None, bad_re), Err(SaveError::BadQuery(_))));
        assert!(l.is_empty());
    }

    #[test]
    fn remove_move_and_active() {
        let mut l = vec![f("a", "x"), f("b", "y"), f("c", "z")];
        move_to(&mut l, "C", 0);
        assert_eq!(l.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["c", "a", "b"]);
        move_to(&mut l, "c", 99);
        assert_eq!(l.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert!(remove(&mut l, "B"));
        assert!(!remove(&mut l, "b"));
        assert_eq!(active(&l, " z ", false).map(|x| x.name.as_str()), Some("c"));
        assert!(active(&l, "z", true).is_none(), "the mode must match too");
        assert!(active(&l, "", false).is_none());
    }

    #[test]
    fn suggested_names_are_unique() {
        let l = vec![f("cpu>5", "cpu>5"), f("cpu>5 2", "x")];
        assert_eq!(suggest_name(&l, "cpu>5"), "cpu>5 3");
        assert_eq!(suggest_name(&l, "mem>1G"), "mem>1G");
        assert!(!suggest_name(&l, "   ").is_empty());
        assert!(suggest_name(&[], &"x".repeat(200)).chars().count() <= NAME_MAX);
    }
}
