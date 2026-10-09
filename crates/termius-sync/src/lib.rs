//! Termius cloud sync API client over reqwest: device token auth, host/snippet/keychain/group sync, brand theming, account. Replaces the JS sync/saga layer.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).

pub mod client;

pub mod models;

pub mod error;
