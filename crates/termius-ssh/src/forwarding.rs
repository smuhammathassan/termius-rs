//! forwarding — see crate docs.
//!
//! Local (`-L`) and dynamic (`-D`, SOCKS5) port forwarding over `direct-tcpip`
//! channels, driven by `ConnectionParams::forwardings` — the same rule tuple
//! the Electron background engine passed to `libtermius`
//! `(type, listen_port, dest_host, dest_port)`.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use russh::ChannelMsg;
use termius_core::ForwardType;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, instrument, warn};

use crate::client::SharedHandle;
use crate::error::{Result, SshError};

const SOCKS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const SOCKS_MAX_MESSAGE: usize = 4096;
const SHUTTLE_BUF_SIZE: usize = 16 * 1024;

/// A validated local (`-L`) forwarding rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRule {
    pub listen_port: u16,
    pub dest_host: String,
    pub dest_port: u16,
}

/// A validated dynamic (`-D`) SOCKS rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicRule {
    pub listen_port: u16,
}

/// A rule we deliberately did not start, with the reason (surfaced as a
/// warning instead of failing the whole connection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedRule {
    pub forward_type: ForwardType,
    pub listen_port: u16,
    pub reason: String,
}

/// Result of classifying `ConnectionParams::forwardings`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForwardingPlan {
    pub local: Vec<LocalRule>,
    pub dynamic: Vec<DynamicRule>,
    pub skipped: Vec<SkippedRule>,
}

/// Pure classification of the raw forwarding tuples.
///
/// * `Local` / `LocalAuto` → local listener (needs a destination),
/// * `Dynamic` → SOCKS5 listener (destination ignored, per SSH `-D`),
/// * `Remote` / `RemoteAuto` → skipped: a server-side listener needs
///   `forwarded-tcpip` support we have not wired up (PORT-TODO),
/// * `LocalSerial*` → skipped: serial tunnels belong to `termius-serial`.
pub fn plan_forwardings(rules: &[(ForwardType, u16, String, u16)]) -> ForwardingPlan {
    let mut plan = ForwardingPlan::default();
    for (forward_type, listen_port, dest_host, dest_port) in rules {
        let forward_type = *forward_type;
        match forward_type {
            ForwardType::Local | ForwardType::LocalAuto => {
                if dest_host.trim().is_empty() {
                    plan.skipped.push(SkippedRule {
                        forward_type,
                        listen_port: *listen_port,
                        reason: "local forwarding has no destination host".into(),
                    });
                } else if *dest_port == 0 {
                    plan.skipped.push(SkippedRule {
                        forward_type,
                        listen_port: *listen_port,
                        reason: "local forwarding has no destination port".into(),
                    });
                } else {
                    plan.local.push(LocalRule {
                        listen_port: *listen_port,
                        dest_host: dest_host.clone(),
                        dest_port: *dest_port,
                    });
                }
            }
            ForwardType::Dynamic => {
                plan.dynamic.push(DynamicRule { listen_port: *listen_port });
            }
            ForwardType::Remote | ForwardType::RemoteAuto => {
                plan.skipped.push(SkippedRule {
                    forward_type,
                    listen_port: *listen_port,
                    reason: "remote forwarding requires a server-side listener (not implemented yet)"
                        .into(),
                });
            }
            ForwardType::LocalSerial | ForwardType::LocalSerialAuto => {
                plan.skipped.push(SkippedRule {
                    forward_type,
                    listen_port: *listen_port,
                    reason: "serial forwarding is handled by termius-serial".into(),
                });
            }
        }
    }
    plan
}

/// Parsed SOCKS5 `CONNECT` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Socks5Request {
    Connect { host: String, port: u16 },
}

/// Parse failure that distinguishes "need more bytes" from "malformed".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Socks5ParseError {
    Incomplete,
    UnsupportedVersion(u8),
    UnsupportedCommand(u8),
    UnsupportedAddressType(u8),
}

/// Validate a SOCKS5 method-negotiation greeting.
pub fn parse_socks5_greeting(buf: &[u8]) -> std::result::Result<(), Socks5ParseError> {
    if buf.len() < 2 {
        return Err(Socks5ParseError::Incomplete);
    }
    if buf[0] != 0x05 {
        return Err(Socks5ParseError::UnsupportedVersion(buf[0]));
    }
    let nmethods = usize::from(buf[1]);
    if buf.len() < 2 + nmethods {
        return Err(Socks5ParseError::Incomplete);
    }
    Ok(())
}

/// Parse a SOCKS5 request packet (`CONNECT` only — what SSH `-D` tunnels).
pub fn parse_socks5_connect(buf: &[u8]) -> std::result::Result<Socks5Request, Socks5ParseError> {
    if buf.len() < 4 {
        return Err(Socks5ParseError::Incomplete);
    }
    if buf[0] != 0x05 {
        return Err(Socks5ParseError::UnsupportedVersion(buf[0]));
    }
    if buf[1] != 0x01 {
        return Err(Socks5ParseError::UnsupportedCommand(buf[1]));
    }
    match buf[3] {
        0x01 => {
            // IPv4: 4 bytes address + 2 bytes port.
            if buf.len() < 10 {
                return Err(Socks5ParseError::Incomplete);
            }
            let addr = Ipv4Addr::new(buf[4], buf[5], buf[6], buf[7]);
            let port = u16::from_be_bytes([buf[8], buf[9]]);
            Ok(Socks5Request::Connect { host: addr.to_string(), port })
        }
        0x03 => {
            // Domain name: 1 length byte + name + 2 bytes port.
            if buf.len() < 5 {
                return Err(Socks5ParseError::Incomplete);
            }
            let len = usize::from(buf[4]);
            let total = 5 + len + 2;
            if buf.len() < total {
                return Err(Socks5ParseError::Incomplete);
            }
            let host = String::from_utf8_lossy(&buf[5..5 + len]).into_owned();
            let port = u16::from_be_bytes([buf[5 + len], buf[6 + len]]);
            Ok(Socks5Request::Connect { host, port })
        }
        0x04 => {
            // IPv6: 16 bytes address + 2 bytes port.
            if buf.len() < 22 {
                return Err(Socks5ParseError::Incomplete);
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&buf[4..20]);
            let addr = Ipv6Addr::from(octets);
            let port = u16::from_be_bytes([buf[20], buf[21]]);
            Ok(Socks5Request::Connect { host: addr.to_string(), port })
        }
        other => Err(Socks5ParseError::UnsupportedAddressType(other)),
    }
}

/// SOCKS5 method negotiation reply: version 5, no-auth.
pub fn socks5_greeting_reply() -> [u8; 2] {
    [0x05, 0x00]
}

/// SOCKS5 `CONNECT` reply: bound address reported as `0.0.0.0:0` (we do not
/// know the remote-side binding — the reply only signals success/failure).
pub fn socks5_connect_reply(success: bool) -> [u8; 10] {
    let mut reply = [0x05, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    if !success {
        reply[1] = 0x05; // connection refused
    }
    reply
}

/// Where an accepted socket should be tunneled to.
#[derive(Debug, Clone)]
pub(crate) enum ShuttleDest {
    Fixed { host: String, port: u16 },
    Socks5,
}

/// Handle to a set of running listeners; stops every task on drop.
pub struct ForwardingSet {
    tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Ports actually bound (differs from the request when port 0 was used).
    bound_ports: Vec<u16>,
}

impl ForwardingSet {
    /// Ports the listeners are actually bound to, in start order.
    pub fn bound_ports(&self) -> &[u16] {
        &self.bound_ports
    }

    /// Stop every listener and in-flight tunnel task.
    pub fn stop(mut self) {
        // A `Drop` impl exists, so move the tasks out via `drain` instead of
        // consuming `self.tasks` directly.
        for task in self.tasks.drain(..) {
            task.abort();
        }
    }
}

impl Drop for ForwardingSet {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Bind listeners for every rule in `params.forwardings`.
///
/// Rules that cannot be served (remote/serial, missing destination) are
/// logged and skipped rather than failing the connection — matching how the
/// Electron app surfaced them as separate, non-fatal tunnel entries.
#[instrument(skip_all, fields(rules = rules.len()))]
pub(crate) async fn start(
    shared: SharedHandle,
    rules: &[(ForwardType, u16, String, u16)],
) -> Result<ForwardingSet> {
    let plan = plan_forwardings(rules);
    for skipped in &plan.skipped {
        warn!(
            forward_type = ?skipped.forward_type,
            listen_port = skipped.listen_port,
            reason = %skipped.reason,
            "skipping port forwarding rule"
        );
    }

    let mut tasks = Vec::new();
    let mut bound_ports = Vec::new();

    for rule in plan.local {
        let listener = TcpListener::bind(("127.0.0.1", rule.listen_port)).await?;
        bound_ports.push(listener.local_addr()?.port());
        debug!(
            listen_port = rule.listen_port,
            dest = %rule.dest_host,
            dest_port = rule.dest_port,
            "local forward listening"
        );
        tasks.push(tokio::spawn(local_accept_loop(
            listener,
            shared.clone(),
            rule.dest_host,
            rule.dest_port,
        )));
    }

    for rule in plan.dynamic {
        let listener = TcpListener::bind(("127.0.0.1", rule.listen_port)).await?;
        bound_ports.push(listener.local_addr()?.port());
        debug!(listen_port = rule.listen_port, "dynamic (socks5) forward listening");
        tasks.push(tokio::spawn(dynamic_accept_loop(listener, shared.clone())));
    }

    Ok(ForwardingSet { tasks, bound_ports })
}

async fn local_accept_loop(
    listener: TcpListener,
    shared: SharedHandle,
    dest_host: String,
    dest_port: u16,
) {
    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(err) => {
                warn!(error = %err, "local forwarding listener stopped");
                return;
            }
        };
        debug!(%peer, dest = %dest_host, dest_port, "accepted local forward connection");
        let _task = tokio::spawn(shuttle(
            socket,
            shared.clone(),
            ShuttleDest::Fixed { host: dest_host.clone(), port: dest_port },
        ));
    }
}

async fn dynamic_accept_loop(listener: TcpListener, shared: SharedHandle) {
    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(err) => {
                warn!(error = %err, "dynamic forwarding listener stopped");
                return;
            }
        };
        debug!(%peer, "accepted socks5 connection");
        let _task = tokio::spawn(shuttle(socket, shared.clone(), ShuttleDest::Socks5));
    }
}

/// Drive one forwarded connection: (optional) SOCKS5 handshake, then a
/// bidirectional byte pump over a `direct-tcpip` channel.
async fn shuttle(socket: TcpStream, shared: SharedHandle, dest: ShuttleDest) {
    let mut socket = socket;
    let (dest_host, dest_port) = match dest {
        ShuttleDest::Fixed { host, port } => (host, port),
        ShuttleDest::Socks5 => {
            let negotiated =
                tokio::time::timeout(SOCKS_HANDSHAKE_TIMEOUT, socks5_negotiate(&mut socket)).await;
            match negotiated {
                Err(_) => {
                    warn!("socks5 handshake timed out");
                    return;
                }
                Ok(Err(err)) => {
                    warn!(error = %err, "socks5 handshake failed");
                    return;
                }
                Ok(Ok(target)) => target,
            }
        }
    };

    // russh 0.64: `Handle::channel_open_direct_tcpip(host, port: u32,
    // originator, originator_port: u32)`.
    let opened = {
        let handle = shared.lock().await;
        handle
            .channel_open_direct_tcpip(dest_host.as_str(), u32::from(dest_port), "127.0.0.1", 0)
            .await
    };
    let mut channel = match opened {
        Ok(channel) => channel,
        Err(err) => {
            warn!(dest = %dest_host, dest_port, error = %err, "direct-tcpip channel open failed");
            let _ = socket.shutdown().await;
            return;
        }
    };

    enum PumpEvent {
        FromSocket(std::io::Result<usize>),
        FromChannel(Option<ChannelMsg>),
    }

    let mut read_buf = [0u8; SHUTTLE_BUF_SIZE];
    let mut socket_closed = false;
    loop {
        // The two branch futures borrow disjoint values, and the handlers
        // only wrap owned results — every channel/socket borrow happens
        // after `select!` completes, so this is safe under any
        // `select!` future-drop semantics.
        let event = tokio::select! {
            read = socket.read(&mut read_buf), if !socket_closed => PumpEvent::FromSocket(read),
            msg = channel.wait() => PumpEvent::FromChannel(msg),
        };
        match event {
            PumpEvent::FromSocket(Ok(0)) => {
                socket_closed = true;
                let _ = channel.eof().await;
            }
            PumpEvent::FromSocket(Ok(n)) => {
                if channel.data(&read_buf[..n]).await.is_err() {
                    break;
                }
            }
            PumpEvent::FromSocket(Err(err)) => {
                debug!(error = %err, "forwarded socket read failed");
                break;
            }
            PumpEvent::FromChannel(Some(ChannelMsg::Data { data })) => {
                if socket.write_all(&data).await.is_err() {
                    break;
                }
            }
            PumpEvent::FromChannel(Some(ChannelMsg::ExtendedData { data, .. })) => {
                if socket.write_all(&data).await.is_err() {
                    break;
                }
            }
            PumpEvent::FromChannel(Some(ChannelMsg::Eof)) => break,
            PumpEvent::FromChannel(Some(_)) => {}
            PumpEvent::FromChannel(None) => break,
        }
    }
    let _ = channel.close().await;
    let _ = socket.shutdown().await;
}

/// Speak enough SOCKS5 to extract the `CONNECT` target. Called under a
/// timeout from [`shuttle`].
async fn socks5_negotiate(socket: &mut TcpStream) -> Result<(String, u16)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 512];

    loop {
        match parse_socks5_greeting(&buf) {
            Ok(()) => break,
            Err(Socks5ParseError::Incomplete) => {}
            Err(err) => {
                return Err(SshError::protocol(format!("socks5 greeting: {err:?}")));
            }
        }
        let read = socket
            .read(&mut chunk)
            .await
            .map_err(|e| SshError::connection(format!("socks5 greeting read: {e}")))?;
        if read == 0 {
            return Err(SshError::connection("socks client closed during greeting"));
        }
        buf.extend_from_slice(&chunk[..read]);
        if buf.len() > SOCKS_MAX_MESSAGE {
            return Err(SshError::protocol("socks5 greeting too large"));
        }
    }
    socket
        .write_all(&socks5_greeting_reply())
        .await
        .map_err(|e| SshError::connection(format!("socks5 greeting write: {e}")))?;

    buf.clear();
    let request = loop {
        match parse_socks5_connect(&buf) {
            Ok(request) => break request,
            Err(Socks5ParseError::Incomplete) => {}
            Err(err) => {
                let _ = socket.write_all(&socks5_connect_reply(false)).await;
                return Err(SshError::protocol(format!("socks5 request: {err:?}")));
            }
        }
        let read = socket
            .read(&mut chunk)
            .await
            .map_err(|e| SshError::connection(format!("socks5 request read: {e}")))?;
        if read == 0 {
            let _ = socket.write_all(&socks5_connect_reply(false)).await;
            return Err(SshError::connection("socks client closed during request"));
        }
        buf.extend_from_slice(&chunk[..read]);
        if buf.len() > SOCKS_MAX_MESSAGE {
            let _ = socket.write_all(&socks5_connect_reply(false)).await;
            return Err(SshError::protocol("socks5 request too large"));
        }
    };

    // The parser only yields `CONNECT`.
    let Socks5Request::Connect { host, port } = request;
    socket
        .write_all(&socks5_connect_reply(true))
        .await
        .map_err(|e| SshError::connection(format!("socks5 reply write: {e}")))?;
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(entries: &[(ForwardType, u16, &str, u16)]) -> Vec<(ForwardType, u16, String, u16)> {
        entries
            .iter()
            .map(|(t, p, h, d)| (*t, *p, (*h).to_string(), *d))
            .collect()
    }

    #[test]
    fn plans_local_and_dynamic_rules() {
        let plan = plan_forwardings(&rules(&[
            (ForwardType::Local, 8080, "db.internal", 5432),
            (ForwardType::Dynamic, 1080, "", 0),
            (ForwardType::LocalAuto, 9000, "cache", 6379),
        ]));

        assert_eq!(plan.local.len(), 2);
        assert_eq!(
            plan.local[0],
            LocalRule { listen_port: 8080, dest_host: "db.internal".into(), dest_port: 5432 }
        );
        assert_eq!(plan.local[1].listen_port, 9000);
        assert_eq!(plan.dynamic, vec![DynamicRule { listen_port: 1080 }]);
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn dynamic_ignores_destination() {
        // `-D 1080` carries whatever tuple came from storage; destination is
        // irrelevant for SOCKS.
        let plan = plan_forwardings(&rules(&[(ForwardType::Dynamic, 1080, "ignored", 22)]));
        assert_eq!(plan.dynamic.len(), 1);
        assert!(plan.local.is_empty());
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn skips_remote_and_serial_rules_with_reasons() {
        let plan = plan_forwardings(&rules(&[
            (ForwardType::Remote, 2222, "localhost", 22),
            (ForwardType::RemoteAuto, 2223, "localhost", 22),
            (ForwardType::LocalSerial, 0, "", 0),
            (ForwardType::LocalSerialAuto, 0, "", 0),
        ]));
        assert!(plan.local.is_empty());
        assert!(plan.dynamic.is_empty());
        assert_eq!(plan.skipped.len(), 4);
        assert!(plan.skipped[0].reason.contains("remote"));
        assert!(plan.skipped[2].reason.contains("serial"));
    }

    #[test]
    fn skips_incomplete_local_rules() {
        let plan = plan_forwardings(&rules(&[
            (ForwardType::Local, 8080, "", 80),
            (ForwardType::Local, 8081, "host", 0),
        ]));
        assert!(plan.local.is_empty());
        assert_eq!(plan.skipped.len(), 2);
        assert!(plan.skipped[0].reason.contains("destination host"));
        assert!(plan.skipped[1].reason.contains("destination port"));
    }

    #[test]
    fn parses_socks5_greeting() {
        assert_eq!(parse_socks5_greeting(&[]), Err(Socks5ParseError::Incomplete));
        assert_eq!(parse_socks5_greeting(&[0x05]), Err(Socks5ParseError::Incomplete));
        assert_eq!(parse_socks5_greeting(&[0x05, 0x02, 0x00]), Err(Socks5ParseError::Incomplete));
        assert_eq!(parse_socks5_greeting(&[0x05, 0x01, 0x00]), Ok(()));
        assert_eq!(
            parse_socks5_greeting(&[0x04, 0x01, 0x00]),
            Err(Socks5ParseError::UnsupportedVersion(0x04))
        );
    }

    #[test]
    fn parses_socks5_connect_domain() {
        let mut pkt = vec![0x05, 0x01, 0x00, 0x03];
        pkt.push(3);
        pkt.extend_from_slice(b"db.");
        pkt.extend_from_slice(&8543u16.to_be_bytes());
        assert_eq!(
            parse_socks5_connect(&pkt),
            Ok(Socks5Request::Connect { host: "db.".into(), port: 8543 })
        );
    }

    #[test]
    fn parses_socks5_connect_ipv4_and_ipv6() {
        let pkt = [0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1, 0x1F, 0x90];
        assert_eq!(
            parse_socks5_connect(&pkt),
            Ok(Socks5Request::Connect { host: "127.0.0.1".into(), port: 8080 })
        );

        let mut pkt = vec![0x05, 0x01, 0x00, 0x04];
        pkt.extend_from_slice(&std::net::Ipv6Addr::LOCALHOST.octets());
        pkt.extend_from_slice(&22u16.to_be_bytes());
        assert_eq!(
            parse_socks5_connect(&pkt),
            Ok(Socks5Request::Connect { host: "::1".into(), port: 22 })
        );
    }

    #[test]
    fn socks5_parse_errors_are_classified() {
        assert_eq!(parse_socks5_connect(&[0x05, 0x01]), Err(Socks5ParseError::Incomplete));
        assert_eq!(
            parse_socks5_connect(&[0x05, 0x02, 0x00, 0x01]),
            Err(Socks5ParseError::UnsupportedCommand(0x02))
        );
        assert_eq!(
            parse_socks5_connect(&[0x05, 0x01, 0x00, 0x09]),
            Err(Socks5ParseError::UnsupportedAddressType(0x09))
        );
        // Truncated IPv4 payload must ask for more bytes, not fail.
        assert_eq!(
            parse_socks5_connect(&[0x05, 0x01, 0x00, 0x01, 127, 0]),
            Err(Socks5ParseError::Incomplete)
        );
    }

    #[test]
    fn socks5_replies_have_wire_shape() {
        assert_eq!(socks5_greeting_reply(), [0x05, 0x00]);
        let ok = socks5_connect_reply(true);
        assert_eq!(ok.len(), 10);
        assert_eq!(ok[0], 0x05);
        assert_eq!(ok[1], 0x00);
        assert_eq!(socks5_connect_reply(false)[1], 0x05);
    }
}
