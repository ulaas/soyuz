//! Git integration: repository detection, branch and per-file status for the
//! views (GitStatus column, colored names, badges) and the status-bar branch chip.
//!
//! * **Detection** walks up from a folder looking for `.git` (a directory, or a file with
//!   `gitdir: …` for worktrees / submodules) on a short-lived worker thread; results are
//!   cached per folder.
//! * **Branch** comes from reading `HEAD` directly (no process spawn).
//! * **Status** runs `git --no-optional-locks status --porcelain=v2 -z --branch` in the
//!   repository root on a background thread with a timeout. Results are cached per
//!   repository and refreshed when a watcher event touches a folder inside the repo, or
//!   when the cache is older than `[git] refresh_secs` while the repo is on screen.
//!
//! All state is UI-thread-local (like `services::search`); the painting code calls the
//! cheap lookups [`status_of`] / [`summary_for`] every frame.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};

/// Kill `git status` after this long (huge repos, network drives, hung credential helpers).
pub const STATUS_TIMEOUT: Duration = Duration::from_secs(15);
/// A repo counts as "on screen" for this long after the last lookup.
const TOUCH_WINDOW: Duration = Duration::from_secs(3);
/// Minimum time between two status runs of one repository.
const MIN_INTERVAL: Duration = Duration::from_millis(1500);

/// Summarized status of one path, ordered by display priority (highest last).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitCode {
    Ignored,
    Untracked,
    TypeChange,
    Renamed,
    Added,
    Deleted,
    Modified,
    Conflicted,
}

impl GitCode {
    pub fn letter(self) -> &'static str {
        match self {
            GitCode::Ignored => "!",
            GitCode::Untracked => "?",
            GitCode::TypeChange => "T",
            GitCode::Renamed => "R",
            GitCode::Added => "A",
            GitCode::Deleted => "D",
            GitCode::Modified => "M",
            GitCode::Conflicted => "U",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GitCode::Ignored => tr!("services-git-ignored"),
            GitCode::Untracked => tr!("services-git-untracked"),
            GitCode::TypeChange => tr!("services-git-type-changed"),
            GitCode::Renamed => tr!("services-git-renamed"),
            GitCode::Added => tr!("services-git-added"),
            GitCode::Deleted => tr!("services-git-deleted"),
            GitCode::Modified => tr!("services-git-modified"),
            GitCode::Conflicted => tr!("services-git-conflict"),
        }
    }

    fn from_xy(x: u8, y: u8) -> GitCode {
        let c = if y != b'.' { y } else { x };
        match c {
            b'A' => GitCode::Added,
            b'D' => GitCode::Deleted,
            b'R' | b'C' => GitCode::Renamed,
            b'T' => GitCode::TypeChange,
            b'U' => GitCode::Conflicted,
            _ => GitCode::Modified,
        }
    }
}

/// One changed path from `git status --porcelain=v2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    /// Index (staged) status letter, `.` when unchanged.
    pub x: u8,
    /// Work-tree status letter, `.` when unchanged.
    pub y: u8,
    pub code: GitCode,
    /// Source path of a rename / copy.
    pub orig: Option<String>,
}

impl FileStatus {
    pub fn staged(&self) -> bool {
        self.x != b'.' && !matches!(self.code, GitCode::Untracked | GitCode::Ignored | GitCode::Conflicted)
    }
}

/// Parsed output of `git status --porcelain=v2 -z --branch`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Porcelain {
    pub branch: Option<String>,
    pub oid: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// `(path relative to the repo root with '/' separators, status)`. Untracked and
    /// ignored folders end with `/`.
    pub entries: Vec<(String, FileStatus)>,
}

/// Parse `git status --porcelain=v2 -z [--branch]` output.
pub fn parse_porcelain_v2(bytes: &[u8]) -> Porcelain {
    let mut out = Porcelain::default();
    let mut fields = bytes.split(|b| *b == 0).peekable();
    while let Some(rec) = fields.next() {
        if rec.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(rec);
        let kind = rec[0];
        match kind {
            b'#' => {
                let mut it = text[1..].trim().splitn(2, ' ');
                let key = it.next().unwrap_or("");
                let val = it.next().unwrap_or("").trim();
                match key {
                    "branch.head" if val != "(detached)" => out.branch = Some(val.to_string()),
                    "branch.oid" if val != "(initial)" => out.oid = Some(val.to_string()),
                    "branch.upstream" => out.upstream = Some(val.to_string()),
                    "branch.ab" => {
                        for part in val.split_whitespace() {
                            if let Some(n) = part.strip_prefix('+') {
                                out.ahead = n.parse().unwrap_or(0);
                            } else if let Some(n) = part.strip_prefix('-') {
                                out.behind = n.parse().unwrap_or(0);
                            }
                        }
                    }
                    _ => {}
                }
            }
            b'1' | b'2' | b'u' => {
                // 1 XY sub mH mI mW hH hI path
                // 2 XY sub mH mI mW hH hI Xscore path \0 origPath
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                let n_before_path = match kind {
                    b'1' => 8,
                    b'2' => 9,
                    _ => 10,
                };
                let mut parts = text.splitn(n_before_path + 1, ' ');
                let _kind = parts.next();
                let xy = parts.next().unwrap_or("..").as_bytes().to_vec();
                let path = parts.nth(n_before_path - 2).unwrap_or("").to_string();
                let (x, y) = (xy.first().copied().unwrap_or(b'.'), xy.get(1).copied().unwrap_or(b'.'));
                let code = if kind == b'u' { GitCode::Conflicted } else { GitCode::from_xy(x, y) };
                let orig =
                    if kind == b'2' { fields.next().map(|o| String::from_utf8_lossy(o).into_owned()) } else { None };
                if !path.is_empty() {
                    out.entries.push((path, FileStatus { x, y, code, orig }));
                }
            }
            b'?' | b'!' => {
                let path = text.get(2..).unwrap_or("").to_string();
                let code = if kind == b'?' { GitCode::Untracked } else { GitCode::Ignored };
                if !path.is_empty() {
                    out.entries.push((path, FileStatus { x: kind, y: kind, code, orig: None }));
                }
            }
            _ => {}
        }
    }
    out
}

/// Where a repository lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoLocation {
    /// Work tree root.
    pub root: PathBuf,
    /// The git directory (`.git`, or the target of a `gitdir:` file).
    pub git_dir: PathBuf,
}

/// Walk up from `dir` looking for `.git`. Blocking (a few `stat`s); run off the UI thread.
pub fn discover(dir: &Path) -> Option<RepoLocation> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        let dot = d.join(".git");
        if let Ok(md) = std::fs::metadata(&dot) {
            if md.is_dir() {
                if dot.join("HEAD").is_file() {
                    return Some(RepoLocation { root: d.to_path_buf(), git_dir: dot });
                }
            } else if md.is_file()
                && let Ok(text) = std::fs::read_to_string(&dot)
                && let Some(target) = text.lines().next().and_then(|l| l.strip_prefix("gitdir:"))
            {
                let t = PathBuf::from(target.trim());
                let git_dir = if t.is_absolute() { t } else { d.join(t) };
                return Some(RepoLocation { root: d.to_path_buf(), git_dir });
            }
        }
        cur = d.parent();
    }
    None
}

/// Branch name from `HEAD` content (`ref: refs/heads/x` → `x`; detached → short oid).
pub fn parse_head(text: &str) -> Option<String> {
    let t = text.trim();
    if let Some(r) = t.strip_prefix("ref:") {
        let r = r.trim();
        return Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string());
    }
    (t.len() >= 7 && t.chars().all(|c| c.is_ascii_hexdigit())).then(|| t[..7].to_string())
}

/// Read the current branch without spawning git.
pub fn read_branch(git_dir: &Path) -> Option<String> {
    std::fs::read_to_string(git_dir.join("HEAD")).ok().and_then(|t| parse_head(&t))
}

// Hardening. The folder being browsed may be an untrusted repository (a
// downloaded archive, a USB stick, a network share), and git reads `.git/config` from it.
// What `git status` can be made to *execute* from repo-controlled config, and how each is
// neutralised here:
//
// * `core.fsmonitor` (a hook command)                → `-c core.fsmonitor=false`.
// * clean filters (`filter.<drv>.clean` / `.process`), selected by the in-tree
//   `.gitattributes` and run when a stat-dirty file has to be re-hashed → every driver
//   defined in repo-controlled config is overridden on the command line with empty
//   commands and `required=false` (git treats an empty command as "no filter"). Command
//   line `-c` has the highest precedence. Drivers from the user's own global config
//   (e.g. git-lfs) are left alone; system config is skipped (`GIT_CONFIG_NOSYSTEM`).
//   The in-tree `.gitattributes` cannot be disabled, hence overriding the drivers.
// * `post-index-change` hook (runs when the index is rewritten) → `--no-optional-locks`
//   / `GIT_OPTIONAL_LOCKS=0`: status never writes the index, so it never fires. No other
//   hook is run by `status`.
// * submodules: status recurses by running git *inside each submodule*, whose config we
//   have not inspected                                 → `--ignore-submodules=all`.
// * lazy fetch in partial clones (missing blobs fetched via the repo's remote URL, `ext::`
//   transports, `core.sshCommand`)                     → `GIT_NO_LAZY_FETCH=1`,
//   `-c protocol.allow=never`.
// * pager (`pager.status` + `core.pager`)            → `--no-pager`, `GIT_PAGER=cat`
//   (stdout is a pipe anyway).
// * `core.untrackedCache` would write into `.git`   → `-c core.untrackedCache=false`.
//
// Diff drivers / `textconv`, credential helpers, editors and GPG are not used by `status`.
// git's own `safe.directory` ownership check stays in force. Residual risk: a future git
// release adding another config-driven command to `status`; a driver name that cannot be
// expressed as `-c` makes us refuse to run status at all.

/// Apply the repository-independent hardening flags and environment.
fn hardened(cmd: &mut Command, root: &Path) {
    cmd.current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_PAGER", "cat")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_EXTERNAL_DIFF")
        .stdin(Stdio::null());
    cmd.args([
        "--no-pager",
        "--no-optional-locks",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.untrackedCache=false",
        "-c",
        "protocol.allow=never",
        "-c",
        "core.quotepath=off",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Parse `git config [--show-scope] --name-only --get-regexp ^filter\.` output into the
/// driver names to neutralise (everything not from the user's global/system config).
/// `None` when a name cannot be passed back safely via `-c`.
fn parse_filter_drivers(out: &str) -> Option<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    for line in out.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        // Without `--show-scope` (old git) every driver counts as untrusted.
        let (scope, key) = line.split_once('\t').unwrap_or(("local", line));
        if scope == "global" || scope == "system" {
            continue;
        }
        let Some(rest) = key.strip_prefix("filter.") else { continue };
        let Some((name, _var)) = rest.rsplit_once('.') else { continue };
        if name.is_empty() || name.contains(['=', '\n', '\r', '\0']) {
            return None;
        }
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    Some(names)
}

fn config_filter_keys(root: &Path, show_scope: bool) -> std::io::Result<std::process::Output> {
    let mut cmd = Command::new("git");
    hardened(&mut cmd, root);
    cmd.arg("config");
    if show_scope {
        cmd.arg("--show-scope");
    }
    cmd.args(["--name-only", "--get-regexp", r"^filter\."]).stdout(Stdio::piped()).stderr(Stdio::null());
    cmd.output()
}

/// Filter drivers defined in repo-controlled config. Reading config runs no commands.
fn repo_filter_drivers(root: &Path) -> Result<Vec<String>, String> {
    let mut out = config_filter_keys(root, true).map_err(|e| tr!("services-git-unavailable", error = e.to_string()))?;
    // Exit code 1 = no matching keys; anything else may be a git older than 2.26 that
    // lacks `--show-scope`: retry without it (then every driver is neutralised).
    if !out.status.success() && out.status.code() != Some(1) {
        out = config_filter_keys(root, false).map_err(|e| tr!("services-git-unavailable", error = e.to_string()))?;
    }
    parse_filter_drivers(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| tr!("services-git-unusual-filter").to_string())
}

/// Build the hardened `git status` command for `root` (see the notes above).
fn status_command(root: &Path) -> Result<Command, String> {
    let drivers = repo_filter_drivers(root)?;
    let mut cmd = Command::new("git");
    hardened(&mut cmd, root);
    for d in &drivers {
        for (var, val) in [("clean", ""), ("smudge", ""), ("process", ""), ("required", "false")] {
            cmd.arg("-c").arg(format!("filter.{d}.{var}={val}"));
        }
    }
    cmd.args([
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--untracked-files=normal",
        "--ignored=matching",
        "--ignore-submodules=all",
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    Ok(cmd)
}

/// Run `git status` in `root` with a timeout. Blocking; run off the UI thread.
pub fn run_status(root: &Path, timeout: Duration) -> Result<Porcelain, String> {
    let mut cmd = status_command(root)?;
    let mut child = cmd.spawn().map_err(|e| tr!("services-git-unavailable", error = e.to_string()))?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(tr!("services-git-timeout").into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let out = reader.join().unwrap_or_default();
    let err = err_reader.join().unwrap_or_default();
    if !status.success() {
        return Err(err.lines().next().unwrap_or(tr!("services-git-failed")).to_string());
    }
    Ok(parse_porcelain_v2(&out))
}

/// Per-repository status index used for lookups.
#[derive(Clone, Debug, Default)]
pub struct StatusIndex {
    files: HashMap<String, FileStatus>,
    /// Aggregated status of folders containing changes.
    dirs: HashMap<String, GitCode>,
    /// Untracked / ignored folders (everything below inherits).
    whole_dirs: HashMap<String, GitCode>,
    pub changed: usize,
    pub staged: usize,
    pub untracked: usize,
    pub conflicts: usize,
}

impl StatusIndex {
    pub fn build(p: &Porcelain) -> Self {
        let mut ix = StatusIndex::default();
        for (path, st) in &p.entries {
            match st.code {
                GitCode::Ignored => {}
                GitCode::Untracked => ix.untracked += 1,
                GitCode::Conflicted => ix.conflicts += 1,
                _ => ix.changed += 1,
            }
            if st.staged() {
                ix.staged += 1;
            }
            if let Some(dir) = path.strip_suffix('/') {
                ix.whole_dirs.insert(dir.to_string(), st.code);
                if st.code != GitCode::Ignored {
                    ix.bump_ancestors(dir, st.code);
                }
                continue;
            }
            if st.code != GitCode::Ignored {
                ix.bump_ancestors(path, st.code);
            }
            ix.files.insert(path.clone(), st.clone());
        }
        ix
    }

    fn bump_ancestors(&mut self, path: &str, code: GitCode) {
        let mut cur = path;
        while let Some(i) = cur.rfind('/') {
            cur = &cur[..i];
            let e = self.dirs.entry(cur.to_string()).or_insert(code);
            if code > *e {
                *e = code;
            }
        }
    }

    /// Status of `rel` ('/'-separated, relative to the root).
    pub fn lookup(&self, rel: &str, is_dir: bool) -> Option<GitCode> {
        if is_dir {
            if let Some(c) = self.whole_dirs.get(rel) {
                return Some(*c);
            }
            if let Some(c) = self.dirs.get(rel) {
                return Some(*c);
            }
        } else if let Some(s) = self.files.get(rel) {
            return Some(s.code);
        }
        // Inside an untracked / ignored folder?
        let mut cur = rel;
        while let Some(i) = cur.rfind('/') {
            cur = &cur[..i];
            if let Some(c) = self.whole_dirs.get(cur) {
                return Some(*c);
            }
        }
        None
    }

    pub fn file(&self, rel: &str) -> Option<&FileStatus> {
        self.files.get(rel)
    }
}

/// What the status bar shows for the current folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSummary {
    pub root: PathBuf,
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changed: usize,
    pub staged: usize,
    pub untracked: usize,
    pub conflicts: usize,
    /// A status run is in flight.
    pub busy: bool,
    pub error: Option<String>,
    /// Status has been loaded at least once.
    pub loaded: bool,
}

enum Reply {
    Discovered { dir: PathBuf, repo: Option<RepoLocation>, branch: Option<String> },
    Status { root: PathBuf, result: Result<Porcelain, String>, branch: Option<String> },
}

struct Repo {
    loc: RepoLocation,
    branch: Option<String>,
    porcelain: Porcelain,
    index: StatusIndex,
    loaded_at: Option<Instant>,
    running: bool,
    stale: bool,
    touched: Instant,
    error: Option<String>,
}

struct Registry {
    tx: Sender<Reply>,
    rx: Receiver<Reply>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Folder → repository root (None = not in a repo).
    dir_root: HashMap<PathBuf, Option<PathBuf>>,
    discovering: HashSet<PathBuf>,
    repos: HashMap<PathBuf, Repo>,
    enabled: bool,
}

impl Default for Registry {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Registry {
            tx,
            rx,
            wake: None,
            dir_root: HashMap::new(),
            discovering: HashSet::new(),
            repos: HashMap::new(),
            enabled: true,
        }
    }
}

thread_local! {
    static REG: RefCell<Registry> = RefCell::new(Registry::default());
}

fn key(p: &Path) -> PathBuf {
    soyuz_fs::strip_verbatim(p)
}

/// Install the repaint hook (idempotent; called every frame).
pub fn init(wake: &crate::host::Wake, enabled: bool) {
    REG.with_borrow_mut(|r| {
        if r.wake.is_none() {
            r.wake = Some(wake.clone());
        }
        r.enabled = enabled;
    });
}

/// Drain worker replies and schedule refreshes (`refresh_secs` = 0 disables the timer).
/// Returns `true` when a refresh is waiting (the caller schedules a repaint).
pub fn pump(refresh_secs: u64) -> bool {
    REG.with_borrow_mut(|r| {
        while let Ok(reply) = r.rx.try_recv() {
            match reply {
                Reply::Discovered { dir, repo, branch } => {
                    r.discovering.remove(&dir);
                    let root = repo.as_ref().map(|l| key(&l.root));
                    r.dir_root.insert(dir, root.clone());
                    if let (Some(loc), Some(root)) = (repo, root)
                        && !r.repos.contains_key(&root)
                    {
                        r.repos.insert(
                            root,
                            Repo {
                                loc,
                                branch,
                                porcelain: Porcelain::default(),
                                index: StatusIndex::default(),
                                loaded_at: None,
                                running: false,
                                stale: true,
                                touched: Instant::now(),
                                error: None,
                            },
                        );
                    }
                }
                Reply::Status { root, result, branch } => {
                    if let Some(repo) = r.repos.get_mut(&root) {
                        repo.running = false;
                        repo.loaded_at = Some(Instant::now());
                        if branch.is_some() {
                            repo.branch = branch;
                        }
                        match result {
                            Ok(p) => {
                                repo.index = StatusIndex::build(&p);
                                if p.branch.is_some() {
                                    repo.branch = p.branch.clone();
                                }
                                repo.porcelain = p;
                                repo.error = None;
                            }
                            Err(e) => repo.error = Some(e),
                        }
                    }
                }
            }
        }
        if !r.enabled {
            return false;
        }
        let now = Instant::now();
        let mut pending = false;
        let ttl = (refresh_secs > 0).then(|| Duration::from_secs(refresh_secs));
        let mut start = Vec::new();
        for (root, repo) in r.repos.iter_mut() {
            if repo.running || now.duration_since(repo.touched) > TOUCH_WINDOW {
                continue;
            }
            let expired = match (repo.loaded_at, ttl) {
                (None, _) => true,
                (Some(t), Some(ttl)) => now.duration_since(t) > ttl,
                _ => false,
            };
            // Watcher storms (builds) refresh at most every MIN_INTERVAL.
            let rested = repo.loaded_at.is_none_or(|t| now.duration_since(t) > MIN_INTERVAL);
            if (repo.stale && rested) || expired {
                repo.stale = false;
                repo.running = true;
                start.push((root.clone(), repo.loc.clone()));
            } else if repo.stale {
                pending = true;
            }
        }
        for (root, loc) in start {
            let tx = r.tx.clone();
            let wake = r.wake.clone();
            let spawned = std::thread::Builder::new().name("soyuz-git".into()).spawn(move || {
                let branch = read_branch(&loc.git_dir);
                let result = run_status(&loc.root, STATUS_TIMEOUT);
                let _ = tx.send(Reply::Status { root, result, branch });
                if let Some(w) = wake {
                    w();
                }
            });
            if spawned.is_err() {
                log::warn!("cannot spawn git status thread");
            }
        }
        pending
    })
}

/// Repository root for `dir` (cached), kicking off detection when unknown.
fn root_for(r: &mut Registry, dir: &Path) -> Option<PathBuf> {
    let k = key(dir);
    if let Some(v) = r.dir_root.get(&k) {
        return v.clone();
    }
    if r.discovering.insert(k.clone()) {
        let tx = r.tx.clone();
        let wake = r.wake.clone();
        let dir = k.clone();
        let spawned = std::thread::Builder::new().name("soyuz-git-discover".into()).spawn(move || {
            let repo = discover(&dir);
            let branch = repo.as_ref().and_then(|l| read_branch(&l.git_dir));
            let _ = tx.send(Reply::Discovered { dir, repo, branch });
            if let Some(w) = wake {
                w();
            }
        });
        if spawned.is_err() {
            r.discovering.remove(&k);
        }
    }
    None
}

/// Status of an entry (None = clean, unknown or not in a repo). Cheap; call per row.
pub fn status_of(path: &Path, is_dir: bool) -> Option<GitCode> {
    REG.with_borrow_mut(|r| {
        if !r.enabled {
            return None;
        }
        let parent = path.parent()?;
        let root = root_for(r, parent)?;
        let repo = r.repos.get_mut(&root)?;
        repo.touched = Instant::now();
        let p = key(path);
        let rel = p.strip_prefix(&root).ok()?;
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel.is_empty() || rel == ".git" {
            return None;
        }
        repo.index.lookup(&rel, is_dir)
    })
}

/// Repository summary for the folder `dir` (branch chip). Marks the repo as on screen.
pub fn summary_for(dir: &Path) -> Option<RepoSummary> {
    REG.with_borrow_mut(|r| {
        if !r.enabled {
            return None;
        }
        let root = root_for(r, dir)?;
        let repo = r.repos.get_mut(&root)?;
        repo.touched = Instant::now();
        Some(RepoSummary {
            root: repo.loc.root.clone(),
            branch: repo.branch.clone(),
            ahead: repo.porcelain.ahead,
            behind: repo.porcelain.behind,
            changed: repo.index.changed,
            staged: repo.index.staged,
            untracked: repo.index.untracked,
            conflicts: repo.index.conflicts,
            busy: repo.running,
            error: repo.error.clone(),
            loaded: repo.loaded_at.is_some(),
        })
    })
}

/// A watcher event touched `dir`: refresh the status of the repo containing it.
pub fn invalidate(dir: &Path) {
    REG.with_borrow_mut(|r| {
        let k = key(dir);
        if let Some(Some(root)) = r.dir_root.get(&k).cloned()
            && let Some(repo) = r.repos.get_mut(&root)
        {
            repo.stale = true;
            repo.touched = Instant::now();
        }
    });
}

/// Force a refresh of every known repository (Refresh action).
pub fn refresh_all() {
    REG.with_borrow_mut(|r| {
        r.dir_root.retain(|_, v| v.is_some());
        for repo in r.repos.values_mut() {
            repo.stale = true;
        }
    });
}

/// Full status record of a file (tooltips).
pub fn file_status(path: &Path) -> Option<FileStatus> {
    REG.with_borrow(|r| {
        let parent = key(path.parent()?);
        let root = r.dir_root.get(&parent)?.clone()?;
        let repo = r.repos.get(&root)?;
        let p = key(path);
        let rel = p.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/");
        repo.index.file(&rel).cloned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let recs: Vec<&[u8]> = vec![
            b"# branch.oid 1234567890abcdef1234567890abcdef12345678",
            b"# branch.head main",
            b"# branch.upstream origin/main",
            b"# branch.ab +2 -1",
            b"1 .M N... 100644 100644 100644 aaaaaaa bbbbbbb src/lib.rs",
            b"1 A. N... 000000 100644 100644 0000000 ccccccc docs/new file.md",
            b"2 R. N... 100644 100644 100644 ddddddd ddddddd R100 src/renamed.rs",
            b"src/old.rs",
            b"u UU N... 100644 100644 100644 100644 e1 e2 e3 conflict.txt",
            b"1 .D N... 100644 100644 000000 fffffff fffffff gone.txt",
            b"? notes.txt",
            b"? scratch/",
            b"! target/",
        ];
        let mut out = Vec::new();
        for r in recs {
            out.extend_from_slice(r);
            out.push(0);
        }
        out
    }

    #[test]
    fn parses_porcelain_v2() {
        let p = parse_porcelain_v2(&sample());
        assert_eq!(p.branch.as_deref(), Some("main"));
        assert_eq!(p.upstream.as_deref(), Some("origin/main"));
        assert_eq!((p.ahead, p.behind), (2, 1));
        assert_eq!(p.oid.as_deref(), Some("1234567890abcdef1234567890abcdef12345678"));
        let get = |path: &str| p.entries.iter().find(|(q, _)| q == path).map(|(_, s)| s.clone());
        assert_eq!(get("src/lib.rs").unwrap().code, GitCode::Modified);
        assert!(!get("src/lib.rs").unwrap().staged());
        let added = get("docs/new file.md").expect("paths with spaces");
        assert_eq!(added.code, GitCode::Added);
        assert!(added.staged());
        let ren = get("src/renamed.rs").unwrap();
        assert_eq!(ren.code, GitCode::Renamed);
        assert_eq!(ren.orig.as_deref(), Some("src/old.rs"));
        assert!(get("src/old.rs").is_none(), "orig path is not a separate entry");
        assert_eq!(get("conflict.txt").unwrap().code, GitCode::Conflicted);
        assert_eq!(get("gone.txt").unwrap().code, GitCode::Deleted);
        assert_eq!(get("notes.txt").unwrap().code, GitCode::Untracked);
        assert_eq!(get("scratch/").unwrap().code, GitCode::Untracked);
        assert_eq!(get("target/").unwrap().code, GitCode::Ignored);
        assert_eq!(p.entries.len(), 8);
    }

    #[test]
    fn detached_and_initial() {
        let p = parse_porcelain_v2(b"# branch.oid (initial)\0# branch.head (detached)\0");
        assert_eq!(p.branch, None);
        assert_eq!(p.oid, None);
        assert!(p.entries.is_empty());
        assert_eq!(parse_head("ref: refs/heads/feature/x\n").as_deref(), Some("feature/x"));
        assert_eq!(parse_head("0123456789abcdef0123456789abcdef01234567").as_deref(), Some("0123456"));
        assert_eq!(parse_head("garbage"), None);
    }

    #[test]
    fn index_lookups_and_folder_aggregation() {
        let ix = StatusIndex::build(&parse_porcelain_v2(&sample()));
        assert_eq!(ix.lookup("src/lib.rs", false), Some(GitCode::Modified));
        assert_eq!(ix.lookup("src", true), Some(GitCode::Modified), "folder shows its strongest change");
        assert_eq!(ix.lookup("docs", true), Some(GitCode::Added));
        assert_eq!(ix.lookup("scratch", true), Some(GitCode::Untracked));
        assert_eq!(ix.lookup("scratch/deep/a.txt", false), Some(GitCode::Untracked), "inherits from folder");
        assert_eq!(ix.lookup("target/debug/x.exe", false), Some(GitCode::Ignored));
        assert_eq!(ix.lookup("README.md", false), None);
        assert_eq!(ix.conflicts, 1);
        assert_eq!(ix.untracked, 2);
        assert_eq!(ix.changed, 4);
        assert_eq!(ix.staged, 2);
    }

    #[test]
    fn filter_driver_parsing() {
        let out = "local\tfilter.evil.clean\nglobal\tfilter.lfs.process\nworktree\tfilter.a.b.smudge\n\
                   local\tfilter.evil.required\ncommand\tfilter.x.clean\n";
        assert_eq!(parse_filter_drivers(out).unwrap(), vec!["evil", "a.b", "x"]);
        // Old git without --show-scope: everything counts.
        assert_eq!(parse_filter_drivers("filter.lfs.clean\n").unwrap(), vec!["lfs"]);
        assert!(parse_filter_drivers("local\tfilter.a=b.clean\n").is_none());
        assert_eq!(parse_filter_drivers("").unwrap(), Vec::<String>::new());
    }

    /// End-to-end: a hostile repo config must not get commands executed by our status run.
    #[test]
    fn status_does_not_run_repo_filters_or_fsmonitor() {
        let git = |dir: &Path, args: &[&str]| {
            Command::new("git")
                .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "core.autocrlf=false"])
                .args(args)
                .current_dir(dir)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("r");
        std::fs::create_dir_all(&repo).unwrap();
        if !git(&repo, &["init", "-q"]) {
            eprintln!("git not available; skipping");
            return;
        }
        std::fs::write(repo.join(".gitattributes"), "*.txt filter=evil\n").unwrap();
        std::fs::write(repo.join("a.txt"), "hi\n").unwrap();
        assert!(git(&repo, &["add", "-A"]));
        assert!(git(&repo, &["commit", "-qm", "x"]));
        let filter_marker = t.path().join("PWNED_FILTER");
        let fsm_marker = t.path().join("PWNED_FSMONITOR");
        let fm = filter_marker.to_string_lossy().replace('\\', "/");
        let mm = fsm_marker.to_string_lossy().replace('\\', "/");
        assert!(git(&repo, &["config", "filter.evil.clean", &format!("echo x > '{fm}'; cat")]));
        assert!(git(&repo, &["config", "filter.evil.required", "true"]));
        assert!(git(&repo, &["config", "core.fsmonitor", &format!("echo x > '{mm}'; true")]));
        // Make the file stat-dirty so git must re-hash it through the filter.
        // Same size, new mtime: only hashing (through the filter) can tell.
        let f = std::fs::OpenOptions::new().write(true).open(repo.join("a.txt")).unwrap();
        f.set_modified(std::time::SystemTime::now() + Duration::from_secs(120)).unwrap();
        drop(f);
        let st = run_status(&repo, STATUS_TIMEOUT);
        assert!(st.is_ok(), "hardened status must still work: {:?}", st.err());
        assert!(!filter_marker.exists(), "repo-configured clean filter was executed by git status");
        assert!(!fsm_marker.exists(), "repo-configured fsmonitor was executed by git status");
        // Control: a plain `git status` does run them (proves the setup is meaningful).
        let _ = git(&repo, &["status", "--porcelain"]);
        if !filter_marker.exists() || !fsm_marker.exists() {
            eprintln!(
                "control: plain git status ran filter={} fsmonitor={}",
                filter_marker.exists(),
                fsm_marker.exists()
            );
        }
    }

    #[test]
    fn discovers_repo_and_worktree_files() {
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/dev\n").unwrap();
        std::fs::create_dir_all(repo.join("a/b")).unwrap();
        let loc = discover(&repo.join("a/b")).unwrap();
        assert_eq!(loc.root, repo);
        assert_eq!(read_branch(&loc.git_dir).as_deref(), Some("dev"));
        // A worktree: `.git` is a file pointing elsewhere.
        let wt = t.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let gd = t.path().join("gitdirs/wt");
        std::fs::create_dir_all(&gd).unwrap();
        std::fs::write(gd.join("HEAD"), "ref: refs/heads/topic\n").unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();
        let loc = discover(&wt).unwrap();
        assert_eq!(loc.git_dir, gd);
        assert_eq!(read_branch(&loc.git_dir).as_deref(), Some("topic"));
    }
}
