//! Port-forwarding configuration (local / remote / dynamic tunnels).

use serde::{Deserialize, Serialize};

use crate::RecordId;

/// Tunnel direction, mirroring Termius' port-forward `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ForwardType {
    /// Local: listen locally, forward into the remote network (SSH `-L`).
    #[default]
    Local,
    /// Remote: listen on the server, forward back to the client (SSH `-R`).
    Remote,
    /// Dynamic: SOCKS proxy on the client (SSH `-D`).
    Dynamic,
    /// Serial forwarding (listen on a serial port).
    LocalSerial,
    LocalSerialAuto,
    LocalAuto,
    RemoteAuto,
}

/// One port-forwarding rule bound to a host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PortForwardingConfig {
    pub id: RecordId,
    #[serde(rename = "type")]
    pub forward_type: ForwardType,
    /// Interface to bind the listener on (default loopback).
    pub listen_interface: String,
    pub listen_port: u16,
    /// Destination host the tunnel reaches (for local/remote).
    pub destination_host: Option<String>,
    pub destination_port: Option<u16>,
    /// Host this forwarding is attached to.
    pub host_id: Option<RecordId>,
    /// Bind the listener to all interfaces rather than loopback.
    pub bind_all: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl Default for PortForwardingConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            forward_type: ForwardType::Local,
            listen_interface: "127.0.0.1".into(),
            listen_port: 0,
            destination_host: None,
            destination_port: None,
            host_id: None,
            bind_all: false,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
