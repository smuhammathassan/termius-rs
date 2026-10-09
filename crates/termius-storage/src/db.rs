//! db — SQLite connection handling, per-platform data directory, and the
//! [`Store`] handle that repositories operate through.
//!
//! Replaces Termius' `schema.js` store (crypto/zlib-serialized blob in the
//! utilities process) with a plain SQLite database file. Connections run with
//! WAL journalling, a busy timeout, and `PRAGMA foreign_keys = ON`; schema
//! versioning is handled by [`crate::migrations`] via SQLite `user_version`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::Connection;

use crate::error::{Result, StorageError};
use crate::migrations;

/// Application name used to derive the per-platform data directory
/// (`~/Library/Application Support/Termius` on macOS, `%APPDATA%\Termius` on
/// Windows, `$XDG_DATA_HOME/termius` on Linux).
pub const APP_DIR_NAME: &str = "Termius";

/// Database file name inside the data directory.
pub const DB_FILE_NAME: &str = "termius.db";

/// How long a connection waits on a locked database before erroring.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve the default database path for the current platform, e.g.
/// `~/Library/Application Support/Termius/termius.db` on macOS.
pub fn default_db_path() -> Result<PathBuf> {
    let dirs =
        directories::ProjectDirs::from("", "", APP_DIR_NAME).ok_or(StorageError::NoDataDirectory)?;
    Ok(dirs.data_dir().join(DB_FILE_NAME))
}

/// A handle to the local SQLite store.
///
/// One connection is shared behind a `std::sync::Mutex`; every operation locks
/// it for its own duration (connection-per-operation semantics without the
/// reconnect cost). Callers never hold the lock across other work — the
/// [`Store::with_connection`] escape hatch takes a plain callback.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (creating parent directories as needed) the database at `path`,
    /// apply connection pragmas, and run pending migrations.
    ///
    /// The special path `":memory:"` opens a private in-memory database —
    /// used by the unit tests.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !is_memory(path) {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
        }
        let mut conn = Connection::open(path)?;
        init_connection(&mut conn)?;
        migrations::run(&mut conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Open the database at [`default_db_path`], creating it on first use.
    pub fn open_default() -> Result<Self> {
        Self::open(default_db_path()?)
    }

    /// Run a closure against the underlying connection while holding the lock.
    ///
    /// Intended for ad-hoc queries (counts, reports, maintenance) that the
    /// typed repositories do not cover.
    pub fn with_connection<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self.lock();
        f(&conn)
    }

    /// Lock the connection for the current operation.
    ///
    /// A poisoned mutex is recovered rather than panicking: the SQLite
    /// connection itself stays valid even if a previous holder panicked.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Connection> {
        match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn is_memory(path: &Path) -> bool {
    path == Path::new(":memory:")
}

fn init_connection(conn: &mut Connection) -> Result<()> {
    // WAL returns the effective mode as a row ("wal" on disk, "memory" for
    // in-memory databases), so it is read through query_row.
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    // Enforce the FK constraints declared in the schema (none return rows).
    conn.execute("PRAGMA foreign_keys = ON", [])?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn open_memory_applies_pragmas_and_runs_migrations() {
        let store = Store::open(":memory:").expect("open in-memory store");

        let foreign_keys: i64 = store
            .with_connection(|conn| Ok(conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?))
            .expect("read foreign_keys pragma");
        assert_eq!(foreign_keys, 1);

        let version = store
            .with_connection(crate::migrations::current_version)
            .expect("read schema version");
        assert_eq!(version, crate::migrations::LATEST_VERSION);
    }

    #[test]
    fn file_database_uses_wal_and_creates_parent_dirs() {
        let dir = std::env::temp_dir().join(format!("termius-storage-{}", Uuid::new_v4()));
        let path = dir.join("nested").join(DB_FILE_NAME);

        let store = Store::open(&path).expect("open file-backed store");
        assert!(path.exists(), "database file should have been created");

        let mode: String = store
            .with_connection(|conn| Ok(conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?))
            .expect("read journal_mode");
        assert_eq!(mode.to_lowercase(), "wal");

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_db_path_is_named_termius_db() {
        let path = default_db_path().expect("data directory");
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some(DB_FILE_NAME));
    }
}
