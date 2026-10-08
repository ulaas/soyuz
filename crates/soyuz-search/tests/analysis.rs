//! Folder sizes, size tree, duplicate finder and folder compare.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use soyuz_core::OpControl;
use soyuz_search::{
    CompareResult, DupeEvent, SizeEvent, TreeNode, compare_dirs, find_duplicates, folder_sizes, size_tree,
};

fn write(root: &Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

fn set_mtime(p: &Path, t: SystemTime) {
    fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
}

/// Sum of regular-file sizes / count of non-dir entries / dirs below root, via walkdir.
fn walkdir_totals(root: &Path) -> (u64, u64, u64) {
    let (mut bytes, mut files, mut dirs) = (0, 0, 0);
    for e in walkdir::WalkDir::new(root).min_depth(1) {
        let e = e.unwrap();
        let md = e.metadata().unwrap();
        if md.is_dir() {
            dirs += 1;
        } else {
            files += 1;
            if md.is_file() {
                bytes += md.len();
            }
        }
    }
    (bytes, files, dirs)
}

fn sized_tree() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    for d in 0..12 {
        for f in 0..30 {
            write(r, &format!("a{d}/b{}/f{f}.dat", d % 3), &vec![7u8; (d * 31 + f * 17) % 5000]);
        }
    }
    write(r, ".hidden/h.bin", &[1u8; 12345]);
    write(r, "big.iso", &[0u8; 300_000]);
    fs::create_dir_all(r.join("empty/deeper")).unwrap();
    write(r, "tiny1.txt", b"1");
    write(r, "tiny2.txt", b"22");
    t
}

#[test]
fn folder_sizes_equal_walkdir_sum() {
    let t = sized_tree();
    let a0 = t.path().join("a0");
    let (rx, _ctl) = folder_sizes(vec![t.path().to_path_buf(), a0.clone(), t.path().join("big.iso")], Arc::new(|| {}));
    let mut done = Vec::new();
    for ev in rx.iter() {
        match ev {
            SizeEvent::Done { path, bytes, files, dirs, errors } => done.push((path, bytes, files, dirs, errors)),
            SizeEvent::Partial { .. } => {}
        }
    }
    assert_eq!(done.len(), 3);
    let (b, f, d) = walkdir_totals(t.path());
    assert_eq!(done[0], (t.path().to_path_buf(), b, f, d, 0));
    let (b, f, d) = walkdir_totals(&a0);
    assert_eq!(done[1], (a0, b, f, d, 0));
    assert_eq!((done[2].1, done[2].2, done[2].3), (300_000, 1, 0));
}

#[test]
fn folder_sizes_cancel_disconnects() {
    let t = sized_tree();
    let (rx, ctl) = folder_sizes(vec![t.path().to_path_buf(); 50], Arc::new(|| {}));
    ctl.cancel();
    let n = rx.iter().count();
    assert!(n < 50, "{n} events after cancel");
}

#[cfg(unix)]
#[test]
fn folder_sizes_dedupe_hardlinks() {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "x/orig", &[1u8; 10_000]);
    fs::hard_link(t.path().join("x/orig"), t.path().join("x/link")).unwrap();
    let (rx, _) = folder_sizes(vec![t.path().to_path_buf()], Arc::new(|| {}));
    let done = rx.iter().find_map(|e| match e {
        SizeEvent::Done { bytes, files, .. } => Some((bytes, files)),
        _ => None,
    });
    assert_eq!(done, Some((10_000, 1)));
    let tree = size_tree(t.path(), 3, &OpControl::default()).unwrap();
    assert_eq!(tree.bytes, 10_000);
}

fn check_sorted(n: &TreeNode) {
    assert!(n.children.windows(2).all(|w| w[0].bytes >= w[1].bytes), "{:?}", n.path);
    let sum: u64 = n.children.iter().map(|c| c.bytes).sum();
    if !n.children.is_empty() {
        assert_eq!(sum, n.bytes, "children of {:?} must add up", n.path);
    }
    n.children.iter().for_each(check_sorted);
}

#[test]
fn size_tree_totals_and_folding() {
    let t = sized_tree();
    let ctl = OpControl::default();
    let tree = size_tree(t.path(), 2, &ctl).unwrap();
    let (b, f, _) = walkdir_totals(t.path());
    assert_eq!((tree.bytes, tree.files), (b, f));
    check_sorted(&tree);
    // big.iso dominates and is its own tile; small things are folded into "other".
    assert_eq!(tree.children[0].path, t.path().join("big.iso"));
    assert!(tree.children.iter().any(TreeNode::is_other));
    // depth 2 nodes have no children
    for c in &tree.children {
        for g in &c.children {
            assert!(g.children.is_empty());
        }
    }
    let flat = size_tree(t.path(), 0, &ctl).unwrap();
    assert!(flat.children.is_empty());
    assert_eq!(flat.bytes, b);

    let cancelled = OpControl::default();
    cancelled.cancel();
    assert!(size_tree(t.path(), 2, &cancelled).unwrap_err().is_cancel());
    assert!(size_tree(&t.path().join("missing"), 2, &ctl).is_err());
}

fn dupes(roots: Vec<PathBuf>, min: u64) -> (Vec<Vec<PathBuf>>, u64, u64) {
    let (rx, _ctl) = find_duplicates(roots, min, Arc::new(|| {}));
    let mut groups = Vec::new();
    let mut done = None;
    for ev in rx.iter() {
        match ev {
            DupeEvent::Group(g) => groups.push(g.into_iter().map(|e| e.path).collect::<Vec<_>>()),
            DupeEvent::Done { groups, wasted_bytes } => done = Some((groups, wasted_bytes)),
            DupeEvent::Progress { hashed, total, .. } => assert!(hashed <= total || total == 0),
        }
    }
    groups.sort();
    let (g, w) = done.expect("Done");
    (groups, g, w)
}

#[test]
fn duplicate_groups() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    // small identical trio
    for p in ["a/one.txt", "b/two.txt", "c/d/three.txt"] {
        write(r, p, b"same small content");
    }
    // big identical pair (> head size) + same-size impostor differing only at the tail
    let mut big = vec![9u8; 100_000];
    write(r, "big1.bin", &big);
    write(r, "z/big2.bin", &big);
    *big.last_mut().unwrap() = 1;
    write(r, "z/big3.bin", &big);
    // same size, different head
    write(r, "s1", b"aaaa");
    write(r, "s2", b"bbbb");
    // empty files are never duplicates
    write(r, "e1", b"");
    write(r, "e2", b"");

    let (groups, n, wasted) = dupes(vec![r.to_path_buf()], 0);
    assert_eq!(n, 2);
    assert_eq!(groups.len(), 2);
    let names: Vec<BTreeSet<String>> = groups
        .iter()
        .map(|g| g.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect())
        .collect();
    assert!(names.contains(&["one.txt", "two.txt", "three.txt"].iter().map(|s| s.to_string()).collect()));
    assert!(names.contains(&["big1.bin", "big2.bin"].iter().map(|s| s.to_string()).collect()));
    assert_eq!(wasted, 100_000 + 2 * 18);

    // min_size drops the small group; overlapping roots do not double count.
    let (groups, n, _) = dupes(vec![r.to_path_buf(), r.join("z")], 1000);
    assert_eq!(n, 1);
    assert_eq!(groups[0].len(), 2);
}

#[cfg(unix)]
#[test]
fn hardlinks_are_not_duplicates() {
    let t = tempfile::tempdir().unwrap();
    write(t.path(), "orig", b"payload payload");
    fs::hard_link(t.path().join("orig"), t.path().join("link")).unwrap();
    let (groups, n, _) = dupes(vec![t.path().to_path_buf()], 0);
    assert_eq!((groups.len(), n), (0, 0));
}

fn rel(items: &[&str]) -> Vec<PathBuf> {
    items.iter().map(PathBuf::from).collect()
}

#[test]
fn compare_categories() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    let (a, b) = (l.path(), r.path());
    let base = SystemTime::now() - Duration::from_secs(3600);
    let later = base + Duration::from_secs(600);
    let put = |root: &Path, p: &str, data: &[u8], t: SystemTime| {
        write(root, p, data);
        set_mtime(&root.join(p), t);
    };
    put(a, "same.txt", b"same", base);
    put(b, "same.txt", b"same", base);
    put(a, "sub/newer_l.txt", b"v2", later);
    put(b, "sub/newer_l.txt", b"v1", base);
    put(a, "newer_r.txt", b"old", base);
    put(b, "newer_r.txt", b"new!", later);
    put(a, "differ.txt", b"abcd", base);
    put(b, "differ.txt", b"abcdef", base);
    put(a, "touched.txt", b"identical", later);
    put(b, "touched.txt", b"identical", base);
    put(a, "only_l.txt", b"x", base);
    put(a, "only_l_dir/inner/deep.txt", b"x", base);
    put(b, "only_r.txt", b"y", base);
    put(b, "only_r_dir/x.txt", b"y", base);
    put(a, "clash", b"file", base);
    put(b, "clash/child.txt", b"dir", base);
    // same size + same mtime but different bytes: equal by date, different by content
    put(a, "sneaky.txt", b"aaaa", base);
    put(b, "sneaky.txt", b"bbbb", base);

    let ctl = OpControl::default();
    let res = compare_dirs(a, b, false, &ctl).unwrap();
    assert_eq!(
        res,
        CompareResult {
            only_left: rel(&["only_l.txt", "only_l_dir"]),
            only_right: rel(&["only_r.txt", "only_r_dir"]),
            newer_left: rel(&["sub/newer_l.txt", "touched.txt"]),
            newer_right: rel(&["newer_r.txt"]),
            differ: rel(&["clash", "differ.txt"]),
            same: 2,
        }
    );

    let res = compare_dirs(a, b, true, &ctl).unwrap();
    assert_eq!(res.newer_left, rel(&["sub/newer_l.txt"]));
    assert_eq!(res.differ, rel(&["clash", "differ.txt", "sneaky.txt"]));
    assert_eq!(res.same, 2); // same.txt + touched.txt (identical bytes)

    let cancelled = OpControl::default();
    cancelled.cancel();
    assert!(compare_dirs(a, b, false, &cancelled).unwrap_err().is_cancel());
    assert!(compare_dirs(a, &b.join("missing"), false, &ctl).is_err());
}
