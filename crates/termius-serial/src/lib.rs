//! Serial-port connection engine over the serialport crate: open/configure (baud, data bits, parity, stop bits, flow control), read/write byte streams, terminal integration.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).

pub mod port;

pub mod error;
