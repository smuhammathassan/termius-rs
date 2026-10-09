//! Telnet engine over raw tokio TCP with IAC option negotiation (echo,
//! suppress-go-ahead, terminal-type, window-size, env vars). Mirrors Termius'
//! JS/native telnet transport.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered
//! sources). The Electron app never parsed IAC bytes in JS: its telnet shell
//! provider drove a `TelnetClient` from the `@termius/libtermius` native
//! addon (Rust core vendoring `core/telnet/libtelnet.c`). This crate
//! reproduces that transport behaviour in pure Rust:
//!
//! * [`NegotiationFilter`] — the protocol state machine: strips IAC sequences
//!   from the server stream, agrees to server-side `ECHO`/`SUPPRESS-GO-AHEAD`,
//!   refuses unsupported options, answers `TERMINAL-TYPE` (`xterm`), `NAWS`
//!   (window size) and `NEW-ENVIRON` (`TERM`/`USER`), and encodes client→server
//!   bytes (IAC doubling + NVT line endings).
//! * [`TelnetClient`] — connection lifecycle over `tokio::net::TcpStream`:
//!   `connect` (honouring `connect_timeout_secs`), `shell` (interactive
//!   async read/write session), `resize` (sends `NAWS`), `disconnect`.
//! * [`TelnetError`] — `thiserror` errors preserving DNS / connect / timeout /
//!   io / protocol / closed reasons.
//!
//! Typical use (reads yield payload bytes only; negotiation replies are
//! flushed internally):
//!
//! ```text
//! let mut client = TelnetClient::connect(&params).await?;
//! let mut session = client.shell();
//! session.write_all(b"ls\r").await?;          // tokio AsyncWriteExt
//! let n = session.read_payload(&mut buf).await?;
//! session.resize(TerminalSize { .. }).await?;
//! session.disconnect().await?;
//! ```

pub mod client;

pub mod negotiation;

pub mod error;

pub use client::{TelnetClient, TelnetSession};
pub use error::{Result, TelnetError};
pub use negotiation::{
    encode, NegotiationFilter, DEFAULT_TERMINAL_TYPE, OPT_ECHO, OPT_NAWS, OPT_NEW_ENVIRON,
    OPT_SGA, OPT_TTYPE,
};
