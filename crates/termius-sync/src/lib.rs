//! Termius cloud sync API client over `reqwest`: device-token auth,
//! host/snippet/keychain/group sync, brand theming, account. Replaces the JS
//! sync/saga layer of the Electron app (v10.1.3).
//!
//! # Example
//!
//! ```no_run
//! use termius_sync::SyncClient;
//!
//! # async fn demo() -> Result<(), termius_sync::SyncError> {
//! let client = SyncClient::new(termius_sync::DEFAULT_BASE_URL)?;
//!
//! // 1. device token (persist it where `deviceToken` lived before)
//! let device = client.device_info();
//! println!("device token: {}", device.token);
//!
//! // 2. sign in — stores the bearer token on the client
//! let account = client.signin("dev@example.com", "hunter2").await?;
//! println!("signed in as {:?}", account.bulk_account.account.email);
//!
//! // 3. sync
//! let hosts = client.pull_hosts(None).await?;
//! println!("pulled {} hosts", hosts.len());
//! # Ok(())
//! # }
//! ```
//!
//! # Evidence
//!
//! Endpoints come from `analysis/recovered-background/chunk_Pt.js`, set names
//! from `chunk_sfe.js`, request/response shapes and error classification from
//! the HTTP client (`b1e`/`x1e`) and sync saga in `entry.js`. Reconstructed
//! or unverified surfaces carry `// PORT-TODO` markers — see
//! [`models`] for wire shapes and [`client::endpoints`] for paths.
//!
//! # Security
//!
//! The bearer token and the device token are held in memory only, attached
//! via the `Authorization` header, never logged (`Debug` is redacted) and
//! never embedded in error values.

pub mod client;

pub mod models;

pub mod error;

pub use client::{
    authorization_value, endpoints, join_url, SyncClient, SyncClientBuilder, TokenScheme,
    DEFAULT_BASE_URL,
};
pub use error::SyncError;
pub use models::{
    hash_password, AccountInfo, BrandConfig, BulkAccount, Credentials, DeleteSets, DeviceInfo,
    DeviceRecord, HasWireId, Relation, SignInRequest, SignInResponse, SyncPullResponse,
    SyncPushRequest, TeamInfo, WireGroup, WireHost, WireIdentity, WireKeychain,
    WireKeychainEntry, WireKnownHost, WireSshConfig, WireSnippet, WireSnippetPackage,
    WireTelnetConfig, SET_DELETES, SET_DELETED, SET_GROUPS, SET_HOSTS, SET_KEYCHAINS,
    SET_KNOWN_HOSTS, SET_SNIPPETS,
};
