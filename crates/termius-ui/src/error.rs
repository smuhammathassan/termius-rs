//! error — the UI-layer error type.
//!
//! [`UiError`] wraps every engine/storage failure the renderer can surface
//! (status bar, notifications, tab badges) while preserving the failure class
//! the PORTING plan requires: auth vs network vs protocol failures keep coming
//! from the typed `SshError`, storage problems from `StorageError`, and so on.

use thiserror::Error;

/// Errors produced by the UI layer and the view models it drives.
#[derive(Debug, Error)]
pub enum UiError {
    /// Local persistence failed (open/list/create/update/delete).
    #[error("storage error: {0}")]
    Storage(#[from] termius_storage::StorageError),

    /// The SSH engine failed (connect, auth, host key, channel, SFTP).
    #[error("ssh error: {0}")]
    Ssh(#[from] termius_ssh::SshError),

    /// The terminal emulator rejected an operation (degenerate geometry…).
    #[error("terminal error: {0}")]
    Terminal(#[from] termius_terminal::TerminalError),

    /// A domain record failed validation.
    #[error("domain error: {0}")]
    Core(#[from] termius_core::CoreError),

    /// No open session matched the requested id.
    #[error("no open session `{0}`")]
    NoSession(String),

    /// An operation needed a selected host and none was selected.
    #[error("no host selected")]
    NoSelection,

    /// Free-form failure (bridge task, runtime setup, marshalling…).
    #[error("{0}")]
    Message(String),
}

impl UiError {
    /// Wrap a plain message.
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    /// Short text fit for the status bar / a notification.
    ///
    /// The `Display` impl already carries context, so this is an alias kept at
    /// the call sites that render errors rather than logs them.
    pub fn user_message(&self) -> String {
        self.to_string()
    }
}

impl From<String> for UiError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for UiError {
    fn from(message: &str) -> Self {
        Self::Message(message.to_owned())
    }
}

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, UiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_preserves_wrapped_context() {
        let err = UiError::NoSession("web-1-2".into());
        assert_eq!(err.to_string(), "no open session `web-1-2`");
        assert_eq!(err.user_message(), err.to_string());
    }

    #[test]
    fn message_conversions() {
        assert!(matches!(UiError::from("boom"), UiError::Message(m) if m == "boom"));
        assert!(matches!(UiError::from(String::from("boom")), UiError::Message(_)));
        assert!(matches!(UiError::message("x"), UiError::Message(_)));
    }
}
