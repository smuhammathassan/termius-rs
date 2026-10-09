//! Serial-port connection engine over the `serialport` crate: enumerate
//! ports, open/configure (baud rate, data bits, parity, stop bits, flow
//! control), read/write byte streams, terminal integration.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered
//! sources): Termius used `@termius/serialport-bindings` (node-serialport)
//! behind its serial-terminal feature — the `openSerialConnection` menu
//! action (⌘⌥S / Ctrl+Alt+S) and `serialOptions` dialog. See [`port`] for the
//! recovered config surface and the `ConnectionParams` mapping.
//!
//! [`port::SerialConnection`] bridges the blocking serial handle onto a
//! tokio blocking task: an async read side ([`port::SerialConnection::recv`]),
//! an async write side ([`port::SerialConnection::send`]) and
//! [`port::SerialConnection::close`].
//!
//! # Example
//!
//! ```no_run
//! use bytes::Bytes;
//! use termius_serial::{SerialConfig, SerialConnection};
//!
//! # async fn run() -> termius_serial::Result<()> {
//! let config = SerialConfig::new("/dev/ttyUSB0").with_baud_rate(115200);
//! let mut conn = SerialConnection::open("/dev/ttyUSB0", config).await?;
//! conn.send(Bytes::from_static(b"help\r\n")).await?;
//! while let Some(event) = conn.recv().await {
//!     match event {
//!         Ok(chunk) => {
//!             // Feed the bytes to the terminal emulator.
//!             let _ = chunk;
//!         }
//!         Err(err) => {
//!             eprintln!("serial session ended: {err}");
//!             break;
//!         }
//!     }
//! }
//! conn.close().await?;
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod port;

pub use error::{Result, SerialError};
pub use port::{
    list_ports, FlowControl, Parity, PortKind, SerialConfig, SerialConnection, SerialPortInfo,
    DEFAULT_BAUD_RATE, DEFAULT_CHARSET, DEFAULT_DATA_BITS, DEFAULT_STOP_BITS,
    STANDARD_BAUD_RATES,
};
