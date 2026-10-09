//! error — see crate docs.
//!
//! `SshError` mirrors how the original Termius addon surfaced failures: the UI
//! distinguishes "auth rejected" from "host key" from "network down" (the
//! renderer reads `libtermius_error_type` / `libtermius_error_code`), so every
//! variant keeps the failure reason intact and offers cheap classifiers.

use thiserror::Error;

/// Convenience result alias for the SSH engine.
pub type Result<T> = std::result::Result<T, SshError>;

/// All failures produced by the SSH engine.
///
/// Classification happens at the call site (we know whether we were in the
/// TCP phase, the handshake phase or the auth phase), not by string-matching
/// foreign error payloads — this keeps the reason stable across `russh`
/// versions.
#[derive(Debug, Error)]
pub enum SshError {
    /// TCP / DNS / transport level failure (connection refused, DNS NXDOMAIN,
    /// connection reset mid-session, handshake transport error).
    #[error("connection failed: {message}")]
    Connection { message: String },

    /// The server rejected our credentials (password, key or agent).
    #[error("authentication rejected ({method}): {message}")]
    Authentication { method: String, message: String },

    /// The server host key did not pass verification.
    #[error("host key verification failed: {message}")]
    HostKey { message: String },

    /// Protocol-level failure: malformed messages, unsupported algorithms,
    /// or a `russh` transport error raised outside the connect/auth phases.
    #[error("ssh protocol error: {message}")]
    Protocol { message: String },

    /// A connect or handshake deadline elapsed.
    #[error("connection timed out: {message}")]
    Timeout { message: String },

    /// The channel was closed (or never opened) when an operation needed it.
    #[error("channel closed: {message}")]
    ChannelClosed { message: String },

    /// SFTP subsystem failure (status codes, bad paths, …).
    #[error("sftp error: {message}")]
    Sftp { message: String },

    /// Local I/O failure (listener bind, temp key file, socket read/write).
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

impl SshError {
    pub fn connection(message: impl Into<String>) -> Self {
        Self::Connection { message: message.into() }
    }

    pub fn auth(method: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Authentication { method: method.into(), message: message.into() }
    }

    pub fn host_key(message: impl Into<String>) -> Self {
        Self::HostKey { message: message.into() }
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol { message: message.into() }
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::Timeout { message: message.into() }
    }

    pub fn channel_closed(message: impl Into<String>) -> Self {
        Self::ChannelClosed { message: message.into() }
    }

    pub fn sftp(message: impl Into<String>) -> Self {
        Self::Sftp { message: message.into() }
    }

    /// `true` when the server refused our credentials.
    pub fn is_authentication(&self) -> bool {
        matches!(self, Self::Authentication { .. })
    }

    /// `true` when the failure is a host-key verification problem.
    pub fn is_host_key(&self) -> bool {
        matches!(self, Self::HostKey { .. })
    }

    /// `true` when the failure is a connect/handshake deadline.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }

    /// `true` for TCP/DNS/transport failures (i.e. the server was never, or
    /// is no longer, reachable).
    pub fn is_connection(&self) -> bool {
        matches!(self, Self::Connection { .. })
    }
}

/// `russh` errors raised outside a phase we can classify get filed under
/// `Protocol`; phase-aware call sites (connect / auth) map them explicitly
/// instead of going through this blanket conversion.
impl From<russh::Error> for SshError {
    fn from(err: russh::Error) -> Self {
        Self::Protocol { message: err.to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_preserves_reason() {
        let e = SshError::auth("password", "permission denied");
        assert!(e.to_string().contains("password"));
        assert!(e.to_string().contains("permission denied"));

        let e = SshError::host_key("pinned key mismatch");
        assert!(e.to_string().contains("pinned key mismatch"));
    }

    #[test]
    fn classifiers_match_variants() {
        assert!(SshError::auth("public key", "no").is_authentication());
        assert!(!SshError::auth("public key", "no").is_connection());

        assert!(SshError::host_key("bad").is_host_key());
        assert!(SshError::timeout("30s").is_timeout());
        assert!(SshError::connection("refused").is_connection());
        assert!(!SshError::sftp("no such file").is_authentication());
    }

    #[test]
    fn io_conversion_is_lossless() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let e: SshError = io.into();
        assert!(matches!(e, SshError::Io(_)));
    }

    // NOTE: no test constructs a `russh::Error` directly — variant names are
    // exactly the kind of detail we refuse to guess at. The `From` impl only
    // relies on `Display`, which every `russh::Error` provides.
}
