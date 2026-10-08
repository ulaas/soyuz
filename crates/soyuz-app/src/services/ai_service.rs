//! Background work for the AI settings page (roadmap A1): connection tests, credential
//! store reads / writes and the monthly usage ledger. Everything runs on worker threads;
//! results come back over channels and wake the UI.

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use soyuz_ai::credentials::{self, KeyRef};
use soyuz_ai::{NetSettings, ProbeReport, Provider, Secret, UsageLedger};
use soyuz_core::config::{Ai, AiProfile};

/// Repaint callback.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

fn spawn<T: Send + 'static>(name: &str, wake: Wake, f: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        let _ = tx.send(f());
        wake();
    });
    if let Err(e) = spawned {
        log::warn!("cannot spawn {name}: {e}");
    }
    rx
}

/// `ai-usage.json` next to `config.toml`.
pub fn ledger_path() -> PathBuf {
    soyuz_core::config::config_dir().join("ai-usage.json")
}

/// Load the usage ledger in the background.
pub fn load_ledger(wake: Wake) -> Receiver<Result<UsageLedger, String>> {
    spawn("soyuz-ai-ledger", wake, || UsageLedger::load(&ledger_path()).map_err(|e| e.to_string()))
}

/// Test a (draft) profile: resolve its key (or use the one typed in the page), then run the
/// connection test and tool-calling probe.
pub fn test_profile(profile: AiProfile, ai: &Ai, typed_key: Option<Secret>, wake: Wake) -> Receiver<ProbeReport> {
    let net = NetSettings::from_config(ai);
    spawn("soyuz-ai-test", wake, move || {
        let key = match typed_key {
            Some(k) => Ok(Some(k)),
            None => credentials::resolve(&profile.key),
        };
        let provider = key.and_then(|k| Provider::with_key(&profile, k, net));
        match provider {
            Ok(p) => soyuz_ai::provider::probe::run(&p),
            Err(e) => ProbeReport { error: Some(e.to_string()), ..ProbeReport::default() },
        }
    })
}

/// Save a secret under `name` in the OS credential store.
pub fn store_secret(name: String, secret: Secret, wake: Wake) -> Receiver<Result<(), String>> {
    spawn("soyuz-ai-cred", wake, move || credentials::store(&name, &secret).map_err(|e| e.to_string()))
}

/// Delete a stored secret.
pub fn delete_secret(name: String, wake: Wake) -> Receiver<Result<(), String>> {
    spawn("soyuz-ai-cred", wake, move || credentials::delete(&name).map_err(|e| e.to_string()))
}

/// Where a profile's key comes from, as shown in the settings page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyStatus {
    None,
    Stored,
    Missing,
    FromEnv(String),
    EnvMissing(String),
    Invalid(String),
}

/// Look up the key status of a reference (credential store read; background).
pub fn key_status(reference: String, wake: Wake) -> Receiver<KeyStatus> {
    spawn("soyuz-ai-cred", wake, move || match credentials::parse_ref(&reference) {
        Ok(KeyRef::None) => KeyStatus::None,
        Ok(KeyRef::Env(var)) => {
            if std::env::var_os(&var).is_some() {
                KeyStatus::FromEnv(var)
            } else {
                KeyStatus::EnvMissing(var)
            }
        }
        Ok(KeyRef::Credential(name)) => {
            let var = credentials::env_override_var(&name);
            if std::env::var_os(&var).is_some() {
                return KeyStatus::FromEnv(var);
            }
            match credentials::load(&name) {
                Ok(Some(_)) => KeyStatus::Stored,
                Ok(None) => KeyStatus::Missing,
                Err(e) => KeyStatus::Invalid(e.to_string()),
            }
        }
        Err(e) => KeyStatus::Invalid(e.to_string()),
    })
}

/// Credential name for a profile (`anthropic-default` style).
pub fn credential_name(profile: &str) -> String {
    let mut s: String = profile
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c.to_ascii_lowercase() } else { '-' })
        .collect();
    s.truncate(64);
    if s.trim_matches('-').is_empty() { "default".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_names_are_valid() {
        for n in ["Claude (work)", "", "ünïcode", "a/b\\c"] {
            assert!(credentials::valid_name(&credential_name(n)), "{n}");
        }
        assert_eq!(credential_name("Claude work"), "claude-work");
    }
}
