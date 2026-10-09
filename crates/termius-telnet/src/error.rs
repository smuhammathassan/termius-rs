//! Telnet engine errors.
//!
//! Mirrors how the Electron app reported failures: the native addon threw
//! errors carrying a message plus `libtermius_error_type` /
//! `libtermius_error_code` (see the `errorCallback` wiring in the recovered
//! UI `jY` telnet shell provider). We keep the same information — the failure
//! reason (DNS vs. connect vs. timeout vs. protocol vs. closed) is always
//! preserved via dedicated variants and `#[source]` chains.

use thiserror::Error;

/// All failures produced by the telnet engine.
#[derive(Debug, Error)]
pub enum TelnetError {
    /// DNS resolution of the telnet host failed.
    #[error("failed to resolve telnet host {host}")]
    Dns {
        host: String,
        source: std::io::Error,
    },

    /// TCP connect to the resolved address failed.
    #[error("failed to connect to {addr}")]
    Connect {
        addr: String,
        source: std::io::Error,
    },

    /// The overall connect budget (`ConnectionParams::connect_timeout_secs`)
    /// elapsed before a connection was established.
    #[error("connection to {addr} timed out after {secs}s")]
    Timeout { addr: String, secs: u64 },

    /// Any other I/O failure while reading, writing, or shutting down.
    #[error("telnet io error: {0}")]
    Io(#[from] std::io::Error),

    /// Malformed telnet protocol traffic (kept distinct from I/O so callers
    /// can retry/reset instead of tearing the socket down blindly).
    #[error("telnet protocol error: {reason}")]
    Protocol { reason: String },

    /// The session is closed (or was closed by the peer).
    #[error("telnet connection closed")]
    Closed,
}

impl TelnetError {
    pub fn dns(host: impl Into<String>, source: std::io::Error) -> Self {
        Self::Dns { host: host.into(), source }
    }

    pub fn connect(addr: impl Into<String>, source: std::io::Error) -> Self {
        Self::Connect { addr: addr.into(), source }
    }

    pub fn timeout(addr: impl Into<String>, secs: u64) -> Self {
        Self::Timeout { addr: addr.into(), secs }
    }

    pub fn protocol(reason: impl Into<String>) -> Self {
        Self::Protocol { reason: reason.into() }
    }

    /// Stable category string for UI reporting. Approximates the native
    /// addon's `libtermius_error_type` surface (which was a C++ enum we
    /// cannot recover byte-exactly).
    ///
    /// PORT-TODO: the numeric `libtermius_error_code` values are baked into
    /// the closed-source `termius.node` binary and were not recovered; the UI
    /// integration wave should treat `error_type()` as the source of truth.
    pub fn error_type(&self) -> &'static str {
        match self {
            Self::Dns { .. } => "dns",
            Self::Connect { .. } => "connect",
            Self::Timeout { .. } => "timeout",
            Self::Io(_) => "io",
            Self::Protocol { .. } => "protocol",
            Self::Closed => "closed",
        }
    }
}

/// Convenience result alias for the telnet engine.
pub type Result<T> = std::result::Result<T, TelnetError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn io() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused")
    }

    #[test]
    fn error_type_is_stable_per_variant() {
        assert_eq!(TelnetError::dns("example.com", io()).error_type(), "dns");
        assert_eq!(
            TelnetError::connect("example.com:23", io()).error_type(),
            "connect"
        );
        assert_eq!(TelnetError::timeout("example.com:23", 30).error_type(), "timeout");
        assert_eq!(TelnetError::Io(io()).error_type(), "io");
        assert_eq!(TelnetError::protocol("bad subnegotiation").error_type(), "protocol");
        assert_eq!(TelnetError::Closed.error_type(), "closed");
    }

    #[test]
    fn display_preserves_reasons() {
        let err = TelnetError::connect("10.0.0.1:23", io());
        let rendered = err.to_string();
        assert!(rendered.contains("10.0.0.1:23"));
        // The source chain must keep the underlying OS reason.
        let mut chain = std::error::Error::source(&err);
        let mut found = false;
        while let Some(e) = chain {
            if e.to_string().contains("refused") {
                found = true;
            }
            chain = e.source();
        }
        assert!(found, "underlying io reason lost from the error chain");
    }
}
