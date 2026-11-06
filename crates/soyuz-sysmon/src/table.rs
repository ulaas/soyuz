//! Builds the process table from a frame: flat list, tree, or groups by program or user.
//!
//! Selection and scroll must follow [`RowId`], never a row index: rows reorder every sample.
//! [`build`] allocates a few index arrays, not per row; frontends call it through
//! [`TableCache`], so that it runs when the frame, filter, options or collapsed set changed,
//! never once per painted frame, and call [`cell`] only for the rows on screen.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use soyuz_core::config::monitor::{CpuScale, Monitor, ProcColumn, ProcView};
use soyuz_fs::sysmon::{ProcKey, ProcPriority, ProcState};

use crate::filter::Filter;
use crate::frame::{Frame, ProcRow};

/// One table cell, unformatted: formatting and translation are the frontends'.
#[derive(Clone, Debug, PartialEq)]
pub enum CellValue {
    Text(String),
    Count(u64),
    Bytes(u64),
    /// Bytes per second.
    Rate(f64),
    Percent(f32),
    Millis(u64),
    Seconds(u64),
    UnixMs(i64),
    State(ProcState),
    Priority(ProcPriority),
    Bool(bool),
    Int(i64),
    /// The OS does not offer this value (or not for this process).
    Missing,
}

fn scaled(percent: f32, ncpu: u32, scale: CpuScale) -> f32 {
    match scale {
        CpuScale::Machine => percent / ncpu.max(1) as f32,
        CpuScale::PerCore => percent,
    }
}

fn opt_text(s: &Option<Arc<str>>) -> CellValue {
    s.as_deref().map_or(CellValue::Missing, |s| CellValue::Text(s.to_string()))
}

fn opt_bytes(v: Option<u64>) -> CellValue {
    v.map_or(CellValue::Missing, CellValue::Bytes)
}

/// The value of one column of one process. The Cpu column is `row.cpu_percent`, divided by
/// `ncpu` on the Machine scale.
pub fn cell(col: ProcColumn, row: &ProcRow, now_ms: i64, ncpu: u32, scale: CpuScale) -> CellValue {
    use CellValue as V;
    let r = &row.raw;
    // No figures at all: the OS does not tell (another user's process on macOS, a zombie). These
    // columns are not `Option`s in the record, but a dash is the truth here, not 0.
    let unreadable = r.rss == 0 && r.threads == 0 && r.cpu_user_ms == 0 && r.cpu_system_ms == 0;
    if unreadable
        && matches!(
            col,
            ProcColumn::Cpu
                | ProcColumn::CpuTime
                | ProcColumn::Memory
                | ProcColumn::MemoryPercent
                | ProcColumn::Threads
        )
    {
        return V::Missing;
    }
    match col {
        ProcColumn::Name => V::Text(r.name.to_string()),
        ProcColumn::Pid => V::Int(i64::from(r.key.pid)),
        ProcColumn::Parent => r.parent.map_or(V::Missing, |p| V::Int(i64::from(p))),
        ProcColumn::User => opt_text(&r.user),
        ProcColumn::State => V::State(r.state),
        ProcColumn::Cpu => V::Percent(scaled(row.cpu_percent, ncpu, scale)),
        ProcColumn::CpuTime => V::Millis(r.cpu_user_ms.saturating_add(r.cpu_system_ms)),
        ProcColumn::Memory => V::Bytes(r.rss),
        ProcColumn::Private => opt_bytes(r.private),
        ProcColumn::Shared => opt_bytes(r.shared),
        ProcColumn::Virtual => opt_bytes(r.virt),
        ProcColumn::PeakMemory => opt_bytes(r.peak_rss),
        ProcColumn::MemoryPercent => V::Percent(row.mem_percent),
        ProcColumn::Threads => V::Count(u64::from(r.threads)),
        ProcColumn::Handles => r.handles.map_or(V::Missing, |h| V::Count(u64::from(h))),
        ProcColumn::DiskRead => row.read_rate.map_or(V::Missing, V::Rate),
        ProcColumn::DiskWrite => row.write_rate.map_or(V::Missing, V::Rate),
        ProcColumn::DiskReadTotal => opt_bytes(r.read_bytes),
        ProcColumn::DiskWriteTotal => opt_bytes(r.write_bytes),
        ProcColumn::Gpu => r.gpu_percent.map_or(V::Missing, V::Percent),
        ProcColumn::GpuMemory => opt_bytes(r.gpu_mem),
        ProcColumn::PageFaults => r.page_faults.map_or(V::Missing, V::Count),
        ProcColumn::Priority => r.priority.map_or(V::Missing, V::Priority),
        ProcColumn::Nice => r.nice.map_or(V::Missing, |n| V::Int(i64::from(n))),
        ProcColumn::Session => opt_text(&r.session),
        ProcColumn::Uptime => {
            if r.key.start_ms > 0 {
                V::Seconds((now_ms.saturating_sub(r.key.start_ms).max(0) / 1000) as u64)
            } else {
                V::Missing
            }
        }
        ProcColumn::Started => {
            if r.key.start_ms > 0 {
                V::UnixMs(r.key.start_ms)
            } else {
                V::Missing
            }
        }
        ProcColumn::Path => r.exe.as_deref().map_or(V::Missing, |p| V::Text(p.to_string_lossy().into_owned())),
        ProcColumn::Command => opt_text(&r.cmdline),
        ProcColumn::Arch => opt_text(&r.arch),
        ProcColumn::Elevated => r.elevated.map_or(V::Missing, V::Bool),
        ProcColumn::Cgroup => opt_text(&r.cgroup),
        ProcColumn::Description => opt_text(&r.description),
    }
}

/// Sums over a group or a collapsed subtree. `None` where no member has the value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    /// Processes summed.
    pub count: u32,
    /// Where 100 is one logical CPU, like `ProcRow::cpu_percent`.
    pub cpu_percent: f32,
    pub cpu_ms: u64,
    pub rss: u64,
    pub private: Option<u64>,
    pub shared: Option<u64>,
    pub virt: Option<u64>,
    pub mem_percent: f32,
    pub threads: u64,
    pub handles: Option<u64>,
    pub read_rate: Option<f64>,
    pub write_rate: Option<f64>,
    pub read_total: Option<u64>,
    pub write_total: Option<u64>,
    pub gpu_percent: Option<f32>,
    pub gpu_mem: Option<u64>,
    pub page_faults: Option<u64>,
}

fn add_opt<T: std::ops::Add<Output = T> + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a + b),
        (a, None) => a,
        (None, b) => b,
    }
}

impl Totals {
    /// Adds one process.
    pub fn add(&mut self, row: &ProcRow) {
        let r = &row.raw;
        self.count += 1;
        self.cpu_percent += row.cpu_percent;
        self.cpu_ms = self.cpu_ms.saturating_add(r.cpu_user_ms.saturating_add(r.cpu_system_ms));
        self.rss = self.rss.saturating_add(r.rss);
        self.private = add_opt(self.private, r.private);
        self.shared = add_opt(self.shared, r.shared);
        self.virt = add_opt(self.virt, r.virt);
        self.mem_percent += row.mem_percent;
        self.threads += u64::from(r.threads);
        self.handles = add_opt(self.handles, r.handles.map(u64::from));
        self.read_rate = add_opt(self.read_rate, row.read_rate);
        self.write_rate = add_opt(self.write_rate, row.write_rate);
        self.read_total = add_opt(self.read_total, r.read_bytes);
        self.write_total = add_opt(self.write_total, r.write_bytes);
        self.gpu_percent = add_opt(self.gpu_percent, r.gpu_percent);
        self.gpu_mem = add_opt(self.gpu_mem, r.gpu_mem);
        self.page_faults = add_opt(self.page_faults, r.page_faults);
    }
}

/// The sum of a column over a group or collapsed subtree; `Missing` for columns that do not sum.
pub fn cell_total(col: ProcColumn, t: &Totals, ncpu: u32, scale: CpuScale) -> CellValue {
    use CellValue as V;
    match col {
        ProcColumn::Cpu => V::Percent(scaled(t.cpu_percent, ncpu, scale)),
        ProcColumn::CpuTime => V::Millis(t.cpu_ms),
        ProcColumn::Memory => V::Bytes(t.rss),
        ProcColumn::Private => opt_bytes(t.private),
        ProcColumn::Shared => opt_bytes(t.shared),
        ProcColumn::Virtual => opt_bytes(t.virt),
        ProcColumn::MemoryPercent => V::Percent(t.mem_percent),
        ProcColumn::Threads => V::Count(t.threads),
        ProcColumn::Handles => t.handles.map_or(V::Missing, V::Count),
        ProcColumn::DiskRead => t.read_rate.map_or(V::Missing, V::Rate),
        ProcColumn::DiskWrite => t.write_rate.map_or(V::Missing, V::Rate),
        ProcColumn::DiskReadTotal => opt_bytes(t.read_total),
        ProcColumn::DiskWriteTotal => opt_bytes(t.write_total),
        ProcColumn::Gpu => t.gpu_percent.map_or(V::Missing, V::Percent),
        ProcColumn::GpuMemory => opt_bytes(t.gpu_mem),
        ProcColumn::PageFaults => t.page_faults.map_or(V::Missing, V::Count),
        _ => V::Missing,
    }
}

/// What the table should show.
#[derive(Clone, Debug, PartialEq)]
pub struct TableOpts {
    pub view: ProcView,
    pub sort_by: ProcColumn,
    pub descending: bool,
    /// Tree view: keep the parents of matches on screen (dimmed, `matched` false). Otherwise
    /// matches flatten to roots.
    pub keep_parents: bool,
    pub show_kernel: bool,
    pub show_other_users: bool,
    /// The current user, for `show_other_users`; `None` shows everyone.
    pub me: Option<String>,
    pub scale: CpuScale,
    pub ncpu: u32,
    pub now_ms: i64,
}

impl Default for TableOpts {
    fn default() -> Self {
        TableOpts {
            view: ProcView::List,
            sort_by: ProcColumn::Cpu,
            descending: true,
            keep_parents: true,
            show_kernel: false,
            show_other_users: true,
            me: None,
            scale: CpuScale::Machine,
            ncpu: 1,
            now_ms: 0,
        }
    }
}

impl TableOpts {
    /// The `[monitor]` settings for one frame. `me` is the current user.
    pub fn from_config(cfg: &Monitor, me: Option<String>, frame: &Frame) -> Self {
        TableOpts {
            view: cfg.view,
            sort_by: cfg.sort_by,
            descending: cfg.sort_descending,
            keep_parents: cfg.filter_keeps_parents,
            show_kernel: cfg.show_kernel,
            show_other_users: cfg.show_other_users,
            me,
            scale: cfg.cpu_scale,
            ncpu: frame.ncpu(),
            now_ms: frame.at_ms,
        }
    }
}

/// Stable identity of a row across frames: follow selection and scroll with this.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RowId {
    Proc(ProcKey),
    /// A program or user group, by label.
    Group(String),
}

/// What a table row stands for.
#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    /// Index into `frame.procs`.
    Proc(usize),
    Group {
        label: Arc<str>,
        count: u32,
    },
}

/// One row on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct TableRow {
    pub kind: RowKind,
    pub depth: u16,
    pub has_children: bool,
    pub collapsed: bool,
    /// False: kept only as the parent of a match; draw it dimmed.
    pub matched: bool,
    /// The last child of its parent, for tree guides.
    pub last_sibling: bool,
    /// Bit `d` set: the ancestor at depth `d` was a last sibling, so no guide line at that depth.
    pub ancestors_last: u64,
    /// Subtree totals of a collapsed row, or the totals of a group.
    pub totals: Option<Totals>,
}

impl TableRow {
    /// This row's stable identity.
    pub fn id(&self, frame: &Frame) -> RowId {
        match &self.kind {
            RowKind::Proc(i) => RowId::Proc(frame.procs[*i].raw.key),
            RowKind::Group { label, .. } => RowId::Group(label.to_string()),
        }
    }

    /// The process of a process row.
    pub fn proc<'a>(&self, frame: &'a Frame) -> Option<&'a ProcRow> {
        match self.kind {
            RowKind::Proc(i) => frame.procs.get(i),
            RowKind::Group { .. } => None,
        }
    }

    fn is(&self, frame: &Frame, id: &RowId) -> bool {
        match (&self.kind, id) {
            (RowKind::Proc(i), RowId::Proc(key)) => frame.procs.get(*i).is_some_and(|p| p.raw.key == *key),
            (RowKind::Group { label, .. }, RowId::Group(g)) => **label == **g,
            _ => false,
        }
    }
}

/// The index of a row by identity (O(rows), no allocation).
pub fn position_of(rows: &[TableRow], frame: &Frame, id: &RowId) -> Option<usize> {
    rows.iter().position(|r| r.is(frame, id))
}

/// Sorts processes in the table's order; missing values always last, ties by pid.
struct Order<'a> {
    col: ProcColumn,
    descending: bool,
    procs: &'a [ProcRow],
    now_ms: i64,
}

fn cmp_opt<T: PartialOrd>(a: Option<T>, b: Option<T>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            let o = a.partial_cmp(&b).unwrap_or(Ordering::Equal);
            if descending { o.reverse() } else { o }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn cmp_text_ci(a: &str, b: &str) -> Ordering {
    a.bytes().map(|c| c.to_ascii_lowercase()).cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
}

fn cmp_text(a: Option<&str>, b: Option<&str>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            let o = cmp_text_ci(a, b);
            if descending { o.reverse() } else { o }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn state_rank(s: ProcState) -> u8 {
    match s {
        ProcState::Running => 0,
        ProcState::DiskWait => 1,
        ProcState::Sleeping => 2,
        ProcState::Idle => 3,
        ProcState::Stopped => 4,
        ProcState::Suspended => 5,
        ProcState::Zombie => 6,
        ProcState::Unknown => 7,
    }
}

impl Order<'_> {
    fn cmp(&self, ia: u32, ib: u32) -> Ordering {
        let (ra, rb) = (&self.procs[ia as usize], &self.procs[ib as usize]);
        self.values(ra, rb).then_with(|| ra.raw.key.pid.cmp(&rb.raw.key.pid))
    }

    fn values(&self, ra: &ProcRow, rb: &ProcRow) -> Ordering {
        let d = self.descending;
        let (a, b) = (&ra.raw, &rb.raw);
        match self.col {
            ProcColumn::Name => cmp_text(Some(&*a.name), Some(&*b.name), d),
            ProcColumn::Pid => cmp_opt(Some(a.key.pid), Some(b.key.pid), d),
            ProcColumn::Parent => cmp_opt(a.parent, b.parent, d),
            ProcColumn::User => cmp_text(a.user.as_deref(), b.user.as_deref(), d),
            ProcColumn::State => cmp_opt(Some(state_rank(a.state)), Some(state_rank(b.state)), d),
            ProcColumn::Cpu => cmp_opt(Some(ra.cpu_percent), Some(rb.cpu_percent), d),
            ProcColumn::CpuTime => {
                cmp_opt(Some(a.cpu_user_ms + a.cpu_system_ms), Some(b.cpu_user_ms + b.cpu_system_ms), d)
            }
            ProcColumn::Memory => cmp_opt(Some(a.rss), Some(b.rss), d),
            ProcColumn::Private => cmp_opt(a.private, b.private, d),
            ProcColumn::Shared => cmp_opt(a.shared, b.shared, d),
            ProcColumn::Virtual => cmp_opt(a.virt, b.virt, d),
            ProcColumn::PeakMemory => cmp_opt(a.peak_rss, b.peak_rss, d),
            ProcColumn::MemoryPercent => cmp_opt(Some(ra.mem_percent), Some(rb.mem_percent), d),
            ProcColumn::Threads => cmp_opt(Some(a.threads), Some(b.threads), d),
            ProcColumn::Handles => cmp_opt(a.handles, b.handles, d),
            ProcColumn::DiskRead => cmp_opt(ra.read_rate, rb.read_rate, d),
            ProcColumn::DiskWrite => cmp_opt(ra.write_rate, rb.write_rate, d),
            ProcColumn::DiskReadTotal => cmp_opt(a.read_bytes, b.read_bytes, d),
            ProcColumn::DiskWriteTotal => cmp_opt(a.write_bytes, b.write_bytes, d),
            ProcColumn::Gpu => cmp_opt(a.gpu_percent, b.gpu_percent, d),
            ProcColumn::GpuMemory => cmp_opt(a.gpu_mem, b.gpu_mem, d),
            ProcColumn::PageFaults => cmp_opt(a.page_faults, b.page_faults, d),
            ProcColumn::Priority => cmp_opt(a.priority, b.priority, d),
            ProcColumn::Nice => cmp_opt(a.nice, b.nice, d),
            ProcColumn::Session => cmp_text(a.session.as_deref(), b.session.as_deref(), d),
            ProcColumn::Uptime => {
                let up = |r: &ProcRow| (r.raw.key.start_ms > 0).then(|| self.now_ms - r.raw.key.start_ms);
                cmp_opt(up(ra), up(rb), d)
            }
            ProcColumn::Started => {
                let start = |r: &ProcRow| (r.raw.key.start_ms > 0).then_some(r.raw.key.start_ms);
                cmp_opt(start(ra), start(rb), d)
            }
            ProcColumn::Path => cmp_opt(a.exe.as_deref(), b.exe.as_deref(), d),
            ProcColumn::Command => cmp_text(a.cmdline.as_deref(), b.cmdline.as_deref(), d),
            ProcColumn::Arch => cmp_text(a.arch.as_deref(), b.arch.as_deref(), d),
            ProcColumn::Elevated => cmp_opt(a.elevated, b.elevated, d),
            ProcColumn::Cgroup => cmp_text(a.cgroup.as_deref(), b.cgroup.as_deref(), d),
            ProcColumn::Description => cmp_text(a.description.as_deref(), b.description.as_deref(), d),
        }
    }
}

const NONE: u32 = u32::MAX;

/// Lays out `frame.procs` as `opts` asks. Rows of the idle pseudo process never appear.
pub fn build(frame: &Frame, filter: &Filter, opts: &TableOpts, collapsed: &HashSet<RowId>) -> Vec<TableRow> {
    let procs: &[ProcRow] = &frame.procs;
    let ncpu = opts.ncpu.max(1);
    let visible = |r: &ProcRow| {
        !r.raw.idle
            && (opts.show_kernel || !r.raw.kernel)
            && (opts.show_other_users
                || match (&opts.me, &r.raw.user) {
                    (None, _) => true,
                    (Some(me), Some(user)) => **me == **user,
                    (Some(_), None) => false,
                })
    };
    let order = Order { col: opts.sort_by, descending: opts.descending, procs, now_ms: opts.now_ms };
    let matches = |r: &ProcRow| filter.matches_in(r, ncpu, opts.scale);

    match opts.view {
        ProcView::List => {
            let mut idx: Vec<u32> = (0..procs.len() as u32)
                .filter(|&i| visible(&procs[i as usize]) && matches(&procs[i as usize]))
                .collect();
            idx.sort_unstable_by(|&a, &b| order.cmp(a, b));
            flat_rows(&idx)
        }
        ProcView::Tree => tree(frame, filter, opts, collapsed, &order, &visible, &matches),
        ProcView::ByName | ProcView::ByUser => {
            let mut idx: Vec<u32> = (0..procs.len() as u32)
                .filter(|&i| visible(&procs[i as usize]) && matches(&procs[i as usize]))
                .collect();
            idx.sort_unstable_by(|&a, &b| order.cmp(a, b));
            groups(frame, opts, collapsed, &idx)
        }
    }
}

fn flat_rows(idx: &[u32]) -> Vec<TableRow> {
    let last = idx.len().saturating_sub(1);
    idx.iter()
        .enumerate()
        .map(|(n, &i)| TableRow {
            kind: RowKind::Proc(i as usize),
            depth: 0,
            has_children: false,
            collapsed: false,
            matched: true,
            last_sibling: n == last,
            ancestors_last: 0,
            totals: None,
        })
        .collect()
}

/// A parent link, unless it points at itself, nowhere, or at a process younger than the child
/// (the PID was reused after the real parent exited).
fn parent_links(procs: &[ProcRow], in_table: &[bool]) -> Vec<u32> {
    let mut by_pid: HashMap<u32, u32> = HashMap::with_capacity(procs.len());
    for (i, p) in procs.iter().enumerate() {
        if in_table[i] {
            by_pid.insert(p.raw.key.pid, i as u32);
        }
    }
    let mut parent: Vec<u32> = vec![NONE; procs.len()];
    for (i, p) in procs.iter().enumerate() {
        if !in_table[i] {
            continue;
        }
        let Some(&up) = p.raw.parent.and_then(|pid| by_pid.get(&pid)) else { continue };
        if up as usize == i {
            continue;
        }
        let (child_start, parent_start) = (p.raw.key.start_ms, procs[up as usize].raw.key.start_ms);
        if child_start > 0 && parent_start > child_start {
            continue;
        }
        parent[i] = up;
    }
    // Break cycles: walk up from every process; meeting the current walk again closes a loop.
    let mut seen = vec![0u32; procs.len()];
    for start in 0..procs.len() {
        let walk = start as u32 + 1;
        let mut at = start as u32;
        while at != NONE && seen[at as usize] == 0 {
            seen[at as usize] = walk;
            at = parent[at as usize];
        }
        if at != NONE && seen[at as usize] == walk {
            parent[at as usize] = NONE;
        }
    }
    parent
}

fn tree(
    frame: &Frame,
    filter: &Filter,
    opts: &TableOpts,
    collapsed: &HashSet<RowId>,
    order: &Order<'_>,
    visible: &dyn Fn(&ProcRow) -> bool,
    matches: &dyn Fn(&ProcRow) -> bool,
) -> Vec<TableRow> {
    let procs: &[ProcRow] = &frame.procs;
    let n = procs.len();
    let filtering = !filter.is_empty();

    if filtering && !opts.keep_parents {
        let mut idx: Vec<u32> =
            (0..n as u32).filter(|&i| visible(&procs[i as usize]) && matches(&procs[i as usize])).collect();
        idx.sort_unstable_by(|&a, &b| order.cmp(a, b));
        return flat_rows(&idx);
    }

    let in_table: Vec<bool> = procs.iter().map(visible).collect();
    let parent = parent_links(procs, &in_table);

    // Which rows stay: matches and, when filtering, all their ancestors.
    let is_match: Vec<bool> =
        procs.iter().enumerate().map(|(i, p)| in_table[i] && (!filtering || matches(p))).collect();
    let mut include = is_match.clone();
    if filtering {
        for i in 0..n {
            if is_match[i] {
                let mut up = parent[i];
                while up != NONE && !include[up as usize] {
                    include[up as usize] = true;
                    up = parent[up as usize];
                }
            }
        }
    }

    // Children of each node in table order, in one array (counting sort): no per-node Vec.
    let mut sorted: Vec<u32> = (0..n as u32).filter(|&i| include[i as usize]).collect();
    sorted.sort_unstable_by(|&a, &b| order.cmp(a, b));
    let mut child_start = vec![0u32; n + 2];
    let mut roots: Vec<u32> = Vec::new();
    for &i in &sorted {
        match parent[i as usize] {
            NONE => roots.push(i),
            p => child_start[p as usize + 2] += 1,
        }
    }
    for k in 2..n + 2 {
        child_start[k] += child_start[k - 1];
    }
    let mut children = vec![0u32; sorted.len()];
    for &i in &sorted {
        let p = parent[i as usize];
        if p != NONE {
            children[child_start[p as usize + 1] as usize] = i;
            child_start[p as usize + 1] += 1;
        }
    }
    // child_start[p] .. child_start[p + 1] are now p's children.
    let kids = |p: u32| &children[child_start[p as usize] as usize..child_start[p as usize + 1] as usize];

    let subtree_totals = |root: u32, stack: &mut Vec<u32>| -> Totals {
        let mut t = Totals::default();
        stack.clear();
        stack.push(root);
        while let Some(i) = stack.pop() {
            t.add(&procs[i as usize]);
            stack.extend_from_slice(kids(i));
        }
        t
    };

    let mut rows: Vec<TableRow> = Vec::with_capacity(sorted.len());
    let mut scratch: Vec<u32> = Vec::new();
    // (siblings, next position, depth, ancestors_last)
    let mut stack: Vec<(&[u32], usize, u16, u64)> = vec![(roots.as_slice(), 0, 0, 0)];
    while let Some(top) = stack.last_mut() {
        let (siblings, pos, depth, ancestors) = *top;
        if pos >= siblings.len() {
            stack.pop();
            continue;
        }
        top.1 += 1;
        let i = siblings[pos];
        let last = pos + 1 == siblings.len();
        let my_kids = kids(i);
        let has_children = !my_kids.is_empty();
        let is_collapsed =
            has_children && !collapsed.is_empty() && collapsed.contains(&RowId::Proc(procs[i as usize].raw.key));
        rows.push(TableRow {
            kind: RowKind::Proc(i as usize),
            depth,
            has_children,
            collapsed: is_collapsed,
            matched: is_match[i as usize],
            last_sibling: last,
            ancestors_last: ancestors,
            totals: is_collapsed.then(|| subtree_totals(i, &mut scratch)),
        });
        if has_children && !is_collapsed {
            let bit = if last && depth < 64 { 1u64 << depth } else { 0 };
            stack.push((my_kids, 0, depth.saturating_add(1), ancestors | bit));
        }
    }
    rows
}

fn groups(frame: &Frame, opts: &TableOpts, collapsed: &HashSet<RowId>, idx: &[u32]) -> Vec<TableRow> {
    let procs: &[ProcRow] = &frame.procs;
    struct Group {
        label: Arc<str>,
        totals: Totals,
    }
    let unknown: Arc<str> = Arc::from("");
    let label_of = |r: &ProcRow| -> Arc<str> {
        match opts.view {
            ProcView::ByUser => r.raw.user.clone().unwrap_or_else(|| unknown.clone()),
            _ => r.raw.name.clone(),
        }
    };
    let mut by_label: HashMap<Arc<str>, u32> = HashMap::new();
    let mut list: Vec<Group> = Vec::new();
    let mut member_group: Vec<u32> = Vec::with_capacity(idx.len());
    for &i in idx {
        let row = &procs[i as usize];
        let label = label_of(row);
        let g = match by_label.get(&label) {
            Some(&g) => g,
            None => {
                let g = list.len() as u32;
                list.push(Group { label: label.clone(), totals: Totals::default() });
                by_label.insert(label, g);
                g
            }
        };
        list[g as usize].totals.add(row);
        member_group.push(g);
    }

    // Members of each group, in table order.
    let mut start = vec![0u32; list.len() + 2];
    for &g in &member_group {
        start[g as usize + 2] += 1;
    }
    for k in 2..list.len() + 2 {
        start[k] += start[k - 1];
    }
    let mut members = vec![0u32; idx.len()];
    for (&i, &g) in idx.iter().zip(&member_group) {
        members[start[g as usize + 1] as usize] = i;
        start[g as usize + 1] += 1;
    }

    // Groups sort by the summed column; label columns and columns that do not sum by label.
    let mut group_order: Vec<u32> = (0..list.len() as u32).collect();
    let d = opts.descending;
    group_order.sort_unstable_by(|&a, &b| {
        let (ga, gb) = (&list[a as usize], &list[b as usize]);
        total_cmp(opts.sort_by, &ga.totals, &gb.totals, d)
            // The group of processes whose user cannot be read stays at the end.
            .then_with(|| ga.label.is_empty().cmp(&gb.label.is_empty()))
            .then_with(|| {
                let by_label = cmp_text_ci(&ga.label, &gb.label);
                let label_col = matches!(opts.sort_by, ProcColumn::Name | ProcColumn::User);
                if label_col && d { by_label.reverse() } else { by_label }
            })
    });

    let mut rows = Vec::with_capacity(idx.len() + list.len());
    let last_group = group_order.len().saturating_sub(1);
    for (pos, &g) in group_order.iter().enumerate() {
        let group = &list[g as usize];
        let is_collapsed = !collapsed.is_empty() && collapsed.contains(&RowId::Group(group.label.to_string()));
        let last = pos == last_group;
        rows.push(TableRow {
            kind: RowKind::Group { label: group.label.clone(), count: group.totals.count },
            depth: 0,
            has_children: true,
            collapsed: is_collapsed,
            matched: true,
            last_sibling: last,
            ancestors_last: 0,
            totals: Some(group.totals),
        });
        if is_collapsed {
            continue;
        }
        let own = &members[start[g as usize] as usize..start[g as usize + 1] as usize];
        for (m, &i) in own.iter().enumerate() {
            rows.push(TableRow {
                kind: RowKind::Proc(i as usize),
                depth: 1,
                has_children: false,
                collapsed: false,
                matched: true,
                last_sibling: m + 1 == own.len(),
                ancestors_last: u64::from(last),
                totals: None,
            });
        }
    }
    rows
}

fn total_cmp(col: ProcColumn, a: &Totals, b: &Totals, d: bool) -> Ordering {
    match col {
        ProcColumn::Cpu => cmp_opt(Some(a.cpu_percent), Some(b.cpu_percent), d),
        ProcColumn::CpuTime => cmp_opt(Some(a.cpu_ms), Some(b.cpu_ms), d),
        ProcColumn::Memory => cmp_opt(Some(a.rss), Some(b.rss), d),
        ProcColumn::Private => cmp_opt(a.private, b.private, d),
        ProcColumn::Shared => cmp_opt(a.shared, b.shared, d),
        ProcColumn::Virtual => cmp_opt(a.virt, b.virt, d),
        ProcColumn::MemoryPercent => cmp_opt(Some(a.mem_percent), Some(b.mem_percent), d),
        ProcColumn::Threads => cmp_opt(Some(a.threads), Some(b.threads), d),
        ProcColumn::Handles => cmp_opt(a.handles, b.handles, d),
        ProcColumn::DiskRead => cmp_opt(a.read_rate, b.read_rate, d),
        ProcColumn::DiskWrite => cmp_opt(a.write_rate, b.write_rate, d),
        ProcColumn::DiskReadTotal => cmp_opt(a.read_total, b.read_total, d),
        ProcColumn::DiskWriteTotal => cmp_opt(a.write_total, b.write_total, d),
        ProcColumn::Gpu => cmp_opt(a.gpu_percent, b.gpu_percent, d),
        ProcColumn::GpuMemory => cmp_opt(a.gpu_mem, b.gpu_mem, d),
        ProcColumn::PageFaults => cmp_opt(a.page_faults, b.page_faults, d),
        _ => Ordering::Equal,
    }
}

struct CacheKey {
    generation: u64,
    filter: String,
    filter_regex: bool,
    opts: TableOpts,
}

/// Rebuilds the table only when the frame generation, filter, options or collapsed set changed.
/// `now_ms` changing alone does not rebuild: it only moves with the frames.
#[derive(Default)]
pub struct TableCache {
    key: Option<CacheKey>,
    collapsed: HashSet<RowId>,
    rows: Vec<TableRow>,
    builds: u64,
}

impl TableCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The rows for this state. `generation` is `Hub::generation()` (or any number that changes
    /// with `frame`).
    pub fn rows(
        &mut self,
        frame: &Frame,
        generation: u64,
        filter: &Filter,
        opts: &TableOpts,
        collapsed: &HashSet<RowId>,
    ) -> &[TableRow] {
        let same = self.key.as_ref().is_some_and(|k| {
            k.generation == generation
                && k.filter == filter.text()
                && k.filter_regex == filter.is_regex_mode()
                && same_opts(&k.opts, opts)
                && self.collapsed == *collapsed
        });
        if !same {
            self.rows = build(frame, filter, opts, collapsed);
            self.builds += 1;
            self.collapsed.clone_from(collapsed);
            self.key = Some(CacheKey {
                generation,
                filter: filter.text().to_string(),
                filter_regex: filter.is_regex_mode(),
                opts: opts.clone(),
            });
        }
        &self.rows
    }

    /// Forgets the cached rows.
    pub fn invalidate(&mut self) {
        self.key = None;
    }

    /// How many times this cache has built a table (for tests and frame logs).
    pub fn builds(&self) -> u64 {
        self.builds
    }
}

/// Equal apart from the clock.
fn same_opts(a: &TableOpts, b: &TableOpts) -> bool {
    a.view == b.view
        && a.sort_by == b.sort_by
        && a.descending == b.descending
        && a.keep_parents == b.keep_parents
        && a.show_kernel == b.show_kernel
        && a.show_other_users == b.show_other_users
        && a.me == b.me
        && a.scale == b.scale
        && a.ncpu == b.ncpu
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{frame_of, row};

    fn opts(view: ProcView) -> TableOpts {
        TableOpts { view, scale: CpuScale::PerCore, ..Default::default() }
    }

    fn names(frame: &Frame, rows: &[TableRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match &r.kind {
                RowKind::Proc(i) => format!("{}{}", " ".repeat(r.depth as usize), frame.procs[*i].raw.name),
                RowKind::Group { label, count } => format!("[{label} {count}]"),
            })
            .collect()
    }

    fn nofilter() -> Filter {
        Filter::default()
    }

    fn none() -> HashSet<RowId> {
        HashSet::new()
    }

    #[test]
    fn a_process_without_figures_shows_dashes_not_zeros() {
        let mut row = crate::testutil::row(1, None, "launchd", 0.0, 0);
        row.raw.threads = 0;
        row.raw.cpu_user_ms = 0;
        row.raw.cpu_system_ms = 0;
        for col in [ProcColumn::Cpu, ProcColumn::CpuTime, ProcColumn::Memory, ProcColumn::Threads] {
            assert_eq!(cell(col, &row, 0, 4, CpuScale::Machine), CellValue::Missing, "{col:?}");
        }
        row.raw.threads = 1;
        assert_eq!(cell(ProcColumn::Memory, &row, 0, 4, CpuScale::Machine), CellValue::Bytes(0));
    }
    #[test]
    fn list_sorts_with_a_stable_tie_break_and_missing_last() {
        let mut a = row(3, None, "a", 5.0, 0);
        a.raw.gpu_percent = Some(1.0);
        let b = row(1, None, "b", 5.0, 0);
        let mut c = row(2, None, "c", 9.0, 0);
        c.raw.gpu_percent = Some(2.0);
        let f = frame_of(vec![a, b, c]);
        let rows = build(&f, &nofilter(), &opts(ProcView::List), &none());
        assert_eq!(names(&f, &rows), ["c", "b", "a"], "cpu descending, ties by pid");
        let mut o = opts(ProcView::List);
        o.sort_by = ProcColumn::Gpu;
        assert_eq!(names(&f, &build(&f, &nofilter(), &o, &none())), ["c", "a", "b"]);
        o.descending = false;
        assert_eq!(names(&f, &build(&f, &nofilter(), &o, &none())), ["a", "c", "b"], "missing stays last");
    }

    #[test]
    fn list_filters_and_hides_idle_kernel_and_others() {
        let mut idle = row(0, None, "Idle", 0.0, 0);
        idle.raw.idle = true;
        let mut kernel = row(4, None, "System", 0.0, 0);
        kernel.raw.kernel = true;
        let mut mine = row(10, None, "mine", 1.0, 0);
        mine.raw.user = Some(Arc::from("me"));
        let mut theirs = row(11, None, "theirs", 1.0, 0);
        theirs.raw.user = Some(Arc::from("you"));
        let f = frame_of(vec![idle, kernel, mine, theirs]);
        let mut o = opts(ProcView::List);
        o.sort_by = ProcColumn::Pid;
        o.descending = false;
        assert_eq!(names(&f, &build(&f, &nofilter(), &o, &none())), ["mine", "theirs"]);
        o.show_kernel = true;
        assert_eq!(names(&f, &build(&f, &nofilter(), &o, &none())), ["System", "mine", "theirs"]);
        o.show_kernel = false;
        o.show_other_users = false;
        o.me = Some("me".into());
        assert_eq!(names(&f, &build(&f, &nofilter(), &o, &none())), ["mine"]);
        o.show_other_users = true;
        let q = Filter::parse("theirs", false).unwrap();
        assert_eq!(names(&f, &build(&f, &q, &o, &none())), ["theirs"]);
    }

    #[test]
    fn cells_follow_the_scale_and_missing_is_missing() {
        let mut r = row(5, Some(1), "x", 400.0, 1024);
        r.raw.key.start_ms = 1000;
        r.read_rate = Some(7.0);
        assert_eq!(cell(ProcColumn::Cpu, &r, 0, 8, CpuScale::Machine), CellValue::Percent(50.0));
        assert_eq!(cell(ProcColumn::Cpu, &r, 0, 8, CpuScale::PerCore), CellValue::Percent(400.0));
        assert_eq!(cell(ProcColumn::Memory, &r, 0, 8, CpuScale::Machine), CellValue::Bytes(1024));
        assert_eq!(cell(ProcColumn::Gpu, &r, 0, 8, CpuScale::Machine), CellValue::Missing);
        assert_eq!(cell(ProcColumn::DiskRead, &r, 0, 8, CpuScale::Machine), CellValue::Rate(7.0));
        assert_eq!(cell(ProcColumn::Uptime, &r, 61_000, 8, CpuScale::Machine), CellValue::Seconds(60));
        assert_eq!(cell(ProcColumn::Name, &r, 0, 8, CpuScale::Machine), CellValue::Text("x".into()));
    }

    fn family() -> Frame {
        frame_of(vec![
            row(1, None, "init", 0.0, 10),
            row(2, Some(1), "shell", 3.0, 20),
            row(3, Some(2), "vim", 2.0, 30),
            row(4, Some(2), "cargo", 9.0, 40),
            row(5, Some(1), "daemon", 1.0, 50),
        ])
    }

    #[test]
    fn tree_nests_and_sorts_siblings() {
        let f = family();
        let rows = build(&f, &nofilter(), &opts(ProcView::Tree), &none());
        assert_eq!(names(&f, &rows), ["init", " shell", "  cargo", "  vim", " daemon"]);
        assert!(rows[0].has_children);
        assert!(!rows[2].has_children);
        assert!(rows[0].last_sibling, "the only root");
        assert!(!rows[1].last_sibling);
        assert!(rows[4].last_sibling);
        assert_eq!(rows[2].ancestors_last, 0b01, "init was last; shell was not");
        assert_eq!(rows[3].depth, 2);
    }

    #[test]
    fn tree_handles_missing_parent_self_parent_and_pid_reuse() {
        let mut young_parent = row(10, None, "young", 0.0, 0);
        young_parent.raw.key.start_ms = 500;
        let mut child = row(11, Some(10), "orphan", 0.0, 0);
        child.raw.key.start_ms = 100;
        let f =
            frame_of(vec![row(1, Some(999), "lost", 0.0, 0), row(2, Some(2), "selfish", 0.0, 0), young_parent, child]);
        let mut o = opts(ProcView::Tree);
        o.sort_by = ProcColumn::Pid;
        o.descending = false;
        let rows = build(&f, &nofilter(), &o, &none());
        assert_eq!(names(&f, &rows), ["lost", "selfish", "young", "orphan"], "all roots");
        assert!(rows.iter().all(|r| r.depth == 0));
    }

    #[test]
    fn tree_survives_cycles() {
        let f = frame_of(vec![
            row(1, Some(3), "a", 0.0, 0),
            row(2, Some(1), "b", 0.0, 0),
            row(3, Some(2), "c", 0.0, 0),
            row(4, Some(3), "hanger", 0.0, 0),
            row(5, Some(5), "self", 0.0, 0),
        ]);
        let mut o = opts(ProcView::Tree);
        o.sort_by = ProcColumn::Pid;
        o.descending = false;
        let rows = build(&f, &nofilter(), &o, &none());
        assert_eq!(rows.len(), 5, "every process appears exactly once: {:?}", names(&f, &rows));
        let mut pids: Vec<usize> = rows
            .iter()
            .map(|r| match r.kind {
                RowKind::Proc(i) => i,
                _ => 99,
            })
            .collect();
        pids.sort_unstable();
        assert_eq!(pids, [0, 1, 2, 3, 4]);
        assert!(rows.iter().any(|r| r.depth > 0));
        // Also with a filter walking up the (now cut) chain.
        let q = Filter::parse("hanger", false).unwrap();
        assert!(build(&f, &q, &o, &none()).len() >= 2);
    }

    #[test]
    fn tree_keeps_parents_of_matches_dimmed() {
        let f = family();
        let q = Filter::parse("vim", false).unwrap();
        let rows = build(&f, &q, &opts(ProcView::Tree), &none());
        assert_eq!(names(&f, &rows), ["init", " shell", "  vim"]);
        assert_eq!(rows.iter().map(|r| r.matched).collect::<Vec<_>>(), [false, false, true]);
        assert!(rows[2].last_sibling, "cargo is filtered out, so vim is the last sibling");
    }

    #[test]
    fn tree_without_keep_parents_flattens_matches() {
        let f = family();
        let q = Filter::parse("vim|cargo", false).unwrap();
        let mut o = opts(ProcView::Tree);
        o.keep_parents = false;
        let rows = build(&f, &q, &o, &none());
        assert_eq!(names(&f, &rows), ["cargo", "vim"]);
        assert!(rows.iter().all(|r| r.depth == 0 && r.matched));
    }

    #[test]
    fn collapsed_subtree_hides_children_and_has_totals() {
        let f = family();
        let mut set = HashSet::new();
        set.insert(RowId::Proc(f.procs[1].raw.key)); // shell
        let rows = build(&f, &nofilter(), &opts(ProcView::Tree), &set);
        assert_eq!(names(&f, &rows), ["init", " shell", " daemon"]);
        let shell = &rows[1];
        assert!(shell.collapsed);
        assert!(shell.has_children);
        let t = shell.totals.unwrap();
        assert_eq!(t.count, 3);
        assert_eq!(t.cpu_percent, 14.0);
        assert_eq!(t.rss, 90);
        assert_eq!(cell_total(ProcColumn::Memory, &t, 1, CpuScale::PerCore), CellValue::Bytes(90));
        assert_eq!(cell_total(ProcColumn::Name, &t, 1, CpuScale::PerCore), CellValue::Missing);
        assert_eq!(rows[0].totals, None);
    }

    #[test]
    fn collapsing_a_leaf_changes_nothing() {
        let f = family();
        let mut set = HashSet::new();
        set.insert(RowId::Proc(f.procs[2].raw.key));
        let rows = build(&f, &nofilter(), &opts(ProcView::Tree), &set);
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|r| !r.collapsed));
    }

    #[test]
    fn hidden_parents_make_their_children_roots() {
        let mut kernel = row(1, None, "kthreadd", 0.0, 0);
        kernel.raw.kernel = true;
        let f = frame_of(vec![kernel, row(2, Some(1), "worker", 0.0, 0)]);
        let rows = build(&f, &nofilter(), &opts(ProcView::Tree), &none());
        assert_eq!(names(&f, &rows), ["worker"]);
        assert_eq!(rows[0].depth, 0);
    }

    fn grouped() -> Frame {
        let mut rows =
            vec![row(1, None, "chrome", 5.0, 100), row(2, None, "chrome", 7.0, 200), row(3, None, "code", 20.0, 50)];
        rows[0].raw.user = Some(Arc::from("me"));
        rows[1].raw.user = Some(Arc::from("you"));
        rows[0].raw.handles = Some(5);
        let mut idle = row(0, None, "chrome", 90.0, 1 << 40);
        idle.raw.idle = true;
        rows.push(idle);
        frame_of(rows)
    }

    #[test]
    fn by_name_groups_with_totals_and_sorts_groups_by_the_sum() {
        let f = grouped();
        let rows = build(&f, &nofilter(), &opts(ProcView::ByName), &none());
        assert_eq!(names(&f, &rows), ["[code 1]", " code", "[chrome 2]", " chrome", " chrome"]);
        let chrome = rows[2].totals.unwrap();
        assert_eq!(chrome.count, 2, "the idle process is left out of group totals");
        assert_eq!(chrome.cpu_percent, 12.0);
        assert_eq!(chrome.rss, 300);
        assert_eq!(chrome.handles, Some(5));
        assert_eq!(chrome.gpu_percent, None);
        let mut o = opts(ProcView::ByName);
        o.sort_by = ProcColumn::Memory;
        assert_eq!(names(&f, &build(&f, &nofilter(), &o, &none()))[0], "[chrome 2]");
    }

    #[test]
    fn by_user_groups_and_collapse_by_label() {
        let f = grouped();
        let mut o = opts(ProcView::ByUser);
        o.sort_by = ProcColumn::User;
        o.descending = false;
        let rows = build(&f, &nofilter(), &o, &none());
        assert_eq!(names(&f, &rows), ["[me 1]", " chrome", "[you 1]", " chrome", "[ 1]", " code"]);
        let mut set = HashSet::new();
        set.insert(RowId::Group("me".into()));
        let rows = build(&f, &nofilter(), &o, &set);
        assert_eq!(names(&f, &rows), ["[me 1]", "[you 1]", " chrome", "[ 1]", " code"]);
        assert!(rows[0].collapsed);
        assert!(rows[0].totals.is_some());
    }

    #[test]
    fn a_filter_limits_group_members_and_totals() {
        let f = grouped();
        let q = Filter::parse("pid=2", false).unwrap();
        let rows = build(&f, &q, &opts(ProcView::ByName), &none());
        assert_eq!(names(&f, &rows), ["[chrome 1]", " chrome"]);
        assert_eq!(rows[0].totals.unwrap().rss, 200);
    }

    #[test]
    fn options_come_from_the_settings() {
        let cfg =
            Monitor { view: ProcView::Tree, sort_by: ProcColumn::Memory, show_kernel: true, ..Default::default() };
        let mut f = family();
        f.at_ms = 77;
        f.cpu.cores = vec![Default::default(); 4];
        let o = TableOpts::from_config(&cfg, Some("me".into()), &f);
        assert_eq!((o.view, o.sort_by, o.ncpu, o.now_ms), (ProcView::Tree, ProcColumn::Memory, 4, 77));
        assert!(o.show_kernel);
        assert_eq!(o.me.as_deref(), Some("me"));
    }

    #[test]
    fn row_identity_survives_reordering() {
        let f = family();
        let mut o = opts(ProcView::List);
        let rows = build(&f, &nofilter(), &o, &none());
        let id = rows[0].id(&f);
        assert_eq!(id, RowId::Proc(f.procs[3].raw.key));
        o.sort_by = ProcColumn::Memory;
        let rows2 = build(&f, &nofilter(), &o, &none());
        assert_eq!(position_of(&rows2, &f, &id), Some(1));
        assert_eq!(position_of(&rows2, &f, &RowId::Group("x".into())), None);
    }

    #[test]
    fn cache_rebuilds_only_when_something_changed() {
        let f = family();
        let mut c = TableCache::new();
        let mut o = opts(ProcView::List);
        let q = nofilter();
        let mut set = none();
        assert_eq!(c.rows(&f, 1, &q, &o, &set).len(), 5);
        c.rows(&f, 1, &q, &o, &set);
        o.now_ms = 999;
        c.rows(&f, 1, &q, &o, &set);
        assert_eq!(c.builds(), 1, "same inputs, and a moved clock alone, do not rebuild");
        c.rows(&f, 2, &q, &o, &set);
        assert_eq!(c.builds(), 2);
        o.sort_by = ProcColumn::Pid;
        c.rows(&f, 2, &q, &o, &set);
        assert_eq!(c.builds(), 3);
        let vim = Filter::parse("vim", false).unwrap();
        assert_eq!(c.rows(&f, 2, &vim, &o, &set).len(), 1);
        assert_eq!(c.builds(), 4);
        set.insert(RowId::Group("x".into()));
        c.rows(&f, 2, &vim, &o, &set);
        assert_eq!(c.builds(), 5);
        c.invalidate();
        c.rows(&f, 2, &vim, &o, &set);
        assert_eq!(c.builds(), 6);
    }

    /// `cargo test -p soyuz-sysmon --release -- --ignored --nocapture table_speed`
    #[test]
    #[ignore = "timing report; run in release"]
    fn table_speed_for_2000_processes() {
        let mut rows = Vec::with_capacity(2000);
        for i in 0..2000u32 {
            let parent = if i == 0 { None } else { Some(i / 8 + 1) };
            let mut r = row(
                i + 1,
                parent,
                ["chrome.exe", "code", "svchost.exe", "bash"][i as usize % 4],
                (i % 97) as f32,
                u64::from(i) * 4096,
            );
            r.raw.key.start_ms = 1000 + i64::from(i);
            r.raw.user = Some(Arc::from(["alice", "bob", "SYSTEM"][i as usize % 3]));
            r.raw.cmdline = Some(Arc::from(format!("/usr/bin/program --flag {i} --long-argument-with-text").as_str()));
            r.read_rate = Some(f64::from(i));
            rows.push(r);
        }
        let f = frame_of(rows);
        let q = Filter::parse("program -xyz cpu>1", false).unwrap();
        for (label, view, filter) in [
            ("list", ProcView::List, nofilter()),
            ("list filtered", ProcView::List, q.clone()),
            ("tree", ProcView::Tree, nofilter()),
            ("tree filtered", ProcView::Tree, q.clone()),
            ("by name", ProcView::ByName, nofilter()),
            ("by user", ProcView::ByUser, q.clone()),
        ] {
            let o = opts(view);
            let start = std::time::Instant::now();
            let mut n = 0;
            for _ in 0..100 {
                n += build(&f, &filter, &o, &none()).len();
            }
            println!("{label}: {:?} per build, {} rows", start.elapsed() / 100, n / 100);
        }
    }
}
