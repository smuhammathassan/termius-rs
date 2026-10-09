//! migrations — versioned schema migrations driven by SQLite `user_version`.
//!
//! Replaces `schema.js`'s implicit store shape with an explicit, forward-only
//! migration list. [`run`] reads `PRAGMA user_version`, applies every pending
//! migration inside its own transaction (rolling back atomically on failure),
//! and stamps the new version into the same transaction.

use rusqlite::Connection;

use crate::error::{Result, StorageError};

/// Schema version this build expects after [`run`] completes.
pub const LATEST_VERSION: i64 = 1;

/// `(version, sql)` pairs applied in ascending order.
pub const MIGRATIONS: &[(i64, &str)] = &[(1, MIGRATION_V1)];

/// Read the current on-disk schema version (`PRAGMA user_version`).
pub fn current_version(conn: &Connection) -> Result<i64> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(version)
}

/// Apply every migration newer than the on-disk version.
///
/// Safe to call repeatedly: already-applied versions are skipped, so opening a
/// store twice (or upgrading an existing database) is a no-op.
pub fn run(conn: &mut Connection) -> Result<()> {
    let current = current_version(conn)?;
    if current > LATEST_VERSION {
        return Err(StorageError::migration(
            current,
            format!(
                "database schema version {} is newer than this build supports ({})",
                current, LATEST_VERSION
            ),
        ));
    }

    for &(version, sql) in MIGRATIONS {
        if version <= current {
            continue;
        }
        let tx = conn
            .transaction()
            .map_err(|err| StorageError::migration_with_cause(version, err))?;
        tx.execute_batch(sql)
            .map_err(|err| StorageError::migration_with_cause(version, err))?;
        // `PRAGMA user_version = N` returns no rows, so execute() is safe here,
        // and header changes commit together with the DDL above.
        tx.execute(&format!("PRAGMA user_version = {version}"), [])
            .map_err(|err| StorageError::migration_with_cause(version, err))?;
        tx.commit()
            .map_err(|err| StorageError::migration_with_cause(version, err))?;
        tracing::info!(version, "applied storage migration");
    }
    Ok(())
}

/// v1 — the full Termius record schema.
///
/// Notes:
/// - ids are UUIDv4 TEXT primary keys (`termius-core::RecordId`);
/// - timestamps are RFC 3339 UTC strings (`termius-core::Timestamp`);
/// - embedded arrays/objects (`group_ids`, `port_forwards`, `host_ids`,
///   `entries`) live in JSON TEXT columns;
/// - FK constraints are deliberately limited to "child cannot exist without
///   parent" relationships: a port-forward without its host is meaningless
///   (CASCADE), and a group tree links within its own table (SET NULL). All
///   cross-entity bindings (`hosts.identity_id`, `identities.key_id`, …) stay
///   soft references — exactly like the original schema.js store, which had no
///   referential integrity and tolerated dangling bindings on import.
const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS "vaults" (
    "id"          TEXT PRIMARY KEY,
    "title"       TEXT NOT NULL DEFAULT '',
    "is_default"  INTEGER NOT NULL DEFAULT 0,
    "created_at"  TEXT NOT NULL DEFAULT '',
    "updated_at"  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS "keychains" (
    "id"          TEXT PRIMARY KEY,
    "title"       TEXT NOT NULL DEFAULT '',
    "entries"     TEXT NOT NULL DEFAULT '[]',
    "created_at"  TEXT NOT NULL DEFAULT '',
    "updated_at"  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS "keys" (
    "id"              TEXT PRIMARY KEY,
    "name"            TEXT NOT NULL DEFAULT '',
    "key_type"        TEXT NOT NULL,
    "private_key"     TEXT,
    "public_key"      TEXT,
    "keychain_id"     TEXT,
    "passphrase_hint" TEXT,
    "created_at"      TEXT NOT NULL DEFAULT '',
    "updated_at"      TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS "identities" (
    "id"                         TEXT PRIMARY KEY,
    "name"                       TEXT NOT NULL DEFAULT '',
    "username"                   TEXT NOT NULL DEFAULT '',
    "password"                   TEXT,
    "keychain_id"                TEXT,
    "key_id"                     TEXT,
    "key_passphrase_keychain_id" TEXT,
    "created_at"                 TEXT NOT NULL DEFAULT '',
    "updated_at"                 TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS "known_hosts" (
    "id"          TEXT PRIMARY KEY,
    "host"        TEXT NOT NULL DEFAULT '',
    "port"        INTEGER NOT NULL DEFAULT 22,
    "public_key"  TEXT NOT NULL DEFAULT '',
    "created_at"  TEXT NOT NULL DEFAULT '',
    "updated_at"  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS "groups" (
    "id"          TEXT PRIMARY KEY,
    "title"       TEXT NOT NULL DEFAULT '',
    "parent_id"   TEXT REFERENCES "groups"("id") ON DELETE SET NULL
                  DEFERRABLE INITIALLY DEFERRED,
    "host_ids"    TEXT NOT NULL DEFAULT '[]',
    "sort_order"  INTEGER NOT NULL DEFAULT 0,
    "created_at"  TEXT NOT NULL DEFAULT '',
    "updated_at"  TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS "idx_groups_parent_id" ON "groups"("parent_id");

CREATE TABLE IF NOT EXISTS "snippets" (
    "id"          TEXT PRIMARY KEY,
    "title"       TEXT NOT NULL DEFAULT '',
    "body"        TEXT NOT NULL DEFAULT '',
    "host_ids"    TEXT NOT NULL DEFAULT '[]',
    "command"     TEXT,
    "sort_order"  INTEGER NOT NULL DEFAULT 0,
    "created_at"  TEXT NOT NULL DEFAULT '',
    "updated_at"  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS "hosts" (
    "id"              TEXT PRIMARY KEY,
    "label"           TEXT NOT NULL DEFAULT '',
    "hostname"        TEXT NOT NULL DEFAULT '',
    "port"            INTEGER NOT NULL DEFAULT 22,
    "username"        TEXT NOT NULL DEFAULT '',
    "type"            TEXT NOT NULL,
    "group_ids"       TEXT NOT NULL DEFAULT '[]',
    "identity_id"     TEXT,
    "key_id"          TEXT,
    "keychain_id"     TEXT,
    "port_forwards"   TEXT NOT NULL DEFAULT '[]',
    "snippet_ids"     TEXT NOT NULL DEFAULT '[]',
    "known_host_id"   TEXT,
    "created_at"      TEXT NOT NULL DEFAULT '',
    "updated_at"      TEXT NOT NULL DEFAULT '',
    "notes"           TEXT
);

CREATE INDEX IF NOT EXISTS "idx_hosts_hostname" ON "hosts"("hostname");
CREATE INDEX IF NOT EXISTS "idx_hosts_identity_id" ON "hosts"("identity_id");

CREATE TABLE IF NOT EXISTS "port_forwards" (
    "id"                TEXT PRIMARY KEY,
    "type"              TEXT NOT NULL,
    "listen_interface"  TEXT NOT NULL DEFAULT '127.0.0.1',
    "listen_port"       INTEGER NOT NULL DEFAULT 0,
    "destination_host"  TEXT,
    "destination_port"  INTEGER,
    "host_id"           TEXT REFERENCES "hosts"("id") ON DELETE CASCADE
                        DEFERRABLE INITIALLY DEFERRED,
    "bind_all"          INTEGER NOT NULL DEFAULT 0,
    "created_at"        TEXT NOT NULL DEFAULT '',
    "updated_at"        TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS "idx_port_forwards_host_id" ON "port_forwards"("host_id");

-- localStorage replacement: flat string key/value pairs (deviceToken,
-- feature flags, migration logs — recovered main/background read these via
-- localStorage.getItem/setItem).
CREATE TABLE IF NOT EXISTS "kv_store" (
    "key"   TEXT PRIMARY KEY,
    "value" TEXT NOT NULL DEFAULT ''
);
"#;

#[cfg(test)]
mod tests {
    use rusqlite::{params, Connection};

    use super::*;

    fn in_memory() -> Connection {
        Connection::open_in_memory().expect("in-memory connection")
    }

    #[test]
    fn fresh_migrate_creates_all_tables_and_stamps_version() {
        let mut conn = in_memory();
        run(&mut conn).expect("initial migration");

        assert_eq!(current_version(&conn).expect("version"), LATEST_VERSION);

        let tables = [
            "hosts",
            "identities",
            "keys",
            "keychains",
            "known_hosts",
            "snippets",
            "groups",
            "port_forwards",
            "vaults",
            "kv_store",
        ];
        for table in tables {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .expect("sqlite_master query");
            assert_eq!(count, 1, "table `{table}` was not created");
        }
    }

    #[test]
    fn rerunning_migrations_is_a_no_op() {
        let mut conn = in_memory();
        run(&mut conn).expect("first run");
        let version = current_version(&conn).expect("version");
        run(&mut conn).expect("second run");
        assert_eq!(current_version(&conn).expect("version"), version);
    }

    #[test]
    fn rejects_a_newer_on_disk_schema() {
        let mut conn = in_memory();
        conn.execute("PRAGMA user_version = 99", [])
            .expect("bump user_version");
        let err = run(&mut conn).expect_err("newer schema must be rejected");
        assert!(matches!(err, StorageError::Migration { .. }));
    }
}
