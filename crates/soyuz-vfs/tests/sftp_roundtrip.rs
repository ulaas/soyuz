//! SFTP provider conformance against an in-process russh SFTP server backed by a temp
//! folder: host-key trust (prompt, known_hosts, refusal), password and key-file auth,
//! listing, streaming upload with resume, range download, rename, delete, set_times.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use russh::keys::PrivateKey;
use russh::server::{Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version};
use soyuz_core::FsErrorKind;
use soyuz_vfs::RemoteUri;
use soyuz_vfs::local::LocalProvider;
use soyuz_vfs::registry::{self, Credentials, Hooks, HostKeyPrompt};
use soyuz_vfs::transfer::{FileCopy, NoHooks, copy_file};
use tokio::sync::Mutex;

/// Throwaway keys generated for this test only.
const HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCD7YikLDKpPOTqzqPXufijO8TyuVu/8We7MAib6CyE8wAAAJA9nP3VPZz9
1QAAAAtzc2gtZWQyNTUxOQAAACCD7YikLDKpPOTqzqPXufijO8TyuVu/8We7MAib6CyE8w
AAAEB5FbPrlZTkxs+UOk78TP3A6Lco1jkXTjrCUEs3pHm3WoPtiKQsMqk85OrOo9e5+KM7
xPK5W7/xZ7swCJvoLITzAAAACnNveXV6LXRlc3QBAgM=
-----END OPENSSH PRIVATE KEY-----
";
const USER_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABDLP7O+M9
DPeI0i8iD8uZ8HAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIFTIZl5GKVnnHmoQ
x7NVfocPsyho13y5OzURKmzxcyJqAAAAkF75amkjOhzYA+lRzY4eUuCjS0cmXvo99Ki8zp
7539PKAc3DapPCxlptBV/lJ4xLPA/qi9e/NlXMx9yA8pvBtO3NlKK22rlO9xYs1YqflIzY
dXRzOlzrP0mmhTZknUH78xDTHjnDJZJeuLks2+8S7aABDftZNXcFurWN6FJTb3huKfblJp
c2t0BKA3JSj+uJrw==
-----END OPENSSH PRIVATE KEY-----
";
const USER_PUB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIFTIZl5GKVnnHmoQx7NVfocPsyho13y5OzURKmzxcyJq";

// ---- server ---------------------------------------------------------------------------

#[derive(Clone)]
struct Server {
    root: PathBuf,
}

impl russh::server::Server for Server {
    type Handler = Ssh;
    fn new_client(&mut self, _: Option<SocketAddr>) -> Ssh {
        Ssh { root: self.root.clone(), channels: Arc::new(Mutex::new(HashMap::new())) }
    }
}

struct Ssh {
    root: PathBuf,
    channels: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
}

impl russh::server::Handler for Ssh {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "alice" && password == "wonder" { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_publickey(&mut self, user: &str, key: &russh::keys::PublicKey) -> Result<Auth, Self::Error> {
        let ok = user == "bob" && key.to_openssh().map(|s| s.contains(USER_PUB)).unwrap_or(false);
        Ok(if ok { Auth::Accept } else { Auth::reject() })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.lock().await.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_eof(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn subsystem_request(&mut self, id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        let ch = self.channels.lock().await.remove(&id);
        match (name, ch) {
            ("sftp", Some(ch)) => {
                session.channel_success(id)?;
                russh_sftp::server::run(ch.into_stream(), Sftp { root: self.root.clone(), ..Default::default() }).await;
            }
            _ => session.channel_failure(id)?,
        }
        Ok(())
    }
}

#[derive(Default)]
struct Sftp {
    root: PathBuf,
    files: HashMap<String, std::fs::File>,
    dirs: HashMap<String, Option<Vec<File>>>,
    next: u64,
}

fn ok(id: u32) -> Status {
    Status { id, status_code: StatusCode::Ok, error_message: "Ok".into(), language_tag: "en-US".into() }
}

fn code(e: std::io::Error) -> StatusCode {
    match e.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

impl Sftp {
    fn p(&self, path: &str) -> PathBuf {
        let mut out = self.root.clone();
        for seg in path.split('/').filter(|s| !s.is_empty() && *s != ".") {
            out.push(seg);
        }
        out
    }
    fn handle(&mut self) -> String {
        self.next += 1;
        format!("h{}", self.next)
    }
}

impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }

    async fn init(&mut self, _: u32, _: HashMap<String, String>) -> Result<Version, StatusCode> {
        Ok(Version::new())
    }

    async fn open(&mut self, id: u32, path: String, flags: OpenFlags, _: FileAttributes) -> Result<Handle, StatusCode> {
        let mut o = std::fs::OpenOptions::new();
        o.read(flags.contains(OpenFlags::READ))
            .write(flags.contains(OpenFlags::WRITE) || flags.contains(OpenFlags::APPEND))
            .create(flags.contains(OpenFlags::CREATE))
            .truncate(flags.contains(OpenFlags::TRUNCATE));
        let f = o.open(self.p(&path)).map_err(code)?;
        let h = self.handle();
        self.files.insert(h.clone(), f);
        Ok(Handle { id, handle: h })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, StatusCode> {
        self.files.remove(&handle);
        self.dirs.remove(&handle);
        Ok(ok(id))
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, StatusCode> {
        use std::io::{Read, Seek, SeekFrom};
        let f = self.files.get_mut(&handle).ok_or(StatusCode::Failure)?;
        f.seek(SeekFrom::Start(offset)).map_err(code)?;
        let mut buf = vec![0u8; len as usize];
        let n = f.read(&mut buf).map_err(code)?;
        if n == 0 {
            return Err(StatusCode::Eof);
        }
        buf.truncate(n);
        Ok(Data { id, data: buf })
    }

    async fn write(&mut self, id: u32, handle: String, offset: u64, data: Vec<u8>) -> Result<Status, StatusCode> {
        use std::io::{Seek, SeekFrom, Write};
        let f = self.files.get_mut(&handle).ok_or(StatusCode::Failure)?;
        f.seek(SeekFrom::Start(offset)).map_err(code)?;
        f.write_all(&data).map_err(code)?;
        Ok(ok(id))
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let m = std::fs::metadata(self.p(&path)).map_err(code)?;
        Ok(Attrs { id, attrs: FileAttributes::from(&m) })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        self.stat(id, path).await
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, StatusCode> {
        let f = self.files.get(&handle).ok_or(StatusCode::Failure)?;
        let m = f.metadata().map_err(code)?;
        Ok(Attrs { id, attrs: FileAttributes::from(&m) })
    }

    async fn setstat(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, StatusCode> {
        if let Some(t) = attrs.mtime {
            let f = std::fs::OpenOptions::new().write(true).open(self.p(&path)).map_err(code)?;
            f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(t as u64)).map_err(code)?;
        }
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, StatusCode> {
        let mut v = Vec::new();
        for e in std::fs::read_dir(self.p(&path)).map_err(code)?.flatten() {
            if let Ok(m) = e.metadata() {
                v.push(File::new(e.file_name().to_string_lossy(), FileAttributes::from(&m)));
            }
        }
        let h = self.handle();
        self.dirs.insert(h.clone(), Some(v));
        Ok(Handle { id, handle: h })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, StatusCode> {
        match self.dirs.get_mut(&handle).and_then(Option::take) {
            Some(files) => Ok(Name { id, files }),
            None => Err(StatusCode::Eof),
        }
    }

    async fn remove(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        std::fs::remove_file(self.p(&path)).map_err(code)?;
        Ok(ok(id))
    }

    async fn mkdir(&mut self, id: u32, path: String, _: FileAttributes) -> Result<Status, StatusCode> {
        std::fs::create_dir(self.p(&path)).map_err(code)?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        std::fs::remove_dir(self.p(&path)).map_err(code)?;
        Ok(ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        Ok(Name { id, files: vec![File::dummy(soyuz_vfs::uri::normalize_path(&path))] })
    }

    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, StatusCode> {
        if self.p(&to).exists() {
            return Err(StatusCode::Failure);
        }
        std::fs::rename(self.p(&from), self.p(&to)).map_err(code)?;
        Ok(ok(id))
    }
}

// ---- client hooks -----------------------------------------------------------------------

struct TestHooks {
    trust: AtomicBool,
    prompts: AtomicUsize,
    creds: parking_lot::Mutex<Credentials>,
}

impl Hooks for TestHooks {
    fn credentials(&self, _: &RemoteUri) -> Credentials {
        self.creds.lock().clone()
    }
    fn confirm_host_key(&self, p: &HostKeyPrompt) -> bool {
        self.prompts.fetch_add(1, Ordering::SeqCst);
        assert!(p.fingerprint.starts_with("SHA256:"), "{p:?}");
        assert_eq!(p.algorithm, "ssh-ed25519");
        self.trust.load(Ordering::SeqCst)
    }
}

#[test]
fn sftp_provider_against_in_process_server() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("docs")).unwrap();
    std::fs::write(root.path().join("docs").join("hello.txt"), b"hello sftp").unwrap();

    // Server on its own runtime.
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(russh::server::Config {
        keys: vec![PrivateKey::from_openssh(HOST_KEY).unwrap()],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let mut server = Server { root: root.path().to_path_buf() };
    rt.spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });

    let state = tempfile::tempdir().unwrap();
    registry::set_known_hosts(&state.path().join("known_hosts"));
    let hooks = Arc::new(TestHooks {
        trust: AtomicBool::new(false),
        prompts: AtomicUsize::new(0),
        creds: parking_lot::Mutex::new(Credentials { secret: Some("wonder".into()), ..Default::default() }),
    });
    registry::set_hooks(hooks.clone());
    let uri = RemoteUri::parse(&format!("sftp://alice@127.0.0.1:{port}/")).unwrap();

    // 1. Unknown host key, user says no → refused, nothing saved.
    let e = registry::provider_for(&uri).err().expect("untrusted key refused");
    assert_eq!(e.kind, FsErrorKind::PermissionDenied, "{e}");
    assert_eq!(hooks.prompts.load(Ordering::SeqCst), 1);
    assert!(registry::known_hosts().entries().is_empty());

    // 2. Trusted → saved to known_hosts; a reconnect does not ask again.
    hooks.trust.store(true, Ordering::SeqCst);
    registry::disconnect(&uri);
    let sftp = registry::provider_for(&uri).unwrap();
    assert_eq!(hooks.prompts.load(Ordering::SeqCst), 2);
    assert_eq!(registry::known_hosts().entries().len(), 1);
    registry::disconnect(&uri);
    let sftp2 = registry::provider_for(&uri).unwrap();
    assert_eq!(hooks.prompts.load(Ordering::SeqCst), 2, "known host: no prompt");
    drop(sftp2);

    // Listing and stat.
    let top = sftp.list("/").unwrap();
    assert!(top.iter().any(|e| e.name == "docs" && e.is_dir));
    let docs = sftp.list("/docs").unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].name, "hello.txt");
    assert_eq!(docs[0].size, 10);
    assert!(sftp.stat("/docs").unwrap().is_dir);
    assert_eq!(sftp.stat("/missing").unwrap_err().kind, FsErrorKind::NotFound);

    // Upload with resume: a partial temp file continues at its size.
    let local_dir = tempfile::tempdir().unwrap();
    let src = local_dir.path().join("big.bin");
    let data: Vec<u8> = (0..700_000u32).map(|i| (i % 241) as u8).collect();
    std::fs::write(&src, &data).unwrap();
    sftp.mkdir("/up").unwrap();
    std::fs::write(root.path().join("up").join(".big.bin.soyuzpart"), &data[..250_000]).unwrap();
    let s = src.to_string_lossy().into_owned();
    let local = LocalProvider;
    let st = copy_file(
        &FileCopy {
            src: &local,
            src_path: &s,
            dst: &*sftp,
            dst_path: "/up/big.bin",
            size: data.len() as u64,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000)),
            replace: false,
        },
        &NoHooks,
    )
    .unwrap();
    assert_eq!(st.resumed_from, 250_000);
    assert_eq!(std::fs::read(root.path().join("up").join("big.bin")).unwrap(), data);
    let up = sftp.stat("/up/big.bin").unwrap();
    assert_eq!(up.size, data.len() as u64);
    assert_eq!(
        up.modified.unwrap().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
        1_600_000_000,
        "set_times applied"
    );

    // Range read.
    let mut r = sftp.open_read("/up/big.bin", 699_000).unwrap();
    let mut tail = Vec::new();
    std::io::Read::read_to_end(&mut r, &mut tail).unwrap();
    assert_eq!(tail, &data[699_000..]);

    // Rename, delete.
    sftp.rename("/up/big.bin", "/up/b2.bin").unwrap();
    assert!(root.path().join("up").join("b2.bin").exists());
    sftp.delete("/up/b2.bin", false).unwrap();
    sftp.delete("/up", true).unwrap();
    assert!(!root.path().join("up").exists());

    // Wrong password → sign-in failure.
    registry::disconnect(&uri);
    *hooks.creds.lock() = Credentials { secret: Some("nope".into()), ..Default::default() };
    assert_eq!(registry::provider_for(&uri).err().unwrap().kind, FsErrorKind::PermissionDenied);

    // Key file with passphrase (user bob).
    let key_path = state.path().join("id_ed25519");
    std::fs::write(&key_path, USER_KEY).unwrap();
    *hooks.creds.lock() =
        Credentials { key_file: Some(key_path.clone()), secret: Some("pass phrase".into()), ..Default::default() };
    let bob = RemoteUri::parse(&format!("sftp://bob@127.0.0.1:{port}/")).unwrap();
    let p = registry::provider_for(&bob).unwrap();
    assert!(p.list("/docs").unwrap().iter().any(|e| e.name == "hello.txt"));
    // Wrong passphrase → a clear error.
    registry::disconnect(&bob);
    *hooks.creds.lock() = Credentials { key_file: Some(key_path), secret: Some("bad".into()), ..Default::default() };
    assert_eq!(registry::provider_for(&bob).err().unwrap().kind, FsErrorKind::PermissionDenied);
}
