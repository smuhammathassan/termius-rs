//! Telnet engine over raw tokio TCP with IAC option negotiation (echo, suppress-go-ahead, terminal-type, window-size). Mirrors Termius' JS telnet transport.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).

pub mod client;

pub mod negotiation;

pub mod error;
