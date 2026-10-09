//! Host: a saved remote endpoint (SSH/telnet/serial) with its label, address,
//! auth binding, group membership, and attached port-forwardings.

use serde::{Deserialize, Serialize};

use crate::{forwarding::PortForwardingConfig, RecordId, Timestamp};

/// Transport protocol for a host. Mirrors Termius' host `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum HostType {
    #[default]
    Ssh,
    Telnet,
    Mosh,
    Local,
    Serial,
}

/// A saved host entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Host {
    pub id: RecordId,
    /// Human-readable label shown in the host list.
    pub label: String,
    /// DNS name or IP address.
    pub hostname: String,
    pub port: u16,
    /// Login username (may be overridden by the bound identity).
    pub username: String,
    #[serde(rename = "type")]
    pub host_type: HostType,
    /// Groups this host belongs to.
    pub group_ids: Vec<RecordId>,
    /// Bound identity (login user) id, if any.
    pub identity_id: Option<RecordId>,
    /// Bound key id, if any.
    pub key_id: Option<RecordId>,
    /// Bound keychain (passphrase vault) id, if any.
    pub keychain_id: Option<RecordId>,
    /// Port-forwarding configurations attached to this host.
    #[serde(rename = "port_forwards")]
    pub port_forwardings: Vec<PortForwardingConfig>,
    /// Snippet ids quick-inserted for this host.
    pub snippet_ids: Vec<RecordId>,
    /// Known-host fingerprint id for host-key verification, if pinned.
    pub known_host_id: Option<RecordId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Free-form notes.
    pub notes: Option<String>,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            hostname: String::new(),
            port: 22,
            username: String::new(),
            host_type: HostType::Ssh,
            group_ids: Vec::new(),
            identity_id: None,
            key_id: None,
            keychain_id: None,
            port_forwardings: Vec::new(),
            snippet_ids: Vec::new(),
            known_host_id: None,
            created_at: String::new(),
            updated_at: String::new(),
            notes: None,
        }
    }
}

impl Host {
    /// Validate required, non-empty fields for a connectable host.
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.label.trim().is_empty() {
            return Err(crate::error::CoreError::validation("label", "must not be empty"));
        }
        if self.hostname.trim().is_empty() {
            return Err(crate::error::CoreError::validation("hostname", "must not be empty"));
        }
        if self.port == 0 {
            return Err(crate::error::CoreError::validation("port", "must be 1..=65535"));
        }
        Ok(())
    }
}
