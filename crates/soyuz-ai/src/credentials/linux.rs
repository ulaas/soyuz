//! Linux: the freedesktop Secret Service (`org.freedesktop.secrets`, provided by GNOME
//! Keyring, KWallet ≥ 5.97, KeePassXC …) over the session D-Bus. Safe code only.
//!
//! * Items go to the `default` collection (the login keyring) with the attributes
//!   `service = soyuz`, `account = <name>`, `application = soyuz` and the label
//!   `Soyuz: <name>`; `CreateItem` replaces an existing item with the same attributes.
//! * Secrets travel over a `plain` session: the session bus is local to this user, and the
//!   `dh-ietf1024-sha256-aes128-cbc-pkcs7` mode would need crypto crates for no real gain.
//! * Locked collections / items are unlocked through `Unlock`; the service may then show
//!   its own password prompt. Waiting for it is bounded by [`PROMPT_TIMEOUT`] (the prompt
//!   is dismissed afterwards), every method call by [`CALL_TIMEOUT`] and the bus connection
//!   by [`CONNECT_TIMEOUT`], so a missing / wedged service never hangs the caller.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use zbus::blocking::{Connection, Proxy};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::TARGET_PREFIX;
use crate::error::{AiError, AiResult};

const DEST: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_IFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_IFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_IFACE: &str = "org.freedesktop.Secret.Item";
const PROMPT_IFACE: &str = "org.freedesktop.Secret.Prompt";

/// Connecting to (and authenticating on) the session bus.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Any single method call (activation of the service included).
const CALL_TIMEOUT: Duration = Duration::from_secs(25);
/// A keyring unlock prompt the user has to answer.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// `(session, parameters, value, content_type)`: the Secret Service `Secret` struct.
type SecretStruct = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// The account name for a store target (`soyuz:ai:<name>` → `<name>`).
fn account(target: &str) -> &str {
    target.strip_prefix(TARGET_PREFIX).unwrap_or(target)
}

/// Lookup attributes of an entry (also what `CreateItem` stores).
fn attributes(account: &str) -> HashMap<&'static str, String> {
    HashMap::from([("service", "soyuz".to_string()), ("account", account.to_string())])
}

fn label(account: &str) -> String {
    format!("Soyuz: {account}")
}

/// `/` is the D-Bus "no object" path (no prompt needed, no such collection …).
fn is_none(p: &ObjectPath<'_>) -> bool {
    p.as_str() == "/"
}

fn err(op: &str, e: impl std::fmt::Display) -> AiError {
    AiError::Credential(format!("Secret Service {op} failed: {e}"))
}

fn unavailable(e: impl std::fmt::Display) -> AiError {
    AiError::Unsupported(format!(
        "no Secret Service (org.freedesktop.secrets) on the session bus ({e}); use env:<VAR> or SOYUZ_AI_KEY_<NAME>"
    ))
}

/// Connect to the session bus without ever blocking longer than [`CONNECT_TIMEOUT`].
fn session_bus() -> AiResult<Connection> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("soyuz-secret-connect".into())
        .spawn(move || {
            let c = zbus::blocking::connection::Builder::session().and_then(|b| b.method_timeout(CALL_TIMEOUT).build());
            let _ = tx.send(c);
        })
        .map_err(unavailable)?;
    match rx.recv_timeout(CONNECT_TIMEOUT) {
        Ok(Ok(c)) => Ok(c),
        Ok(Err(e)) => Err(unavailable(e)),
        Err(_) => Err(unavailable("connecting to the session bus timed out")),
    }
}

fn proxy<'a>(conn: &Connection, path: impl Into<ObjectPath<'a>>, iface: &'a str) -> AiResult<Proxy<'a>> {
    zbus::blocking::proxy::Builder::<Proxy<'a>>::new(conn)
        .destination(DEST)
        .and_then(|b| b.path(path.into()))
        .and_then(|b| b.interface(iface))
        .map(|b| b.cache_properties(CacheProperties::No))
        .and_then(|b| b.build())
        .map_err(|e| err("proxy", e))
}

/// An open Secret Service session.
struct Service<'a> {
    conn: &'a Connection,
    service: Proxy<'a>,
    session: OwnedObjectPath,
}

impl<'a> Service<'a> {
    fn open(conn: &'a Connection) -> AiResult<Service<'a>> {
        let service = proxy(conn, ObjectPath::from_static_str_unchecked(SERVICE_PATH), SERVICE_IFACE)?;
        let (_, session): (OwnedValue, OwnedObjectPath) =
            service.call("OpenSession", &("plain", Value::from(""))).map_err(|e| match e {
                // Nobody owns / can activate the name.
                zbus::Error::MethodError(name, ..) if name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown" => {
                    unavailable("the service is not running")
                }
                e => err("OpenSession", e),
            })?;
        Ok(Service { conn, service, session })
    }

    /// Run a prompt object (`/` = none) and wait for its `Completed` signal. Returns the
    /// prompt's result value, or `None` when there was no prompt.
    ///
    /// The wait runs on this (worker) thread with a deadline: there is no waiter thread
    /// that could outlive a failed `Prompt` call or a service that never answers, and the
    /// signal subscription (holding the bus connection) is dropped on every return path.
    fn prompt(&self, path: &OwnedObjectPath, what: &str) -> AiResult<Option<OwnedValue>> {
        if is_none(path) {
            return Ok(None);
        }
        let p = proxy(self.conn, path.as_ref(), PROMPT_IFACE)?;
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().map_err(|e| err("Prompt", e))?;
        // Subscribe before starting the prompt so the signal cannot be missed.
        let signals = rt.block_on(p.inner().receive_signal("Completed")).map_err(|e| err("Prompt", e))?;
        p.call::<_, _, ()>("Prompt", &("",)).map_err(|e| err("Prompt", e))?;
        let completed = wait_first(&rt, signals, PROMPT_TIMEOUT);
        match completed.map(|m| m.map(|m| m.body().deserialize::<(bool, OwnedValue)>())) {
            Some(Some(Ok((false, result)))) => Ok(Some(result)),
            Some(Some(Ok((true, _)))) => Err(AiError::Credential(format!("{what}: the keyring prompt was dismissed"))),
            Some(Some(Err(e))) => Err(err("Prompt", e)),
            Some(None) => Err(err("Prompt", "the bus connection closed")),
            None => {
                // Close the dialog (best effort; nobody waits for its `Completed`).
                let _ = p.call::<_, _, ()>("Dismiss", &());
                Err(AiError::Credential(format!("{what}: the keyring prompt timed out")))
            }
        }
    }

    /// Unlock `objects` (collections or items), prompting if the service wants to.
    fn unlock(&self, objects: &[OwnedObjectPath], what: &str) -> AiResult<()> {
        if objects.is_empty() {
            return Ok(());
        }
        let (unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
            self.service.call("Unlock", &(objects,)).map_err(|e| err("Unlock", e))?;
        if unlocked.len() == objects.len() {
            return Ok(());
        }
        let result = self.prompt(&prompt, what)?;
        let done = result.and_then(|v| Vec::<OwnedObjectPath>::try_from(v).ok()).unwrap_or_default();
        if done.len() + unlocked.len() >= objects.len() {
            Ok(())
        } else {
            Err(AiError::Credential(format!("{what}: the keyring stayed locked")))
        }
    }

    /// The `default` collection, created if the alias is not set yet, and unlocked.
    fn default_collection(&self) -> AiResult<OwnedObjectPath> {
        let what = "opening the default keyring";
        let path: OwnedObjectPath = self.service.call("ReadAlias", &("default",)).map_err(|e| err("ReadAlias", e))?;
        let path = if is_none(&path) {
            let props = HashMap::from([("org.freedesktop.Secret.Collection.Label", Value::from("Default keyring"))]);
            let (created, prompt): (OwnedObjectPath, OwnedObjectPath) =
                self.service.call("CreateCollection", &(props, "default")).map_err(|e| err("CreateCollection", e))?;
            if is_none(&created) {
                let v = self.prompt(&prompt, what)?;
                v.and_then(|v| OwnedObjectPath::try_from(v).ok())
                    .filter(|p| !is_none(p))
                    .ok_or_else(|| AiError::Credential(format!("{what}: no collection was created")))?
            } else {
                created
            }
        } else {
            path
        };
        let locked: bool =
            proxy(self.conn, path.as_ref(), COLLECTION_IFACE)?.get_property("Locked").map_err(|e| err("Locked", e))?;
        if locked {
            self.unlock(std::slice::from_ref(&path), what)?;
        }
        Ok(path)
    }

    /// Every item with our attributes, unlocked.
    fn find(&self, account: &str) -> AiResult<Vec<OwnedObjectPath>> {
        let (mut unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            self.service.call("SearchItems", &(attributes(account),)).map_err(|e| err("SearchItems", e))?;
        if !locked.is_empty() {
            self.unlock(&locked, "unlocking the keyring")?;
            unlocked.extend(locked);
        }
        Ok(unlocked)
    }
}

/// The first item of `stream`, waiting at most `timeout`: `None` on timeout,
/// `Some(None)` when the stream ended. The stream is dropped before returning.
fn wait_first<S: futures_util::Stream + Unpin>(
    rt: &tokio::runtime::Runtime,
    mut stream: S,
    timeout: Duration,
) -> Option<Option<S::Item>> {
    use futures_util::StreamExt;
    rt.block_on(async { tokio::time::timeout(timeout, stream.next()).await }).ok()
}

pub fn store(target: &str, secret: &str) -> AiResult<()> {
    let account = account(target);
    let conn = session_bus()?;
    let svc = Service::open(&conn)?;
    let coll = svc.default_collection()?;
    let mut attrs = attributes(account);
    attrs.insert("application", "soyuz".to_string());
    let props = HashMap::from([
        ("org.freedesktop.Secret.Item.Label", Value::from(label(account))),
        ("org.freedesktop.Secret.Item.Attributes", Value::from(attrs)),
    ]);
    let mut value = secret.as_bytes().to_vec();
    let s: SecretStruct = (svc.session.clone(), Vec::new(), value.clone(), "text/plain; charset=utf8".to_string());
    value.fill(0);
    let coll = proxy(&conn, coll.as_ref(), COLLECTION_IFACE)?;
    let res = coll.call::<_, _, (OwnedObjectPath, OwnedObjectPath)>("CreateItem", &(props, &s, true));
    let mut s = s;
    s.2.fill(0);
    let (item, prompt) = res.map_err(|e| err("CreateItem", e))?;
    if is_none(&item) {
        svc.prompt(&prompt, "saving the secret")?;
    }
    Ok(())
}

pub fn load(target: &str) -> AiResult<Option<String>> {
    let conn = session_bus()?;
    let svc = Service::open(&conn)?;
    let Some(item) = svc.find(account(target))?.into_iter().next() else {
        return Ok(None);
    };
    let item = proxy(&conn, item.as_ref(), ITEM_IFACE)?;
    let (mut s,): (SecretStruct,) = item.call("GetSecret", &(&svc.session,)).map_err(|e| err("GetSecret", e))?;
    let bytes = std::mem::take(&mut s.2);
    String::from_utf8(bytes).map(Some).map_err(|e| {
        let mut b = e.into_bytes();
        b.fill(0);
        AiError::Credential("stored secret is not UTF-8".into())
    })
}

pub fn delete(target: &str) -> AiResult<()> {
    let conn = session_bus()?;
    let svc = Service::open(&conn)?;
    for item in svc.find(account(target))? {
        let p = proxy(&conn, item.as_ref(), ITEM_IFACE)?;
        let prompt: OwnedObjectPath = p.call("Delete", &()).map_err(|e| err("Delete", e))?;
        svc.prompt(&prompt, "deleting the secret")?;
    }
    Ok(())
}

/// Is a Secret Service reachable (session bus up, and the name owned or activatable)?
/// Probed once per process.
pub fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let probe = || -> zbus::Result<bool> {
            let Ok(conn) = session_bus() else { return Ok(false) };
            let bus = zbus::blocking::proxy::Builder::<Proxy<'_>>::new(&conn)
                .destination("org.freedesktop.DBus")?
                .path("/org/freedesktop/DBus")?
                .interface("org.freedesktop.DBus")?
                .cache_properties(CacheProperties::No)
                .build()?;
            if bus.call::<_, _, bool>("NameHasOwner", &(DEST,))? {
                return Ok(true);
            }
            let names: Vec<String> = bus.call("ListActivatableNames", &())?;
            Ok(names.iter().any(|n| n == DEST))
        };
        probe().unwrap_or_else(|e| {
            log::info!("credential store: Secret Service probe failed: {e}");
            false
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_parts() {
        assert_eq!(account("soyuz:ai:anthropic-default"), "anthropic-default");
        assert_eq!(account("other"), "other");
        let a = attributes("x.y");
        assert_eq!(a.len(), 2);
        assert_eq!(a["service"], "soyuz");
        assert_eq!(a["account"], "x.y");
        assert_eq!(label("x.y"), "Soyuz: x.y");
        assert!(is_none(&ObjectPath::from_static_str_unchecked("/")));
        assert!(!is_none(&ObjectPath::from_static_str_unchecked("/org/freedesktop/secrets/prompt/p1")));
    }

    #[test]
    fn prompt_wait_is_bounded() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let t = std::time::Instant::now();
        let never = futures_util::stream::pending::<u8>();
        assert_eq!(wait_first(&rt, never, Duration::from_millis(50)), None, "a silent service times out");
        assert!(t.elapsed() < Duration::from_secs(5));
        assert_eq!(wait_first(&rt, futures_util::stream::iter([7u8]), Duration::from_secs(5)), Some(Some(7)));
        assert_eq!(wait_first(&rt, futures_util::stream::empty::<u8>(), Duration::from_secs(5)), Some(None));
    }

    /// Real keyring round trip; needs a desktop session (`SOYUZ_DBUS_TESTS=1`).
    #[test]
    fn secret_service_round_trip() {
        if std::env::var_os("SOYUZ_DBUS_TESTS").is_none() {
            eprintln!("skipped: set SOYUZ_DBUS_TESTS=1 to use the real Secret Service");
            return;
        }
        assert!(available(), "no Secret Service on the session bus");
        let name = format!("soyuz-test-{}", std::process::id());
        let t = super::super::target(&name);
        let _ = delete(&t);
        assert_eq!(load(&t).unwrap(), None);
        store(&t, "first-sécret").unwrap();
        assert_eq!(load(&t).unwrap().as_deref(), Some("first-sécret"));
        store(&t, "second").unwrap();
        assert_eq!(load(&t).unwrap().as_deref(), Some("second"), "replace=true keeps one item");
        delete(&t).unwrap();
        assert_eq!(load(&t).unwrap(), None);
        delete(&t).unwrap();
    }
}
