//! The icon a Windows program carries in its own resources (`.exe`, `.scr`), read from
//! the PE file directly: no `ExtractIcon`, `SHGetFileInfo` or `LoadLibraryEx`, so it
//! works on every OS and never runs code from the file.
//!
//! **Format** (Microsoft "PE Format" and "Resource File Formats"): the DOS header's
//! `e_lfanew` (at 0x3C) points at `PE\0\0`, the COFF header and the optional header,
//! whose data directory 2 is the resource table (an RVA, mapped to a file offset through
//! the section headers). The resource tree has three levels: type, name, language. The
//! first `RT_GROUP_ICON` (14) entry in directory order is the program's icon (what
//! Explorer shows); its `GRPICONDIR` lists the images (width, height, bit count, size,
//! `RT_ICON` id), and each `RT_ICON` (3) resource is one image, either a PNG or a DIB
//! (a BMP without its file header, with the AND mask below the pixels).
//!
//! Since Windows 10 1903 many programs of Windows itself keep their icons in a resource-only
//! companion, `%SystemRoot%\SystemResources\<file name>.mun` (the same PE layout); [`icon`]
//! reads it for a program inside the Windows folder that carries no icon of its own.
//!
//! [`icon`] picks the image that fits the wanted size best and returns it as a complete
//! one-image `.ico` file, which any ICO decoder reads (decoding is the caller's job). The
//! file is untrusted: every offset, count and size is bounds-checked and capped, and only
//! the headers, the resource directory and the chosen image are read.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
/// Largest single icon image read (a 256×256 32-bit DIB is ~260 KiB; PNGs are smaller).
const MAX_IMAGE_BYTES: u32 = 4 * 1024 * 1024;
/// Most section headers / directory entries looked at.
const MAX_SECTIONS: u16 = 96;
const MAX_ENTRIES: u32 = 4096;

/// Extensions whose files can carry a program icon.
pub fn has_icon_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("scr"))
}

/// The program icon of the PE file at `path` closest to `px` pixels (see [`icon_from`]),
/// or for a program of Windows itself, the one in its `SystemResources` companion.
pub fn icon(path: &Path, px: u32) -> Option<Vec<u8>> {
    let mut f = std::fs::File::open(path).ok()?;
    icon_from(&mut f, px).or_else(|| {
        let mun = system_resources(path, std::env::var_os("SystemRoot")?.as_ref())?;
        icon_from(&mut std::fs::File::open(mun).ok()?, px)
    })
}

/// `<root>\SystemResources\<name>.mun` for a file inside the Windows folder `root`.
fn system_resources(path: &Path, root: &Path) -> Option<std::path::PathBuf> {
    let lower = |p: &Path| crate::strip_verbatim(p).to_string_lossy().to_lowercase().replace('/', "\\");
    let (file, root_s) = (lower(path), lower(root));
    let inside = file.strip_prefix(root_s.trim_end_matches('\\'))?.starts_with('\\');
    let mut name = path.file_name()?.to_os_string();
    name.push(".mun");
    inside.then(|| root.join("SystemResources").join(name))
}

/// The program icon in `r` as a one-image `.ico` file: the smallest image at least `px`
/// pixels wide (the deepest color of that size), else the largest one. `None` when `r`
/// is not a PE file or has no icon.
pub fn icon_from<R: Read + Seek>(r: &mut R, px: u32) -> Option<Vec<u8>> {
    let pe = Pe::read(r)?;
    let root = pe.rsrc_off;
    let group_dir = pe.subdir(r, root, RT_GROUP_ICON)?;
    let group = pe.first_leaf(r, group_dir)?;
    let (g_off, g_len) = pe.data(r, group)?;
    if !(6..=6 + 14 * 512).contains(&g_len) {
        return None;
    }
    let grp = read_at(r, g_off, g_len as usize)?;
    if u16le(&grp, 2)? != 1 {
        return None;
    }
    let count = usize::from(u16le(&grp, 4)?).min((grp.len() - 6) / 14);
    let entries: Vec<GroupEntry> = (0..count)
        .filter_map(|i| {
            let e = grp.get(6 + i * 14..6 + i * 14 + 14)?;
            Some(GroupEntry {
                width: if e[0] == 0 { 256 } else { u32::from(e[0]) },
                height: if e[1] == 0 { 256 } else { u32::from(e[1]) },
                colors: e[2],
                planes: u16::from_le_bytes([e[4], e[5]]),
                bits: u16::from_le_bytes([e[6], e[7]]),
                id: u16::from_le_bytes([e[12], e[13]]),
            })
        })
        .collect();
    let best = pick(&entries, px)?;
    let icons = pe.subdir(r, root, RT_ICON)?;
    let image = pe.entry_by_id(r, icons, u32::from(best.id))?;
    let leaf = pe.first_leaf_or_self(r, image)?;
    let (i_off, i_len) = pe.data(r, leaf)?;
    if i_len == 0 || i_len > MAX_IMAGE_BYTES {
        return None;
    }
    let data = read_at(r, i_off, i_len as usize)?;
    Some(ico_file(&best, &data))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GroupEntry {
    width: u32,
    height: u32,
    colors: u8,
    planes: u16,
    bits: u16,
    id: u16,
}

/// The smallest entry at least `px` wide (deepest color first), else the largest.
fn pick(entries: &[GroupEntry], px: u32) -> Option<GroupEntry> {
    let depth = |e: &GroupEntry| if e.bits == 0 && e.colors > 0 { 8 } else { e.bits };
    let fits = entries.iter().filter(|e| e.width >= px).min_by_key(|e| (e.width, std::cmp::Reverse(depth(e))));
    fits.or_else(|| entries.iter().max_by_key(|e| (e.width, depth(e)))).cloned()
}

/// A one-image `.ico` file around `data`.
fn ico_file(e: &GroupEntry, data: &[u8]) -> Vec<u8> {
    let dim = |v: u32| if v >= 256 { 0 } else { v as u8 };
    let mut out = Vec::with_capacity(22 + data.len());
    out.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    out.extend_from_slice(&[dim(e.width), dim(e.height), e.colors, 0]);
    out.extend_from_slice(&e.planes.to_le_bytes());
    out.extend_from_slice(&e.bits.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes());
    out.extend_from_slice(data);
    out
}

struct Section {
    va: u32,
    raw_off: u32,
    raw_size: u32,
}

struct Pe {
    sections: Vec<Section>,
    /// File offset of the resource directory root.
    rsrc_off: u64,
}

impl Pe {
    fn read<R: Read + Seek>(r: &mut R) -> Option<Pe> {
        let dos = read_at(r, 0, 64)?;
        if &dos[..2] != b"MZ" {
            return None;
        }
        let pe_off = u64::from(u32le(&dos, 0x3C)?);
        let coff = read_at(r, pe_off, 24)?;
        if &coff[..4] != b"PE\0\0" {
            return None;
        }
        let n_sections = u16le(&coff, 6)?.min(MAX_SECTIONS);
        let opt_size = u16le(&coff, 20)?;
        let opt = read_at(r, pe_off + 24, usize::from(opt_size))?;
        let (n_dirs_at, dirs_at) = match u16le(&opt, 0)? {
            0x10B => (92, 96),
            0x20B => (108, 112),
            _ => return None,
        };
        if u32le(&opt, n_dirs_at)? < 3 {
            return None;
        }
        let rsrc_rva = u32le(&opt, dirs_at + 2 * 8)?;
        if rsrc_rva == 0 {
            return None;
        }
        let sh = read_at(r, pe_off + 24 + u64::from(opt_size), usize::from(n_sections) * 40)?;
        let sections = sh
            .as_chunks::<40>()
            .0
            .iter()
            .map(|s| Section {
                va: u32::from_le_bytes([s[12], s[13], s[14], s[15]]),
                raw_size: u32::from_le_bytes([s[16], s[17], s[18], s[19]]),
                raw_off: u32::from_le_bytes([s[20], s[21], s[22], s[23]]),
            })
            .collect();
        let mut pe = Pe { sections, rsrc_off: 0 };
        pe.rsrc_off = pe.rva_to_off(rsrc_rva)?;
        Some(pe)
    }

    /// File offset of `rva` (only inside a section's raw data).
    fn rva_to_off(&self, rva: u32) -> Option<u64> {
        self.sections.iter().find_map(|s| {
            let within = rva.checked_sub(s.va)?;
            (within < s.raw_size).then(|| u64::from(s.raw_off) + u64::from(within))
        })
    }

    /// The entries of the directory at `dir` (file offset): `(name or id, offset field)`.
    fn entries<R: Read + Seek>(&self, r: &mut R, dir: u64) -> Option<Vec<(u32, u32)>> {
        let head = read_at(r, dir, 16)?;
        let n = (u32::from(u16le(&head, 12)?) + u32::from(u16le(&head, 14)?)).min(MAX_ENTRIES);
        let raw = read_at(r, dir + 16, n as usize * 8)?;
        let field = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        Some(raw.as_chunks::<8>().0.iter().map(|e| (field(&e[..4]), field(&e[4..]))).collect())
    }

    /// The subdirectory with the numeric id `id` under `dir`.
    fn subdir<R: Read + Seek>(&self, r: &mut R, dir: u64, id: u32) -> Option<u64> {
        let off = self.entry_by_id(r, dir, id)?;
        off.1.then_some(off.0)
    }

    /// `(file offset, is a directory)` of the entry with the numeric id `id`.
    fn entry_by_id<R: Read + Seek>(&self, r: &mut R, dir: u64, id: u32) -> Option<(u64, bool)> {
        let (_, field) = self.entries(r, dir)?.into_iter().find(|(name, _)| name & 0x8000_0000 == 0 && *name == id)?;
        Some(self.target(field))
    }

    fn target(&self, field: u32) -> (u64, bool) {
        (self.rsrc_off + u64::from(field & 0x7FFF_FFFF), field & 0x8000_0000 != 0)
    }

    /// The first data entry below the directory `dir` (name level, then language level).
    fn first_leaf<R: Read + Seek>(&self, r: &mut R, dir: u64) -> Option<u64> {
        let (_, field) = *self.entries(r, dir)?.first()?;
        self.first_leaf_or_self(r, self.target(field))
    }

    /// `node` itself when it is a data entry, else its first data entry (one level down).
    fn first_leaf_or_self<R: Read + Seek>(&self, r: &mut R, node: (u64, bool)) -> Option<u64> {
        match node {
            (off, false) => Some(off),
            (dir, true) => {
                let (_, field) = *self.entries(r, dir)?.first()?;
                match self.target(field) {
                    (off, false) => Some(off),
                    (sub, true) => {
                        let (_, f) = *self.entries(r, sub)?.first()?;
                        let (off, is_dir) = self.target(f);
                        (!is_dir).then_some(off)
                    }
                }
            }
        }
    }

    /// `(file offset, size)` of the resource data entry at `leaf`.
    fn data<R: Read + Seek>(&self, r: &mut R, leaf: u64) -> Option<(u64, u32)> {
        let e = read_at(r, leaf, 16)?;
        let rva = u32le(&e, 0)?;
        let size = u32le(&e, 4)?;
        Some((self.rva_to_off(rva)?, size))
    }
}

fn read_at<R: Read + Seek>(r: &mut R, off: u64, len: usize) -> Option<Vec<u8>> {
    r.seek(SeekFrom::Start(off)).ok()?;
    let mut buf = vec![0; len];
    r.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A minimal PE32+ file with one `.rsrc` section holding an icon group of the given
    /// `(width, bits, image bytes)` images (`RT_ICON` ids 1..).
    fn tiny_pe(images: &[(u8, u16, Vec<u8>)]) -> Vec<u8> {
        const RSRC_RVA: u32 = 0x1000;
        const RAW_OFF: usize = 0x200;
        let n = images.len() as u32;
        let payload_len: usize = images.iter().map(|(_, _, d)| d.len().next_multiple_of(16)).sum();
        // Offsets relative to the section start: root 0, RT_ICON names 0x100, RT_GROUP_ICON
        // names 0x200, group language dir 0x300, group data entry 0x380, icon language
        // dirs 0x400 + i*0x20, icon data entries 0x800 + i*0x10, payloads from 0x1000,
        // then the GRPICONDIR.
        let mut rs = vec![0u8; 0x1000 + payload_len + 6 + 14 * images.len()];
        let put = |rs: &mut Vec<u8>, at: usize, b: &[u8]| rs[at..at + b.len()].copy_from_slice(b);
        let dir = |rs: &mut Vec<u8>, at: usize, entries: &[(u32, u32)]| {
            put(rs, at + 14, &(entries.len() as u16).to_le_bytes());
            for (i, (id, off)) in entries.iter().enumerate() {
                put(rs, at + 16 + i * 8, &id.to_le_bytes());
                put(rs, at + 20 + i * 8, &off.to_le_bytes());
            }
        };
        dir(&mut rs, 0, &[(RT_ICON, 0x8000_0100), (RT_GROUP_ICON, 0x8000_0200)]);
        let names: Vec<(u32, u32)> = (0..n).map(|i| (i + 1, 0x8000_0000 | (0x400 + i * 0x20))).collect();
        dir(&mut rs, 0x100, &names);
        dir(&mut rs, 0x200, &[(1, 0x8000_0300)]);
        dir(&mut rs, 0x300, &[(0x409, 0x380)]);
        let mut at = 0x1000usize;
        let mut group = vec![0, 0, 1, 0];
        group.extend_from_slice(&(n as u16).to_le_bytes());
        for (i, (w, bits, data)) in images.iter().enumerate() {
            dir(&mut rs, 0x400 + i * 0x20, &[(0x409, 0x800 + i as u32 * 0x10)]);
            put(&mut rs, 0x800 + i * 0x10, &(RSRC_RVA + at as u32).to_le_bytes());
            put(&mut rs, 0x804 + i * 0x10, &(data.len() as u32).to_le_bytes());
            put(&mut rs, at, data);
            group.extend_from_slice(&[*w, *w, 0, 0, 1, 0]);
            group.extend_from_slice(&bits.to_le_bytes());
            group.extend_from_slice(&(data.len() as u32).to_le_bytes());
            group.extend_from_slice(&(i as u16 + 1).to_le_bytes());
            at += data.len().next_multiple_of(16);
        }
        put(&mut rs, 0x380, &(RSRC_RVA + at as u32).to_le_bytes());
        put(&mut rs, 0x384, &(group.len() as u32).to_le_bytes());
        put(&mut rs, at, &group);

        let mut f = vec![0u8; RAW_OFF];
        put(&mut f, 0, b"MZ");
        put(&mut f, 0x3C, &0x40u32.to_le_bytes());
        put(&mut f, 0x40, b"PE\0\0");
        let coff = 0x44;
        put(&mut f, coff + 2, &1u16.to_le_bytes()); // one section
        put(&mut f, coff + 16, &240u16.to_le_bytes()); // optional header size
        let opt = coff + 20;
        put(&mut f, opt, &0x20Bu16.to_le_bytes());
        put(&mut f, opt + 108, &16u32.to_le_bytes());
        put(&mut f, opt + 112 + 16, &RSRC_RVA.to_le_bytes());
        put(&mut f, opt + 112 + 20, &(rs.len() as u32).to_le_bytes());
        let sh = opt + 240;
        put(&mut f, sh, b".rsrc");
        put(&mut f, sh + 8, &(rs.len() as u32).to_le_bytes());
        put(&mut f, sh + 12, &RSRC_RVA.to_le_bytes());
        put(&mut f, sh + 16, &(rs.len() as u32).to_le_bytes());
        put(&mut f, sh + 20, &(RAW_OFF as u32).to_le_bytes());
        f.extend_from_slice(&rs);
        f
    }

    #[test]
    fn picks_the_smallest_image_that_fits_and_wraps_it_in_an_ico() {
        let pe = tiny_pe(&[(16, 32, vec![0x16; 40]), (48, 32, vec![0x48; 50]), (0, 32, b"\x89PNG-256".to_vec())]);
        let ico = icon_from(&mut Cursor::new(&pe), 24).expect("icon");
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(ico[6], 48, "the 48 px image for 24 px");
        assert_eq!(u32le(&ico, 14), Some(50));
        assert_eq!(u32le(&ico, 18), Some(22));
        assert_eq!(&ico[22..], &[0x48; 50][..]);
        let big = icon_from(&mut Cursor::new(&pe), 512).expect("largest");
        assert_eq!(big[6], 0, "256 px is written as 0");
        assert_eq!(&big[22..], b"\x89PNG-256");
        assert_eq!(icon_from(&mut Cursor::new(&pe), 16).unwrap()[22..], [0x16; 40]);
    }

    #[test]
    fn deeper_color_wins_at_the_same_size() {
        let depth = |bits| GroupEntry { width: 32, height: 32, colors: 0, planes: 1, bits, id: bits };
        assert_eq!(pick(&[depth(4), depth(32), depth(8)], 32).unwrap().bits, 32);
    }

    #[test]
    fn not_a_pe_or_no_icon_is_none() {
        assert!(icon_from(&mut Cursor::new(b"not a program at all, just text".to_vec()), 32).is_none());
        assert!(icon_from(&mut Cursor::new(tiny_pe(&[])), 32).is_none());
        let pe = tiny_pe(&[(32, 32, vec![1; 64])]);
        for cut in [0, 2, 0x40, 0x100, 0x200, 0x300, pe.len() - 8] {
            let _ = icon_from(&mut Cursor::new(&pe[..cut]), 32);
        }
    }

    #[cfg(windows)]
    #[test]
    fn system_resources_only_inside_the_windows_folder() {
        let root = Path::new(r"C:\Windows");
        assert_eq!(
            system_resources(Path::new(r"c:\WINDOWS\System32\Taskmgr.exe"), root),
            Some(root.join("SystemResources").join("Taskmgr.exe.mun"))
        );
        assert_eq!(system_resources(Path::new(r"C:\Windows2\x.exe"), root), None);
        assert_eq!(system_resources(Path::new(r"D:\Tools\Taskmgr.exe"), root), None);
    }

    #[test]
    fn extensions() {
        assert!(has_icon_ext(Path::new("C:/x/Setup.EXE")));
        assert!(has_icon_ext(Path::new("a.scr")));
        assert!(!has_icon_ext(Path::new("a.dll")));
        assert!(!has_icon_ext(Path::new("exe")));
    }

    /// A real program of the running system (read-only).
    #[cfg(windows)]
    #[test]
    fn reads_a_windows_program_icon() {
        let Some(root) = std::env::var_os("SystemRoot") else { return };
        let exe = Path::new(&root).join("System32").join("cmd.exe");
        if !exe.exists() {
            return;
        }
        let ico = icon(&exe, 32).expect("cmd.exe has an icon");
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert!(ico.len() > 22 + 40);
        // Task Manager's icon lives in SystemResources\Taskmgr.exe.mun (Windows 10 1903+).
        let taskmgr = Path::new(&root).join("System32").join("Taskmgr.exe");
        if taskmgr.exists() && Path::new(&root).join("SystemResources").join("Taskmgr.exe.mun").exists() {
            assert!(icon(&taskmgr, 32).is_some(), "Task Manager's icon from its .mun");
        }
    }
}
