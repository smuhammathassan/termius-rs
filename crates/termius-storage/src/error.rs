//! error — `StorageError`, the crate-wide failure type (see crate docs).
//!
//! Every variant preserves its cause: SQLite, serde_json, and I/O failures keep
//! the original error as `source`, migration failures box the underlying cause,
//! and keychain failures carry the OS backend's message (never secret values).

/// Result alias used by every `termius-storage` API.
pub type Result<T> = std::result::Result<T, StorageError>;

/// Every failure the storage layer can produce.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// SQLite-level failure: open, constraint, query, pragma, or transaction.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// `get_by_id` / `update` targeted a record that does not exist.
    #[error("record not found in `{table}`: {id}")]
    NotFound {
        /// Table the record was sought in.
        table: &'static str,
        /// Primary key that was sought.
        id: String,
    },

    /// serde_json failed while marshalling a record or a JSON column.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The OS keychain could not be reached at all (locked, missing backend,
    /// headless CI, …). Callers should surface this to the user rather than
    /// retrying silently.
    #[error("OS keychain unavailable: {0}")]
    KeychainUnavailable(String),

    /// The OS keychain rejected or failed one specific operation.
    #[error("OS keychain access failed: {0}")]
    KeychainAccess(String),

    /// A migration could not be applied, or the on-disk schema was written by a
    /// newer build than this one.
    #[error("migration to version {version} failed: {source}")]
    Migration {
        /// Schema version the migration targeted (or the version found on disk).
        version: i64,
        /// Underlying cause, preserved for diagnostics.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// Filesystem failure while creating or opening the data directory.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The per-platform data directory could not be derived (no home dir).
    #[error("application data directory could not be determined")]
    NoDataDirectory,

    /// A stored column value could not be converted back into its field.
    #[error("invalid stored value for `{field}`: {message}")]
    InvalidValue {
        /// Column / field the value belonged to (never contains secret data).
        field: &'static str,
        /// Description of what was wrong.
        message: String,
    },
}

impl StorageError {
    /// Build a [`StorageError::Migration`] from a plain message.
    pub fn migration(version: i64, message: impl Into<String>) -> Self {
        let message: String = message.into();
        Self::Migration {
            version,
            source: Box::new(std::io::Error::other(message)),
        }
    }

    /// Build a [`StorageError::Migration`] preserving the underlying cause.
    pub fn migration_with_cause(
        version: i64,
        cause: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Migration {
            version,
            source: Box::new(cause),
        }
    }

    /// True when the target record simply does not exist.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }

    /// True when the OS keychain could not be used at all.
    pub fn is_keychain_unavailable(&self) -> bool {
        matches!(self, Self::KeychainUnavailable(_))
    }
}
