//! Vault (Termius "Cluster"): the top-level container that groups hosts,
//! identities, keys, snippets, and port-forwardings and drives cloud sync.

use serde::{Deserialize, Serialize};

use crate::{RecordId, Timestamp};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Vault {
    pub id: RecordId,
    pub title: String,
    /// Whether this is the personal (default) vault.
    pub is_default: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Default for Vault {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: "Default".into(),
            is_default: true,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
