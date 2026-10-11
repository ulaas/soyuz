//! SMB provider against a real Samba server started unprivileged on a high port in a
//! temp directory. Runs only with `SOYUZ_SMB_TESTS=1` and `smbd` + `pdbedit` installed.
#![cfg(not(windows))]

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use soyuz_core::FsErrorKind;
use soyuz_vfs::local::LocalProvider;
use soyuz_vfs::provider::stat_opt;
use soyuz_vfs::registry::Credentials;
use soyuz_vfs::smb::SmbProvider;
use soyuz_vfs::transfer::{FileCopy, NoHooks, copy_file};
use soyuz_vfs::{Provider, RemoteUri, WriteOpts};

const PASSWORD: &str = "soyuz-test-pw";

struct Smbd(Child, std::path::PathBuf);

impl Drop for Smbd {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        // smbd starts samba-dcerpcd / rpcd_* helpers in their own sessions: stop every
        // process started with this test's (unique, temporary) config file.
        let _ = Command::new("pkill").arg("-f").arg(format!("configfile={}", self.1.display())).status();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn start_samba(dir: &Path, port: u16) -> Smbd {
    for d in ["private", "lock", "state", "cache", "pid", "log", "ncalrpc", "share"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let user = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout).unwrap();
    let user = user.trim();
    let d = dir.display();
    let conf = format!(
        "[global]
  smb ports = {port}
  bind interfaces only = yes
  interfaces = lo
  private dir = {d}/private
  lock directory = {d}/lock
  state directory = {d}/state
  cache directory = {d}/cache
  pid directory = {d}/pid
  ncalrpc dir = {d}/ncalrpc
  log file = {d}/log/log.%m
  passdb backend = tdbsam:{d}/private/passdb.tdb
  map to guest = Bad User
  guest account = {user}
  server role = standalone server
  load printers = no
  printing = bsd
  printcap name = /dev/null
  disable spoolss = yes
  server min protocol = SMB2_10
[pub]
  path = {d}/share
  read only = no
  guest ok = yes
[secret]
  path = {d}/share
  read only = no
  guest ok = no
  valid users = {user}
"
    );
    let conf_path = dir.join("smb.conf");
    std::fs::write(&conf_path, conf).unwrap();
    let mut pdb = Command::new("pdbedit")
        .arg("-s")
        .arg(&conf_path)
        .args(["-a", "-t", "-u", user])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("pdbedit");
    pdb.stdin.take().unwrap().write_all(format!("{PASSWORD}\n{PASSWORD}\n").as_bytes()).unwrap();
    assert!(pdb.wait().unwrap().success(), "pdbedit failed");
    let child = Command::new("smbd")
        .args(["--foreground", "--no-process-group", "-s"])
        .arg(&conf_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .expect("smbd");
    let s = Smbd(child, conf_path.clone());
    let until = Instant::now() + Duration::from_secs(20);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < until, "smbd did not start listening on {port}");
        std::thread::sleep(Duration::from_millis(100));
    }
    s
}

fn connect(port: u16, user: Option<&str>, pw: Option<&str>) -> Result<SmbProvider, soyuz_core::FsError> {
    let uri = match user {
        Some(u) => format!("smb://{u}@127.0.0.1:{port}/"),
        None => format!("smb://127.0.0.1:{port}/"),
    };
    let uri = RemoteUri::parse(&uri).unwrap();
    SmbProvider::connect(uri, Credentials { secret: pw.map(str::to_string), ..Default::default() })
}

fn read_all(p: &dyn Provider, path: &str, offset: u64) -> Vec<u8> {
    let mut v = Vec::new();
    p.open_read(path, offset).unwrap().read_to_end(&mut v).unwrap();
    v
}

#[test]
fn smb_provider_against_samba() {
    if std::env::var_os("SOYUZ_SMB_TESTS").is_none() {
        eprintln!("skipped: set SOYUZ_SMB_TESTS=1 (needs smbd and pdbedit)");
        return;
    }
    // Short path: Samba's messaging sockets must fit in `sun_path`.
    let dir = tempfile::Builder::new().prefix("smbt").tempdir_in("/tmp").unwrap();
    let port = free_port();
    let _smbd = start_samba(dir.path(), port);
    let share_dir = dir.path().join("share");
    std::fs::create_dir(share_dir.join("docs")).unwrap();
    std::fs::write(share_dir.join("docs").join("a b.txt"), b"hello smb").unwrap();
    let user = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout).unwrap();
    let user = user.trim().to_string();

    // Wrong password: a sign-in error (PermissionDenied → the app's sign-in dialog).
    let e = connect(port, Some(&user), Some("wrong")).err().expect("wrong password must fail");
    assert_eq!(e.kind, FsErrorKind::PermissionDenied, "{e}");

    // Guest: shares are listed, the guest share is readable.
    let guest = connect(port, None, None).unwrap();
    let shares: Vec<String> = guest.list("/").unwrap().into_iter().map(|e| e.name).collect();
    assert!(shares.contains(&"pub".to_string()), "{shares:?}");
    assert!(!shares.iter().any(|s| s.ends_with('$')), "admin shares are hidden: {shares:?}");
    assert_eq!(read_all(&guest, "/pub/docs/a b.txt", 0), b"hello smb");

    // Domain form of the user name (Samba ignores an unknown workgroup for local users).
    let smb = connect(port, Some(&format!("WORKGROUP;{user}")), Some(PASSWORD)).unwrap();
    assert!(smb.stat("/").unwrap().is_dir);
    assert!(smb.stat("/secret").unwrap().is_dir);
    assert_eq!(smb.stat("/nosuchshare").unwrap_err().kind, FsErrorKind::NotFound);
    let docs = smb.list("/secret/docs").unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].name, "a b.txt");
    assert_eq!(docs[0].size, 9);
    assert!(docs[0].modified.is_some());
    assert!(smb.stat("/secret/docs").unwrap().is_dir);
    assert_eq!(smb.stat("/secret/nope").unwrap_err().kind, FsErrorKind::NotFound);
    assert_eq!(read_all(&smb, "/secret/docs/a b.txt", 6), b"smb");

    // mkdir, upload through the transfer engine (temp + rename), range read.
    smb.mkdir("/secret/up").unwrap();
    assert_eq!(smb.mkdir("/secret/up").unwrap_err().kind, FsErrorKind::AlreadyExists);
    let local_dir = tempfile::tempdir().unwrap();
    let src = local_dir.path().join("big.bin");
    let data: Vec<u8> = (0..9_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&src, &data).unwrap();
    let s = src.to_string_lossy().into_owned();
    let local = LocalProvider;
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    copy_file(
        &FileCopy {
            src: &local,
            src_path: &s,
            dst: &smb,
            dst_path: "/secret/up/big.bin",
            size: data.len() as u64,
            modified: Some(mtime),
            replace: false,
        },
        &NoHooks,
    )
    .unwrap();
    assert_eq!(std::fs::read(share_dir.join("up").join("big.bin")).unwrap(), data);
    assert!(!share_dir.join("up").join(".big.bin.soyuzpart").exists());
    let st = smb.stat("/secret/up/big.bin").unwrap();
    assert_eq!(st.size, data.len() as u64);
    assert_eq!(st.modified, Some(mtime), "set_times keeps the source date");
    assert_eq!(read_all(&smb, "/secret/up/big.bin", 8_999_990), &data[8_999_990..]);

    // Download through the transfer engine.
    let back = local_dir.path().join("back.bin");
    let b = back.to_string_lossy().into_owned();
    copy_file(
        &FileCopy {
            src: &smb,
            src_path: "/secret/up/big.bin",
            dst: &local,
            dst_path: &b,
            size: data.len() as u64,
            modified: None,
            replace: false,
        },
        &NoHooks,
    )
    .unwrap();
    assert_eq!(std::fs::read(&back).unwrap(), data);

    // Resumed write continues a partial file at its size.
    std::fs::write(share_dir.join("up").join("part.bin"), b"01234").unwrap();
    let mut w = smb.create_write("/secret/up/part.bin", &WriteOpts { resume: true, size_hint: Some(10) }).unwrap();
    assert_eq!(w.resumed_at(), 5);
    w.write_all(b"56789").unwrap();
    w.commit().unwrap();
    assert_eq!(std::fs::read(share_dir.join("up").join("part.bin")).unwrap(), b"0123456789");
    // Without resume it truncates; an aborted write without keep_partial is removed.
    let mut w = smb.create_write("/secret/up/part.bin", &WriteOpts::default()).unwrap();
    assert_eq!(w.resumed_at(), 0);
    w.write_all(b"x").unwrap();
    w.abort(false);
    assert!(stat_opt(&smb, "/secret/up/part.bin").unwrap().is_none());

    // Rename, cross-share rename, delete (non-empty folder refused), share-level ops.
    smb.rename("/secret/up/big.bin", "/secret/up/renamed.bin").unwrap();
    assert!(share_dir.join("up").join("renamed.bin").exists());
    assert_eq!(smb.rename("/secret/up/renamed.bin", "/pub/x.bin").unwrap_err().kind, FsErrorKind::CrossDevice);
    assert_eq!(smb.delete("/secret/up", true).unwrap_err().kind, FsErrorKind::NotEmpty);
    smb.delete("/secret/up/renamed.bin", false).unwrap();
    smb.delete("/secret/up", true).unwrap();
    assert!(!share_dir.join("up").exists());
    assert_eq!(smb.mkdir("/newshare").unwrap_err().kind, FsErrorKind::Unsupported);
    assert_eq!(smb.delete("/secret", true).unwrap_err().kind, FsErrorKind::Unsupported);

    // Names SMB forbids round-trip (mapped to private-use code points on the wire).
    let mut w = smb.create_write("/secret/what?.txt", &WriteOpts::default()).unwrap();
    w.write_all(b"q").unwrap();
    w.commit().unwrap();
    assert!(smb.list("/secret").unwrap().iter().any(|e| e.name == "what?.txt"));
    smb.delete("/secret/what?.txt", false).unwrap();

    // A hostile name: a Windows / Mac client stores `..\..\pub\victim.txt` (Samba keeps
    // the `\` as U+F026 on disk, smb2 decodes it back to `\`). It must not be listed, and a
    // recursive delete must not follow it into another share.
    std::fs::write(share_dir.join("victim.txt"), b"v").unwrap();
    std::fs::create_dir(share_dir.join("hostile")).unwrap();
    std::fs::write(share_dir.join("hostile").join("ok.txt"), b"o").unwrap();
    std::fs::write(share_dir.join("hostile").join("..\u{F026}..\u{F026}pub\u{F026}victim.txt"), b"x").unwrap();
    let names: Vec<String> = smb.list("/secret/hostile").unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["ok.txt"]);
    assert_eq!(smb.stat("/secret/hostile/..\\..\\pub\\victim.txt").unwrap_err().kind, FsErrorKind::InvalidName);
    let e = soyuz_vfs::provider::delete_tree(&smb, "/secret/hostile", true, &|| Ok(())).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::NotEmpty, "{e}");
    assert!(!share_dir.join("hostile").join("ok.txt").exists());
    assert_eq!(std::fs::read(share_dir.join("victim.txt")).unwrap(), b"v");
    assert!(smb.stat("/pub/victim.txt").is_ok());
}
