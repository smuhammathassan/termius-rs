//! Mosh (mobile shell) support. The original Electron app shipped a native
//! `@termius/mosh` addon (`moshclient.node`) embedding mosh's SSH bootstrap
//! plus the SSHP state-synchronization protocol over UDP. Per the porting
//! plan this crate takes the pragmatic shell-out route: it drives the system
//! `mosh` binary as a child process and bridges the session as an async byte
//! stream, matching the app-facing `connect` / `read` / `write` / `resize` /
//! `disconnect` surface the SSH shell provider exposes. Marked experimental.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered
//! sources). Public surface:
//!
//! * [`MoshClient`] — spawn/bridge/teardown of a mosh session (which-style
//!   `mosh` detection, `--ssh` command construction, stderr-tail capture for
//!   bootstrap errors).
//! * [`MoshError`] — `thiserror` errors preserving client-not-found / spawn /
//!   connect / io / closed / unsupported reasons.
//! * [`find_mosh_binary`] / [`build_ssh_command`] — the PATH lookup and ssh
//!   command construction helpers, individually testable.

pub mod client;
pub mod error;

pub use client::{
    build_ssh_command, find_in_path, find_mosh_binary, MoshClient, DEFAULT_TERM,
};
pub use error::{MoshError, Result};
