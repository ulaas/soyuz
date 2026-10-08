//! SQLite storage: schema, row upserts / deletes, crawl bookkeeping and queries.
//!
//! Tables: `files` (one row per indexed entry; `path` unique, binary-collated so
//! `in:` / subtree deletes are index range scans), `names` (trigram FTS5 over
//! `files.name`, kept in sync by triggers), `contents` (contentless FTS5 of text file
//! bodies, rowid = `files.id`), `content_state`, `tags`, `roots` (crawl state per index
//! root), `volumes` (USN journal checkpoints) and `meta`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use soyuz_core::{Attributes, EntryInfo, EntryKind, LinkTarget};

use crate::query::unix_secs;
use crate::sql::{self, KIND_DIR, KIND_FILE, KIND_JUNCTION, KIND_OTHER, KIND_SYMLINK, Sql};

/// Bumped when the schema changes; an older database is rebuilt from scratch.
pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS files(
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    ext TEXT NOT NULL,
    kind INTEGER NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER,
    ctime INTEGER,
    attrs INTEGER NOT NULL,
    hidden INTEGER NOT NULL,
    frn INTEGER,
    gen INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS files_ext ON files(ext);
CREATE INDEX IF NOT EXISTS files_size ON files(size);
CREATE INDEX IF NOT EXISTS files_mtime ON files(mtime);
CREATE INDEX IF NOT EXISTS files_frn ON files(frn);
CREATE VIRTUAL TABLE IF NOT EXISTS names USING fts5(name, content='files', content_rowid='id', tokenize='trigram');
CREATE VIRTUAL TABLE IF NOT EXISTS contents USING fts5(body, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');
CREATE TABLE IF NOT EXISTS content_state(id INTEGER PRIMARY KEY, mtime INTEGER, size INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS tags(path TEXT PRIMARY KEY, tags TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS roots(path TEXT PRIMARY KEY, complete INTEGER NOT NULL, gen INTEGER NOT NULL, crawled_at INTEGER);
CREATE TABLE IF NOT EXISTS volumes(root TEXT PRIMARY KEY, journal_id INTEGER NOT NULL, next_usn INTEGER NOT NULL);
CREATE TRIGGER IF NOT EXISTS files_ai AFTER INSERT ON files BEGIN
    INSERT INTO names(rowid, name) VALUES (new.id, new.name);
END;
CREATE TRIGGER IF NOT EXISTS files_ad AFTER DELETE ON files BEGIN
    INSERT INTO names(names, rowid, name) VALUES ('delete', old.id, old.name);
    DELETE FROM contents WHERE rowid = old.id;
    DELETE FROM content_state WHERE id = old.id;
END;
CREATE TRIGGER IF NOT EXISTS files_au AFTER UPDATE OF name ON files WHEN old.name IS NOT new.name BEGIN
    INSERT INTO names(names, rowid, name) VALUES ('delete', old.id, old.name);
    INSERT INTO names(rowid, name) VALUES (new.id, new.name);
END;
"#;

/// Register `regexp(pattern, text)` (case-insensitive unless the pattern says `(?-i)`).
fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function("regexp", 2, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let re: Arc<regex::Regex> = ctx.get_or_create_aux(0, |v| -> Result<regex::Regex, regex::Error> {
            let pat = v.as_str().unwrap_or_default();
            regex::RegexBuilder::new(pat).case_insensitive(true).size_limit(1 << 20).build()
        })?;
        let text = ctx.get_raw(1).as_str().unwrap_or_default();
        Ok(re.is_match(text))
    })
}

/// Open (creating if needed) the read-write index database.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA temp_store=MEMORY;")?;
    let version: Option<i64> = conn
        .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| r.get::<_, String>(0))
        .optional()
        .unwrap_or(None)
        .and_then(|s| s.parse().ok());
    if version.is_some_and(|v| v != SCHEMA_VERSION) {
        log::info!("index schema {version:?} → {SCHEMA_VERSION}: rebuilding");
        reset(&conn)?;
    }
    conn.execute_batch(SCHEMA)?;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES('schema', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [SCHEMA_VERSION.to_string()],
    )?;
    register_functions(&conn)?;
    Ok(conn)
}

/// A read-only connection for queries (WAL lets it read while the crawler writes).
pub fn open_reader(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(Duration::from_secs(2))?;
    register_functions(&conn)?;
    Ok(conn)
}

/// Drop every table (rebuild).
pub fn reset(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS files_ai; DROP TRIGGER IF EXISTS files_ad; DROP TRIGGER IF EXISTS files_au;
         DROP TABLE IF EXISTS names; DROP TABLE IF EXISTS contents; DROP TABLE IF EXISTS content_state;
         DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS tags; DROP TABLE IF EXISTS roots;
         DROP TABLE IF EXISTS volumes; DROP TABLE IF EXISTS meta;",
    )
}

/// One `files` row.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub path: String,
    pub name: String,
    pub ext: String,
    pub kind: i64,
    pub size: i64,
    pub mtime: Option<i64>,
    pub ctime: Option<i64>,
    pub attrs: i64,
    pub hidden: bool,
    pub frn: Option<i64>,
}

pub fn kind_code(k: EntryKind) -> i64 {
    match k {
        EntryKind::File => KIND_FILE,
        EntryKind::Dir => KIND_DIR,
        EntryKind::Symlink { .. } => KIND_SYMLINK,
        EntryKind::Junction => KIND_JUNCTION,
        _ => KIND_OTHER,
    }
}

pub fn kind_from_code(c: i64) -> EntryKind {
    match c {
        KIND_FILE => EntryKind::File,
        KIND_DIR => EntryKind::Dir,
        KIND_SYMLINK => EntryKind::Symlink { target: LinkTarget::Unknown },
        KIND_JUNCTION => EntryKind::Junction,
        _ => EntryKind::Unknown,
    }
}

impl Row {
    /// Row for a listed entry; `hidden_above`: an ancestor folder is hidden.
    pub fn from_entry(e: &EntryInfo, hidden_above: bool) -> Row {
        let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| e.name.clone());
        let ext = if e.kind == EntryKind::File {
            e.extension().map(str::to_lowercase).unwrap_or_default()
        } else {
            String::new()
        };
        Row {
            path: e.path.to_string_lossy().into_owned(),
            name,
            ext,
            kind: kind_code(e.kind),
            size: if e.kind == EntryKind::File { e.size.min(i64::MAX as u64) as i64 } else { 0 },
            mtime: e.modified.map(unix_secs),
            ctime: e.created.map(unix_secs),
            attrs: i64::from(e.attrs.bits()),
            hidden: hidden_above || e.is_hidden(),
            frn: e.inode.map(|i| i as i64),
        }
    }
}

pub fn upsert(conn: &Connection, r: &Row, generation: i64) -> rusqlite::Result<()> {
    let mut st = conn.prepare_cached(
        "INSERT INTO files(path, name, ext, kind, size, mtime, ctime, attrs, hidden, frn, gen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(path) DO UPDATE SET name=excluded.name, ext=excluded.ext, kind=excluded.kind,
            size=excluded.size, mtime=excluded.mtime, ctime=excluded.ctime, attrs=excluded.attrs,
            hidden=excluded.hidden, frn=excluded.frn, gen=excluded.gen",
    )?;
    st.execute(params![r.path, r.name, r.ext, r.kind, r.size, r.mtime, r.ctime, r.attrs, r.hidden, r.frn, generation])?;
    Ok(())
}

/// Delete `path` and everything below it; returns the number of rows removed.
pub fn delete_tree(conn: &Connection, path: &Path) -> rusqlite::Result<usize> {
    let p = path.to_string_lossy();
    let (lo, hi) = sql::descendant_bounds(path);
    let mut st = conn.prepare_cached("DELETE FROM files WHERE path = ?1 OR (path >= ?2 AND path < ?3)")?;
    st.execute(params![p, lo, hi])
}

/// Paths of rows with this file id below `volume_root`.
pub fn paths_by_frn(conn: &Connection, frn: u64, volume_root: &Path) -> rusqlite::Result<Vec<PathBuf>> {
    let (lo, hi) = sql::descendant_bounds(volume_root);
    let mut st = conn.prepare_cached("SELECT path FROM files WHERE frn = ?1 AND path >= ?2 AND path < ?3")?;
    let rows = st.query_map(params![frn as i64, lo, hi], |r| r.get::<_, String>(0))?;
    rows.map(|r| r.map(PathBuf::from)).collect()
}

/// Delete rows below `root` that the crawl generation `gen` didn't see.
pub fn sweep(conn: &Connection, root: &Path, generation: i64) -> rusqlite::Result<usize> {
    let (lo, hi) = sql::descendant_bounds(root);
    conn.execute("DELETE FROM files WHERE path >= ?1 AND path < ?2 AND gen < ?3", params![lo, hi, generation])
}

/// `hidden` flag of an indexed folder.
pub fn hidden_of(conn: &Connection, path: &Path) -> rusqlite::Result<Option<bool>> {
    conn.prepare_cached("SELECT hidden FROM files WHERE path = ?1")?
        .query_row([path.to_string_lossy()], |r| r.get(0))
        .optional()
}

/// Kind code of an indexed path.
pub fn kind_of(conn: &Connection, path: &Path) -> rusqlite::Result<Option<i64>> {
    conn.prepare_cached("SELECT kind FROM files WHERE path = ?1")?
        .query_row([path.to_string_lossy()], |r| r.get(0))
        .optional()
}

/// Crawl state of an index root.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RootState {
    pub complete: bool,
    pub generation: i64,
    pub crawled_at: Option<i64>,
}

pub fn root_state(conn: &Connection, root: &Path) -> rusqlite::Result<Option<RootState>> {
    conn.query_row("SELECT complete, gen, crawled_at FROM roots WHERE path = ?1", [root.to_string_lossy()], |r| {
        Ok(RootState { complete: r.get(0)?, generation: r.get(1)?, crawled_at: r.get(2)? })
    })
    .optional()
}

pub fn set_root_state(conn: &Connection, root: &Path, s: RootState) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO roots(path, complete, gen, crawled_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET complete=excluded.complete, gen=excluded.gen, crawled_at=excluded.crawled_at",
        params![root.to_string_lossy(), s.complete, s.generation, s.crawled_at],
    )?;
    Ok(())
}

/// Forget roots that are no longer configured (and their rows).
pub fn prune_roots(conn: &Connection, keep: &[PathBuf]) -> rusqlite::Result<usize> {
    let known: Vec<String> =
        conn.prepare("SELECT path FROM roots")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut removed = 0;
    for k in known {
        let kp = PathBuf::from(&k);
        if !keep.contains(&kp) {
            // Rows below another configured root stay.
            if !keep.iter().any(|r| kp.starts_with(r)) {
                removed += delete_tree(conn, &kp)?;
            }
            conn.execute("DELETE FROM roots WHERE path = ?1", [&k])?;
        }
    }
    // Rows outside every root (e.g. roots edited while the app was closed).
    if keep.is_empty() {
        removed += conn.execute("DELETE FROM files", [])?;
    }
    Ok(removed)
}

/// USN checkpoint of a volume: `(journal_id, next_usn)`.
pub fn volume_checkpoint(conn: &Connection, volume: &Path) -> rusqlite::Result<Option<(u64, i64)>> {
    conn.query_row("SELECT journal_id, next_usn FROM volumes WHERE root = ?1", [volume.to_string_lossy()], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get(1)?))
    })
    .optional()
}

pub fn set_volume_checkpoint(conn: &Connection, volume: &Path, journal_id: u64, next_usn: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO volumes(root, journal_id, next_usn) VALUES (?1, ?2, ?3)
         ON CONFLICT(root) DO UPDATE SET journal_id=excluded.journal_id, next_usn=excluded.next_usn",
        params![volume.to_string_lossy(), journal_id as i64, next_usn],
    )?;
    Ok(())
}

pub fn clear_volume_checkpoints(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM volumes", [])?;
    Ok(())
}

/// `(files, folders)` in the index.
pub fn counts(conn: &Connection) -> rusqlite::Result<(u64, u64)> {
    conn.query_row(
        &format!(
            "SELECT IFNULL(SUM(kind <> {KIND_DIR} AND kind <> {KIND_JUNCTION}), 0),
                    IFNULL(SUM(kind = {KIND_DIR} OR kind = {KIND_JUNCTION}), 0) FROM files"
        ),
        [],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
    )
}

pub fn content_count(conn: &Connection) -> rusqlite::Result<u64> {
    conn.query_row("SELECT COUNT(*) FROM content_state", [], |r| r.get::<_, i64>(0)).map(|n| n as u64)
}

/// Replace the tag table: `(path, space-separated lower-case slugs)`.
pub fn set_tags(conn: &Connection, tags: &[(PathBuf, Vec<String>)]) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM tags", [])?;
    let mut st = conn.prepare_cached("INSERT OR REPLACE INTO tags(path, tags) VALUES (?1, ?2)")?;
    for (p, t) in tags {
        if t.is_empty() {
            continue;
        }
        let joined = format!(" {} ", t.iter().map(|s| s.to_lowercase()).collect::<Vec<_>>().join(" "));
        st.execute(params![p.to_string_lossy(), joined])?;
    }
    Ok(())
}

/// A file whose contents should be (re)indexed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentTodo {
    pub id: i64,
    pub path: PathBuf,
    pub size: i64,
    pub mtime: Option<i64>,
    /// An older body is stored (replace it rather than just insert).
    pub stale: bool,
}

/// Files whose contents should be (re)indexed.
pub fn content_todo(
    conn: &Connection,
    exts: &[String],
    max_size: u64,
    limit: usize,
) -> rusqlite::Result<Vec<ContentTodo>> {
    if exts.is_empty() {
        return Ok(Vec::new());
    }
    let ph = vec!["?"; exts.len()].join(", ");
    let text = format!(
        "SELECT f.id, f.path, f.size, f.mtime, c.id IS NOT NULL FROM files f LEFT JOIN content_state c ON c.id = f.id
         WHERE f.kind = {KIND_FILE} AND f.size <= ? AND f.ext IN ({ph})
           AND (c.id IS NULL OR c.size <> f.size OR c.mtime IS NOT f.mtime)
         LIMIT ?"
    );
    let mut params: Vec<rusqlite::types::Value> = vec![(max_size.min(i64::MAX as u64) as i64).into()];
    params.extend(exts.iter().map(|e| rusqlite::types::Value::Text(e.to_lowercase())));
    params.push((limit as i64).into());
    let mut st = conn.prepare_cached(&text)?;
    let rows = st.query_map(rusqlite::params_from_iter(params), |r| {
        Ok(ContentTodo {
            id: r.get(0)?,
            path: PathBuf::from(r.get::<_, String>(1)?),
            size: r.get(2)?,
            mtime: r.get(3)?,
            stale: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// Store a file body (or `None` for binary / unreadable files, which are only marked).
pub fn set_content(conn: &Connection, t: &ContentTodo, body: Option<&str>) -> rusqlite::Result<()> {
    let (id, size, mtime) = (t.id, t.size, t.mtime);
    if t.stale {
        conn.prepare_cached("DELETE FROM contents WHERE rowid = ?1")?.execute([id])?;
    }
    if let Some(b) = body {
        conn.prepare_cached("INSERT INTO contents(rowid, body) VALUES (?1, ?2)")?.execute(params![id, b])?;
    }
    conn.prepare_cached(
        "INSERT INTO content_state(id, mtime, size) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET mtime=excluded.mtime, size=excluded.size",
    )?
    .execute(params![id, mtime, size])?;
    Ok(())
}

/// Forget every indexed body (content indexing turned off).
pub fn clear_contents(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("DELETE FROM contents; DELETE FROM content_state;")
}

fn to_time(s: Option<i64>) -> Option<SystemTime> {
    let s = s?;
    if s >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(s as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(s.unsigned_abs()))
    }
}

/// Run a generated query; rows become [`EntryInfo`]s (name = file name).
pub fn run(conn: &Connection, q: &Sql) -> rusqlite::Result<Vec<EntryInfo>> {
    let mut st = conn.prepare(&q.text)?;
    let rows = st.query_map(rusqlite::params_from_iter(q.params.iter()), |r| {
        let path = PathBuf::from(r.get::<_, String>(0)?);
        let kind = kind_from_code(r.get(2)?);
        let mut e = EntryInfo::minimal(path, kind);
        e.name = r.get(1)?;
        e.size = r.get::<_, i64>(3)?.max(0) as u64;
        e.modified = to_time(r.get(4)?);
        e.created = to_time(r.get(5)?);
        e.attrs = Attributes::from_bits_truncate(r.get::<_, i64>(6)? as u32);
        e.inode = r.get::<_, Option<i64>>(7)?.map(|i| i as u64);
        Ok(e)
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parse;
    use crate::sql::{SqlOptions, select};

    fn entry(path: &str, kind: EntryKind, size: u64) -> EntryInfo {
        let mut e = EntryInfo::minimal(PathBuf::from(path), kind);
        e.size = size;
        e.modified = Some(SystemTime::now());
        e
    }

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("i.db")).unwrap();
        (dir, conn)
    }

    fn names(conn: &Connection, q: &str) -> Vec<String> {
        let o = SqlOptions { now: unix_secs(SystemTime::now()), include_hidden: true, ..Default::default() };
        let mut v: Vec<String> =
            run(conn, &select(&parse(q).unwrap(), &o)).unwrap().into_iter().map(|e| e.name).collect();
        v.sort();
        v
    }

    #[test]
    fn upsert_query_delete() {
        let (_d, c) = db();
        let sep = std::path::MAIN_SEPARATOR;
        let p = |s: &str| s.replace('/', &sep.to_string());
        for (path, kind, size) in [
            ("/r/docs", EntryKind::Dir, 0),
            ("/r/docs/Annual Report.pdf", EntryKind::File, 6 << 20),
            ("/r/docs/notes.txt", EntryKind::File, 10),
            ("/r/pics/cat.PNG", EntryKind::File, 2 << 20),
            ("/r/pics/dog.jpg", EntryKind::File, 9 << 20),
        ] {
            upsert(&c, &Row::from_entry(&entry(&p(path), kind, size), false), 1).unwrap();
        }
        assert_eq!(counts(&c).unwrap(), (4, 1));
        assert_eq!(names(&c, "report"), ["Annual Report.pdf"]);
        assert_eq!(names(&c, "REPORT"), ["Annual Report.pdf"], "trigram match is case-insensitive");
        assert_eq!(names(&c, "kind:image"), ["cat.PNG", "dog.jpg"]);
        assert_eq!(names(&c, "kind:image size:>5MB"), ["dog.jpg"]);
        assert_eq!(names(&c, "*.png"), ["cat.PNG"]);
        assert_eq!(names(&c, "ext:pdf OR ext:txt"), ["Annual Report.pdf", "notes.txt"]);
        assert_eq!(names(&c, "kind:folder"), ["docs"]);
        assert_eq!(names(&c, "path:pics -dog"), ["cat.PNG"]);
        assert_eq!(names(&c, "modified:<1d kind:file").len(), 4);
        assert!(names(&c, "modified:>1d").is_empty());
        assert_eq!(names(&c, "regex:^(cat|dog)\\."), ["cat.PNG", "dog.jpg"]);
        assert_eq!(names(&c, &format!("in:\"{}\"", p("/r/docs"))), ["Annual Report.pdf", "notes.txt"]);
        assert_eq!(names(&c, "og"), ["dog.jpg"], "short words scan names");

        // Rename (name change → FTS row updated).
        let mut e = entry(&p("/r/docs/notes.txt"), EntryKind::File, 11);
        e.name = "notes.txt".into();
        upsert(&c, &Row::from_entry(&e, false), 2).unwrap();
        assert_eq!(names(&c, "notes"), ["notes.txt"]);

        // Subtree delete.
        assert_eq!(delete_tree(&c, Path::new(&p("/r/docs"))).unwrap(), 3);
        assert_eq!(names(&c, "report"), Vec::<String>::new());
        assert_eq!(counts(&c).unwrap(), (2, 0));
        // `/r/pics` must not be touched by deleting `/r/pic`.
        assert_eq!(delete_tree(&c, Path::new(&p("/r/pic"))).unwrap(), 0);
    }

    #[test]
    fn sweep_hidden_tags_and_state() {
        let (_d, c) = db();
        let sep = std::path::MAIN_SEPARATOR;
        let root = PathBuf::from(format!("{sep}root"));
        let a = root.join("a.txt");
        let b = root.join(".hidden");
        upsert(&c, &Row::from_entry(&entry(&a.to_string_lossy(), EntryKind::File, 1), false), 1).unwrap();
        upsert(&c, &Row::from_entry(&entry(&b.to_string_lossy(), EntryKind::Dir, 0), false), 2).unwrap();
        assert_eq!(hidden_of(&c, &b).unwrap(), Some(true), "dot-folders are hidden");
        assert_eq!(kind_of(&c, &b).unwrap(), Some(KIND_DIR));
        let o = SqlOptions { include_hidden: false, ..Default::default() };
        assert_eq!(run(&c, &select(&parse("").unwrap(), &o)).unwrap().len(), 1);

        set_tags(&c, &[(a.clone(), vec!["red".into(), "blue".into()])]).unwrap();
        assert_eq!(names(&c, "tag:red"), ["a.txt"]);
        assert_eq!(names(&c, "tag:Blue"), ["a.txt"]);
        assert!(names(&c, "tag:re").is_empty(), "whole slugs only");

        assert_eq!(sweep(&c, &root, 2).unwrap(), 1, "gen 1 row was not seen by crawl 2");
        assert_eq!(counts(&c).unwrap(), (0, 1));

        assert_eq!(root_state(&c, &root).unwrap(), None);
        set_root_state(&c, &root, RootState { complete: true, generation: 2, crawled_at: Some(5) }).unwrap();
        assert_eq!(root_state(&c, &root).unwrap().unwrap().generation, 2);
        prune_roots(&c, &[]).unwrap();
        assert_eq!(root_state(&c, &root).unwrap(), None);
        assert_eq!(counts(&c).unwrap(), (0, 0));

        set_volume_checkpoint(&c, Path::new("C:\\"), 7, 99).unwrap();
        assert_eq!(volume_checkpoint(&c, Path::new("C:\\")).unwrap(), Some((7, 99)));
    }

    #[test]
    fn frn_lookup_and_contents() {
        let (_d, c) = db();
        let sep = std::path::MAIN_SEPARATOR;
        let vol = PathBuf::from(format!("{sep}v"));
        let mut e = entry(&vol.join("code.rs").to_string_lossy(), EntryKind::File, 20);
        e.inode = Some(77);
        upsert(&c, &Row::from_entry(&e, false), 1).unwrap();
        assert_eq!(paths_by_frn(&c, 77, &vol).unwrap(), vec![vol.join("code.rs")]);
        assert!(paths_by_frn(&c, 77, Path::new("/elsewhere")).unwrap().is_empty());

        let todo = content_todo(&c, &["rs".into()], 1000, 10).unwrap();
        assert_eq!(todo.len(), 1);
        assert!(!todo[0].stale);
        set_content(&c, &todo[0], Some("fn main() { println!(\"hello world\") }")).unwrap();
        assert!(content_todo(&c, &["rs".into()], 1000, 10).unwrap().is_empty(), "up to date");
        assert_eq!(content_count(&c).unwrap(), 1);
        assert_eq!(names(&c, "content:hello"), ["code.rs"]);
        assert_eq!(names(&c, "content:\"hello world\""), ["code.rs"]);
        assert!(names(&c, "content:\"world hello\"").is_empty());
        assert_eq!(names(&c, "-content:nothing"), ["code.rs"]);
        // Deleting the file drops its body too.
        delete_tree(&c, &vol.join("code.rs")).unwrap();
        assert_eq!(content_count(&c).unwrap(), 0);
        assert!(names(&c, "content:hello").is_empty());
    }

    #[test]
    fn schema_upgrade_rebuilds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("i.db");
        {
            let c = open(&path).unwrap();
            upsert(&c, &Row::from_entry(&entry("/x", EntryKind::File, 1), false), 1).unwrap();
            c.execute("UPDATE meta SET value='0' WHERE key='schema'", []).unwrap();
        }
        let c = open(&path).unwrap();
        assert_eq!(counts(&c).unwrap(), (0, 0));
        let r = open_reader(&path).unwrap();
        assert_eq!(counts(&r).unwrap(), (0, 0));
    }
}
