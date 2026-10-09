//! Provider-neutral domain models for the Termius Rust port.
//!
//! These types mirror the data model of the Termius Electron app (v10.1.3),
//! reconstructed from REA static analysis. They are the shared contract every
//! other crate builds against: storage persists them, engines consume them,
//! the UI edits them, and the sync client replicates them.
//!
//! Field names serialize as `camelCase` to match Termius' wire/local-storage
//! representation exactly.

pub mod connection;
pub mod error;
pub mod forwarding;
pub mod group;
pub mod host;
pub mod identity;
pub mod keychain;
pub mod known_host;
pub mod snippet;
pub mod vault;

pub use connection::{
    AuthMethod, ConnectionParams, SshAlgorithm, TerminalSize, KEEPALIVE_DEFAULT_INTERVAL_SECS,
};
pub use error::CoreError;
pub use forwarding::{ForwardType, PortForwardingConfig};
pub use group::Group;
pub use host::Host;
pub use identity::{Identity, Key, KeyType};
pub use keychain::Keychain;
pub use known_host::KnownHost;
pub use snippet::Snippet;
pub use vault::Vault;

/// Stable identifier used across every Termius record. Termius uses UUIDv4
/// strings for all record ids.
pub type RecordId = String;

/// UTC timestamp in RFC 3339 (seconds precision), matching Termius' ISO dates.
pub type Timestamp = String;
