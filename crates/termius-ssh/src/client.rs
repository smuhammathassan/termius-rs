//! client — see crate docs.
//!
//! Session lifecycle for the SSH engine: TCP (optionally through a
//! jump/proxy host), SSH handshake, authentication, keepalives, and the
//! channel operations the Termius background engine exposed as
//! `connect(options)` / `exec(cmd)` / `shell()` / `resize(rows, cols)` /
//! `sftp()` / `disconnect()`.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use russh::client::{Handle, Handler};
use russh::ChannelMsg;
use termius_core::{ConnectionParams, SshAlgorithm, TerminalSize};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, instrument, warn};

use crate::auth;
use crate::error::{Result, SshError};
use crate::forwarding::{self, ForwardingSet};
use crate::shell::{self, ChannelHandle, ExitEvent, ShellCommand};
use crate::sftp::SftpClient;

/// Shared handle used to open new channels concurrently (forwarding
/// listeners, SFTP, exec, shell). `russh` channel opens need exclusive
/// access to the handle, so every opener serializes briefly on this mutex;
/// channels are independent once opened.
pub type SharedHandle = Arc<Mutex<Handle<TermiusHandler>>>;

/// Host-key acceptance decision for a presented server key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyDecision {
    Accept,
    Reject(&'static str),
}

/// Decide whether a server host key is acceptable.
///
/// * `strict && pinned` — the key must match the pin,
/// * `strict && !pinned` — PORT-TODO: should consult the `termius-core`
///   `KnownHost` store; `ConnectionParams` does not carry the store yet, so
///   we accept (and log) rather than rejecting every first connection,
/// * non-strict — accept, matching Termius' default `strict_host_key_check`
///   = off behavior.
pub fn host_key_decision(
    strict: bool,
    pinned_fingerprint: Option<&str>,
    presented_fingerprint: Option<&str>,
) -> HostKeyDecision {
    match (strict, pinned_fingerprint) {
        (true, Some(pin)) => match presented_fingerprint {
            Some(presented) if presented == pin => HostKeyDecision::Accept,
            Some(_) => HostKeyDecision::Reject("host key does not match the pinned key"),
            None => HostKeyDecision::Reject("no host key was presented for verification"),
        },
        _ => HostKeyDecision::Accept,
    }
}

/// Client-side SSH handler. Records whether a host key was rejected so the
/// connect phase can classify the resulting transport error as
/// [`SshError::HostKey`] instead of a generic connection failure.
pub struct TermiusHandler {
    strict_host_key_checking: bool,
    host_key_rejected: Arc<AtomicBool>,
}

impl TermiusHandler {
    pub fn new(strict_host_key_checking: bool) -> (Self, Arc<AtomicBool>) {
        let flag = Arc::new(AtomicBool::new(false));
        (
            Self { strict_host_key_checking, host_key_rejected: flag.clone() },
            flag,
        )
    }

    /// `true` once this handler refused a server host key.
    pub fn host_key_rejected(&self) -> bool {
        self.host_key_rejected.load(Ordering::SeqCst)
    }
}

impl Handler for TermiusHandler {
    type Error = russh::Error;

    // PORT-TODO: verify the `check_server_key` signature against russh 0.64
    // (`&PublicKey`, one argument) — this matches every recent release.
    // PORT-TODO: if russh's `Handler` still uses the `async-trait` crate,
    // add `#[async_trait]` here; native `async fn` impls (stable since 1.75,
    // and russh's current style) need no attribute.
    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        match host_key_decision(self.strict_host_key_checking, None, None) {
            HostKeyDecision::Accept => {
                debug!(key = ?server_public_key, "accepted server host key");
                Ok(true)
            }
            HostKeyDecision::Reject(reason) => {
                self.host_key_rejected.store(true, Ordering::SeqCst);
                warn!(reason, "rejecting server host key");
                Ok(false)
            }
        }
    }
}

/// Build the russh client config: keepalive ping interval from
/// `ConnectionParams`, and an inactivity timeout approximating
/// `keepalive_count_max` ("give up after N missed keepalives").
///
/// PORT-TODO: verify the `keepalive_interval` / `inactivity_timeout` field
/// names against russh 0.64's `client::Config`.
fn build_config(params: &ConnectionParams) -> Arc<russh::client::Config> {
    let keepalive = Duration::from_secs(params.keepalive_interval_secs);
    let inactivity = if keepalive.is_zero() {
        None
    } else {
        Some(keepalive * params.keepalive_count_max.max(1))
    };
    Arc::new(russh::client::Config {
        keepalive_interval: keepalive,
        inactivity_timeout: inactivity,
        ..Default::default()
    })
}

/// Resolve host:port up front so DNS failures are reported distinctly from
/// TCP failures (the JS surface surfaced them as separate error types).
async fn resolve(host: &str, port: u16) -> Result<SocketAddr> {
    let mut addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| SshError::connection(format!("dns resolution failed for {host}:{port}: {e}")))?;
    addrs.next().ok_or_else(|| {
        SshError::connection(format!("dns resolution returned no addresses for {host}:{port}"))
    })
}

/// A completed handshake: the session handle plus the shared flag telling us
/// whether a host key was rejected during it.
struct Handshake {
    handle: Handle<TermiusHandler>,
    host_key_rejected: Arc<AtomicBool>,
}

/// TCP connect + SSH handshake directly to the target, with deadline.
async fn open_direct(
    params: &ConnectionParams,
    config: Arc<russh::client::Config>,
    deadline: Duration,
) -> Result<Handshake> {
    let addr = resolve(&params.host, params.port).await?;
    let (handler, rejected) = TermiusHandler::new(params.strict_host_key_checking);
    let attempt = tokio::time::timeout(deadline, russh::client::connect(config, addr, handler));
    match attempt.await {
        Err(_) => Err(SshError::timeout(format!(
            "timed out connecting to {}:{}",
            params.host, params.port
        ))),
        Ok(Err((err, _handler))) => Err(classify_handshake(params, &rejected, err)),
        Ok(Ok(handle)) => Ok(Handshake { handle, host_key_rejected: rejected }),
    }
}

/// Classify a handshake failure using the handler's rejection flag (we do
/// not pattern-match on russh error variants — variant names shift between
/// releases).
fn classify_handshake(
    params: &ConnectionParams,
    rejected: &AtomicBool,
    err: russh::Error,
) -> SshError {
    if rejected.load(Ordering::SeqCst) {
        SshError::host_key(format!(
            "{}:{} refused the server host key",
            params.host, params.port
        ))
    } else {
        SshError::connection(format!(
            "ssh handshake with {}:{} failed: {err}",
            params.host, params.port
        ))
    }
}

/// Connect through a jump/proxy host (`ssh -J`): authenticate to the proxy,
/// open a `direct-tcpip` channel to the target, then run a second SSH
/// handshake inside it.
///
/// Returns the proxy session *and* the target session — the proxy handle
/// must outlive the tunnel, so `SshClient` keeps it alive.
async fn open_via_jump(
    params: &ConnectionParams,
    config: Arc<russh::client::Config>,
    proxy_host: &str,
    deadline: Duration,
) -> Result<(Handshake, Handshake)> {
    let proxy_port = params.proxy_port.unwrap_or(22);
    let proxy_username = params.proxy_username.as_deref().unwrap_or(&params.username);
    let jump_addr = resolve(proxy_host, proxy_port).await?;

    let stream = match tokio::time::timeout(deadline, tokio::net::TcpStream::connect(jump_addr))
        .await
    {
        Err(_) => {
            return Err(SshError::timeout(format!(
                "timed out connecting to proxy {proxy_host}:{proxy_port}"
            )));
        }
        Ok(Err(err)) => {
            return Err(SshError::connection(format!(
                "proxy connect to {proxy_host}:{proxy_port} failed: {err}"
            )));
        }
        Ok(Ok(stream)) => stream,
    };

    let (jump_handler, jump_rejected) = TermiusHandler::new(params.strict_host_key_checking);
    // PORT-TODO: `client::connect_over(config, stream, handler)` is the
    // stream-based constructor we expect on russh 0.64 — verify the name.
    let attempt = tokio::time::timeout(deadline, russh::client::connect_over(config.clone(), stream, jump_handler));
    let mut jump_handle = match attempt.await {
        Err(_) => {
            return Err(SshError::timeout(format!(
                "timed out handshaking with proxy {proxy_host}:{proxy_port}"
            )));
        }
        Ok(Err((err, _handler))) => {
            if jump_rejected.load(Ordering::SeqCst) {
                return Err(SshError::host_key(format!(
                    "proxy {proxy_host}:{proxy_port} refused the server host key"
                )));
            }
            return Err(SshError::connection(format!(
                "proxy handshake with {proxy_host}:{proxy_port} failed: {err}"
            )));
        }
        Ok(Ok(handle)) => handle,
    };

    // ConnectionParams has no separate proxy identity yet, so the jump host
    // authenticates with the same material as the target (PORT-TODO:
    // termius-core could add `proxy_auth`).
    auth::authenticate(&mut jump_handle, proxy_username, &params.auth).await?;

    // PORT-TODO: `channel_direct_tcpip` port argument types hedged with
    // `.into()` (u16 vs wire u32).
    let channel = jump_handle
        .channel_direct_tcpip(params.host.as_str(), params.port.into(), "127.0.0.1", 0u16.into())
        .await
        .map_err(|err| {
            SshError::connection(format!(
                "proxy {proxy_host}:{proxy_port} could not open a tunnel to {}:{}: {err}",
                params.host, params.port
            ))
        })?;

    let (target_handler, target_rejected) = TermiusHandler::new(params.strict_host_key_checking);
    // PORT-TODO: the tunneled channel is passed as the transport; if russh
    // requires an adapter, use `channel.into_stream()` here.
    let attempt = tokio::time::timeout(deadline, russh::client::connect_over(config, channel, target_handler));
    let target_handle = match attempt.await {
        Err(_) => {
            return Err(SshError::timeout(format!(
                "timed out handshaking with {}:{} through the proxy",
                params.host, params.port
            )));
        }
        Ok(Err((err, _handler))) => {
            if target_rejected.load(Ordering::SeqCst) {
                return Err(SshError::host_key(format!(
                    "{}:{} refused the server host key",
                    params.host, params.port
                )));
            }
            return Err(SshError::connection(format!(
                "ssh handshake with {}:{} through the proxy failed: {err}",
                params.host, params.port
            )));
        }
        Ok(Ok(handle)) => handle,
    };

    Ok((
        Handshake { handle: jump_handle, host_key_rejected: jump_rejected },
        Handshake { handle: target_handle, host_key_rejected: target_rejected },
    ))
}

/// Opens TCP connections, performs the SSH handshake and manages session
/// lifecycle — the `connect(options)` entry point from the JS surface.
#[derive(Debug, Clone, Default)]
pub struct Connector;

impl Connector {
    pub fn new() -> Self {
        Self
    }

    /// Establish a fully authenticated session, including keepalives,
    /// connect timeout, optional jump/proxy host and port forwardings.
    #[instrument(skip_all, fields(host = %params.host, port = params.port))]
    pub async fn connect(&self, params: &ConnectionParams) -> Result<SshClient> {
        if params.algorithm != SshAlgorithm::Auto {
            // PORT-TODO: russh exposes algorithm filtering through
            // `client::Config::preferred` with version-specific enum slices;
            // we deliberately do not guess those names. Defaults negotiate
            // modern algorithms; the pin applies once verified.
            warn!("ssh algorithm preference is not applied yet (PORT-TODO); using russh defaults");
        }

        let deadline = Duration::from_secs(params.connect_timeout_secs.max(1));
        let config = build_config(params);

        let proxy = params
            .proxy_host
            .as_deref()
            .map(str::trim)
            .filter(|host| !host.is_empty());

        let (mut handle, jump_handle, rejected) = match proxy {
            Some(proxy_host) => {
                let (jump, target) = open_via_jump(params, config, proxy_host, deadline).await?;
                // The jump session must outlive the target session — it is
                // stored (not dropped) so the tunnel stays up.
                (target.handle, Some(jump.handle), target.host_key_rejected)
            }
            None => {
                let direct = open_direct(params, config, deadline).await?;
                (direct.handle, None, direct.host_key_rejected)
            }
        };

        if rejected.load(Ordering::SeqCst) {
            return Err(SshError::host_key(format!(
                "{}:{} refused the server host key",
                params.host, params.port
            )));
        }

        auth::authenticate(&mut handle, &params.username, &params.auth).await?;

        let shared: SharedHandle = Arc::new(Mutex::new(handle));
        let forwardings = forwarding::start(shared.clone(), &params.forwardings).await?;

        Ok(SshClient {
            handle: shared,
            jump_handle,
            params: params.clone(),
            forwardings: Some(forwardings),
            shell_commands: None,
            host_key_rejected: rejected,
        })
    }
}

/// Result of a one-shot `exec` channel: stdout, stderr and the exit state,
/// mirroring what the addon's `exec(cmd, cb)` callback delivered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_status: Option<u32>,
    pub exit_signal: Option<String>,
}

impl ExecOutput {
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// `true` when the remote command reported exit status 0.
    pub fn success(&self) -> bool {
        matches!(self.exit_status, Some(0))
    }
}

/// An authenticated SSH session. All channel operations serialize their
/// handle access briefly; see [`SharedHandle`].
pub struct SshClient {
    handle: SharedHandle,
    /// Jump-host session, kept alive for the lifetime of the target session.
    jump_handle: Option<Handle<TermiusHandler>>,
    params: ConnectionParams,
    forwardings: Option<ForwardingSet>,
    /// Sender half of the active shell's command queue (so `resize` works
    /// even though the full [`ChannelHandle`] was handed to the caller).
    shell_commands: Option<mpsc::Sender<ShellCommand>>,
    host_key_rejected: Arc<AtomicBool>,
}

impl SshClient {
    /// The parameters this session was opened with.
    pub fn params(&self) -> &ConnectionParams {
        &self.params
    }

    /// `true` if this session rejected the server host key at any point.
    pub fn host_key_rejected(&self) -> bool {
        self.host_key_rejected.load(Ordering::SeqCst)
    }

    /// Ports actually bound by the session's port forwardings (empty when no
    /// rules were configured).
    pub fn forwarding_ports(&self) -> &[u16] {
        self.forwardings
            .as_ref()
            .map(|set| set.bound_ports())
            .unwrap_or(&[])
    }

    /// Run a one-shot command on a fresh exec channel and collect its
    /// stdout/stderr/exit status (`exec(cmd, cb)` from the JS surface).
    #[instrument(skip_all, fields(command = %command))]
    pub async fn exec(&mut self, command: &str) -> Result<ExecOutput> {
        let mut channel = {
            let mut guard = self.handle.lock().await;
            guard.channel_open_session().await
        }
        .map_err(|err| SshError::connection(format!("failed to open exec channel: {err}")))?;

        // PORT-TODO: `request_exec(&str)` — verify against russh 0.64.
        channel
            .request_exec(command)
            .await
            .map_err(|err| SshError::protocol(format!("exec request rejected: {err}")))?;

        let mut output = ExecOutput::default();
        loop {
            // PORT-TODO: `Channel::recv()` — verify against russh 0.64
            // (older releases called this `wait()`).
            match channel.recv().await {
                Some(ChannelMsg::Data { data }) => output.stdout.extend_from_slice(&data),
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    output.stderr.extend_from_slice(&data)
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    output.exit_status = Some(exit_status)
                }
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    output.exit_signal = Some(signal_name)
                }
                Some(_) => {}
                None => break,
            }
        }
        let _ = channel.close().await;
        debug!(
            exit_status = ?output.exit_status,
            stdout_len = output.stdout.len(),
            stderr_len = output.stderr.len(),
            "exec channel finished"
        );
        Ok(output)
    }

    /// Open an interactive PTY shell (`request_pty` + `request_shell`) and
    /// spawn the pump task that bridges it to a [`ChannelHandle`].
    ///
    /// The initial terminal size comes from `ConnectionParams::terminal`;
    /// later resizes go through [`resize`](Self::resize) or the returned
    /// handle.
    pub async fn shell(&mut self) -> Result<ChannelHandle> {
        if self.shell_commands.is_some() {
            return Err(SshError::protocol("a shell channel is already open on this session"));
        }

        let size = self.params.terminal;
        let mut channel = {
            let mut guard = self.handle.lock().await;
            guard.channel_open_session().await
        }
        .map_err(|err| SshError::connection(format!("failed to open shell channel: {err}")))?;

        let (cols, rows, width, height) = shell::pty_args(&size);
        // PORT-TODO: `request_pty(term, cols, rows, pixel_w, pixel_h, modes)`
        // and `request_shell()` signatures — verify against russh 0.64.
        channel
            .request_pty(
                shell::DEFAULT_TERM,
                cols.into(),
                rows.into(),
                width.into(),
                height.into(),
                &[],
            )
            .await
            .map_err(|err| SshError::protocol(format!("pty request rejected: {err}")))?;
        channel
            .request_shell()
            .await
            .map_err(|err| SshError::protocol(format!("shell request rejected: {err}")))?;

        let (tx_input, rx_input) = mpsc::channel::<Bytes>(64);
        let (tx_command, rx_command) = mpsc::channel::<ShellCommand>(16);
        let (tx_output, rx_output) = mpsc::channel::<Bytes>(256);
        let (tx_exit, rx_exit) = mpsc::channel::<ExitEvent>(1);

        let _task = tokio::spawn(async move {
            enum PumpEvent {
                Input(Option<Bytes>),
                Command(Option<ShellCommand>),
                Ssh(Option<ChannelMsg>),
            }

            let mut pending_input: VecDeque<Bytes> = VecDeque::new();
            let mut pending_commands: VecDeque<ShellCommand> = VecDeque::new();
            let mut stdin_open = true;
            let mut commands_open = true;
            let mut eof_sent = false;
            let mut exit = ExitEvent::default();

            loop {
                // Drain queued commands first — all channel borrows happen
                // here, sequentially, never inside `select!` handlers.
                while let Some(command) = pending_commands.pop_front() {
                    match command {
                        ShellCommand::Resize(target) => {
                            let (cols, rows, width, height) = shell::pty_args(&target);
                            // PORT-TODO: `request_pty_change_dimensions` is
                            // our best guess for the `window-change` sender
                            // on russh 0.64 — verify name/argument order.
                            let _ = channel
                                .request_pty_change_dimensions(
                                    cols.into(),
                                    rows.into(),
                                    width.into(),
                                    height.into(),
                                )
                                .await;
                        }
                        ShellCommand::Eof => {
                            if !eof_sent {
                                let _ = channel.eof().await;
                                eof_sent = true;
                            }
                        }
                    }
                }
                while let Some(bytes) = pending_input.pop_front() {
                    if channel.data(&bytes[..]).await.is_err() {
                        let _ = tx_exit.send(exit).await;
                        let _ = channel.close().await;
                        return;
                    }
                }

                // Wait for progress on any leg. Handlers only wrap owned
                // values or push to local queues, so no borrow overlaps the
                // branch futures.
                let event = tokio::select! {
                    item = rx_input.recv(), if stdin_open => PumpEvent::Input(item),
                    item = rx_command.recv(), if commands_open => PumpEvent::Command(item),
                    msg = channel.recv() => PumpEvent::Ssh(msg),
                };
                match event {
                    PumpEvent::Input(Some(bytes)) => pending_input.push_back(bytes),
                    PumpEvent::Input(None) => {
                        stdin_open = false;
                        pending_commands.push_back(ShellCommand::Eof);
                    }
                    PumpEvent::Command(Some(command)) => pending_commands.push_back(command),
                    PumpEvent::Command(None) => commands_open = false,
                    PumpEvent::Ssh(Some(ChannelMsg::Data { data })) => {
                        if tx_output.send(data).await.is_err() {
                            break;
                        }
                    }
                    PumpEvent::Ssh(Some(ChannelMsg::ExtendedData { data, .. })) => {
                        if tx_output.send(data).await.is_err() {
                            break;
                        }
                    }
                    PumpEvent::Ssh(Some(ChannelMsg::ExitStatus { exit_status })) => {
                        exit.exit_status = Some(exit_status);
                    }
                    PumpEvent::Ssh(Some(ChannelMsg::ExitSignal { signal_name, .. })) => {
                        exit.signal = Some(signal_name);
                    }
                    PumpEvent::Ssh(Some(_)) => {}
                    PumpEvent::Ssh(None) => break,
                }
            }

            let _ = tx_exit.send(exit).await;
            let _ = channel.close().await;
        });

        let handle = ChannelHandle::from_parts(tx_input, tx_command.clone(), rx_output, rx_exit);
        self.shell_commands = Some(tx_command);
        Ok(handle)
    }

    /// Resize the active shell's pty (`resize(rows, cols)` from the JS
    /// surface). Errors when no shell is open.
    pub async fn resize(&mut self, size: TerminalSize) -> Result<()> {
        match &self.shell_commands {
            Some(commands) => commands
                .send(ShellCommand::Resize(size))
                .await
                .map_err(|_| SshError::channel_closed("shell channel is closed")),
            None => Err(SshError::channel_closed("no active shell to resize")),
        }
    }

    /// Open an SFTP session on this connection (`sftp()` from the JS
    /// surface): session channel + `sftp` subsystem.
    pub async fn sftp(&mut self) -> Result<SftpClient> {
        let mut channel = {
            let mut guard = self.handle.lock().await;
            guard.channel_open_session().await
        }
        .map_err(|err| SshError::connection(format!("failed to open sftp channel: {err}")))?;

        // PORT-TODO: `request_subsystem("sftp")` — verify against russh 0.64
        // (some builds also offer a `request_sftp()` convenience).
        channel
            .request_subsystem("sftp")
            .await
            .map_err(|err| SshError::protocol(format!("sftp subsystem request rejected: {err}")))?;

        // The channel type is inferred here; `SftpClient` only ever sees the
        // resulting `SftpSession`.
        // PORT-TODO: if `SftpSession::new` does not take the channel
        // directly, try `channel.into_stream()`.
        let session = russh_sftp::client::sftp::SftpSession::new(channel);
        Ok(SftpClient::new(session))
    }

    /// Stop forwardings, close the shell and disconnect gracefully
    /// (`disconnect()` from the JS surface).
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(forwardings) = self.forwardings.take() {
            forwardings.stop();
        }
        self.shell_commands = None;

        let mut guard = self.handle.lock().await;
        // PORT-TODO: `russh::Disconnect::ByApplication` — verify the reason
        // enum path against russh 0.64.
        let result = guard
            .disconnect(russh::Disconnect::ByApplication, "termius", "en")
            .await
            .map_err(|err| SshError::connection(format!("disconnect failed: {err}")));
        drop(guard);
        // Drop the jump session last: the target session's transport lives
        // inside it.
        self.jump_handle = None;
        result
    }
}

/// The async session surface the app layer codes against — the same verbs
/// as the addon's JS API: `connect` / `exec` / `shell` / `resize` /
/// `disconnect`.
#[async_trait]
pub trait Session: Send {
    /// Open + authenticate a session from resolved connection parameters.
    async fn connect(params: ConnectionParams) -> Result<Self>
    where
        Self: Sized;
    /// Run a one-shot command and collect its output.
    async fn exec(&mut self, command: &str) -> Result<ExecOutput>;
    /// Open an interactive PTY shell.
    async fn shell(&mut self) -> Result<ChannelHandle>;
    /// Resize the active shell's pty.
    async fn resize(&mut self, size: TerminalSize) -> Result<()>;
    /// Tear the session down.
    async fn disconnect(&mut self) -> Result<()>;
}

#[async_trait]
impl Session for SshClient {
    async fn connect(params: ConnectionParams) -> Result<Self> {
        Connector::new().connect(&params).await
    }

    async fn exec(&mut self, command: &str) -> Result<ExecOutput> {
        SshClient::exec(self, command).await
    }

    async fn shell(&mut self) -> Result<ChannelHandle> {
        SshClient::shell(self).await
    }

    async fn resize(&mut self, size: TerminalSize) -> Result<()> {
        SshClient::resize(self, size).await
    }

    async fn disconnect(&mut self) -> Result<()> {
        SshClient::disconnect(self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termius_core::ConnectionParams;

    #[test]
    fn host_key_policy_rejects_pin_mismatch() {
        assert_eq!(
            host_key_decision(true, Some("SHA256:aaa"), Some("SHA256:aaa")),
            HostKeyDecision::Accept
        );
        assert_eq!(
            host_key_decision(true, Some("SHA256:aaa"), Some("SHA256:bbb")),
            HostKeyDecision::Reject("host key does not match the pinned key")
        );
        assert_eq!(
            host_key_decision(true, Some("SHA256:aaa"), None),
            HostKeyDecision::Reject("no host key was presented for verification")
        );
    }

    #[test]
    fn host_key_policy_without_pin_accepts() {
        // Strict mode with no pinned key: deferred to the KnownHost store
        // (PORT-TODO) — accept for now, both when unpinned and non-strict.
        assert_eq!(host_key_decision(true, None, Some("SHA256:x")), HostKeyDecision::Accept);
        assert_eq!(host_key_decision(false, None, None), HostKeyDecision::Accept);
        // Non-strict ignores any stale pin.
        assert_eq!(host_key_decision(false, Some("SHA256:x"), None), HostKeyDecision::Accept);
    }

    #[test]
    fn handler_tracks_rejection() {
        let (handler, flag) = TermiusHandler::new(true);
        assert!(!handler.host_key_rejected());
        assert!(!flag.load(Ordering::SeqCst));
        flag.store(true, Ordering::SeqCst);
        assert!(handler.host_key_rejected());
    }

    #[test]
    fn config_wires_keepalive_from_params() {
        let mut params = ConnectionParams::default();
        params.keepalive_interval_secs = 15;
        params.keepalive_count_max = 4;
        let config = build_config(&params);
        assert_eq!(config.keepalive_interval, Duration::from_secs(15));
        assert_eq!(config.inactivity_timeout, Some(Duration::from_secs(60)));

        params.keepalive_interval_secs = 0;
        let config = build_config(&params);
        assert!(config.inactivity_timeout.is_none(), "keepalive 0 disables inactivity timeout");
    }

    #[test]
    fn exec_output_text_is_lossy() {
        let mut output = ExecOutput::default();
        output.stdout.extend_from_slice(b"hello");
        output.stderr.extend_from_slice(&[0xff, 0xfe]);
        output.exit_status = Some(0);
        assert_eq!(output.stdout_text(), "hello");
        assert!(output.success());
        assert!(!output.stderr_text().is_empty());
    }
}
