//! KnownHost: a pinned host key fingerprint for strict host-key verification.

use serde::{Deserialize, Serialize};

use crate::RecordId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KnownHost {
    pub id: RecordId,
    /// Hostname or address the key was observed on.
    pub host: String,
    pub port: u16,
    /// Host public key (OpenSSH known_hosts format, possibly hashed).
    pub public_key: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Default for KnownHost {
    fn default() -> Self {
        Self {
            id: String::new(),
            host: String::new(),
            port: 22,
            public_key: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
