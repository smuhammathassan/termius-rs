//! Local persistence: SQLite-backed repositories (hosts, identities, keys,
//! snippets, keychains, known-hosts, groups, port-forwardings, vaults),
//! versioned migrations, and OS-keychain (keyring) secret storage. Replaces
//! Termius' `schema.js` + repositories + `@termius/keytar`.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).
//!
//! # Layout
//! - [`db`] — [`Store`]: opens/creates the SQLite database at a per-platform
//!   data directory (`~/Library/Application Support/Termius/termius.db` on
//!   macOS), applies WAL + `foreign_keys` pragmas, and runs migrations.
//! - [`migrations`] — `user_version`-driven schema migrations (v1 = the nine
//!   record tables plus the `kv_store` localStorage replacement).
//! - [`repositories`] — typed CRUD (`create` / `get_by_id` / `list` /
//!   `update` / `delete`) per `termius-core` record type, plus a
//!   `SettingsRepository` for localStorage-style keys (`deviceToken`, …).
//! - [`keychain`] — OS-keychain secrets (`com.termius.rust` service) with
//!   helpers keyed by record id; unavailable keychains yield typed errors, and
//!   secrets are never logged.
//! - [`error`] — [`StorageError`] with preserved causes.
//!
//! # Example
//! ```
//! use termius_core::Host;
//! use termius_storage::Store;
//!
//! fn main() -> termius_storage::Result<()> {
//!     let store = Store::open(":memory:")?;
//!     let mut host = Host::default();
//!     host.label = "web-1".into();
//!     host.hostname = "example.com".into();
//!     let host = store.hosts().create(&host)?;
//!     assert_eq!(store.hosts().get_by_id(&host.id)?.label, "web-1");
//!     Ok(())
//! }
//! ```

pub mod db;
pub mod error;
pub mod keychain;
pub mod migrations;
pub mod repositories;

pub use db::{default_db_path, Store, APP_DIR_NAME, DB_FILE_NAME};
pub use error::{Result, StorageError};
pub use keychain::{
    delete_host_passphrase, delete_identity_password, delete_key_passphrase, delete_secret,
    get_host_passphrase, get_identity_password, get_key_passphrase, get_secret, host_account,
    identity_account, key_account, set_host_passphrase, set_identity_password, set_key_passphrase,
    set_secret, KEYCHAIN_SERVICE,
};
pub use migrations::{current_version as schema_version, run as run_migrations, LATEST_VERSION};
pub use repositories::{
    GroupRepository, HostRepository, IdentityRepository, KeyRepository, KeychainRepository,
    KnownHostRepository, PortForwardingRepository, SettingsRepository, SnippetRepository,
    Storable, VaultRepository, DEVICE_TOKEN_KEY,
};
