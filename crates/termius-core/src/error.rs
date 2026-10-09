//! Core error type for domain validation and model operations.

use thiserror::Error;

/// Domain-level failure: malformed model, failed validation, or id/reference
/// resolution failure. Kept distinct from engine (I/O) errors.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("validation failed for {field}: {reason}")]
    Validation {
        field: String,
        reason: String,
    },
    #[error("record not found: {kind} {id}")]
    NotFound { kind: String, id: String },
    #[error("invalid {kind}: {reason}")]
    Invalid { kind: String, reason: String },
}

impl CoreError {
    pub fn validation(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Validation { field: field.into(), reason: reason.into() }
    }
    pub fn invalid(kind: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Invalid { kind: kind.into(), reason: reason.into() }
    }
}

/// Convenience result alias for domain operations.
pub type Result<T> = std::result::Result<T, CoreError>;
