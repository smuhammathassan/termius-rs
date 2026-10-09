//! error — crate error type.

use thiserror::Error;

/// Errors produced by the terminal emulation layer.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TerminalError {
    /// The requested terminal geometry is unusable (zero rows/cols or
    /// otherwise out of range).
    #[error("invalid terminal size: {reason}")]
    InvalidSize { reason: String },

    /// The VT parser rejected or failed on an input sequence.
    #[error("terminal parse error: {reason}")]
    Parse { reason: String },

    /// A feature the UI asked for is not supported by this layer.
    #[error("unsupported terminal operation: {reason}")]
    Unsupported { reason: String },
}

impl TerminalError {
    pub fn invalid_size(reason: impl Into<String>) -> Self {
        Self::InvalidSize { reason: reason.into() }
    }

    pub fn parse(reason: impl Into<String>) -> Self {
        Self::Parse { reason: reason.into() }
    }

    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::Unsupported { reason: reason.into() }
    }
}

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, TerminalError>;
