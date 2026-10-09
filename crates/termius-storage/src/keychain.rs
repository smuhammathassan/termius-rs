//! keychain — OS-keychain secret storage over the `keyring` crate.
//!
//! Replaces Termius' `@termius/keytar` wrapper (recovered `background/entry.js`
//! `class tW`: `getPassword(account)` / `setPassword(account, secret)` /
//! `deletePassword(account)`, all scoped by a `service` name). This port uses
//! [`KEYCHAIN_SERVICE`] as the service and `"<kind>:<record id>"` as the
//! account, so every host passphrase / key passphrase / identity password is
//! retrievable by its `termius-core` record id.
//!
//! Guarantees:
//! - no panics: an unavailable or locked keychain surfaces as
//!   [`StorageError::KeychainUnavailable`] / [`StorageError::KeychainAccess`];
//! - a missing entry is `Ok(None)` / `Ok(false)`, not an error;
//! - secrets are never logged — only service/account names appear in tracing.

use keyring::Entry;

use crate::error::{Result, StorageError};

/// Service name every Termius secret is registered under (the keytar `service`
/// argument of the original app). The Electron build used the app display name
/// (`Termius`, or `Termius (MAS)` for Mac App Store builds); the port keeps a
/// reverse-DNS constant so credentials stay stable across builds.
pub const KEYCHAIN_SERVICE: &str = "com.termius.rust";

const HOST_KIND: &str = "host";
const KEY_KIND: &str = "key";
const IDENTITY_KIND: &str = "identity";

fn account_for(kind: &str, record_id: &str) -> String {
    format!("{kind}:{record_id}")
}

/// Keychain account for a host's passphrase: `host:<record id>`.
pub fn host_account(record_id: &str) -> String {
    account_for(HOST_KIND, record_id)
}

/// Keychain account for an SSH key's passphrase: `key:<record id>`.
pub fn key_account(record_id: &str) -> String {
    account_for(KEY_KIND, record_id)
}

/// Keychain account for an identity's password: `identity:<record id>`.
pub fn identity_account(record_id: &str) -> String {
    account_for(IDENTITY_KIND, record_id)
}

/// Store (or overwrite) `secret` under `service` / `account`.
pub fn set_secret(service: &str, account: &str, secret: &str) -> Result<()> {
    let entry =
        Entry::new(service, account).map_err(|err| classify("create keychain entry", err))?;
    entry
        .set_password(secret)
        .map_err(|err| classify("store secret", err))?;
    tracing::debug!(%service, %account, "secret stored in OS keychain");
    Ok(())
}

/// Retrieve a secret; `Ok(None)` when no entry exists for that account.
pub fn get_secret(service: &str, account: &str) -> Result<Option<String>> {
    let entry =
        Entry::new(service, account).map_err(|err| classify("open keychain entry", err))?;
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(classify("read secret", err)),
    }
}

/// Delete a secret; `Ok(false)` when there was nothing to delete.
pub fn delete_secret(service: &str, account: &str) -> Result<bool> {
    let entry =
        Entry::new(service, account).map_err(|err| classify("open keychain entry", err))?;
    match entry.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(err) => Err(classify("delete secret", err)),
    }
}

/// Store a host's passphrase by host record id (service
/// [`KEYCHAIN_SERVICE`], account `host:<id>`).
pub fn set_host_passphrase(host_id: &str, secret: &str) -> Result<()> {
    set_secret(KEYCHAIN_SERVICE, &host_account(host_id), secret)
}

/// Retrieve a host's passphrase by host record id; `Ok(None)` when unset.
pub fn get_host_passphrase(host_id: &str) -> Result<Option<String>> {
    get_secret(KEYCHAIN_SERVICE, &host_account(host_id))
}

/// Delete a host's passphrase by host record id.
pub fn delete_host_passphrase(host_id: &str) -> Result<bool> {
    delete_secret(KEYCHAIN_SERVICE, &host_account(host_id))
}

/// Store an SSH key's passphrase by key record id (account `key:<id>`).
pub fn set_key_passphrase(key_id: &str, secret: &str) -> Result<()> {
    set_secret(KEYCHAIN_SERVICE, &key_account(key_id), secret)
}

/// Retrieve an SSH key's passphrase by key record id; `Ok(None)` when unset.
pub fn get_key_passphrase(key_id: &str) -> Result<Option<String>> {
    get_secret(KEYCHAIN_SERVICE, &key_account(key_id))
}

/// Delete an SSH key's passphrase by key record id.
pub fn delete_key_passphrase(key_id: &str) -> Result<bool> {
    delete_secret(KEYCHAIN_SERVICE, &key_account(key_id))
}

/// Store an identity's login password by identity record id
/// (account `identity:<id>`).
pub fn set_identity_password(identity_id: &str, secret: &str) -> Result<()> {
    set_secret(KEYCHAIN_SERVICE, &identity_account(identity_id), secret)
}

/// Retrieve an identity's login password by identity record id; `Ok(None)` when
/// unset.
pub fn get_identity_password(identity_id: &str) -> Result<Option<String>> {
    get_secret(KEYCHAIN_SERVICE, &identity_account(identity_id))
}

/// Delete an identity's login password by identity record id.
pub fn delete_identity_password(identity_id: &str) -> Result<bool> {
    delete_secret(KEYCHAIN_SERVICE, &identity_account(identity_id))
}

/// Map a `keyring` failure onto the typed storage errors: backend/platform
/// failures and inaccessible stores mean "keychain unavailable", everything
/// else (bad input, ambiguous match, encoding) is a per-operation access
/// failure. The OS message is preserved; secrets never are.
fn classify(operation: &str, err: keyring::Error) -> StorageError {
    // In `keyring` 3.x `PlatformFailure` wraps a boxed error while
    // `NoStorageAccess` wraps a String, so they cannot share an or-pattern.
    // Both mean "the OS keychain is unavailable"; everything else (bad input,
    // ambiguous match, encoding, missing entry) is a per-operation failure.
    match &err {
        keyring::Error::PlatformFailure(detail) => {
            StorageError::KeychainUnavailable(format!("{operation}: {detail}"))
        }
        keyring::Error::NoStorageAccess(detail) => {
            StorageError::KeychainUnavailable(format!("{operation}: {detail}"))
        }
        _ => StorageError::KeychainAccess(format!("{operation}: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn service_name_is_stable() {
        assert_eq!(KEYCHAIN_SERVICE, "com.termius.rust");
    }

    #[test]
    fn accounts_are_scoped_by_kind_and_record_id() {
        assert_eq!(host_account("h-1"), "host:h-1");
        assert_eq!(key_account("k-1"), "key:k-1");
        assert_eq!(identity_account("i-1"), "identity:i-1");
    }

    /// Live round-trip against the real OS keychain. Skipped by default because
    /// CI runners have no user-approved/unlocked keychain; opt in with
    /// `TERMIUS_KEYCHAIN_TESTS=1` on a desktop machine.
    #[test]
    fn live_keychain_round_trip_when_enabled() {
        if std::env::var_os("TERMIUS_KEYCHAIN_TESTS").is_none() {
            eprintln!("skipped: set TERMIUS_KEYCHAIN_TESTS=1 to exercise the OS keychain");
            return;
        }
        let account = format!("test:{}", Uuid::new_v4());
        set_secret(KEYCHAIN_SERVICE, &account, "correct horse battery staple")
            .expect("set secret");
        assert_eq!(
            get_secret(KEYCHAIN_SERVICE, &account).expect("get secret"),
            Some("correct horse battery staple".to_owned())
        );
        assert!(delete_secret(KEYCHAIN_SERVICE, &account).expect("delete secret"));
        assert_eq!(get_secret(KEYCHAIN_SERVICE, &account).expect("get deleted"), None);
        assert!(!delete_secret(KEYCHAIN_SERVICE, &account).expect("delete again"));
    }
}
