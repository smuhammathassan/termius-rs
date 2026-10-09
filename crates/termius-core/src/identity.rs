//! Identity (login user), Key (SSH keypair), and KeyType models.

use serde::{Deserialize, Serialize};

use crate::{RecordId, Timestamp};

/// Asymmetric key algorithm, mirroring Termius' key types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum KeyType {
    #[default]
    Rsa,
    Dsa,
    Ecdsa,
    Ed25519,
    /// X509 / certificate-based key.
    X509,
}

/// A stored SSH keypair (private + public), optionally passphrase-protected
/// via a keychain entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Key {
    pub id: RecordId,
    pub name: String,
    pub key_type: KeyType,
    /// Base64 or PEM private key material. Secret — stored via keychain when
    /// `keychain_id` is set, otherwise local encrypted storage.
    pub private_key: Option<String>,
    /// Public key (OpenSSH `authorized_keys` format).
    pub public_key: Option<String>,
    /// Keychain entry id holding the private-key passphrase, if protected.
    pub keychain_id: Option<RecordId>,
    /// Optional passphrase hint (never the secret itself).
    pub passphrase_hint: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Default for Key {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            key_type: KeyType::Ed25519,
            private_key: None,
            public_key: None,
            keychain_id: None,
            passphrase_hint: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}

/// A login identity: username + optional password/key/keychain bindings used
/// to authenticate to a host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Identity {
    pub id: RecordId,
    pub name: String,
    pub username: String,
    /// Stored password (secret). Prefer keychain-backed storage.
    pub password: Option<String>,
    /// Keychain entry id holding the password, if stored there.
    pub keychain_id: Option<RecordId>,
    /// SSH key id to authenticate with, if key-based.
    pub key_id: Option<RecordId>,
    /// Keychain entry id holding the key passphrase.
    pub key_passphrase_keychain_id: Option<RecordId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            username: String::new(),
            password: None,
            keychain_id: None,
            key_id: None,
            key_passphrase_keychain_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
