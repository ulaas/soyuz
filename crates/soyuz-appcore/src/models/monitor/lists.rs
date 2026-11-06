//! The Connections, Services and Startup pages: lists read on a worker, with a filter box
//! and a sort column. Frontends draw [`ListState::visible`].

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use soyuz_sysmon::fetch::Loader;
use soyuz_sysmon::{Connection, Frame, ProcKey, ServiceInfo, StartupItem};

/// Process names by PID, for one pass over a list.
pub type Names = HashMap<u32, Arc<str>>;

/// One row of a list page.
pub trait Sortable {
    /// Text the filter searches.
    fn haystack(&self, names: &Names) -> String;
    /// Compare by column `col`.
    fn compare(&self, other: &Self, col: u8, names: &Names) -> Ordering;
    /// Whether the rows show process names.
    fn uses_names() -> bool {
        false
    }
    /// Number of columns.
    fn columns() -> u8;
}

/// The filter of a list page: words that must all occur, or `/regex/`.
#[derive(Clone, Debug, Default)]
pub enum ListFilter {
    #[default]
    All,
    Words(Vec<String>),
    Regex(regex::Regex),
}

impl ListFilter {
    /// Parse the box's text. A broken regular expression matches like plain text.
    pub fn parse(text: &str) -> ListFilter {
        let t = text.trim();
        if t.is_empty() {
            return ListFilter::All;
        }
        if t.len() >= 2
            && t.starts_with('/')
            && t.ends_with('/')
            && let Ok(re) =
                regex::RegexBuilder::new(&t[1..t.len() - 1]).case_insensitive(true).size_limit(1 << 20).build()
        {
            return ListFilter::Regex(re);
        }
        ListFilter::Words(t.split_whitespace().map(str::to_lowercase).collect())
    }

    /// Whether `hay` (any case) passes.
    pub fn matches(&self, hay: &str) -> bool {
        match self {
            ListFilter::All => true,
            ListFilter::Words(w) => {
                let hay = hay.to_lowercase();
                w.iter().all(|x| hay.contains(x.as_str()))
            }
            ListFilter::Regex(r) => r.is_match(hay),
        }
    }
}

/// A list page.
pub struct ListState<T> {
    /// As read, unsorted.
    pub rows: Vec<T>,
    loader: Option<Loader<Vec<T>>>,
    read_at: Option<Instant>,
    /// Read again at the next pump.
    pub stale: bool,
    pub filter_text: String,
    filter: ListFilter,
    pub sort: u8,
    pub descending: bool,
    /// The row under the cursor, in [`ListState::visible`] order.
    pub cursor: usize,
    /// Ask the frontend to put the keyboard into the filter box.
    pub focus_filter: bool,
    /// Bumped when `rows` change.
    version: u64,
    /// The last [`ListState::visible`]: it is asked for several times per painted frame.
    shown: Mutex<Option<(ShownKey, Vec<usize>)>>,
}

/// What [`ListState::visible`] depends on.
type ShownKey = (u64, i64, String, u8, bool);

impl<T: Send + 'static> ListState<T> {
    pub fn new(sort: u8, descending: bool) -> Self {
        ListState {
            rows: Vec::new(),
            loader: None,
            read_at: None,
            stale: false,
            filter_text: String::new(),
            filter: ListFilter::All,
            sort,
            descending,
            cursor: 0,
            focus_filter: false,
            version: 0,
            shown: Mutex::new(None),
        }
    }

    /// Take a finished read and start the next when `every` has passed. No read runs while
    /// the page is not on screen, since only the visible page is pumped.
    pub fn pump(&mut self, every: Duration, start: impl FnOnce() -> Loader<Vec<T>>) {
        if let Some(l) = self.loader.as_mut()
            && let Some(rows) = l.poll()
        {
            self.rows = rows;
            self.version += 1;
            self.loader = None;
            self.read_at = Some(Instant::now());
        }
        let due = self.stale || self.read_at.is_none_or(|t| t.elapsed() >= every);
        if due && self.loader.is_none() {
            self.stale = false;
            self.loader = Some(start());
        }
    }

    /// No read has finished yet.
    pub fn loading(&self) -> bool {
        self.read_at.is_none()
    }

    pub fn set_filter(&mut self, text: String) {
        self.filter = ListFilter::parse(&text);
        self.filter_text = text;
        self.cursor = 0;
    }

    /// Click on a column title: sort by it, or turn the order around.
    pub fn sort_by(&mut self, col: u8) {
        if self.sort == col {
            self.descending = !self.descending;
        } else {
            self.sort = col;
            self.descending = false;
        }
    }
}

impl<T: Sortable + Send + 'static> ListState<T> {
    /// Indices into `rows`: filtered and sorted. Cached until the rows, the filter, the sort or
    /// (for rows that show process names) the frame change.
    pub fn visible(&self, frame: Option<&Frame>) -> Vec<usize> {
        let at = if T::uses_names() { frame.map_or(0, |f| f.at_ms) } else { 0 };
        let key = (self.version, at, self.filter_text.clone(), self.sort, self.descending);
        let mut shown = self.shown.lock();
        if let Some((k, idx)) = shown.as_ref()
            && *k == key
        {
            return idx.clone();
        }
        let names: Names = match frame {
            Some(f) if T::uses_names() => f.procs.iter().map(|p| (p.raw.key.pid, p.raw.name.clone())).collect(),
            _ => Names::new(),
        };
        let mut idx: Vec<usize> = (0..self.rows.len())
            .filter(|&i| matches!(self.filter, ListFilter::All) || self.filter.matches(&self.rows[i].haystack(&names)))
            .collect();
        idx.sort_by(|&a, &b| {
            let o = self.rows[a].compare(&self.rows[b], self.sort, &names);
            if self.descending { o.reverse() } else { o }
        });
        *shown = Some((key, idx.clone()));
        idx
    }
}

/// Connections columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Column {
    Proto,
    Local,
    Remote,
    State,
    Pid,
    Process,
}

impl Column {
    pub const ALL: [Column; 6] =
        [Column::Proto, Column::Local, Column::Remote, Column::State, Column::Pid, Column::Process];
}

/// The name of the process with `pid` in `frame`.
pub fn process_name(frame: Option<&Frame>, pid: Option<u32>) -> String {
    let (Some(frame), Some(pid)) = (frame, pid) else { return String::new() };
    frame.procs.iter().find(|p| p.raw.key.pid == pid).map(|p| p.raw.name.to_string()).unwrap_or_default()
}

/// The process with `pid` in `frame`.
pub fn process_key(frame: Option<&Frame>, pid: Option<u32>) -> Option<ProcKey> {
    let (frame, pid) = (frame?, pid?);
    frame.procs.iter().find(|p| p.raw.key.pid == pid).map(|p| p.raw.key)
}

/// `1.2.3.4:80`, `[::1]:80`, `*` for an unset remote side.
pub fn endpoint(addr: &str, port: u16) -> String {
    if addr.is_empty() || (port == 0 && (addr == "0.0.0.0" || addr == "::")) {
        return "*".into();
    }
    if addr.contains(':') { format!("[{addr}]:{port}") } else { format!("{addr}:{port}") }
}

fn name_of(names: &Names, pid: Option<u32>) -> &str {
    pid.and_then(|p| names.get(&p)).map_or("", |n| n)
}

impl Sortable for Connection {
    fn uses_names() -> bool {
        true
    }

    fn haystack(&self, names: &Names) -> String {
        format!(
            "{:?} {} {} {} {}",
            self.proto,
            endpoint(&self.local_addr, self.local_port),
            endpoint(&self.remote_addr, self.remote_port),
            self.state,
            name_of(names, self.pid),
        ) + self.pid.map_or(String::new(), |p| format!(" {p}")).as_str()
    }

    fn compare(&self, o: &Self, col: u8, names: &Names) -> Ordering {
        match col {
            0 => (self.proto as u8).cmp(&(o.proto as u8)),
            1 => (&self.local_addr, self.local_port).cmp(&(&o.local_addr, o.local_port)),
            2 => (&self.remote_addr, self.remote_port).cmp(&(&o.remote_addr, o.remote_port)),
            3 => self.state.cmp(&o.state),
            4 => self.pid.cmp(&o.pid),
            _ => name_of(names, self.pid).to_lowercase().cmp(&name_of(names, o.pid).to_lowercase()),
        }
        .then_with(|| self.local_port.cmp(&o.local_port))
    }

    fn columns() -> u8 {
        6
    }
}

impl Sortable for ServiceInfo {
    fn haystack(&self, _names: &Names) -> String {
        format!("{} {} {} {} {:?}", self.name, self.display_name, self.description, self.start_mode, self.state)
    }

    fn compare(&self, o: &Self, col: u8, _names: &Names) -> Ordering {
        match col {
            0 => self.display_name.to_lowercase().cmp(&o.display_name.to_lowercase()),
            1 => (self.state as u8).cmp(&(o.state as u8)),
            2 => self.start_mode.cmp(&o.start_mode),
            3 => self.pid.cmp(&o.pid),
            _ => self.name.to_lowercase().cmp(&o.name.to_lowercase()),
        }
        .then_with(|| self.name.cmp(&o.name))
    }

    fn columns() -> u8 {
        5
    }
}

impl Sortable for StartupItem {
    fn haystack(&self, _names: &Names) -> String {
        format!("{} {} {}", self.name, self.command, self.source)
    }

    fn compare(&self, o: &Self, col: u8, _names: &Names) -> Ordering {
        match col {
            0 => self.name.to_lowercase().cmp(&o.name.to_lowercase()),
            1 => self.enabled.cmp(&o.enabled),
            2 => self.source.cmp(&o.source),
            _ => self.command.cmp(&o.command),
        }
        .then_with(|| self.name.cmp(&o.name))
    }

    fn columns() -> u8 {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_words_and_regex() {
        assert!(ListFilter::parse("").matches("anything"));
        assert!(ListFilter::parse("SSH run").matches("sshd running"));
        assert!(!ListFilter::parse("ssh stop").matches("sshd running"));
        assert!(ListFilter::parse("/^ss.d$/").matches("SSHD"));
        // A broken expression searches for itself.
        assert!(ListFilter::parse("/[/").matches("a /[/ b"));
    }

    #[test]
    fn endpoints() {
        assert_eq!(endpoint("1.2.3.4", 80), "1.2.3.4:80");
        assert_eq!(endpoint("::1", 80), "[::1]:80");
        assert_eq!(endpoint("0.0.0.0", 0), "*");
        assert_eq!(endpoint("", 0), "*");
    }

    #[test]
    fn visible_filters_and_sorts() {
        let mut l: ListState<StartupItem> = ListState::new(0, false);
        l.rows = ["b", "A", "c"].iter().map(|n| StartupItem { name: (*n).into(), ..Default::default() }).collect();
        let names =
            |l: &ListState<StartupItem>| l.visible(None).iter().map(|&i| l.rows[i].name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&l), ["A", "b", "c"]);
        l.sort_by(0);
        assert_eq!(names(&l), ["c", "b", "A"]);
        l.set_filter("a".into());
        assert_eq!(names(&l), ["A"]);
    }
}
