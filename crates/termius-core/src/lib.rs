//! Provider-neutral domain models for Termius: hosts, identities, port-forwarding, snippets, groups, known-hosts, keychains, vaults/clusters, connection parameters. Pure serde types + validation. No I/O.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).

pub mod host;

pub mod identity;

pub mod forwarding;

pub mod snippet;

pub mod group;

pub mod known_host;

pub mod keychain;

pub mod vault;

pub mod connection;

pub mod error;
