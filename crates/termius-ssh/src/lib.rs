//! SSH engine over russh: connect, auth (password/pubkey/agent), interactive shell (PTY), exec, SFTP, local/dynamic port forwarding, keepalives, jump/proxy hosts. Replaces @termius/libtermius + libssh2 native addon.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).

pub mod client;

pub mod auth;

pub mod shell;

pub mod sftp;

pub mod forwarding;

pub mod error;
