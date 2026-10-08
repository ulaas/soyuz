//! Read-only formats (ISO / UDF, CAB, deb, rpm, RAR) and byte-split volumes, with
//! fixtures generated in the test (plus two tiny gzip'd IMAPI2 disc images under
//! `tests/fixtures`).

mod common;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use common::*;
use soyuz_archive::{ArchiveEntry, ArchiveError, ArchiveFormat, ArchiveReader, ExtractOptions, detect, list_dir, open};
use soyuz_core::OpControl;

// ---- helpers -------------------------------------------------------------------------

fn read_entry(r: &mut dyn ArchiveReader, path: &str) -> Vec<u8> {
    let idx = r.entries().unwrap().iter().position(|e| e.path == path).unwrap_or_else(|| panic!("no {path}"));
    let mut v = Vec::new();
    r.open_entry(idx).unwrap().read_to_end(&mut v).unwrap();
    v
}

fn entry(r: &mut dyn ArchiveReader, path: &str) -> ArchiveEntry {
    r.entries().unwrap().iter().find(|e| e.path == path).unwrap_or_else(|| panic!("no {path}")).clone()
}

fn names(r: &mut dyn ArchiveReader) -> Vec<String> {
    let mut v: Vec<String> = r.entries().unwrap().iter().map(|e| e.path.clone()).collect();
    v.sort();
    v
}

/// Extract everything into a fresh dir and return its snapshot.
fn extract_all(r: &mut dyn ArchiveReader) -> BTreeMap<String, Option<Vec<u8>>> {
    let d = tempfile::tempdir().unwrap();
    let mut sink = TestSink::default();
    let sum = r.extract(None, d.path(), &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert!(sink.errors.is_empty(), "extract errors: {:?}", sink.errors);
    assert_eq!(sum.entries_failed, 0);
    assert_no_temp_files(d.path());
    snapshot(d.path())
}

fn test_ok(r: &mut dyn ArchiveReader) {
    let rep = r.test(&OpControl::new(), &mut ()).unwrap();
    assert!(rep.bad.is_empty(), "test found problems: {:?}", rep.bad);
}

// ---- ISO 9660 builder -----------------------------------------------------------------

enum Node {
    Dir(BTreeMap<String, Node>),
    File(Vec<u8>),
    Link(String),
}

fn tree(items: &[(&str, Option<&[u8]>)]) -> Node {
    let mut root = BTreeMap::new();
    for (path, content) in items {
        let parts: Vec<&str> = path.split('/').collect();
        let mut cur = &mut root;
        for (i, p) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            if last {
                let node = match content {
                    Some(c) if c.starts_with(b"->") => Node::Link(String::from_utf8_lossy(&c[2..]).into_owned()),
                    Some(c) => Node::File(c.to_vec()),
                    None => Node::Dir(BTreeMap::new()),
                };
                cur.entry(p.to_string()).or_insert(node);
            } else {
                let next = cur.entry(p.to_string()).or_insert_with(|| Node::Dir(BTreeMap::new()));
                let Node::Dir(m) = next else { panic!("not a dir") };
                cur = m;
            }
        }
    }
    Node::Dir(root)
}

const S: usize = 2048;

fn both32(v: u32) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&v.to_le_bytes());
    b[4..].copy_from_slice(&v.to_be_bytes());
    b
}

fn su_entry(sig: &[u8; 2], body: &[u8]) -> Vec<u8> {
    let mut v = vec![sig[0], sig[1], (4 + body.len()) as u8, 1];
    v.extend_from_slice(body);
    v
}

fn dir_record(lba: u32, size: u32, flags: u8, name: &[u8], su: &[u8]) -> Vec<u8> {
    let pad = usize::from(name.len().is_multiple_of(2));
    let mut len = 33 + name.len() + pad + su.len();
    len += len % 2;
    let mut r = vec![0u8; len];
    r[0] = len as u8;
    r[2..10].copy_from_slice(&both32(lba));
    r[10..18].copy_from_slice(&both32(size));
    r[18..25].copy_from_slice(&[124, 5, 6, 7, 8, 9, 0]); // 2024-05-06 07:08:09 UTC
    r[25] = flags;
    r[28..32].copy_from_slice(&[1, 0, 0, 1]);
    r[32] = name.len() as u8;
    r[33..33 + name.len()].copy_from_slice(name);
    r[33 + name.len() + pad..33 + name.len() + pad + su.len()].copy_from_slice(su);
    r
}

#[derive(Clone, Copy, PartialEq)]
enum Names {
    Plain,
    RockRidge,
    Joliet,
}

/// Directory records of one tree flavour; `lbas[id]` = (lba, size) of every dir id.
struct Flavour {
    names: Names,
}

impl Flavour {
    fn name(&self, n: &str, is_dir: bool) -> Vec<u8> {
        match self.names {
            Names::Joliet => {
                let s = if is_dir { n.to_string() } else { format!("{n};1") };
                s.encode_utf16().flat_map(u16::to_be_bytes).collect()
            }
            _ => {
                let up: String = n
                    .to_uppercase()
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' })
                    .collect();
                if is_dir { up.into_bytes() } else { format!("{up};1").into_bytes() }
            }
        }
    }

    fn su(&self, n: &str, mode: u32, link: Option<&str>) -> Vec<u8> {
        if self.names != Names::RockRidge {
            return Vec::new();
        }
        let mut px = Vec::new();
        for v in [mode, 1, 0, 0] {
            px.extend_from_slice(&both32(v));
        }
        let mut su = su_entry(b"PX", &px);
        let mut nm = vec![0u8];
        nm.extend_from_slice(n.as_bytes());
        su.extend(su_entry(b"NM", &nm));
        if let Some(t) = link {
            let mut sl = vec![0u8];
            for comp in t.split('/') {
                match comp {
                    ".." => sl.extend_from_slice(&[0x04, 0]),
                    "." => sl.extend_from_slice(&[0x02, 0]),
                    c => {
                        sl.extend_from_slice(&[0, c.len() as u8]);
                        sl.extend_from_slice(c.as_bytes());
                    }
                }
            }
            su.extend(su_entry(b"SL", &sl));
        }
        su
    }

    /// Records of `dir` given child dir positions and file extents.
    #[allow(clippy::too_many_arguments)]
    fn records(
        &self,
        dir: &BTreeMap<String, Node>,
        me: (u32, u32),
        parent: (u32, u32),
        child_dirs: &[(u32, u32)],
        files: &mut dyn Iterator<Item = (u32, u32)>,
        root: bool,
    ) -> Vec<Vec<u8>> {
        let mut recs = Vec::new();
        let sp = if root && self.names == Names::RockRidge { su_entry(b"SP", &[0xBE, 0xEF, 0]) } else { Vec::new() };
        recs.push(dir_record(me.0, me.1, 2, &[0], &sp));
        recs.push(dir_record(parent.0, parent.1, 2, &[1], &[]));
        let mut cd = child_dirs.iter();
        for (n, node) in dir {
            match node {
                Node::Dir(_) => {
                    let (lba, size) = *cd.next().unwrap();
                    recs.push(dir_record(lba, size, 2, &self.name(n, true), &self.su(n, 0o040755, None)));
                }
                Node::File(data) => {
                    let (lba, _) = files.next().unwrap();
                    if data.len() > S && data.len() < 2 * S && self.names != Names::Joliet {
                        // Multi-extent file: first S bytes, then the rest.
                        let nm = self.name(n, false);
                        recs.push(dir_record(lba, S as u32, 0x80, &nm, &self.su(n, 0o100644, None)));
                        recs.push(dir_record(lba + 1, (data.len() - S) as u32, 0, &nm, &self.su(n, 0o100644, None)));
                    } else {
                        recs.push(dir_record(
                            lba,
                            data.len() as u32,
                            0,
                            &self.name(n, false),
                            &self.su(n, 0o100644, None),
                        ));
                    }
                }
                Node::Link(t) => {
                    let (lba, _) = files.next().unwrap();
                    if self.names == Names::RockRidge {
                        recs.push(dir_record(lba, 0, 0, &self.name(n, false), &self.su(n, 0o120777, Some(t))));
                    }
                }
            }
        }
        recs
    }
}

fn pack(recs: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for r in recs {
        let room = S - out.len() % S;
        if r.len() > room {
            out.resize(out.len() + room, 0);
        }
        out.extend_from_slice(r);
    }
    out.resize(out.len().div_ceil(S).max(1) * S, 0);
    out
}

/// Build an ISO image of `root` with the primary tree (plain or Rock Ridge names) and
/// optionally a Joliet tree.
fn build_iso(root: &Node, names: Names, joliet: bool) -> Vec<u8> {
    let Node::Dir(top) = root else { panic!() };
    // Files: allocate data first (depth-first order, the same order `records` walks).
    let mut img = vec![0u8; 24 * S];
    fn alloc_files(dir: &BTreeMap<String, Node>, img: &mut Vec<u8>, out: &mut Vec<(u32, u32)>) {
        for node in dir.values() {
            match node {
                Node::File(d) => {
                    let lba = (img.len() / S) as u32;
                    img.extend_from_slice(d);
                    img.resize(img.len().div_ceil(S) * S, 0);
                    out.push((lba, d.len() as u32));
                }
                Node::Link(_) => out.push((0, 0)),
                Node::Dir(_) => {}
            }
        }
        for node in dir.values() {
            if let Node::Dir(m) = node {
                alloc_files(m, img, out);
            }
        }
    }
    let mut files = Vec::new();
    alloc_files(top, &mut img, &mut files);

    let mut flavours = vec![Flavour { names }];
    if joliet {
        flavours.push(Flavour { names: Names::Joliet });
    }
    let mut roots = Vec::new();
    for fl in &flavours {
        // Dir sizes (independent of positions): compute with dummy positions.
        fn sizes(fl: &Flavour, dir: &BTreeMap<String, Node>, out: &mut Vec<u32>, root: bool) -> usize {
            let id = out.len();
            out.push(0);
            let kids: Vec<usize> = dir
                .values()
                .filter_map(|n| if let Node::Dir(m) = n { Some(sizes(fl, m, out, false)) } else { None })
                .collect();
            let cd = vec![(0u32, 0u32); kids.len()];
            let mut it = std::iter::repeat((0u32, 0u32));
            out[id] = pack(&fl.records(dir, (0, 0), (0, 0), &cd, &mut it, root)).len() as u32;
            id
        }
        let mut sz = Vec::new();
        sizes(fl, top, &mut sz, true);
        // Positions.
        let mut pos = Vec::new();
        for s in &sz {
            pos.push(((img.len() / S) as u32, *s));
            img.resize(img.len() + *s as usize, 0);
        }
        // Write records (same pre-order ids as `sizes`).
        #[allow(clippy::too_many_arguments)]
        fn write(
            fl: &Flavour,
            dir: &BTreeMap<String, Node>,
            pos: &[(u32, u32)],
            next_id: &mut usize,
            parent: (u32, u32),
            files: &mut std::slice::Iter<'_, (u32, u32)>,
            img: &mut [u8],
            root: bool,
        ) {
            let me = pos[*next_id];
            *next_id += 1;
            // Child dir ids are assigned in order during recursion; precompute them.
            let mut child_ids = Vec::new();
            let mut probe = *next_id;
            for n in dir.values() {
                if let Node::Dir(m) = n {
                    child_ids.push(probe);
                    probe += count_dirs(m);
                }
            }
            let cd: Vec<(u32, u32)> = child_ids.iter().map(|&i| pos[i]).collect();
            let own_files: Vec<(u32, u32)> =
                dir.values().filter(|n| !matches!(n, Node::Dir(_))).map(|_| *files.next().unwrap()).collect();
            let recs = fl.records(dir, me, if root { me } else { parent }, &cd, &mut own_files.into_iter(), root);
            let bytes = pack(&recs);
            let at = me.0 as usize * S;
            img[at..at + bytes.len()].copy_from_slice(&bytes);
            for n in dir.values() {
                if let Node::Dir(m) = n {
                    write(fl, m, pos, next_id, me, files, img, false);
                }
            }
        }
        fn count_dirs(dir: &BTreeMap<String, Node>) -> usize {
            1 + dir.values().map(|n| if let Node::Dir(m) = n { count_dirs(m) } else { 0 }).sum::<usize>()
        }
        let mut id = 0;
        let mut fit = files.iter();
        write(fl, top, &pos, &mut id, pos[0], &mut fit, &mut img, true);
        roots.push(pos[0]);
    }
    // Volume descriptors.
    let total = (img.len() / S) as u32;
    let vd = |ty: u8, root: (u32, u32), esc: &[u8]| {
        let mut s = vec![0u8; S];
        s[0] = ty;
        s[1..6].copy_from_slice(b"CD001");
        s[6] = 1;
        s[80..88].copy_from_slice(&both32(total));
        s[88..88 + esc.len()].copy_from_slice(esc);
        s[128..130].copy_from_slice(&2048u16.to_le_bytes());
        s[130..132].copy_from_slice(&2048u16.to_be_bytes());
        s[156..190].copy_from_slice(&dir_record(root.0, root.1, 2, &[0], &[]));
        s
    };
    img[16 * S..17 * S].copy_from_slice(&vd(1, roots[0], &[]));
    let mut next = 17;
    if joliet {
        img[next * S..(next + 1) * S].copy_from_slice(&vd(2, roots[1], b"%/E"));
        next += 1;
    }
    let mut term = vec![0u8; S];
    term[0] = 255;
    term[1..6].copy_from_slice(b"CD001");
    term[6] = 1;
    img[next * S..(next + 1) * S].copy_from_slice(&term);
    img
}

fn sample_tree() -> Node {
    tree(&[
        ("readme.txt", Some(b"hello iso")),
        ("docs", None),
        ("docs/Long File Name.md", Some(&[b'a'; 3000])),
        ("docs/deep/inner.bin", Some(&[7u8; 5000])),
        ("empty", None),
        ("link", Some(b"->docs/deep/inner.bin")),
    ])
}

#[test]
fn iso_rock_ridge() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("disc.iso");
    std::fs::write(&p, build_iso(&sample_tree(), Names::RockRidge, true)).unwrap();
    assert_eq!(detect(&p), Some(ArchiveFormat::Iso));
    // Detected by its volume descriptors even without the extension.
    let bare = d.path().join("disc_image");
    std::fs::copy(&p, &bare).unwrap();
    assert_eq!(detect(&bare), Some(ArchiveFormat::Iso));

    let mut r = open(&p, None).unwrap();
    assert_eq!(r.format(), ArchiveFormat::Iso);
    assert_eq!(
        names(r.as_mut()),
        ["docs", "docs/Long File Name.md", "docs/deep", "docs/deep/inner.bin", "empty", "link", "readme.txt"]
    );
    assert_eq!(read_entry(r.as_mut(), "readme.txt"), b"hello iso");
    // Multi-extent file (two directory records).
    assert_eq!(read_entry(r.as_mut(), "docs/Long File Name.md"), vec![b'a'; 3000]);
    let e = entry(r.as_mut(), "link");
    assert!(e.is_symlink);
    assert_eq!(e.link_target.as_deref(), Some("docs/deep/inner.bin"));
    let e = entry(r.as_mut(), "docs/deep/inner.bin");
    assert_eq!(e.mode, Some(0o100644));
    assert_eq!(soyuz_archive::vpath::list_dir(r.as_mut(), "docs").unwrap().len(), 2);
    test_ok(r.as_mut());
    let snap = extract_all(r.as_mut());
    assert_eq!(snap["readme.txt"], Some(b"hello iso".to_vec()));
    assert_eq!(snap["docs/deep/inner.bin"], Some(vec![7u8; 5000]));
    assert_eq!(snap["empty"], None);
}

#[test]
fn iso_joliet_and_plain() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("j.iso");
    std::fs::write(&p, build_iso(&sample_tree(), Names::Plain, true)).unwrap();
    let mut r = open(&p, None).unwrap();
    // Joliet keeps case and spaces.
    let n = names(r.as_mut());
    assert!(n.contains(&"docs/Long File Name.md".to_string()), "{n:?}");
    assert_eq!(read_entry(r.as_mut(), "docs/deep/inner.bin"), vec![7u8; 5000]);
    test_ok(r.as_mut());

    let p = d.path().join("plain.iso");
    std::fs::write(&p, build_iso(&sample_tree(), Names::Plain, false)).unwrap();
    let mut r = open(&p, None).unwrap();
    let n = names(r.as_mut());
    assert!(n.contains(&"README.TXT".to_string()), "{n:?}");
    assert!(n.contains(&"DOCS/LONG_FILE_NAME.MD".to_string()), "{n:?}");
    assert_eq!(read_entry(r.as_mut(), "DOCS/LONG_FILE_NAME.MD"), vec![b'a'; 3000]);
}

#[test]
fn iso_rejects_out_of_range_extents_and_loops() {
    let d = tempfile::tempdir().unwrap();
    let mut img = build_iso(&sample_tree(), Names::Plain, false);
    // Point readme's extent far past the end: listing works, reading fails cleanly.
    let root_lba = u32::from_le_bytes(img[16 * S + 158..16 * S + 162].try_into().unwrap()) as usize;
    let dir = &mut img[root_lba * S..(root_lba + 1) * S];
    let mut pos = 0;
    while dir[pos] != 0 {
        let len = dir[pos] as usize;
        let nlen = dir[pos + 32] as usize;
        if &dir[pos + 33..pos + 33 + nlen] == b"README.TXT;1" {
            dir[pos + 2..pos + 10].copy_from_slice(&both32(0x00FF_FFFF));
        }
        // Make the "DOCS" directory point at the root itself: a loop.
        if &dir[pos + 33..pos + 33 + nlen] == b"DOCS" {
            let me = both32(root_lba as u32);
            dir[pos + 2..pos + 10].copy_from_slice(&me);
        }
        pos += len;
    }
    let p = d.path().join("bad.iso");
    std::fs::write(&p, &img).unwrap();
    let mut r = open(&p, None).unwrap();
    let idx = r.entries().unwrap().iter().position(|e| e.path == "README.TXT").unwrap();
    assert!(matches!(r.open_entry(idx).err(), Some(ArchiveError::Corrupt(_))));
    let rep = r.test(&OpControl::new(), &mut ()).unwrap();
    assert!(rep.bad.iter().any(|(p, _)| p == "README.TXT"), "{:?}", rep.bad);
    // Truncated image: open fails with an error, no panic.
    let p = d.path().join("short.iso");
    std::fs::write(&p, &img[..17 * S + 100]).unwrap();
    assert!(open(&p, None).is_err());
}

fn gunzip_fixture(name: &str, dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&src).unwrap()).read_to_end(&mut out).unwrap();
    let p = dir.join(name.trim_end_matches(".gz"));
    std::fs::write(&p, out).unwrap();
    p
}

/// Images made by Windows' IMAPI2 (ISO 9660 + Joliet + UDF 1.02, and UDF 2.50 only) from
/// `hello.txt`, `Docs/ünïcödé.txt`, `Docs/Deep/a long file name with spaces.txt`.
#[test]
fn udf_imapi_fixtures() {
    let d = tempfile::tempdir().unwrap();
    for name in ["imapi-udf102.iso.gz", "imapi-udf250.iso.gz"] {
        let p = gunzip_fixture(name, d.path());
        assert_eq!(detect(&p), Some(ArchiveFormat::Iso), "{name}");
        let mut r = open(&p, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        let n = names(r.as_mut());
        for want in ["hello.txt", "Docs/ünïcödé.txt", "Docs/Deep/a long file name with spaces.txt"] {
            assert!(n.iter().any(|x| x == want), "{name}: missing {want} in {n:?}");
        }
        assert_eq!(read_entry(r.as_mut(), "hello.txt"), b"hello from soyuz", "{name}");
        assert_eq!(read_entry(r.as_mut(), "Docs/Deep/a long file name with spaces.txt"), vec![b'x'; 5000]);
        let e = entry(r.as_mut(), "hello.txt");
        assert!(e.modified.is_some());
        test_ok(r.as_mut());
        let snap = extract_all(r.as_mut());
        assert_eq!(snap["Docs/ünïcödé.txt"], Some(b"unicode".to_vec()));
    }
}

// ---- CAB ------------------------------------------------------------------------------

fn build_cab(path: &Path, files: &[(&str, Vec<u8>, cab::CompressionType)]) {
    // One folder per run of equal compression types.
    let mut b = cab::CabinetBuilder::new();
    let mut i = 0;
    while i < files.len() {
        let ct = files[i].2;
        let folder = b.add_folder(ct);
        while i < files.len() && files[i].2 == ct {
            folder.add_file(files[i].0);
            i += 1;
        }
    }
    let f = std::fs::File::create(path).unwrap();
    let mut w = b.build(f).unwrap();
    while let Some(mut fw) = w.next_file().unwrap() {
        let data = &files.iter().find(|(n, _, _)| *n == fw.file_name()).unwrap().1;
        fw.write_all(data).unwrap();
    }
    w.finish().unwrap();
}

#[test]
fn cab_stored_and_mszip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("setup.cab");
    let big = noise(200_000, 3);
    let text: Vec<u8> = (0..150_000u32).map(|i| b"abcdefgh"[(i % 7) as usize]).collect();
    build_cab(
        &p,
        &[
            ("readme.txt", b"cab hello".to_vec(), cab::CompressionType::None),
            ("bin\\tool.exe", big.clone(), cab::CompressionType::None),
            ("docs\\manual.txt", text.clone(), cab::CompressionType::MsZip),
            ("docs\\empty.txt", Vec::new(), cab::CompressionType::MsZip),
            ("docs\\more.txt", b"more".to_vec(), cab::CompressionType::MsZip),
        ],
    );
    assert_eq!(detect(&p), Some(ArchiveFormat::Cab));
    let mut r = open(&p, None).unwrap();
    assert_eq!(names(r.as_mut()), ["bin/tool.exe", "docs/empty.txt", "docs/manual.txt", "docs/more.txt", "readme.txt"]);
    assert_eq!(read_entry(r.as_mut(), "docs/more.txt"), b"more");
    assert_eq!(read_entry(r.as_mut(), "docs/manual.txt"), text);
    assert_eq!(read_entry(r.as_mut(), "bin/tool.exe"), big);
    assert_eq!(list_dir(r.as_mut(), "").unwrap().len(), 3);
    test_ok(r.as_mut());
    let snap = extract_all(r.as_mut());
    assert_eq!(snap["docs/manual.txt"], Some(text));
    assert_eq!(snap["docs/empty.txt"], Some(Vec::new()));
    assert_eq!(snap["readme.txt"], Some(b"cab hello".to_vec()));

    // Corrupt a compressed data block: the test reports it.
    let mut bytes = std::fs::read(&p).unwrap();
    let n = bytes.len();
    bytes[n - 40] ^= 0xFF;
    let bad = d.path().join("bad.cab");
    std::fs::write(&bad, &bytes).unwrap();
    let mut r = open(&bad, None).unwrap();
    let rep = r.test(&OpControl::new(), &mut ()).unwrap();
    assert!(!rep.bad.is_empty());
}

// ---- deb / rpm ------------------------------------------------------------------------

fn tar_of(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    build(&mut b);
    b.into_inner().unwrap()
}

fn tar_add(b: &mut tar::Builder<Vec<u8>>, path: &str, kind: tar::EntryType, data: &[u8], link: Option<&str>) {
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(kind);
    h.set_mode(if kind == tar::EntryType::Directory { 0o755 } else { 0o644 });
    h.set_mtime(1_700_000_000);
    h.set_size(data.len() as u64);
    if let Some(l) = link {
        h.set_link_name(l).unwrap();
    }
    b.append_data(&mut h, path, data).unwrap();
}

fn ar_member(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    let mut h = format!("{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}", "0", "0", "0", "100644", data.len());
    h.push_str("`\n");
    out.extend_from_slice(h.as_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(b'\n');
    }
}

fn gz(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn xz(data: &[u8]) -> Vec<u8> {
    let mut w = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
    w.write_all(data).unwrap();
    w.finish().unwrap()
}

fn deb_payload_tar() -> Vec<u8> {
    tar_of(|b| {
        tar_add(b, "./", tar::EntryType::Directory, b"", None);
        tar_add(b, "./usr/", tar::EntryType::Directory, b"", None);
        tar_add(b, "./usr/bin/", tar::EntryType::Directory, b"", None);
        tar_add(b, "./usr/bin/hello", tar::EntryType::Regular, b"#!/bin/sh\necho hi\n", None);
        tar_add(b, "./usr/share/doc/hello/copyright", tar::EntryType::Regular, &noise(70_000, 9), None);
        tar_add(b, "./usr/bin/hi", tar::EntryType::Symlink, b"", Some("hello"));
        tar_add(b, "./usr/bin/hello2", tar::EntryType::Link, b"", Some("./usr/bin/hello"));
    })
}

#[test]
fn deb_packages() {
    let d = tempfile::tempdir().unwrap();
    let payload = deb_payload_tar();
    for (member, data) in [("data.tar.xz", xz(&payload)), ("data.tar.gz", gz(&payload)), ("data.tar", payload.clone())]
    {
        let mut deb = b"!<arch>\n".to_vec();
        ar_member(&mut deb, "debian-binary", b"2.0\n");
        ar_member(
            &mut deb,
            "control.tar.gz",
            &gz(&tar_of(|b| tar_add(b, "./control", tar::EntryType::Regular, b"Package: hello\n", None))),
        );
        ar_member(&mut deb, member, &data);
        let p = d.path().join(format!("hello_{member}.deb"));
        std::fs::write(&p, &deb).unwrap();
        assert_eq!(detect(&p), Some(ArchiveFormat::Deb));
        let mut r = open(&p, None).unwrap_or_else(|e| panic!("{member}: {e}"));
        assert_eq!(
            names(r.as_mut()),
            ["usr", "usr/bin", "usr/bin/hello", "usr/bin/hello2", "usr/bin/hi", "usr/share/doc/hello/copyright"],
            "{member}"
        );
        assert_eq!(read_entry(r.as_mut(), "usr/bin/hello"), b"#!/bin/sh\necho hi\n");
        assert_eq!(read_entry(r.as_mut(), "usr/share/doc/hello/copyright"), noise(70_000, 9));
        assert_eq!(entry(r.as_mut(), "usr/bin/hi").link_target.as_deref(), Some("hello"));
        test_ok(r.as_mut());
        let snap = extract_all(r.as_mut());
        assert_eq!(snap["usr/bin/hello2"], Some(b"#!/bin/sh\necho hi\n".to_vec()), "{member}: hard link");
        assert_eq!(snap["usr/share/doc/hello/copyright"], Some(noise(70_000, 9)));
    }
}

fn cpio_entry(out: &mut Vec<u8>, name: &str, mode: u32, ino: u32, nlink: u32, data: &[u8]) {
    let fields = [ino, mode, 0, 0, nlink, 1_700_000_000, data.len() as u32, 0, 0, 0, 0, name.len() as u32 + 1, 0];
    out.extend_from_slice(b"070701");
    for f in fields {
        out.extend_from_slice(format!("{f:08X}").as_bytes());
    }
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn rpm_with(payload: Vec<u8>) -> Vec<u8> {
    let mut rpm = vec![0u8; 96];
    rpm[..4].copy_from_slice(&[0xED, 0xAB, 0xEE, 0xDB]);
    // Signature header with one 4-byte entry (so padding to 8 matters), then an empty header.
    let mut sig = vec![0x8E, 0xAD, 0xE8, 0x01, 0, 0, 0, 0];
    sig.extend_from_slice(&1u32.to_be_bytes());
    sig.extend_from_slice(&4u32.to_be_bytes());
    sig.extend_from_slice(&[0, 0, 0x03, 0xE8, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 1]);
    sig.extend_from_slice(&[1, 2, 3, 4]);
    while !sig.len().is_multiple_of(8) {
        sig.push(0);
    }
    rpm.extend(sig);
    rpm.extend_from_slice(&[0x8E, 0xAD, 0xE8, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    rpm.extend(payload);
    rpm
}

#[test]
fn rpm_packages() {
    let d = tempfile::tempdir().unwrap();
    let mut cpio = Vec::new();
    cpio_entry(&mut cpio, "./usr", 0o040755, 1, 2, b"");
    cpio_entry(&mut cpio, "./usr/bin/tool", 0o100755, 2, 1, b"tool binary");
    cpio_entry(&mut cpio, "./usr/bin/link", 0o120777, 3, 1, b"tool");
    // Hard links: data on the last link.
    cpio_entry(&mut cpio, "./usr/lib/a.so", 0o100644, 4, 2, b"");
    cpio_entry(&mut cpio, "./usr/lib/b.so", 0o100644, 4, 2, &noise(10_000, 1));
    cpio_entry(&mut cpio, "./dev/null", 0o020666, 5, 1, b"");
    cpio_entry(&mut cpio, "TRAILER!!!", 0, 0, 1, b"");
    for (label, payload) in [("gz", gz(&cpio)), ("xz", xz(&cpio))] {
        let p = d.path().join(format!("tool-1.0-{label}.rpm"));
        std::fs::write(&p, rpm_with(payload)).unwrap();
        assert_eq!(detect(&p), Some(ArchiveFormat::Rpm));
        let mut r = open(&p, None).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(
            names(r.as_mut()),
            ["usr", "usr/bin/link", "usr/bin/tool", "usr/lib/a.so", "usr/lib/b.so"],
            "{label}"
        );
        assert_eq!(read_entry(r.as_mut(), "usr/bin/tool"), b"tool binary");
        assert_eq!(read_entry(r.as_mut(), "usr/lib/a.so"), noise(10_000, 1));
        assert_eq!(entry(r.as_mut(), "usr/bin/link").link_target.as_deref(), Some("tool"));
        assert_eq!(entry(r.as_mut(), "usr/bin/tool").mode, Some(0o755));
        test_ok(r.as_mut());
        let snap = extract_all(r.as_mut());
        assert_eq!(snap["usr/lib/a.so"], Some(noise(10_000, 1)));
        assert_eq!(snap["usr/lib/b.so"], Some(noise(10_000, 1)));
        assert_eq!(snap["usr/bin/tool"], Some(b"tool binary".to_vec()));
    }
    // Garbage payload: error, no panic.
    let p = d.path().join("bad.rpm");
    std::fs::write(&p, rpm_with(gz(b"070701ZZZZ not a cpio"))).unwrap();
    assert!(open(&p, None).is_err());
}

#[test]
fn package_path_traversal_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let mut cpio = Vec::new();
    cpio_entry(&mut cpio, "../../evil.txt", 0o100644, 1, 1, b"evil");
    cpio_entry(&mut cpio, "ok.txt", 0o100644, 2, 1, b"ok");
    cpio_entry(&mut cpio, "TRAILER!!!", 0, 0, 1, b"");
    let p = d.path().join("evil.rpm");
    std::fs::write(&p, rpm_with(gz(&cpio))).unwrap();
    let mut r = open(&p, None).unwrap();
    let dest = d.path().join("out");
    let mut sink = TestSink::default();
    r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert!(sink.errors.iter().any(|(_, e)| matches!(e, ArchiveError::ZipSlip(_))), "{:?}", sink.errors);
    assert!(!d.path().join("evil.txt").exists());
    assert!(dest.join("ok.txt").exists());
}

// ---- RAR ------------------------------------------------------------------------------

#[cfg(feature = "rar")]
mod rar {
    use super::*;
    use rars::{ArchiveVersion, Builder};

    fn build(version: ArchiveVersion, solid: bool, password: Option<&str>) -> Vec<u8> {
        let mut b = Builder::new(version).solid(solid).password(password.map(|p| p.as_bytes().to_vec()));
        b.add_directory(b"dir".to_vec(), None, None).unwrap();
        b.add_bytes(b"dir/a.txt".to_vec(), b"alpha".to_vec(), None, None).unwrap();
        b.add_bytes(b"dir/big.bin".to_vec(), noise(300_000, 5), None, None).unwrap();
        b.add_bytes(b"z.txt".to_vec(), b"zulu".to_vec(), None, None).unwrap();
        b.to_bytes().unwrap()
    }

    #[test]
    fn rar_versions_solid_and_plain() {
        let d = tempfile::tempdir().unwrap();
        for (version, solid) in [
            (ArchiveVersion::Rar50, false),
            (ArchiveVersion::Rar50, true),
            (ArchiveVersion::Rar29, false),
            (ArchiveVersion::Rar29, true),
        ] {
            let p = d.path().join(format!("t-{version:?}-{solid}.rar"));
            std::fs::write(&p, build(version, solid, None)).unwrap();
            assert_eq!(detect(&p), Some(ArchiveFormat::Rar));
            let mut r = open(&p, None).unwrap_or_else(|e| panic!("{version:?}: {e}"));
            assert_eq!(names(r.as_mut()), ["dir", "dir/a.txt", "dir/big.bin", "z.txt"], "{version:?} {solid}");
            assert_eq!(read_entry(r.as_mut(), "z.txt"), b"zulu");
            assert_eq!(read_entry(r.as_mut(), "dir/big.bin"), noise(300_000, 5));
            assert_eq!(read_entry(r.as_mut(), "dir/a.txt"), b"alpha");
            test_ok(r.as_mut());
            let snap = extract_all(r.as_mut());
            assert_eq!(snap["dir/big.bin"], Some(noise(300_000, 5)));
            assert_eq!(snap["z.txt"], Some(b"zulu".to_vec()));
            // Selective extraction of the last entry only.
            let idx = r.entries().unwrap().iter().position(|e| e.path == "z.txt").unwrap();
            let out = tempfile::tempdir().unwrap();
            let mut sink = TestSink::default();
            r.extract(Some(&[idx]), out.path(), &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
            assert!(sink.errors.is_empty(), "{:?}", sink.errors);
            assert_eq!(std::fs::read(out.path().join("z.txt")).unwrap(), b"zulu");
            assert!(!out.path().join("dir/big.bin").exists());
        }
    }

    #[test]
    fn rar_passwords() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("secret.rar");
        std::fs::write(&p, build(ArchiveVersion::Rar50, false, Some("pw"))).unwrap();
        let mut r = open(&p, None).unwrap();
        let idx = r.entries().unwrap().iter().position(|e| e.path == "z.txt").unwrap();
        assert!(r.entries().unwrap()[idx].encrypted);
        assert_eq!(r.open_entry(idx).err(), Some(ArchiveError::PasswordRequired));
        // RAR5 stores a password check value: a wrong one is refused on open.
        assert_eq!(open(&p, Some("nope")).err(), Some(ArchiveError::WrongPassword));
        let mut r = open(&p, Some("pw")).unwrap();
        assert_eq!(read_entry(r.as_mut(), "z.txt"), b"zulu");
        test_ok(r.as_mut());
    }
}

// ---- byte-split volumes ----------------------------------------------------------------

#[test]
fn split_7z_and_zip_volumes() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    for format in [ArchiveFormat::SevenZ, ArchiveFormat::Zip] {
        let whole = d.path().join(format!("whole.{}", ext(format)));
        soyuz_archive::create(&whole, std::slice::from_ref(&tree), &opts(format, d.path()), &OpControl::new(), &mut ())
            .unwrap();
        let bytes = std::fs::read(&whole).unwrap();
        for (i, chunk) in bytes.chunks(bytes.len() / 3 + 1).enumerate() {
            std::fs::write(d.path().join(format!("split.{}.{:03}", ext(format), i + 1)), chunk).unwrap();
        }
        let first = d.path().join(format!("split.{}.001", ext(format)));
        assert_eq!(detect(&first), Some(format));
        let mut r = open(&first, None).unwrap_or_else(|e| panic!("{format:?}: {e}"));
        assert_eq!(read_entry(r.as_mut(), "tree/sub/big.bin"), noise(1_500_000, 7), "{format:?}");
        test_ok(r.as_mut());
        let snap = extract_all(r.as_mut());
        assert_eq!(snap["tree/hello.txt"], Some(b"hello world\n".to_vec()));
    }
}

#[test]
fn bomb_limits_apply_to_new_readers() {
    let d = tempfile::tempdir().unwrap();
    let iso = d.path().join("disc.iso");
    std::fs::write(&iso, build_iso(&sample_tree(), Names::RockRidge, false)).unwrap();
    let mut cpio = Vec::new();
    cpio_entry(&mut cpio, "big.bin", 0o100644, 1, 1, &vec![0u8; 200_000]);
    cpio_entry(&mut cpio, "TRAILER!!!", 0, 0, 1, b"");
    let rpm = d.path().join("zeros.rpm");
    std::fs::write(&rpm, rpm_with(gz(&cpio))).unwrap();
    for p in [iso, rpm] {
        let mut r = open(&p, None).unwrap();
        let mut o = ExtractOptions::default();
        o.limits.max_entry_bytes = 1000;
        let mut sink = TestSink { on_error: Some(soyuz_core::ErrorAction::Abort), ..Default::default() };
        let res = r.extract(None, &d.path().join("out"), &o, &OpControl::new(), &mut sink);
        assert!(matches!(res, Err(ArchiveError::LimitExceeded(_))), "{}: {res:?}", p.display());
        // Reading one entry outside a job is capped as well.
        let idx = r.entries().unwrap().iter().position(|e| e.size > 1000).unwrap();
        assert!(matches!(
            soyuz_archive::open_entry_capped(r.as_mut(), idx, 1000).err(),
            Some(ArchiveError::LimitExceeded(_))
        ));
    }
}
