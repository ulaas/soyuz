//! macOS Keychain: generic passwords through Security.framework (`SecItemAdd` /
//! `SecItemCopyMatching` / `SecItemUpdate` / `SecItemDelete`, via the
//! `security-framework` crate) with `kSecAttrService = "soyuz"` and
//! `kSecAttrAccount = <credential name>`, in the login keychain.
//!
//! The first read by a new build may show the Keychain's "allow access" prompt (the
//! item's access list names the binary that created it); call off the UI thread.

use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

use super::TARGET_PREFIX;
use crate::error::{AiError, AiResult};

/// `kSecAttrService` of every Soyuz item.
pub const SERVICE: &str = "soyuz";
/// `errSecItemNotFound`.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

/// The account (credential name) of a store target `soyuz:ai:<name>`.
fn account(target: &str) -> &str {
    target.strip_prefix(TARGET_PREFIX).unwrap_or(target)
}

fn fail(op: &str, e: &security_framework::base::Error) -> AiError {
    AiError::Credential(format!("Keychain {op} failed: {e} (OSStatus {})", e.code()))
}

pub fn store(target: &str, secret: &str) -> AiResult<()> {
    set_generic_password(SERVICE, account(target), secret.as_bytes()).map_err(|e| fail("write", &e))
}

pub fn load(target: &str) -> AiResult<Option<String>> {
    match get_generic_password(SERVICE, account(target)) {
        Ok(mut bytes) => {
            let s = String::from_utf8(bytes.clone())
                .map_err(|_| AiError::Credential("the Keychain item is not valid UTF-8".into()));
            bytes.fill(0);
            s.map(Some)
        }
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
        Err(e) => Err(fail("read", &e)),
    }
}

pub fn delete(target: &str) -> AiResult<()> {
    match delete_generic_password(SERVICE, account(target)) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(e) => Err(fail("delete", &e)),
    }
}
