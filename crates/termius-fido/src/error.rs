//! FIDO2 error type for the security-key layer.
//!
//! Mirrors the failure modes Termius surfaces through `@termius/libfido2`, the
//! Windows WebAuthn API and the UI's WebAuthn error union (see `chunk_c2e.js`
//! in the recovered sources): user cancellation, a missing platform
//! authenticator, WebAuthn DLL/API load failures and CTAP protocol errors.

use thiserror::Error;

/// Coarse classification of a [`FidoError`], so callers (SSH unlock, UI
/// wizards) can branch without string-matching messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FidoErrorKind {
    /// No usable security key / platform authenticator present.
    Device,
    /// Operation not supported by this build or platform.
    Unsupported,
    /// CTAP2/transport protocol failure (preserves the reason).
    Protocol,
    /// The user cancelled (touch timeout, dialog dismissed).
    Cancelled,
    /// I/O failure while talking to the device.
    Io,
    /// Feature whose backend has not been ported yet.
    Unimplemented,
}

/// Every failure this crate can produce.
///
/// The failure *reason* is always preserved (auth vs protocol vs device vs
/// permission), per the workspace error conventions in `PORTING.md`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FidoError {
    /// Enumeration found no security key (or no platform authenticator).
    #[error("no FIDO2 security key was detected")]
    DeviceNotFound,

    /// Operation is not supported by this build/platform (e.g. enumeration
    /// without a linked CTAP2 backend).
    #[error("unsupported FIDO2 operation: {0}")]
    Unsupported(String),

    /// The CTAP2/CBOR exchange or device transport failed.
    #[error("FIDO2 protocol error: {reason}")]
    Protocol {
        /// Human-readable protocol failure reason.
        reason: String,
    },

    /// The user cancelled the operation (touch, PIN prompt, dialog).
    #[error("FIDO2 operation cancelled by the user")]
    Cancelled,

    /// Low-level device I/O failure (HID read/write, interrupted exchange).
    #[error("FIDO2 device I/O error: {0}")]
    Io(String),

    /// The API exists and is stable, but the hardware backend is deferred.
    #[error("FIDO2 feature not implemented: {what}")]
    NotImplemented {
        /// What exactly is missing, and why.
        what: String,
    },
}

impl FidoError {
    /// `Unsupported` with a caller-supplied reason.
    pub fn unsupported(what: impl Into<String>) -> Self {
        Self::Unsupported(what.into())
    }

    /// `Protocol` with a caller-supplied reason.
    pub fn protocol(reason: impl Into<String>) -> Self {
        Self::Protocol { reason: reason.into() }
    }

    /// `NotImplemented` with a caller-supplied reason.
    pub fn not_implemented(what: impl Into<String>) -> Self {
        Self::NotImplemented { what: what.into() }
    }

    /// Coarse classification of this error.
    pub fn kind(&self) -> FidoErrorKind {
        match self {
            Self::DeviceNotFound => FidoErrorKind::Device,
            Self::Unsupported(_) => FidoErrorKind::Unsupported,
            Self::Protocol { .. } => FidoErrorKind::Protocol,
            Self::Cancelled => FidoErrorKind::Cancelled,
            Self::Io(_) => FidoErrorKind::Io,
            Self::NotImplemented { .. } => FidoErrorKind::Unimplemented,
        }
    }

    /// True when the user cancelled the operation.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// True when the failure is a deferred-backend stub.
    pub fn is_not_implemented(&self) -> bool {
        matches!(self, Self::NotImplemented { .. })
    }

    /// True when no security key / platform authenticator was found.
    pub fn is_device_missing(&self) -> bool {
        matches!(self, Self::DeviceNotFound)
    }

    /// Map a raw message coming from the platform/backend onto a typed error.
    ///
    /// PORT-TODO: once a real backend is linked, classify CTAP2 status codes
    /// (`FIDO_ERR_*` from libfido2 / CTAP2 `err` map) directly instead of
    /// matching on message text.
    ///
    /// Known inputs are ported 1:1 from the recovered sources:
    /// * `"The action was cancelled by the user."` — the cancellation check in
    ///   `g2e` (recovered-ui `chunk_c2e.js`);
    /// * the `h2e` WebAuthn error union in the same chunk
    ///   (platform authenticator / webauthn.dll / API-version messages);
    /// * `"Communication with device is interrupted"` and
    ///   `"PIN code cannot be empty."` — strings recovered from the shipped
    ///   `@termius/libfido2` addon (`mac-arm64/libfido2-nodejs.node`).
    pub fn from_backend_message(message: &str) -> Self {
        match message {
            "The action was cancelled by the user." => Self::Cancelled,
            "Webauthn Platform Authenticator not available" => Self::DeviceNotFound,
            "Error loading webauthn API methods"
            | "Error loading webauthn.dll"
            | "Webauthn API version is too old and does not support getPlatformCredentialList API" => {
                Self::Unsupported(message.to_string())
            }
            "Communication with device is interrupted" => Self::Io(message.to_string()),
            other => Self::Protocol { reason: other.to_string() },
        }
    }
}

impl From<std::io::Error> for FidoError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

/// Crate-wide result alias (`PORTING.md`: thiserror per crate).
pub type Result<T> = std::result::Result<T, FidoError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_buckets_match_variant() {
        assert_eq!(FidoError::DeviceNotFound.kind(), FidoErrorKind::Device);
        assert_eq!(FidoError::unsupported("x").kind(), FidoErrorKind::Unsupported);
        assert_eq!(FidoError::protocol("boom").kind(), FidoErrorKind::Protocol);
        assert_eq!(FidoError::Cancelled.kind(), FidoErrorKind::Cancelled);
        assert_eq!(
            FidoError::from(std::io::Error::other("reset")).kind(),
            FidoErrorKind::Io
        );
        assert_eq!(
            FidoError::not_implemented("later").kind(),
            FidoErrorKind::Unimplemented
        );
    }

    #[test]
    fn backend_messages_map_to_expected_errors() {
        assert_eq!(
            FidoError::from_backend_message("The action was cancelled by the user."),
            FidoError::Cancelled
        );
        assert_eq!(
            FidoError::from_backend_message("Webauthn Platform Authenticator not available"),
            FidoError::DeviceNotFound
        );
        for message in [
            "Error loading webauthn API methods",
            "Error loading webauthn.dll",
            "Webauthn API version is too old and does not support getPlatformCredentialList API",
        ] {
            assert_eq!(
                FidoError::from_backend_message(message),
                FidoError::Unsupported(message.to_string()),
                "message: {message}"
            );
        }
        assert_eq!(
            FidoError::from_backend_message("Communication with device is interrupted"),
            FidoError::Io("Communication with device is interrupted".to_string())
        );
        // Unknown backend messages keep their reason instead of being swallowed.
        assert_eq!(
            FidoError::from_backend_message("CTAP2: keepalive timeout"),
            FidoError::protocol("CTAP2: keepalive timeout")
        );
    }

    #[test]
    fn predicate_helpers_classify() {
        assert!(FidoError::Cancelled.is_cancelled());
        assert!(FidoError::not_implemented("x").is_not_implemented());
        assert!(FidoError::DeviceNotFound.is_device_missing());
        assert!(!FidoError::protocol("x").is_cancelled());
        assert!(!FidoError::DeviceNotFound.is_not_implemented());
    }

    #[test]
    fn display_preserves_the_reason() {
        let protocol = FidoError::protocol("bad CID");
        assert!(protocol.to_string().contains("bad CID"));

        let stub = FidoError::not_implemented("CTAP2 getAssertion");
        assert!(stub.to_string().contains("CTAP2 getAssertion"));
    }
}
