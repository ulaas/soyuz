//! Installed fonts as the OS lists them, for font pickers and family-name lookups.
//!
//! Windows: the `Fonts` registry key of the machine
//! (`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts`, file names relative to
//! `%WINDIR%\Fonts`) and of the user (`HKCU\…`, absolute paths into
//! `%LOCALAPPDATA%\Microsoft\Windows\Fonts`). Value names look like
//! `Segoe UI Semibold (TrueType)` or `Cambria & Cambria Math (TrueType)`; they are
//! returned raw, the app parses them. Read-only advapi32.
//!
//! Linux / BSD: a scan of the fontconfig directories (`/usr/share/fonts`,
//! `/usr/local/share/fonts`, `$XDG_DATA_HOME/fonts`, `~/.fonts` and every `<dir>` of
//! `/etc/fonts/fonts.conf` and its `conf.d` includes). Names come from each face's
//! `name` table (read with skrifa; only the table directory and the `name` table are
//! read from disk, also for `.ttc` / `.otc` collections). The result uses the Windows
//! value-name shape (`Full Name A & Full Name B (TrueType)`, one entry per file, faces
//! in collection order) so the app parses both the same way; [`InstalledFont::faces`]
//! has the details. Cached per file (path, size, mtime).
//!
//! macOS: CoreText's font file list (`CTFontManagerCopyAvailableFontURLs`: system fonts,
//! `/Library/Fonts`, `~/Library/Fonts`, activated fonts), each file's faces read with the
//! same `name`-table parser as Linux, so the result has the same shape (untested on
//! macOS). `.dfont` / resource-fork fonts have no readable `name` table and are skipped.
//!
//! Other platforms return nothing.

use std::path::PathBuf;

/// One installed font file: the display name (`Segoe UI Semibold (TrueType)`) and the
/// file it names (absolute).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledFont {
    pub name: String,
    pub file: PathBuf,
    /// From the per-user font key / folder.
    pub per_user: bool,
    /// The faces in the file, in collection order (Linux / macOS; empty on Windows, where the
    /// registry name is all there is).
    pub faces: Vec<FontFace>,
}

/// One face of a font file (from its `name` table).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontFace {
    /// Face index inside a `.ttc` / `.otc` collection (0 for single fonts).
    pub index: u32,
    /// Typographic family (`Noto Sans`), falling back to the legacy family.
    pub family: String,
    /// Typographic subfamily (`Bold Italic`), falling back to the legacy one.
    pub style: String,
    /// Full name (`Noto Sans Bold Italic`).
    pub full_name: String,
}

/// The system fonts folder (`%WINDIR%\Fonts`; `/usr/share/fonts` on Linux / BSD).
pub fn system_font_dir() -> PathBuf {
    if cfg!(windows) {
        let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
        return windir.join("Fonts");
    }
    if cfg!(target_os = "macos") {
        return PathBuf::from("/Library/Fonts");
    }
    PathBuf::from("/usr/share/fonts")
}

/// The per-user fonts folder (`%LOCALAPPDATA%\Microsoft\Windows\Fonts`;
/// `$XDG_DATA_HOME/fonts` on Linux / BSD).
pub fn user_font_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join(r"Microsoft\Windows\Fonts"));
    }
    if cfg!(target_os = "macos") {
        return Some(crate::path::home().join("Library/Fonts"));
    }
    Some(crate::launch::linux::host_data_home().join("fonts"))
}

/// Every installed font registered with the OS (machine first, then per-user).
pub fn installed() -> Vec<InstalledFont> {
    #[cfg(windows)]
    {
        use crate::sys::windows::registry::{Hive, string_values};
        const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts";
        let sys = system_font_dir();
        let user = user_font_dir();
        let mut out = Vec::new();
        for (hive, per_user) in [(Hive::LocalMachine, false), (Hive::CurrentUser, true)] {
            for (name, value) in string_values(hive, KEY) {
                let p = PathBuf::from(value.trim());
                let file = if p.is_absolute() {
                    p
                } else if per_user {
                    user.clone().unwrap_or_else(|| sys.clone()).join(p)
                } else {
                    sys.join(p)
                };
                out.push(InstalledFont { name, file, per_user, faces: Vec::new() });
            }
        }
        out
    }
    #[cfg(target_os = "linux")]
    {
        scan::installed()
    }
    #[cfg(target_os = "macos")]
    {
        mac::installed()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

/// The file and collection index of the face named `family` (a full name such as
/// `Noto Sans Bold`, or a family name, which picks its regular face). Uses the
/// [`installed`] list (Linux / BSD; Windows entries carry no face details).
pub fn find_face(family: &str) -> Option<(PathBuf, u32)> {
    find_in(&installed(), family)
}

/// [`find_face`] over a given list.
pub fn find_in(fonts: &[InstalledFont], family: &str) -> Option<(PathBuf, u32)> {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let want = norm(family);
    if want.is_empty() {
        return None;
    }
    let faces = || fonts.iter().flat_map(|f| f.faces.iter().map(move |x| (f, x)));
    let regular = |s: &str| matches!(norm(s).as_str(), "regular" | "book" | "normal" | "roman" | "");
    faces()
        .find(|(_, x)| norm(&x.full_name) == want)
        .or_else(|| faces().find(|(_, x)| norm(&x.family) == want && regular(&x.style)))
        .or_else(|| faces().find(|(_, x)| norm(&x.family) == want))
        .map(|(f, x)| (f.file.clone(), x.index))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
/// Faces of an in-memory font file (`name` table parsing; tests and fuzzing).
pub fn faces_of_bytes(data: &[u8]) -> Vec<FontFace> {
    names::faces(&mut std::io::Cursor::new(data)).unwrap_or_default()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
/// The per-face name lookup, reading only what is needed from a file.
mod names {
    use std::io::{Read, Seek, SeekFrom};

    use skrifa::raw::types::NameId;
    use skrifa::raw::{FontData, FontRead, tables::name::Name};

    use super::FontFace;

    /// Upper bound for one `name` table (they are a few KiB; CJK fonts reach ~100 KiB).
    const MAX_NAME: u32 = 4 * 1024 * 1024;
    /// Upper bound for faces in a collection.
    const MAX_FACES: u32 = 256;

    fn read_at<R: Read + Seek>(r: &mut R, off: u64, len: usize) -> Option<Vec<u8>> {
        r.seek(SeekFrom::Start(off)).ok()?;
        let mut v = vec![0u8; len];
        r.read_exact(&mut v).ok()?;
        Some(v)
    }

    fn u16be(b: &[u8], o: usize) -> Option<u16> {
        Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
    }

    fn u32be(b: &[u8], o: usize) -> Option<u32> {
        Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
    }

    /// The `name` table of the font whose table directory starts at `off`.
    fn name_table<R: Read + Seek>(r: &mut R, off: u64) -> Option<Vec<u8>> {
        let head = read_at(r, off, 12)?;
        let n = u16be(&head, 4)? as usize;
        let dir = read_at(r, off + 12, n * 16)?;
        for i in 0..n {
            let rec = &dir[i * 16..i * 16 + 16];
            if &rec[..4] == b"name" {
                let (o, len) = (u32be(rec, 8)?, u32be(rec, 12)?);
                if len > MAX_NAME {
                    return None;
                }
                return read_at(r, u64::from(o), len as usize);
            }
        }
        None
    }

    /// Best English (else any) string for `id`: Windows platform first, then Mac Roman.
    fn get(name: &Name<'_>, id: NameId) -> Option<String> {
        let data = name.string_data();
        let mut best: Option<(u8, String)> = None;
        for rec in name.name_record() {
            if rec.name_id() != id {
                continue;
            }
            let rank = match (rec.platform_id(), rec.language_id()) {
                (3, 0x0409) => 0,
                (3, _) => 2,
                (0, _) => 1,
                (1, 0) => 3,
                _ => 4,
            };
            if best.as_ref().is_some_and(|b| b.0 <= rank) {
                continue;
            }
            if let Ok(s) = rec.string(data) {
                let s: String = s.chars().collect();
                let s = s.trim().to_string();
                if !s.is_empty() {
                    best = Some((rank, s));
                }
            }
        }
        best.map(|b| b.1)
    }

    fn face(table: &[u8], index: u32) -> Option<FontFace> {
        let name = Name::read(FontData::new(table)).ok()?;
        let family = get(&name, NameId::TYPOGRAPHIC_FAMILY_NAME).or_else(|| get(&name, NameId::FAMILY_NAME))?;
        let style = get(&name, NameId::TYPOGRAPHIC_SUBFAMILY_NAME)
            .or_else(|| get(&name, NameId::SUBFAMILY_NAME))
            .unwrap_or_default();
        let full_name = get(&name, NameId::FULL_NAME).unwrap_or_else(|| {
            if style.is_empty() || style.eq_ignore_ascii_case("regular") {
                family.clone()
            } else {
                format!("{family} {style}")
            }
        });
        Some(FontFace { index, family, style, full_name })
    }

    /// Faces of a font file (`.ttf` / `.otf` / `.ttc` / `.otc`).
    pub(super) fn faces<R: Read + Seek>(r: &mut R) -> Option<Vec<FontFace>> {
        let head = read_at(r, 0, 12)?;
        if &head[..4] == b"ttcf" {
            let n = u32be(&head, 8)?.min(MAX_FACES);
            let offs = read_at(r, 12, n as usize * 4)?;
            let mut out = Vec::new();
            for i in 0..n {
                let off = u32be(&offs, i as usize * 4)?;
                if let Some(f) = name_table(r, u64::from(off)).and_then(|t| face(&t, i)) {
                    out.push(f);
                }
            }
            return Some(out);
        }
        let tag = u32be(&head, 0)?;
        if !matches!(tag, 0x0001_0000 | 0x4F54_544F | 0x7472_7565) {
            return None; // not sfnt (`\0\1\0\0`, `OTTO`, `true`)
        }
        Some(vec![face(&name_table(r, 0)?, 0)?])
    }
}

/// Per-file cache of parsed faces (path, size, mtime).
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod cache {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    use super::{FontFace, names};

    type Key = (PathBuf, u64, Option<SystemTime>);

    static CACHE: parking_lot::Mutex<Option<HashMap<Key, Vec<FontFace>>>> = parking_lot::Mutex::new(None);

    pub(super) fn faces_cached(p: &Path, meta: &std::fs::Metadata) -> Vec<FontFace> {
        let key = (p.to_path_buf(), meta.len(), meta.modified().ok());
        if let Some(v) = CACHE.lock().as_ref().and_then(|c| c.get(&key)) {
            return v.clone();
        }
        let faces =
            std::fs::File::open(p).ok().and_then(|f| names::faces(&mut std::io::BufReader::new(f))).unwrap_or_default();
        CACHE.lock().get_or_insert_with(HashMap::new).insert(key, faces.clone());
        faces
    }

    /// The Windows-style display name: `Full A & Full B (TrueType)`.
    pub(super) fn display_name(faces: &[FontFace], ext: &str) -> String {
        let kind = if matches!(ext, "otf" | "otc") { "OpenType" } else { "TrueType" };
        let names: Vec<&str> = faces.iter().map(|f| f.full_name.as_str()).collect();
        format!("{} ({kind})", names.join(" & "))
    }
}

/// macOS: CoreText's file list, faces from the `name` tables.
#[cfg(target_os = "macos")]
mod mac {
    use super::InstalledFont;
    use super::cache::{display_name, faces_cached};

    pub(super) fn installed() -> Vec<InstalledFont> {
        let user = super::user_font_dir();
        let mut out = Vec::new();
        for p in crate::sys::macos::fonts::available_font_files() {
            let Ok(meta) = std::fs::metadata(&p) else { continue };
            if !meta.is_file() {
                continue;
            }
            let faces = faces_cached(&p, &meta);
            if faces.is_empty() {
                continue;
            }
            let ext = p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let per_user = user.as_ref().is_some_and(|u| p.starts_with(u));
            out.push(InstalledFont { name: display_name(&faces, &ext), file: p, per_user, faces });
        }
        out
    }
}

#[cfg(target_os = "linux")]
mod scan {
    use std::path::{Path, PathBuf};

    use super::InstalledFont;
    use super::cache::{display_name, faces_cached};

    /// `<dir>` / `<include>` values of a fontconfig file (`prefix="xdg"` → relative to
    /// `$XDG_DATA_HOME`, `~` → home; relative includes → relative to the file's folder).
    pub(super) fn conf_entries(
        text: &str,
        conf_dir: &Path,
        home: &Path,
        data_home: &Path,
    ) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut dirs = Vec::new();
        let mut includes = Vec::new();
        for (tag, list) in [("dir", &mut dirs), ("include", &mut includes)] {
            let open = format!("<{tag}");
            let close = format!("</{tag}>");
            let mut rest = text;
            while let Some(i) = rest.find(&open) {
                let after = &rest[i + open.len()..];
                // `<dirs>` / `<directory>` are other tags.
                if !after.starts_with(['>', ' ', '\t', '\n']) {
                    rest = after;
                    continue;
                }
                let Some(gt) = after.find('>') else { break };
                let attrs = &after[..gt];
                let Some(end) = after[gt + 1..].find(&close) else { break };
                let value = after[gt + 1..gt + 1 + end].trim();
                rest = &after[gt + 1 + end..];
                let value = value.replace("&amp;", "&");
                let p = if attrs.contains("prefix=\"xdg\"") {
                    data_home.join(&value)
                } else if let Some(r) = value.strip_prefix("~/") {
                    home.join(r)
                } else if value == "~" {
                    home.to_path_buf()
                } else if attrs.contains("prefix=\"relative\"") || !value.starts_with('/') {
                    conf_dir.join(&value)
                } else {
                    PathBuf::from(&value)
                };
                list.push(p);
            }
        }
        (dirs, includes)
    }

    /// Font folders: the fixed list, then `<dir>`s of fonts.conf and its includes.
    pub(super) fn font_dirs() -> Vec<(PathBuf, bool)> {
        let home = crate::path::home();
        let data_home = crate::launch::linux::data_home();
        let mut dirs: Vec<(PathBuf, bool)> = vec![
            ("/usr/share/fonts".into(), false),
            ("/usr/local/share/fonts".into(), false),
            (data_home.join("fonts"), true),
            (home.join(".fonts"), true),
            // Inside Flatpak: the host's fonts, which Flatpak mounts here (the runtime's
            // own fonts are the `/usr/share/fonts` above). Absent elsewhere.
            ("/run/host/fonts".into(), false),
            ("/run/host/local-fonts".into(), false),
            ("/run/host/user-fonts".into(), true),
        ];
        let mut queue = vec![PathBuf::from("/etc/fonts/fonts.conf")];
        let mut seen = std::collections::HashSet::new();
        while let Some(f) = queue.pop() {
            if !seen.insert(f.clone()) || seen.len() > 256 {
                continue;
            }
            if f.is_dir() {
                let mut confs: Vec<PathBuf> = std::fs::read_dir(&f)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "conf"))
                    .collect();
                confs.sort();
                queue.extend(confs.into_iter().rev());
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let base = f.parent().unwrap_or(Path::new("/"));
            let (ds, incs) = conf_entries(&text, base, &home, &data_home);
            for d in ds {
                let user = d.starts_with(&home);
                if !dirs.iter().any(|(x, _)| *x == d) {
                    dirs.push((d, user));
                }
            }
            queue.extend(incs.into_iter().rev());
        }
        dirs
    }

    pub(super) fn installed() -> Vec<InstalledFont> {
        let mut out: Vec<InstalledFont> = Vec::new();
        let mut seen_files = std::collections::HashSet::new();
        for (root, per_user) in font_dirs() {
            let mut stack = vec![root];
            let mut visited = 0usize;
            while let Some(dir) = stack.pop() {
                visited += 1;
                if visited > 20_000 {
                    break;
                }
                let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                let mut entries: Vec<_> = rd.flatten().collect();
                entries.sort_by_key(|e| e.file_name());
                for e in entries {
                    let p = e.path();
                    let Ok(meta) = std::fs::metadata(&p) else { continue };
                    if meta.is_dir() {
                        stack.push(p);
                        continue;
                    }
                    let ext = p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                    if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
                        continue;
                    }
                    let real = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
                    if !seen_files.insert(real) {
                        continue;
                    }
                    let faces = faces_cached(&p, &meta);
                    if faces.is_empty() {
                        continue;
                    }
                    out.push(InstalledFont { name: display_name(&faces, &ext), file: p, per_user, faces });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use std::path::Path;

    use super::*;

    /// A minimal sfnt with only a `name` table (Windows-platform UTF-16BE strings).
    #[cfg(target_os = "linux")]
    pub(crate) fn sfnt(names: &[(u16, &str)]) -> Vec<u8> {
        let mut strings = Vec::new();
        let mut recs = Vec::new();
        for (id, s) in names {
            let b: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
            recs.push((3u16, 1u16, 0x0409u16, *id, b.len() as u16, strings.len() as u16));
            strings.extend(b);
        }
        let mut name = Vec::new();
        name.extend_from_slice(&0u16.to_be_bytes());
        name.extend_from_slice(&(recs.len() as u16).to_be_bytes());
        name.extend_from_slice(&((6 + recs.len() * 12) as u16).to_be_bytes());
        for r in &recs {
            for v in [r.0, r.1, r.2, r.3, r.4, r.5] {
                name.extend_from_slice(&v.to_be_bytes());
            }
        }
        name.extend(strings);
        let mut f = Vec::new();
        f.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        f.extend_from_slice(&1u16.to_be_bytes());
        f.extend_from_slice(&[0; 6]);
        f.extend_from_slice(b"name");
        f.extend_from_slice(&0u32.to_be_bytes());
        f.extend_from_slice(&28u32.to_be_bytes());
        f.extend_from_slice(&(name.len() as u32).to_be_bytes());
        f.extend(name);
        f
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn name_table_parsing() {
        let f = sfnt(&[(1, "Noto Sans Bold"), (2, "Regular"), (16, "Noto Sans"), (17, "Bold"), (4, "Noto Sans Bold")]);
        let faces = faces_of_bytes(&f);
        assert_eq!(
            faces,
            [FontFace {
                index: 0,
                family: "Noto Sans".into(),
                style: "Bold".into(),
                full_name: "Noto Sans Bold".into()
            }]
        );
        assert!(faces_of_bytes(b"garbage!").is_empty());
        assert!(faces_of_bytes(&[]).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn collections_and_lookup() {
        // ttcf header with two fonts pointing at two sfnt bodies.
        let a = sfnt(&[(1, "Mono A"), (2, "Regular"), (4, "Mono A")]);
        let b = sfnt(&[(1, "Mono A"), (2, "Bold"), (4, "Mono A Bold")]);
        let rebase = |mut f: Vec<u8>, base: u32| {
            let off = u32::from_be_bytes(f[20..24].try_into().unwrap()) + base;
            f[20..24].copy_from_slice(&off.to_be_bytes());
            f
        };
        let header = 12 + 8;
        let mut ttc = b"ttcf".to_vec();
        ttc.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        ttc.extend_from_slice(&2u32.to_be_bytes());
        ttc.extend_from_slice(&(header as u32).to_be_bytes());
        ttc.extend_from_slice(&((header + a.len()) as u32).to_be_bytes());
        ttc.extend(rebase(a.clone(), header as u32));
        ttc.extend(rebase(b, (header + a.len()) as u32));
        let faces = faces_of_bytes(&ttc);
        assert_eq!(faces.len(), 2);
        assert_eq!((faces[1].index, faces[1].full_name.as_str()), (1, "Mono A Bold"));
        let list = [InstalledFont { name: "x".into(), file: "/f/mono.ttc".into(), per_user: false, faces }];
        assert_eq!(find_in(&list, "Mono A Bold"), Some(("/f/mono.ttc".into(), 1)));
        assert_eq!(find_in(&list, "mono a"), Some(("/f/mono.ttc".into(), 0)), "family → regular face");
        assert_eq!(find_in(&list, "Other"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fontconfig_dirs() {
        let conf = r#"<fontconfig>
  <dir>/usr/share/fonts</dir>
  <dir prefix="xdg">fonts</dir>
  <dir>~/.fonts</dir>
  <dirs>ignored</dirs>
  <include ignore_missing="yes">conf.d</include>
</fontconfig>"#;
        let (dirs, inc) =
            scan::conf_entries(conf, Path::new("/etc/fonts"), Path::new("/home/u"), Path::new("/home/u/.local/share"));
        assert_eq!(
            dirs,
            [
                PathBuf::from("/usr/share/fonts"),
                PathBuf::from("/home/u/.local/share/fonts"),
                PathBuf::from("/home/u/.fonts")
            ]
        );
        assert_eq!(inc, [PathBuf::from("/etc/fonts/conf.d")]);
    }

    /// The real font folders of this machine: at least one face, all files absolute.
    #[cfg(target_os = "linux")]
    #[test]
    fn scans_installed_fonts() {
        let t = std::time::Instant::now();
        let all = installed();
        println!("{} font files in {:?}", all.len(), t.elapsed());
        if all.is_empty() {
            return; // minimal container without fonts
        }
        assert!(all.iter().all(|f| f.file.is_absolute() && !f.faces.is_empty() && f.name.ends_with(')')));
        let first = &all[0].faces[0];
        assert_eq!(find_face(&first.full_name).map(|x| x.1), Some(first.index));
    }

    #[cfg(windows)]
    #[test]
    fn segoe_ui_is_registered() {
        let all = super::installed();
        let seg = all.iter().find(|f| f.name.starts_with("Segoe UI (")).expect("Segoe UI registered");
        assert!(seg.file.is_absolute());
        assert!(seg.file.is_file(), "{}", seg.file.display());
    }

    #[test]
    fn folders() {
        assert!(system_font_dir().is_absolute() || cfg!(windows));
        let _ = user_font_dir();
    }
}
