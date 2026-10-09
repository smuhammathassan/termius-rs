//! Serial-port connection engine: enumerate ports, open a port with Termius'
//! serial configuration (baud rate, data bits, parity, stop bits, flow
//! control) and bridge the blocking `serialport` handle onto an async byte
//! stream for the terminal.
//!
//! # Ported behavior (Termius `serialOptions` + `@serialport/stream` shell)
//!
//! The recovered sources show Termius' serial feature as a `serialOptions`
//! object edited by the "Open Serial Connection" dialog (menu action
//! `openSerialConnection`, ⌘⌥S / Ctrl+Alt+S) and consumed by a shell service
//! that opened a node-serialport `SerialPort`:
//!
//! ```text
//! { path: "", baudRate: 9600, dataBits: 8, stopBits: 1,
//!   parity: "none", controlFlow: "none", charset: "UTF-8" }
//! ```
//!
//! * Dialog presets — baud: 50…115200 (17 rates), data bits: 8/7/6/5,
//!   stop bits: 1/2, parity: none/odd/even, flow control: none/RTS-CTS/
//!   DSR-DTR/XON-XOFF, plus a charset field.
//! * Connect is disabled until `path` and `baudRate` are set — enforced here
//!   as [`SerialError::InvalidConfig`] from [`SerialConfig::validate`].
//! * Port enumeration used `SerialPort.list()` (re-polled by the dialog every
//!   3 s; auto-selected the first port when `path` was empty) — see
//!   [`list_ports`].
//! * Log lines: `Starting a new connection to "<path>"`,
//!   `Baud rate: ..., data bits: ..., stop bits: ..., parity: ..., flow
//!   control: ...`, `Connection to "<path>" established`, and on failure
//!   `Connection can not be established: ...` / `Connection closed with error:
//!   ...` — reproduced with `tracing`.
//!
//! # Configuration mapping from `ConnectionParams` / `Host`
//!
//! Termius carried `serialOptions` alongside the connection, not inside the
//! shared connection params. The Rust port maps the shared contract as
//! follows (see [`SerialConfig::from_connection_params`] /
//! [`SerialConfig::from_host`]):
//!
//! * `host` (resp. `Host::hostname`) → `path`, the device path
//!   (`/dev/ttyUSB0`, `/dev/tty.usbserial-...`, `COM3`).
//! * `port` (resp. `Host::port`, a `u16`) → `baud_rate` **iff** it equals one
//!   of Termius' baud presets that fit in a `u16` (see
//!   [`STANDARD_BAUD_RATES`]); otherwise the Termius default 9600 is used, so
//!   an SSH-shaped `port: 22` never becomes "22 baud". Termius' 115200 preset
//!   cannot travel through a `u16` — callers needing it build
//!   [`SerialConfig`] directly (the UI dialog does).
//! * Every other field keeps Termius' initial `serialOptions` value:
//!   8 data bits, 1 stop bit, parity none, flow control none, UTF-8.
//!
//! # Blocking → async bridge
//!
//! `serialport` is fully blocking. [`SerialConnection::open`] spawns a single
//! `tokio::task::spawn_blocking` actor that *creates and owns* the port handle
//! for its whole lifetime (the handle never crosses threads), pumps queued
//! writes into it, and pushes incoming bytes into a `tokio::sync::mpsc`
//! channel. A read timeout (`READ_POLL_INTERVAL`) wakes the actor regularly so
//! it can observe close requests. The async side exposes [`recv`](SerialConnection::recv)
//! (bytes out of the port), [`send`](SerialConnection::send) (bytes into the
//! port) and [`close`](SerialConnection::close).

use std::fmt;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info};

use termius_core::connection::ConnectionParams;
use termius_core::host::{Host, HostType};

use crate::error::{Result, SerialError};

/// Termius' default `serialOptions.baudRate` (recovered initial state).
pub const DEFAULT_BAUD_RATE: u32 = 9600;
/// Termius' default `serialOptions.dataBits`.
pub const DEFAULT_DATA_BITS: u8 = 8;
/// Termius' default `serialOptions.stopBits`.
pub const DEFAULT_STOP_BITS: u8 = 1;
/// Termius' default `serialOptions.charset`.
pub const DEFAULT_CHARSET: &str = "UTF-8";

/// The baud-rate presets offered by Termius' serial dialog that fit inside
/// `ConnectionParams::port` (a `u16`). Termius also offers 115200, which is
/// unreachable through the `u16` port field — see the module docs.
pub const STANDARD_BAUD_RATES: [u32; 16] = [
    50, 75, 110, 134, 150, 200, 300, 600, 1200, 1800, 2400, 4800, 9600, 19200, 38400, 57600,
];

/// How long the actor blocks in `read` before checking for close requests.
/// Short enough that keystroke writes feel instant, long enough to keep the
/// wake-up rate negligible.
const READ_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Size of one read pulled from the port per actor iteration.
const READ_BUF_LEN: usize = 4096;
/// Buffered chunks flowing port → terminal.
const EVENT_CAPACITY: usize = 64;
/// Buffered chunks flowing terminal → port.
const CMD_CAPACITY: usize = 32;
/// Upper bound `close()` waits for the actor to release the port.
const CLOSE_TIMEOUT: Duration = Duration::from_millis(500);

/// Serial parity (Termius `serialOptions.parity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Parity {
    #[default]
    None,
    Odd,
    Even,
}

impl Parity {
    /// Termius' wire spelling: `"none" | "odd" | "even"`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Parity::None => "none",
            Parity::Odd => "odd",
            Parity::Even => "even",
        }
    }
}

/// Flow control (Termius `serialOptions.controlFlow`).
///
/// The wire values are exactly Termius': `"none" | "rtscts" | "dsrdtr" |
/// "xonxoff"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FlowControl {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "rtscts")]
    RtsCts,
    #[serde(rename = "dsrdtr")]
    DsrDtr,
    #[serde(rename = "xonxoff")]
    XonXoff,
}

impl FlowControl {
    /// Termius' wire spelling: `"none" | "rtscts" | "dsrdtr" | "xonxoff"`.
    pub fn as_str(&self) -> &'static str {
        match self {
            FlowControl::None => "none",
            FlowControl::RtsCts => "rtscts",
            FlowControl::DsrDtr => "dsrdtr",
            FlowControl::XonXoff => "xonxoff",
        }
    }
}

/// Full serial-port configuration — the Rust form of Termius' `serialOptions`.
///
/// Field names serialize as `camelCase` (`path`, `baudRate`, `dataBits`,
/// `stopBits`, `parity`, `controlFlow`, `charset`) so the value round-trips
/// with the shape Termius persisted and passed through Redux.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialConfig {
    /// Device path/name (`/dev/ttyUSB0`, `/dev/tty.usbserial-A10K3`, `COM3`).
    pub path: String,
    /// Baud rate in bits per second.
    pub baud_rate: u32,
    /// Data bits per frame: 5, 6, 7 or 8.
    pub data_bits: u8,
    /// Stop bits per frame: 1 or 2.
    pub stop_bits: u8,
    /// Parity mode.
    pub parity: Parity,
    /// Flow control. Serialized as `controlFlow` to match Termius.
    #[serde(rename = "controlFlow")]
    pub flow_control: FlowControl,
    /// Character set for the bytes ⇄ text transcoding (Termius used
    /// iconv-lite for this; the engine itself passes raw bytes, transcoding
    /// belongs to the terminal layer).
    pub charset: String,
}

impl Default for SerialConfig {
    /// Termius' initial `serialOptions`: empty path, 9600 8N1, no flow
    /// control, UTF-8.
    fn default() -> Self {
        Self {
            path: String::new(),
            baud_rate: DEFAULT_BAUD_RATE,
            data_bits: DEFAULT_DATA_BITS,
            stop_bits: DEFAULT_STOP_BITS,
            parity: Parity::None,
            flow_control: FlowControl::None,
            charset: DEFAULT_CHARSET.to_string(),
        }
    }
}

impl SerialConfig {
    /// A config for `path` with every other setting at the Termius default.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            ..Self::default()
        }
    }

    /// Override the baud rate.
    pub fn with_baud_rate(mut self, baud_rate: u32) -> Self {
        self.baud_rate = baud_rate;
        self
    }

    /// Validate the configuration (Termius disabled *Connect* while `path` or
    /// `baudRate` were unset; we additionally check the frame settings).
    pub fn validate(&self) -> Result<()> {
        if self.path.trim().is_empty() {
            return Err(SerialError::InvalidConfig(
                "path must not be empty (set the serial device path)".to_string(),
            ));
        }
        if self.baud_rate == 0 {
            return Err(SerialError::InvalidConfig(
                "baud rate must be greater than 0".to_string(),
            ));
        }
        if !matches!(self.data_bits, 5..=8) {
            return Err(SerialError::InvalidConfig(format!(
                "data bits must be 5, 6, 7 or 8 (got {})",
                self.data_bits
            )));
        }
        if !matches!(self.stop_bits, 1 | 2) {
            return Err(SerialError::InvalidConfig(format!(
                "stop bits must be 1 or 2 (got {})",
                self.stop_bits
            )));
        }
        if self.charset.trim().is_empty() {
            return Err(SerialError::InvalidConfig(
                "charset must not be empty".to_string(),
            ));
        }
        Ok(())
    }

    /// Build a config from shared [`ConnectionParams`].
    ///
    /// Mapping (documented in the module docs): `params.host` is the device
    /// path; `params.port` is taken as the baud rate when it equals one of
    /// [`STANDARD_BAUD_RATES`], otherwise the Termius default 9600 applies;
    /// all other settings keep the Termius defaults (8N1, no flow, UTF-8).
    pub fn from_connection_params(params: &ConnectionParams) -> Result<Self> {
        let path = params.host.trim().to_string();
        if path.is_empty() {
            return Err(SerialError::InvalidConfig(
                "serial connection host must hold the device path".to_string(),
            ));
        }
        Ok(Self {
            path,
            baud_rate: Self::baud_from_port(params.port),
            ..Self::default()
        })
    }

    /// Build a config from a saved [`Host`] (`HostType::Serial`).
    /// Uses `hostname` as the device path and applies the same port → baud
    /// mapping as [`from_connection_params`](Self::from_connection_params).
    pub fn from_host(host: &Host) -> Result<Self> {
        if host.host_type != HostType::Serial {
            return Err(SerialError::Unsupported(format!(
                "host \"{}\" is not a serial host (type {:?})",
                host.label, host.host_type
            )));
        }
        let path = host.hostname.trim().to_string();
        if path.is_empty() {
            return Err(SerialError::InvalidConfig(
                "serial host must hold the device path in its hostname".to_string(),
            ));
        }
        Ok(Self {
            path,
            baud_rate: Self::baud_from_port(host.port),
            ..Self::default()
        })
    }

    /// Map a `u16` connection port onto a baud rate: a Termius baud preset is
    /// used verbatim, anything else falls back to [`DEFAULT_BAUD_RATE`] (9600).
    pub fn baud_from_port(port: u16) -> u32 {
        let candidate = u32::from(port);
        if STANDARD_BAUD_RATES.contains(&candidate) {
            candidate
        } else {
            DEFAULT_BAUD_RATE
        }
    }

    // ---- mapping onto `serialport` enums (exercised by unit tests) ----

    pub(crate) fn to_data_bits(&self) -> Result<serialport::DataBits> {
        match self.data_bits {
            5 => Ok(serialport::DataBits::Five),
            6 => Ok(serialport::DataBits::Six),
            7 => Ok(serialport::DataBits::Seven),
            8 => Ok(serialport::DataBits::Eight),
            other => Err(SerialError::InvalidConfig(format!(
                "data bits must be 5, 6, 7 or 8 (got {other})"
            ))),
        }
    }

    pub(crate) fn to_stop_bits(&self) -> Result<serialport::StopBits> {
        match self.stop_bits {
            1 => Ok(serialport::StopBits::One),
            2 => Ok(serialport::StopBits::Two),
            other => Err(SerialError::InvalidConfig(format!(
                "stop bits must be 1 or 2 (got {other})"
            ))),
        }
    }

    pub(crate) fn to_parity(&self) -> serialport::Parity {
        match self.parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        }
    }

    pub(crate) fn to_flow_control(&self) -> serialport::FlowControl {
        match self.flow_control {
            // Termius only ever toggled the `rtscts`/`xon`/`xoff` options on
            // the port; `dsrdtr` reached node-serialport with every flow flag
            // off, so it behaves exactly like "none".
            // PORT-TODO: serialport has no DSR/DTR flow-control mode; the
            // DSR half of Termius' post-open `set({dsr, dtr})` has no
            // equivalent either (see the actor below).
            FlowControl::None | FlowControl::DsrDtr => serialport::FlowControl::None,
            FlowControl::RtsCts => serialport::FlowControl::Hardware,
            FlowControl::XonXoff => serialport::FlowControl::Software,
        }
    }
}

/// Classification of an enumerated serial port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PortKind {
    Usb,
    Pci,
    Bluetooth,
    #[default]
    Unknown,
}

/// One port returned by [`list_ports`], mirroring the metadata node-serialport
/// exposed to Termius' dialog (which only read `path`, but the rest is kept
/// for the UI's port list).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialPortInfo {
    /// Device path (`/dev/ttyUSB0`, `COM3`, ...).
    pub path: String,
    /// Transport classification of the port.
    pub kind: PortKind,
    pub manufacturer: Option<String>,
    pub serial_number: Option<String>,
    pub product: Option<String>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
}

/// Enumerate the serial ports available on this machine (Termius' dialog
/// called `SerialPort.list()` here, refreshed every 3 seconds by the UI).
///
/// This is a blocking enumeration (udev / IOKit); callers on an async runtime
/// should wrap it in `spawn_blocking`.
pub fn list_ports() -> Result<Vec<SerialPortInfo>> {
    let ports = serialport::available_ports()
        .map_err(|e| SerialError::classify_serialport("serial port enumeration", e))?;
    Ok(ports
        .into_iter()
        .map(|info| {
            let (kind, usb) = match info.port_type {
                serialport::SerialPortType::Usb(usb) => (PortKind::Usb, Some(usb)),
                // PORT-TODO: serialport 4.x also reports `Pci` and
                // `Bluetooth` kinds; they fall through as `Unknown` here —
                // Termius' dialog only ever displays `path`, and the wildcard
                // keeps this compiling against every serialport 4.x.
                _ => (PortKind::Unknown, None),
            };
            let (manufacturer, serial_number, product, vendor_id, product_id) = match usb {
                Some(usb) => (
                    usb.manufacturer,
                    usb.serial_number,
                    usb.product,
                    Some(usb.vid),
                    Some(usb.pid),
                ),
                None => (None, None, None, None, None),
            };
            SerialPortInfo {
                path: info.port_name,
                kind,
                manufacturer,
                serial_number,
                product,
                vendor_id,
                product_id,
            }
        })
        .collect())
}

/// Messages flowing from the async side to the blocking port actor.
enum ActorCmd {
    Write(Bytes),
    Close,
}

/// An open serial connection.
///
/// Created by [`SerialConnection::open`], which starts a blocking actor that
/// owns the `serialport` handle. Read and write are async; `close()` releases
/// the port. Dropping the connection also tears the actor down (channel
/// closure is the shutdown signal), so no background work outlives the value.
pub struct SerialConnection {
    config: SerialConfig,
    data_rx: mpsc::Receiver<Result<Bytes>>,
    cmd_tx: mpsc::Sender<ActorCmd>,
    closed: Arc<AtomicBool>,
    actor: Option<tokio::task::JoinHandle<()>>,
}

impl fmt::Debug for SerialConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SerialConnection")
            .field("path", &self.path())
            .field("config", &self.config)
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl SerialConnection {
    /// Open `path` with `config` (the explicit `path` argument wins over
    /// `config.path`).
    ///
    /// The port handle is created *inside* the blocking actor task, so it
    /// never crosses threads; this method only waits for the actor to report
    /// whether the open succeeded. On failure the reason is classified
    /// (missing device vs. permission vs. I/O) by [`SerialError`].
    pub async fn open(path: impl Into<String>, mut config: SerialConfig) -> Result<Self> {
        config.path = path.into();
        let path = config.path.clone();
        // Fail fast on bad configuration, before any port is touched.
        config.validate()?;
        let data_bits = config.to_data_bits()?;
        let stop_bits = config.to_stop_bits()?;
        let parity = config.to_parity();
        let flow_control = config.to_flow_control();

        info!("Starting a new connection to \"{path}\"");
        debug!(
            "Baud rate: {}, data bits: {}, stop bits: {}, parity: {}, flow control: {}",
            config.baud_rate,
            config.data_bits,
            config.stop_bits,
            config.parity.as_str(),
            config.flow_control.as_str()
        );

        let (ready_tx, ready_rx) = oneshot::channel::<Result<()>>();
        let (data_tx, data_rx) = mpsc::channel::<Result<Bytes>>(EVENT_CAPACITY);
        let (cmd_tx, cmd_rx) = mpsc::channel::<ActorCmd>(CMD_CAPACITY);
        let closed = Arc::new(AtomicBool::new(false));

        let actor_path = path.clone();
        let actor_config = config.clone();

        let actor = tokio::task::spawn_blocking(move || {
            let mut port = match serialport::new(actor_path.as_str(), actor_config.baud_rate)
                .data_bits(data_bits)
                .stop_bits(stop_bits)
                .parity(parity)
                .flow_control(flow_control)
                .timeout(READ_POLL_INTERVAL)
                .open()
            {
                Ok(port) => port,
                Err(err) => {
                    let err = SerialError::classify_serialport(&actor_path, err);
                    error!("Connection can not be established: {err}");
                    let _ = ready_tx.send(Err(err));
                    return;
                }
            };

            // Termius quirk: right after `open` it called
            // `set({ dsr: false, dtr: false })` whenever `controlFlow` was
            // "none" (logging "Can not set options: ..." on failure).
            // PORT-TODO: drive DTR low here via
            // `SerialPort::write_data_terminal_ready(false)` once the exact
            // serialport 4.x method name is verified; the `dsr` half has no
            // serialport equivalent at all (DSR is an input line, serialport
            // only lets us read it).
            let _ = ready_tx.send(Ok(()));

            let mut read_buf = [0u8; READ_BUF_LEN];
            let mut closing = false;
            let mut failure: Option<SerialError> = None;

            'actor: loop {
                // Drain queued writes / close requests without blocking.
                loop {
                    match cmd_rx.try_recv() {
                        Ok(ActorCmd::Write(chunk)) => {
                            if let Err(err) = port.write_all(&chunk) {
                                failure = Some(SerialError::classify_io(&actor_path, err));
                                break 'actor;
                            }
                        }
                        Ok(ActorCmd::Close) | Err(mpsc::error::TryRecvError::Disconnected) => {
                            closing = true;
                            break;
                        }
                        Err(mpsc::error::TryRecvError::Empty) => break,
                    }
                }
                if closing {
                    break;
                }

                // Blocking read; the configured timeout wakes us up to check
                // for commands and close requests.
                match port.read(&mut read_buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        let chunk = Bytes::copy_from_slice(&read_buf[..n]);
                        if data_tx.blocking_send(Ok(chunk)).is_err() {
                            // Terminal side is gone — stop the actor.
                            break;
                        }
                    }
                    Err(err)
                        if matches!(
                            err.kind(),
                            io::ErrorKind::TimedOut
                                | io::ErrorKind::WouldBlock
                                | io::ErrorKind::Interrupted
                        ) =>
                    {
                        // Routine poll timeout (or EINTR), not a failure.
                    }
                    Err(err) => {
                        failure = Some(SerialError::classify_io(&actor_path, err));
                        break;
                    }
                }
            }

            match failure {
                Some(err) => {
                    error!("Connection closed with error: {err}");
                    let _ = data_tx.try_send(Err(err));
                }
                None => {
                    // Clean close: the terminal still gets an explicit event.
                    let _ = data_tx.try_send(Err(SerialError::Closed));
                }
            }
            // Dropping `port` releases the device; dropping the senders ends
            // the terminal's `recv()` stream.
        });

        match ready_rx.await {
            Ok(Ok(())) => {
                info!("Connection to \"{path}\" established");
                Ok(Self {
                    config,
                    data_rx,
                    cmd_tx,
                    closed,
                    actor: Some(actor),
                })
            }
            Ok(Err(err)) => {
                // The actor already reported the failure; reap it.
                let _ = actor.await;
                Err(err)
            }
            Err(_) => {
                // Actor exited/panicked before reporting: preserve a reason.
                let _ = actor.await;
                Err(SerialError::Io {
                    port: path,
                    source: io::Error::other(
                        "serial worker exited before reporting the open result",
                    ),
                })
            }
        }
    }

    /// The device path this connection was opened with.
    pub fn path(&self) -> &str {
        &self.config.path
    }

    /// The effective configuration (path argument wins, see [`open`](Self::open)).
    pub fn config(&self) -> &SerialConfig {
        &self.config
    }

    /// Whether `close()` has been called (or the connection is otherwise shut
    /// down on this side).
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Receive the next item from the port:
    ///
    /// * `Some(Ok(bytes))` — a chunk received from the device,
    /// * `Some(Err(_))` — a terminal event: [`SerialError::Closed`] after a
    ///   clean remote-side close, or the classified I/O/transport failure,
    /// * `None` — the stream has ended (nothing more will arrive); this is
    ///   also what a [`close`](Self::close)d connection returns.
    pub async fn recv(&mut self) -> Option<Result<Bytes>> {
        if self.is_closed() {
            return None;
        }
        self.data_rx.recv().await
    }

    /// Queue bytes for transmission to the device.
    ///
    /// Returns [`SerialError::Closed`] once the connection is closed or the
    /// transport has failed; otherwise the write is handed to the actor and
    /// flushed with back-pressure.
    pub async fn send(&self, data: impl Into<Bytes>) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SerialError::Closed);
        }
        self.cmd_tx
            .send(ActorCmd::Write(data.into()))
            .await
            .map_err(|_| SerialError::Closed)
    }

    /// Close the connection (idempotent).
    ///
    /// Signals the actor to stop, discards buffered output nobody will read
    /// anymore, and waits (bounded by `CLOSE_TIMEOUT` in total) for the actor
    /// to release the port so it can be reopened immediately. Afterwards
    /// [`recv`](Self::recv) returns `None` and [`send`](Self::send) returns
    /// [`SerialError::Closed`].
    pub async fn close(&mut self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + CLOSE_TIMEOUT;
        // Unblock a pending `blocking_send` first so the actor can observe the
        // close request even when the data channel was full.
        while self.data_rx.try_recv().is_ok() {}
        let _ = tokio::time::timeout_at(deadline, self.cmd_tx.send(ActorCmd::Close)).await;
        // Discard output pushed between the signal and the actor's exit.
        while self.data_rx.try_recv().is_ok() {}
        if let Some(actor) = self.actor.take() {
            match tokio::time::timeout_at(deadline, actor).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => debug!("serial actor task failed while closing: {err}"),
                Err(_) => debug!("serial actor did not stop within {CLOSE_TIMEOUT:?}"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_termius_initial_serial_options() {
        let config = SerialConfig::default();
        assert_eq!(config.path, "");
        assert_eq!(config.baud_rate, 9600);
        assert_eq!(config.data_bits, 8);
        assert_eq!(config.stop_bits, 1);
        assert_eq!(config.parity, Parity::None);
        assert_eq!(config.flow_control, FlowControl::None);
        assert_eq!(config.charset, "UTF-8");
        // Termius disabled Connect while path/baudRate were unset.
        assert!(config.validate().is_err());
    }

    #[test]
    fn config_maps_onto_serialport_enums() {
        let mut config = SerialConfig::new("/dev/ttyUSB0");
        assert!(matches!(
            config.to_data_bits().ok(),
            Some(serialport::DataBits::Eight)
        ));
        config.data_bits = 5;
        assert!(matches!(
            config.to_data_bits().ok(),
            Some(serialport::DataBits::Five)
        ));
        config.data_bits = 4;
        assert!(matches!(
            config.to_data_bits(),
            Err(SerialError::InvalidConfig(_))
        ));
        config.data_bits = 8;

        assert!(matches!(
            config.to_stop_bits().ok(),
            Some(serialport::StopBits::One)
        ));
        config.stop_bits = 2;
        assert!(matches!(
            config.to_stop_bits().ok(),
            Some(serialport::StopBits::Two)
        ));
        config.stop_bits = 3;
        assert!(matches!(
            config.to_stop_bits(),
            Err(SerialError::InvalidConfig(_))
        ));
        config.stop_bits = 1;

        config.parity = Parity::None;
        assert!(matches!(config.to_parity(), serialport::Parity::None));
        config.parity = Parity::Odd;
        assert!(matches!(config.to_parity(), serialport::Parity::Odd));
        config.parity = Parity::Even;
        assert!(matches!(config.to_parity(), serialport::Parity::Even));
    }

    #[test]
    fn flow_control_maps_onto_serialport_enums() {
        let mut config = SerialConfig::new("/dev/ttyUSB0");
        assert!(matches!(
            config.to_flow_control(),
            serialport::FlowControl::None
        ));
        config.flow_control = FlowControl::RtsCts;
        assert!(matches!(
            config.to_flow_control(),
            serialport::FlowControl::Hardware
        ));
        config.flow_control = FlowControl::XonXoff;
        assert!(matches!(
            config.to_flow_control(),
            serialport::FlowControl::Software
        ));
        // Termius never passed dsrdtr options to the port (only rtscts/xon/xoff
        // flags were toggled), so dsrdtr behaves exactly like none — and
        // serialport has no DSR/DTR flow-control mode at all.
        config.flow_control = FlowControl::DsrDtr;
        assert!(matches!(
            config.to_flow_control(),
            serialport::FlowControl::None
        ));
    }

    #[test]
    fn baud_from_port_uses_termius_presets_otherwise_default() {
        assert_eq!(SerialConfig::baud_from_port(9600), 9600);
        assert_eq!(SerialConfig::baud_from_port(57600), 57600);
        assert_eq!(SerialConfig::baud_from_port(50), 50);
        assert_eq!(SerialConfig::baud_from_port(134), 134);
        // Not a baud preset (e.g. the SSH default port) → Termius default.
        assert_eq!(SerialConfig::baud_from_port(22), DEFAULT_BAUD_RATE);
        assert_eq!(SerialConfig::baud_from_port(0), DEFAULT_BAUD_RATE);
        assert_eq!(SerialConfig::baud_from_port(u16::MAX), DEFAULT_BAUD_RATE);
    }

    #[test]
    fn from_connection_params_maps_host_and_port() {
        let mut params = ConnectionParams::default();
        params.host = "/dev/tty.usbserial-A10K3".to_string();
        params.port = 19200;
        let config = SerialConfig::from_connection_params(&params).expect("valid params");
        assert_eq!(config.path, "/dev/tty.usbserial-A10K3");
        assert_eq!(config.baud_rate, 19200);
        // Everything else keeps the Termius defaults.
        assert_eq!(config.data_bits, 8);
        assert_eq!(config.stop_bits, 1);
        assert_eq!(config.parity, Parity::None);
        assert_eq!(config.flow_control, FlowControl::None);
        assert!(config.validate().is_ok());

        params.host = "   ".to_string();
        assert!(matches!(
            SerialConfig::from_connection_params(&params),
            Err(SerialError::InvalidConfig(_))
        ));
    }

    #[test]
    fn from_host_requires_a_serial_host() {
        let mut host = Host::default(); // HostType::Ssh by default
        host.hostname = "/dev/ttyACM0".to_string();
        host.port = 38400;
        assert!(matches!(
            SerialConfig::from_host(&host),
            Err(SerialError::Unsupported(_))
        ));

        host.host_type = HostType::Serial;
        let config = SerialConfig::from_host(&host).expect("serial host");
        assert_eq!(config.path, "/dev/ttyACM0");
        assert_eq!(config.baud_rate, 38400);
    }

    #[test]
    fn validate_rejects_bad_values() {
        let mut config = SerialConfig::new("/dev/ttyUSB0");
        assert!(config.validate().is_ok());

        config.baud_rate = 0;
        assert!(config.validate().is_err());
        config.baud_rate = 9600;

        config.data_bits = 9;
        assert!(config.validate().is_err());
        config.data_bits = 8;

        config.stop_bits = 3;
        assert!(config.validate().is_err());
        config.stop_bits = 1;

        config.charset = "  ".to_string();
        assert!(config.validate().is_err());
        config.charset = "UTF-8".to_string();

        config.path = " ".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn serial_config_uses_termius_field_names() {
        let config = SerialConfig {
            path: "/dev/ttyUSB0".to_string(),
            baud_rate: 115200,
            data_bits: 7,
            stop_bits: 2,
            parity: Parity::Even,
            flow_control: FlowControl::XonXoff,
            charset: "UTF-8".to_string(),
        };
        let value = serde_json::to_value(&config).expect("serialize");
        assert_eq!(value["path"], "/dev/ttyUSB0");
        assert_eq!(value["baudRate"], 115200);
        assert_eq!(value["dataBits"], 7);
        assert_eq!(value["stopBits"], 2);
        assert_eq!(value["parity"], "even");
        assert_eq!(value["controlFlow"], "xonxoff");
        assert_eq!(value["charset"], "UTF-8");

        let back: SerialConfig = serde_json::from_str(
            r#"{"path":"/dev/ttyACM0","baudRate":9600,"dataBits":8,"stopBits":1,"parity":"none","controlFlow":"rtscts","charset":"UTF-8"}"#,
        )
        .expect("deserialize Termius serialOptions");
        assert_eq!(back.path, "/dev/ttyACM0");
        assert_eq!(back.baud_rate, 9600);
        assert_eq!(back.flow_control, FlowControl::RtsCts);
        assert_eq!(back.parity, Parity::None);

        assert_eq!(Parity::Even.as_str(), "even");
        assert_eq!(FlowControl::RtsCts.as_str(), "rtscts");
        assert_eq!(FlowControl::DsrDtr.as_str(), "dsrdtr");
    }

    #[tokio::test]
    async fn open_rejects_invalid_config_before_touching_hardware() {
        let result = SerialConnection::open("", SerialConfig::default()).await;
        assert!(matches!(result, Err(SerialError::InvalidConfig(_))));

        let result = SerialConnection::open("/dev/ttyUSB0", SerialConfig::default().with_baud_rate(0))
            .await;
        assert!(matches!(result, Err(SerialError::InvalidConfig(_))));
    }

    #[tokio::test]
    async fn open_classifies_a_missing_device_as_port_not_found() {
        // Negative test only: no real port is touched, the path cannot exist.
        let result =
            SerialConnection::open("/nonexistent/termius-no-such-port", SerialConfig::default())
                .await;
        assert!(matches!(result, Err(SerialError::PortNotFound(_))));
    }
}

