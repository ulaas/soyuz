//! API keys in the OS credential store. `config.toml` only holds references:
//!
//! * `credential:<name>` — the OS store entry `soyuz:ai:<name>` (Windows Credential
//!   Manager via advapi32 `CredWriteW` / `CredReadW` / `CredDeleteW`; on Linux the
//!   freedesktop Secret Service over D-Bus, item `Soyuz: <name>` with the attributes
//!   `service = soyuz`, `account = <name>`; on macOS the login Keychain, a generic
//!   password with service `soyuz` and account `<name>`). The environment variable `SOYUZ_AI_KEY_<NAME>` (name upper-cased, non-alphanumerics → `_`) overrides
//!   it, for CI and headless use.
//! * `env:<VAR>` — read the environment variable `VAR`.
//! * empty — no key.

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

use crate::error::{AiError, AiResult};

/// Prefix of every entry Soyuz writes to the OS store.
pub const TARGET_PREFIX: &str = "soyuz:ai:";

/// A secret string: redacted in `Debug`, overwritten on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Secret(s.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Best effort: overwrite the bytes before the allocation is freed.
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        for b in bytes.iter_mut() {
            // `black_box` keeps the optimizer from eliding the wipe.
            *b = std::hint::black_box(0);
        }
        drop(std::hint::black_box(bytes));
    }
}

/// A parsed key reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyRef {
    None,
    Credential(String),
    Env(String),
}

/// Parse a `key = "…"` value. Anything that is not a reference is rejected, so a pasted
/// raw key never silently ends up working from `config.toml`.
pub fn parse_ref(s: &str) -> AiResult<KeyRef> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(KeyRef::None);
    }
    if let Some(name) = s.strip_prefix("credential:") {
        return if valid_name(name) {
            Ok(KeyRef::Credential(name.to_string()))
        } else {
            Err(AiError::Config(format!("invalid credential name “{name}” (use letters, digits, . _ -)")))
        };
    }
    if let Some(var) = s.strip_prefix("env:") {
        return if !var.is_empty() && var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            Ok(KeyRef::Env(var.to_string()))
        } else {
            Err(AiError::Config(format!("invalid environment variable name “{var}”")))
        };
    }
    Err(AiError::Config(
        "the key must be a reference (credential:<name> or env:<VAR>); raw keys are never stored in config.toml".into(),
    ))
}

/// Credential names: 1–64 of `A-Z a-z 0-9 . _ -`.
pub fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Environment variable that overrides `credential:<name>`.
pub fn env_override_var(name: &str) -> String {
    let up: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
    format!("SOYUZ_AI_KEY_{up}")
}

/// Resolve a key reference to the secret (environment override first).
pub fn resolve(reference: &str) -> AiResult<Option<Secret>> {
    match parse_ref(reference)? {
        KeyRef::None => Ok(None),
        KeyRef::Env(var) => Ok(std::env::var(&var).ok().filter(|v| !v.is_empty()).map(Secret)),
        KeyRef::Credential(name) => {
            if let Ok(v) = std::env::var(env_override_var(&name))
                && !v.is_empty()
            {
                return Ok(Some(Secret(v)));
            }
            load(&name)
        }
    }
}

/// The OS store target for a credential name.
pub fn target(name: &str) -> String {
    format!("{TARGET_PREFIX}{name}")
}

/// Is an OS credential store available on this system? On Linux this probes the session
/// bus for a Secret Service once per process (bounded by a few seconds; call it off the UI
/// thread the first time).
pub fn store_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::available()
    }
    #[cfg(not(target_os = "linux"))]
    {
        cfg!(any(windows, target_os = "macos"))
    }
}

/// Save (create or replace) a secret.
pub fn store(name: &str, secret: &Secret) -> AiResult<()> {
    check_name(name)?;
    imp::store(&target(name), secret.expose())
}

/// Read a secret; `Ok(None)` when there is no such entry.
pub fn load(name: &str) -> AiResult<Option<Secret>> {
    check_name(name)?;
    imp::load(&target(name)).map(|o| o.map(Secret))
}

/// Delete a secret; deleting a missing entry is not an error.
pub fn delete(name: &str) -> AiResult<()> {
    check_name(name)?;
    imp::delete(&target(name))
}

fn check_name(name: &str) -> AiResult<()> {
    if valid_name(name) { Ok(()) } else { Err(AiError::Credential(format!("invalid credential name “{name}”"))) }
}

#[cfg(windows)]
use windows as imp;

#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    //! No OS credential store backend on this platform.
    use crate::error::{AiError, AiResult};

    fn unsupported() -> AiError {
        AiError::Unsupported(
            "no OS credential store backend on this platform yet; use env:<VAR> or SOYUZ_AI_KEY_<NAME>".into(),
        )
    }

    pub fn store(_target: &str, _secret: &str) -> AiResult<()> {
        Err(unsupported())
    }

    pub fn load(_target: &str) -> AiResult<Option<String>> {
        Err(unsupported())
    }

    pub fn delete(_target: &str) -> AiResult<()> {
        Err(unsupported())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references() {
        assert_eq!(parse_ref("").unwrap(), KeyRef::None);
        assert_eq!(parse_ref("credential:anthropic-default").unwrap(), KeyRef::Credential("anthropic-default".into()));
        assert_eq!(parse_ref("env:MY_KEY").unwrap(), KeyRef::Env("MY_KEY".into()));
        assert!(parse_ref("sk-ant-raw-key").is_err(), "raw keys are rejected");
        assert!(parse_ref("credential:bad name").is_err());
        assert!(parse_ref("env:A-B").is_err());
        assert_eq!(env_override_var("anthropic-default"), "SOYUZ_AI_KEY_ANTHROPIC_DEFAULT");
        assert_eq!(format!("{:?}", Secret::new("sk")), "Secret(***)");
    }

    #[test]
    fn env_reference_resolves() {
        // `set_var` is unsafe in edition 2024; use PATH (always set) and an unset name.
        let var = format!("SOYUZ_AI_TEST_UNSET_{}", std::process::id());
        let path = resolve("env:PATH").unwrap();
        assert!(path.is_some());
        assert!(resolve(&format!("env:{var}")).unwrap().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_credential_round_trip() {
        let name =
            format!("soyuz-test-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
        assert_eq!(load(&name).unwrap(), None);
        store(&name, &Secret::new("first-sécret")).unwrap();
        assert_eq!(load(&name).unwrap().unwrap().expose(), "first-sécret");
        store(&name, &Secret::new("second")).unwrap();
        assert_eq!(resolve(&format!("credential:{name}")).unwrap().unwrap().expose(), "second");
        delete(&name).unwrap();
        assert_eq!(load(&name).unwrap(), None);
        delete(&name).unwrap();
    }
}
