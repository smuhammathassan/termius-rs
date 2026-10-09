//! SSH engine over russh: connect, auth (password/pubkey/agent), interactive
//! shell (PTY), exec, SFTP, local/dynamic port forwarding, keepalives,
//! jump/proxy hosts. Replaces `@termius/libtermius` + the libssh2 native
//! addon from the Termius Electron app.
//!
//! # Ported JS surface
//!
//! The addon was driven by the background process with these calls — each
//! maps onto this crate as follows:
//!
//! | JS (libtermius) | here |
//! |---|---|
//! | `connect(options)` | [`Connector::connect`] / [`Session::connect`] |
//! | `auth(...)` | [`auth::authenticate`] (run during connect) |
//! | `exec(cmd, cb)` | [`SshClient::exec`] → [`ExecOutput`] |
//! | `write` / `read` | [`ChannelHandle::write`] / [`ChannelHandle::recv`] |
//! | `resize(rows, cols)` | [`ChannelHandle::resize`] / [`Session::resize`] |
//! | `sftp()` | [`SshClient::sftp`] → [`SftpClient`] |
//! | `disconnect()` | [`SshClient::disconnect`] |
//!
//! `ConnectionParams` (from `termius-core`) supplies keepalive interval and
//! count max, connect timeout, proxy/jump host, terminal size and the
//! forwarding rules; failures surface as [`SshError`], which preserves the
//! auth / host-key / network / timeout distinction the UI renders.

pub mod auth;
pub mod client;
pub mod error;
pub mod forwarding;
pub mod shell;
pub mod sftp;

pub use auth::{plan as plan_auth, AuthPlan};
pub use client::{
    host_key_decision, Connector, ExecOutput, HostKeyDecision, Session, SharedHandle, SshClient,
    TermiusHandler,
};
pub use error::{Result, SshError};
pub use forwarding::{
    parse_socks5_connect, parse_socks5_greeting, plan_forwardings, socks5_connect_reply,
    socks5_greeting_reply, DynamicRule, ForwardingPlan, ForwardingSet, LocalRule, SkippedRule,
    Socks5ParseError, Socks5Request,
};
pub use shell::{ChannelHandle, ExitEvent, DEFAULT_TERM};
pub use sftp::{join_path, SftpClient, SftpEntry, SftpStat};
