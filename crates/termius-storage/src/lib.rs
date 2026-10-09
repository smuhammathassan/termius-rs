//! Local persistence: SQLite-backed repositories (hosts, identities, snippets, keychains, known-hosts, groups, port-forwardings, brands), migrations, and OS keychain (keyring) secret storage. Replaces Termius schema.js + repositories + keytar.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).

pub mod db;

pub mod migrations;

pub mod repositories;

pub mod keychain;

pub mod error;
