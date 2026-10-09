//! Mosh engine errors.
//!
//! The original Termius app bundled a native `@termius/mosh` addon
//! (`moshclient.node`) that embedded the mosh protocol (SSH bootstrap + SSHP
//! state-synchronization over UDP). The Rust port instead shells out to the
//! system `mosh` binary, so failures map to: binary missing, spawn failure,
//! bootstrap (SSH phase) failure, I/O on the bridged stdio, closed session,
//! and operations the shell-out bridge cannot honour (e.g. password auth).
//! Failure reasons are always preserved via dedicated variants and
//! `#[source]` chains.

use thiserror::Error;

/// All failures produced by the mosh engine.
#[derive(Debug, Error)]
pub enum MoshError {
    /// No `mosh` binary found on `PATH` — the shell-out bridge cannot work
    /// without it.
    #[error(
        "mosh client not found on PATH; install mosh (e.g. `brew install mosh` or `apt install mosh`)"
    )]
    ClientNotFound,

    /// Spawning the `mosh` child process failed (permission, resource, …).
    /// The full command line is preserved for diagnostics.
    #[error("failed to spawn mosh: {command}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },

    /// The mosh bootstrap (SSH phase that launches `mosh-server` on the
    /// remote) failed. `reason` carries the captured stderr tail or exit
    /// status so the UI can show why.
    #[error("mosh connection to {target} failed: {reason}")]
    Connect { target: String, reason: String },

    /// Any other I/O failure while reading, writing, or shutting down the
    /// bridged child stdio.
    #[error("mosh io error: {0}")]
    Io(#[from] std::io::Error),

    /// The session is closed (or the child exited) — no further reads or
    /// writes are possible.
    #[error("mosh session closed")]
    Closed,

    /// An operation the shell-out bridge cannot honour (e.g. password or
    /// inline-key auth, which need an askpass helper / temp keyfile).
    #[error("unsupported mosh operation: {reason}")]
    Unsupported { reason: String },
}

impl MoshError {
    pub fn spawn(command: impl Into<String>, source: std::io::Error) -> Self {
        Self::Spawn { command: command.into(), source }
    }

    pub fn connect(target: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Connect { target: target.into(), reason: reason.into() }
    }

    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::Unsupported { reason: reason.into() }
    }

    /// Stable category string for UI reporting, mirroring the
    /// `libtermius_error_type`-style categories the Electron app used.
    pub fn error_type(&self) -> &'static str {
        match self {
            Self::ClientNotFound => "client_not_found",
            Self::Spawn { .. } => "spawn",
            Self::Connect { .. } => "connect",
            Self::Io(_) => "io",
            Self::Closed => "closed",
            Self::Unsupported { .. } => "unsupported",
        }
    }
}

/// Convenience result alias for the mosh engine.
pub type Result<T> = std::result::Result<T, MoshError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn io() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused")
    }

    #[test]
    fn error_type_is_stable_per_variant() {
        assert_eq!(MoshError::ClientNotFound.error_type(), "client_not_found");
        assert_eq!(
            MoshError::spawn("mosh --ssh=ssh user@host", io()).error_type(),
            "spawn"
        );
        assert_eq!(
            MoshError::connect("user@host", "Permission denied").error_type(),
            "connect"
        );
        assert_eq!(MoshError::Io(io()).error_type(), "io");
        assert_eq!(MoshError::Closed.error_type(), "closed");
        assert_eq!(MoshError::unsupported("password auth").error_type(), "unsupported");
    }

    #[test]
    fn display_preserves_reasons() {
        let err = MoshError::connect("root@10.0.0.1", "Permission denied (publickey)");
        let rendered = err.to_string();
        assert!(rendered.contains("root@10.0.0.1"));
        assert!(rendered.contains("Permission denied (publickey)"));

        let spawn = MoshError::spawn("mosh --ssh=ssh user@host", io());
        let rendered = spawn.to_string();
        assert!(rendered.contains("mosh --ssh=ssh user@host"));
        let mut chain = std::error::Error::source(&spawn);
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
