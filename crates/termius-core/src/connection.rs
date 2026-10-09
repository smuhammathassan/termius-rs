//! Connection parameters: the fully-resolved request an engine uses to open a
//! session, derived from a Host + Identity + Key + Keychain.

use serde::{Deserialize, Serialize};

use crate::forwarding::ForwardType;

/// Default keepalive interval (seconds) used when the caller does not override.
pub const KEEPALIVE_DEFAULT_INTERVAL_SECS: u64 = 30;

/// SSH key-exchange / host-key algorithm preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SshAlgorithm {
    #[default]
    Auto,
    SshDss,
    SshRsa,
    EcdsaSha2Nistp256,
    EcdsaSha2Nistp384,
    EcdsaSha2Nistp521,
    SshEd25519,
}

/// How to authenticate to the remote endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AuthMethod {
    #[default]
    None,
    Password(String),
    /// Private key material (PEM/OpenSSH), with optional passphrase.
    PrivateKey {
        private_key: String,
        passphrase: Option<String>,
    },
    /// SSH agent (via `SSH_AUTH_SOCK`).
    Agent,
}

/// Initial terminal dimensions for an interactive session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self { cols: 80, rows: 24, width_px: None, height_px: None }
    }
}

/// A resolved connection request handed to an engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectionParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
    pub algorithm: SshAlgorithm,
    /// Verify the server host key against a known-hosts pin.
    pub strict_host_key_checking: bool,
    pub keepalive_interval_secs: u64,
    pub keepalive_count_max: u32,
    /// Connect timeout in seconds.
    pub connect_timeout_secs: u64,
    pub terminal: TerminalSize,
    /// Proxy/jump host (SSH `-J`) address, if any.
    pub proxy_host: Option<String>,
    pub proxy_port: Option<u16>,
    pub proxy_username: Option<String>,
    /// Forwarding rules to establish (type, listen port, destination).
    pub forwardings: Vec<(ForwardType, u16, String, u16)>,
    /// Optional startup command to exec instead of an interactive shell.
    pub command: Option<String>,
}

impl Default for ConnectionParams {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            username: String::new(),
            auth: AuthMethod::None,
            algorithm: SshAlgorithm::Auto,
            strict_host_key_checking: false,
            keepalive_interval_secs: KEEPALIVE_DEFAULT_INTERVAL_SECS,
            keepalive_count_max: 3,
            connect_timeout_secs: 30,
            terminal: TerminalSize::default(),
            proxy_host: None,
            proxy_port: None,
            proxy_username: None,
            forwardings: Vec::new(),
            command: None,
        }
    }
}
