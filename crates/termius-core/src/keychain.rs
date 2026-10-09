//! Keychain: a named vault of secrets (passphrases / passwords), backed by the
//! OS keychain via the `keyring` crate in the storage layer.

use serde::{Deserialize, Serialize};

use crate::{RecordId, Timestamp};

/// One secret slot inside a keychain, keyed by an owner label (e.g. a key id).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeychainEntry {
    /// Owner record this secret belongs to (key id, host id, …).
    pub owner_id: RecordId,
    /// The secret passphrase/password. Never logged.
    pub secret: String,
}

/// A collection of named secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Keychain {
    pub id: RecordId,
    pub title: String,
    pub entries: Vec<KeychainEntry>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Default for Keychain {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            entries: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
