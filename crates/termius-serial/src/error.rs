//! Serial-port engine errors.
//!
//! Termius surfaced serial failures through the connection delegate
//! (`Connection closed with error: ...`, `Connection can not be established: ...`).
//! The Rust port keeps the *reason* distinct so the UI can render the right
//! recovery hint: a missing device, a permission denial (missing `dialout`
//! group / udev rule / macOS TCC), a bad configuration, an I/O failure, a
//! closed connection and an unsupported operation are separate variants.

use std::io;
use std::result;

/// Result alias used across this crate.
pub type Result<T> = result::Result<T, SerialError>;

/// Every way a serial-port operation can fail.
#[derive(Debug, thiserror::Error)]
pub enum SerialError {
    /// The device path does not exist (unplugged device, wrong name, ...).
    #[error("serial port not found: {0}")]
    PortNotFound(String),

    /// The process is not allowed to open the device.
    #[error("permission denied opening serial port: {0}")]
    PermissionDenied(String),

    /// The configuration itself is invalid (empty path, 0 baud, data bits
    /// outside 5..=8, stop bits outside 1..=2, empty charset, ...).
    #[error("invalid serial configuration: {0}")]
    InvalidConfig(String),

    /// An I/O error while reading from or writing to the port.
    #[error("serial I/O error on {port}: {source}")]
    Io {
        port: String,
        #[source]
        source: io::Error,
    },

    /// The connection was closed (locally via `close()`, or by the transport).
    #[error("serial connection closed")]
    Closed,

    /// The requested operation cannot be supported (wrong host type, feature
    /// unavailable on this platform, ...).
    #[error("unsupported serial operation: {0}")]
    Unsupported(String),
}

impl SerialError {
    /// Classify a plain `io::Error` raised while talking to `port`.
    ///
    /// Preserves the reason: a permission denial and a missing device become
    /// their own variants instead of a generic [`SerialError::Io`].
    pub fn classify_io(port: &str, err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(format!("{port}: {err}")),
            io::ErrorKind::NotFound => Self::PortNotFound(format!("{port}: {err}")),
            _ => Self::Io {
                port: port.to_string(),
                source: err,
            },
        }
    }

    /// Classify a `serialport` crate error raised while talking to `port`.
    ///
    /// `serialport::Error` wraps an `io::Error` whenever the OS refused the
    /// call; walking the source chain recovers the exact reason. When the
    /// crate carries only message text, fall back to matching that text.
    pub fn classify_serialport(port: &str, err: serialport::Error) -> Self {
        let message = err.to_string();
        let mut source = std::error::Error::source(&err);
        while let Some(current) = source {
            if let Some(io_err) = current.downcast_ref::<io::Error>() {
                // Rebuild the io::Error with serialport's (often richer)
                // message while keeping the authoritative kind.
                return Self::classify_io(port, io::Error::new(io_err.kind(), message));
            }
            source = std::error::Error::source(current);
        }
        Self::classify_message(port, message)
    }

    /// Best-effort classification from a message alone, used when `serialport`
    /// reports no `io::Error` source.
    pub(crate) fn classify_message(port: &str, message: String) -> Self {
        let lower = message.to_lowercase();
        if lower.contains("permission denied")
            || lower.contains("operation not permitted")
            || lower.contains("os error 13")
        {
            Self::PermissionDenied(format!("{port}: {message}"))
        } else if lower.contains("no such file")
            || lower.contains("not found")
            || lower.contains("cannot find")
            || lower.contains("no such device")
            || lower.contains("device not configured")
            || lower.contains("os error 2")
        {
            Self::PortNotFound(format!("{port}: {message}"))
        } else {
            Self::Io {
                port: port.to_string(),
                source: io::Error::other(message),
            }
        }
    }
}

impl From<serialport::Error> for SerialError {
    /// Convenience conversion when no port context is available; the reason
    /// (permission vs. missing device vs. I/O) is still classified.
    fn from(err: serialport::Error) -> Self {
        Self::classify_serialport("<serial port>", err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn permission_denied_io_error_maps_to_permission_denied() {
        let err = SerialError::classify_io(
            "/dev/ttyUSB0",
            io::Error::from(io::ErrorKind::PermissionDenied),
        );
        assert!(matches!(err, SerialError::PermissionDenied(_)));
        // Reason preserved in the message.
        assert!(err.to_string().contains("permission denied"));
        assert!(err.to_string().contains("/dev/ttyUSB0"));
    }

    #[test]
    fn missing_device_io_error_maps_to_port_not_found() {
        let err = SerialError::classify_io("/dev/ttyUSB0", io::Error::from(io::ErrorKind::NotFound));
        assert!(matches!(err, SerialError::PortNotFound(_)));
        // A missing device must not be mistaken for a permission problem.
        assert!(!matches!(err, SerialError::PermissionDenied(_)));
    }

    #[test]
    fn other_io_errors_stay_io_and_keep_the_kind() {
        let err = SerialError::classify_io("COM3", io::Error::from(io::ErrorKind::BrokenPipe));
        match err {
            SerialError::Io { port, source } => {
                assert_eq!(port, "COM3");
                assert_eq!(source.kind(), io::ErrorKind::BrokenPipe);
            }
            other => panic!("expected SerialError::Io, got {other:?}"),
        }
    }

    #[test]
    fn message_fallback_classifies_os_reasons() {
        assert!(matches!(
            SerialError::classify_message("/dev/ttyS0", "Permission denied (os error 13)".into()),
            SerialError::PermissionDenied(_)
        ));
        assert!(matches!(
            SerialError::classify_message(
                "/dev/ttyS0",
                "No such file or directory (os error 2)".into()
            ),
            SerialError::PortNotFound(_)
        ));
        assert!(matches!(
            SerialError::classify_message("COM3", "device unreachable".into()),
            SerialError::Io { .. }
        ));
    }
}
