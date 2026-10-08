//! WebDAV provider conformance against the in-process test server.

use std::sync::Arc;

use soyuz_core::FsErrorKind;
use soyuz_vfs::RemoteUri;
use soyuz_vfs::dav_server::DavServer;
use soyuz_vfs::local::LocalProvider;
use soyuz_vfs::registry::{self, Credentials, Hooks, HostKeyPrompt};
use soyuz_vfs::transfer::{FileCopy, NoHooks, copy_file};

struct TestHooks;

impl Hooks for TestHooks {
    fn credentials(&self, uri: &RemoteUri) -> Credentials {
        let pw = if uri.user.as_deref() == Some("alice") { "wonder" } else { "nope" };
        Credentials { secret: Some(pw.into()), ..Default::default() }
    }
    fn confirm_host_key(&self, _: &HostKeyPrompt) -> bool {
        false
    }
}

#[test]
fn webdav_provider_against_local_server() {
    registry::set_hooks(Arc::new(TestHooks));
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("docs")).unwrap();
    std::fs::write(root.path().join("docs").join("a b.txt"), b"hello dav").unwrap();
    let server = DavServer::start(root.path(), Some(("alice".into(), "wonder".into()))).unwrap();

    // Wrong password → permission denied on the first listing.
    let bad = RemoteUri::parse(&format!("webdav://mallory@127.0.0.1:{}/", server.port)).unwrap();
    let p = registry::provider_for(&bad).unwrap();
    assert_eq!(p.list("/").unwrap_err().kind, FsErrorKind::PermissionDenied);

    let uri = RemoteUri::parse(&format!("webdav://alice@127.0.0.1:{}/", server.port)).unwrap();
    let dav = registry::provider_for(&uri).unwrap();
    let top = dav.list("/").unwrap();
    assert_eq!(top.len(), 1);
    assert!(top[0].is_dir && top[0].name == "docs");
    let docs = dav.list("/docs").unwrap();
    assert_eq!(docs[0].name, "a b.txt");
    assert_eq!(docs[0].size, 9);
    assert!(docs[0].modified.is_some());
    assert!(dav.stat("/docs").unwrap().is_dir);
    assert_eq!(dav.stat("/nope").unwrap_err().kind, FsErrorKind::NotFound);

    // mkdir + upload (streamed, chunked) through the transfer engine.
    dav.mkdir("/up").unwrap();
    assert_eq!(dav.mkdir("/up").unwrap_err().kind, FsErrorKind::AlreadyExists);
    let local_dir = tempfile::tempdir().unwrap();
    let src = local_dir.path().join("big.bin");
    let data: Vec<u8> = (0..600_000u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(&src, &data).unwrap();
    let s = src.to_string_lossy().into_owned();
    let local = LocalProvider;
    copy_file(
        &FileCopy {
            src: &local,
            src_path: &s,
            dst: &*dav,
            dst_path: "/up/big.bin",
            size: data.len() as u64,
            modified: None,
            replace: false,
        },
        &NoHooks,
    )
    .unwrap();
    assert_eq!(std::fs::read(root.path().join("up").join("big.bin")).unwrap(), data);
    assert!(!root.path().join("up").join(".big.bin.soyuzpart").exists());

    // Range read.
    let mut r = dav.open_read("/up/big.bin", 599_990).unwrap();
    let mut tail = Vec::new();
    std::io::Read::read_to_end(&mut r, &mut tail).unwrap();
    assert_eq!(tail, &data[599_990..]);

    // Download resume: a partial local file continues with a Range request.
    let dst = local_dir.path().join("down.bin");
    std::fs::write(local_dir.path().join(".down.bin.soyuzpart"), &data[..100_000]).unwrap();
    let d = dst.to_string_lossy().into_owned();
    let st = copy_file(
        &FileCopy {
            src: &*dav,
            src_path: "/up/big.bin",
            dst: &local,
            dst_path: &d,
            size: data.len() as u64,
            modified: None,
            replace: false,
        },
        &NoHooks,
    )
    .unwrap();
    assert_eq!(st.resumed_from, 100_000);
    assert_eq!(std::fs::read(&dst).unwrap(), data);

    // Rename (MOVE, no overwrite) and delete.
    dav.rename("/up/big.bin", "/up/renamed.bin").unwrap();
    assert!(root.path().join("up").join("renamed.bin").exists());
    dav.rename("/docs/a b.txt", "/docs/c.txt").unwrap();
    assert_eq!(dav.rename("/up/renamed.bin", "/docs/c.txt").unwrap_err().kind, FsErrorKind::AlreadyExists);
    dav.delete("/up/renamed.bin", false).unwrap();
    dav.delete("/up", true).unwrap();
    assert!(!root.path().join("up").exists());
}
