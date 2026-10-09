//! repositories — typed CRUD over the SQLite schema for every `termius-core`
//! domain record: `Host`, `Identity`, `Key`, `Keychain`, `KnownHost`,
//! `Snippet`, `Group`, `PortForwardingConfig`, and `Vault`.
//!
//! Marshalling rules (uniform across all nine tables):
//! - scalar fields map to typed columns (`TEXT` / `INTEGER`);
//! - embedded arrays and objects (`group_ids`, `port_forwards`, `host_ids`,
//!   `entries`) are stored as JSON `TEXT` via serde — mirroring the original
//!   `schema.js`, which persisted serialized documents;
//! - enums (`hosts.type`, `keys.key_type`, `port_forwards.type`) are stored as
//!   their serde JSON string form (e.g. `"ssh"`) so round-trips are exact;
//! - `u16` ports go through `i64` columns and are range-checked on read.
//!
//! Each repository is a thin, uniform wrapper over [`Store`]'s generic
//! [`Storable`] helpers, so every record exposes the same five operations:
//! `create`, `get_by_id`, `list`, `update`, `delete`.
//!
//! Secrets note: `identities.password`, `keys.private_key`, and
//! `keychains.entries` are persisted verbatim so records round-trip exactly as
//! the original app stored them in its (encrypted) schema blob. Prefer routing
//! new secrets through [`crate::keychain`], which stores them in the OS
//! keychain instead of SQLite.

use rusqlite::{named_params, params, Connection, Row};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use uuid::Uuid;

use termius_core::{
    Group, Host, Identity, Key, Keychain, KnownHost, PortForwardingConfig, Snippet, Vault,
};

use crate::db::Store;
use crate::error::{Result, StorageError};

/// A `termius-core` record that maps to exactly one row in [`Self::TABLE`].
pub trait Storable: Serialize + DeserializeOwned + Sized {
    /// Backing table name (unquoted; quoted at use sites).
    const TABLE: &'static str;
    /// Ordering applied by `list` — the `ORDER BY` expression, no prefix.
    const DEFAULT_ORDER: &'static str = "created_at ASC, id ASC";

    /// The record's primary key.
    fn id(&self) -> &str;
    /// INSERT the full record.
    fn insert(&self, conn: &Connection) -> rusqlite::Result<()>;
    /// UPDATE every column of the record matched by primary key; returns the
    /// number of changed rows (`0` when the id does not exist).
    fn update_by_id(&self, conn: &Connection) -> rusqlite::Result<usize>;
    /// Map a full row (any column order) back into the record.
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self>;
}

/// Wrap a serde failure as a `rusqlite` error so row mapping can stay in
/// `rusqlite::Result`; it is unwrapped back into a `StorageError` downstream.
fn json_err(err: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(err))
}

fn to_json_text<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value).map_err(json_err)
}

fn port_from_sql(raw: i64) -> rusqlite::Result<u16> {
    u16::try_from(raw).map_err(|_| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(StorageError::InvalidValue {
            field: "port",
            message: format!("{raw} does not fit in a u16 port"),
        }))
    })
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn json_str<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

/// Fill empty `id` / `createdAt` / `updatedAt` on a record about to be created.
fn with_created_defaults<T: Storable>(record: &T) -> Result<T> {
    let mut value = serde_json::to_value(record)?;
    let obj = value.as_object_mut().ok_or_else(|| StorageError::InvalidValue {
        field: "record",
        message: "record did not serialize to a JSON object".to_owned(),
    })?;
    if json_str(obj, "id").is_none_or(str::is_empty) {
        obj.insert("id".to_owned(), Value::String(Uuid::new_v4().to_string()));
    }
    let stamp = now();
    for field in ["createdAt", "updatedAt"] {
        if json_str(obj, field).is_none_or(str::is_empty) {
            obj.insert(field.to_owned(), Value::String(stamp.clone()));
        }
    }
    Ok(serde_json::from_value(value)?)
}

/// Fill an empty `updatedAt` on a record about to be updated; callers own the
/// timestamp otherwise (sync imports may set an exact value).
fn with_updated_defaults<T: Storable>(record: &T) -> Result<T> {
    let mut value = serde_json::to_value(record)?;
    let obj = value.as_object_mut().ok_or_else(|| StorageError::InvalidValue {
        field: "record",
        message: "record did not serialize to a JSON object".to_owned(),
    })?;
    if json_str(obj, "updatedAt").is_none_or(str::is_empty) {
        obj.insert("updatedAt".to_owned(), Value::String(now()));
    }
    Ok(serde_json::from_value(value)?)
}

impl Store {
    /// Insert a new record. Empty `id` / `createdAt` / `updatedAt` are filled
    /// with a fresh UUIDv4 and an RFC 3339 UTC timestamp. Returns the record as
    /// stored.
    pub fn create_record<T: Storable>(&self, record: &T) -> Result<T> {
        let record = with_created_defaults(record)?;
        let conn = self.lock();
        record.insert(&conn)?;
        tracing::debug!(table = T::TABLE, id = record.id(), "record created");
        Ok(record)
    }

    /// Fetch a record by id, or [`StorageError::NotFound`].
    pub fn get_record<T: Storable>(&self, id: &str) -> Result<T> {
        let conn = self.lock();
        let sql = format!("SELECT * FROM \"{}\" WHERE id = ?1", T::TABLE);
        conn.query_row(&sql, params![id], T::from_row)
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound {
                    table: T::TABLE,
                    id: id.to_owned(),
                },
                other => other.into(),
            })
    }

    /// List every record of this type in [`Storable::DEFAULT_ORDER`].
    pub fn list_records<T: Storable>(&self) -> Result<Vec<T>> {
        let conn = self.lock();
        let sql = format!("SELECT * FROM \"{}\" ORDER BY {}", T::TABLE, T::DEFAULT_ORDER);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], T::from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    /// Replace the stored row matched by `record.id()`. An empty `updatedAt` is
    /// refreshed to now. Fails with [`StorageError::NotFound`] when the id is
    /// unknown. Returns the record as stored.
    pub fn update_record<T: Storable>(&self, record: &T) -> Result<T> {
        let record = with_updated_defaults(record)?;
        let conn = self.lock();
        let changed = record.update_by_id(&conn)?;
        if changed == 0 {
            return Err(StorageError::NotFound {
                table: T::TABLE,
                id: record.id().to_owned(),
            });
        }
        tracing::debug!(table = T::TABLE, id = record.id(), "record updated");
        Ok(record)
    }

    /// Delete a record by id; `false` when nothing matched.
    pub fn delete_record<T: Storable>(&self, id: &str) -> Result<bool> {
        let conn = self.lock();
        let sql = format!("DELETE FROM \"{}\" WHERE id = ?1", T::TABLE);
        let changed = conn.execute(&sql, params![id])?;
        if changed > 0 {
            tracing::debug!(table = T::TABLE, id, "record deleted");
        }
        Ok(changed > 0)
    }
}

/// Generate a repository with the uniform five-operation CRUD surface.
macro_rules! define_repository {
    ($($(#[$meta:meta])* $name:ident, $record:ty);+ $(;)?) => {
        $(
            $(#[$meta])*
            #[derive(Debug, Clone, Copy)]
            pub struct $name<'a> {
                store: &'a Store,
            }

            impl<'a> $name<'a> {
                /// Wrap `store` for CRUD access.
                pub fn new(store: &'a Store) -> Self {
                    Self { store }
                }

                /// Insert a new record (see [`Store::create_record`]).
                pub fn create(&self, record: &$record) -> Result<$record> {
                    self.store.create_record(record)
                }

                /// Fetch by id, or [`StorageError::NotFound`].
                pub fn get_by_id(&self, id: &str) -> Result<$record> {
                    self.store.get_record(id)
                }

                /// List every record in default order.
                pub fn list(&self) -> Result<Vec<$record>> {
                    self.store.list_records()
                }

                /// Replace the record matched by `record.id()`.
                pub fn update(&self, record: &$record) -> Result<$record> {
                    self.store.update_record(record)
                }

                /// Delete by id; `false` when nothing matched.
                pub fn delete(&self, id: &str) -> Result<bool> {
                    self.store.delete_record(id)
                }
            }
        )+
    };
}

define_repository! {
    /// CRUD for [`Host`] records (`hosts` table).
    HostRepository, Host;
    /// CRUD for [`Identity`] records (`identities` table).
    IdentityRepository, Identity;
    /// CRUD for [`Key`] records (`keys` table).
    KeyRepository, Key;
    /// CRUD for [`Keychain`] records (`keychains` table).
    KeychainRepository, Keychain;
    /// CRUD for [`KnownHost`] records (`known_hosts` table).
    KnownHostRepository, KnownHost;
    /// CRUD for [`Snippet`] records (`snippets` table).
    SnippetRepository, Snippet;
    /// CRUD for [`Group`] records (`groups` table).
    GroupRepository, Group;
    /// CRUD for [`PortForwardingConfig`] records (`port_forwards` table).
    PortForwardingRepository, PortForwardingConfig;
    /// CRUD for [`Vault`] records (`vaults` table).
    VaultRepository, Vault;
}

impl HostRepository<'_> {
    /// Hosts whose `groupIds` JSON array contains `group_id`.
    pub fn list_in_group(&self, group_id: &str) -> Result<Vec<Host>> {
        let conn = self.store.lock();
        let mut stmt = conn.prepare(
            "SELECT * FROM \"hosts\" AS h
             WHERE EXISTS (
                 SELECT 1 FROM json_each(h.\"group_ids\") AS g WHERE g.value = ?1
             )
             ORDER BY h.created_at ASC, h.id ASC",
        )?;
        let rows = stmt.query_map(params![group_id], Host::from_row)?;
        let mut hosts = Vec::new();
        for row in rows {
            hosts.push(row?);
        }
        Ok(hosts)
    }
}

impl Store {
    /// [`HostRepository`] bound to this store.
    pub fn hosts(&self) -> HostRepository<'_> {
        HostRepository::new(self)
    }
    /// [`IdentityRepository`] bound to this store.
    pub fn identities(&self) -> IdentityRepository<'_> {
        IdentityRepository::new(self)
    }
    /// [`KeyRepository`] bound to this store.
    pub fn keys(&self) -> KeyRepository<'_> {
        KeyRepository::new(self)
    }
    /// [`KeychainRepository`] bound to this store.
    pub fn keychains(&self) -> KeychainRepository<'_> {
        KeychainRepository::new(self)
    }
    /// [`KnownHostRepository`] bound to this store.
    pub fn known_hosts(&self) -> KnownHostRepository<'_> {
        KnownHostRepository::new(self)
    }
    /// [`SnippetRepository`] bound to this store.
    pub fn snippets(&self) -> SnippetRepository<'_> {
        SnippetRepository::new(self)
    }
    /// [`GroupRepository`] bound to this store.
    pub fn groups(&self) -> GroupRepository<'_> {
        GroupRepository::new(self)
    }
    /// [`PortForwardingRepository`] bound to this store.
    pub fn port_forwards(&self) -> PortForwardingRepository<'_> {
        PortForwardingRepository::new(self)
    }
    /// [`VaultRepository`] bound to this store.
    pub fn vaults(&self) -> VaultRepository<'_> {
        VaultRepository::new(self)
    }
    /// [`SettingsRepository`] bound to this store.
    pub fn settings(&self) -> SettingsRepository<'_> {
        SettingsRepository::new(self)
    }
}

/// `localStorage.getItem("deviceToken")` key (recovered `background/entry.js`
/// `_getDeviceToken`, `recovered-ui/entry.js`).
pub const DEVICE_TOKEN_KEY: &str = "deviceToken";

/// Flat string key/value settings — the port's `localStorage` replacement for
/// `deviceToken`, feature flags, and one-off migration markers. Unlike the
/// record repositories this is an upsert surface (`set`), not five-op CRUD.
#[derive(Debug, Clone, Copy)]
pub struct SettingsRepository<'a> {
    store: &'a Store,
}

impl<'a> SettingsRepository<'a> {
    /// Wrap `store` for settings access.
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Read a key; `Ok(None)` when it was never set.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.store.lock();
        let sql = "SELECT \"value\" FROM \"kv_store\" WHERE \"key\" = ?1";
        match conn.query_row(sql, params![key], |row| row.get(0)) {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Write a key (insert or overwrite).
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.store.lock();
        conn.execute(
            "INSERT INTO \"kv_store\" (\"key\", \"value\") VALUES (?1, ?2)
             ON CONFLICT(\"key\") DO UPDATE SET \"value\" = excluded.\"value\"",
            params![key, value],
        )?;
        tracing::debug!(key, "setting saved");
        Ok(())
    }

    /// Remove a key; `false` when it did not exist.
    pub fn delete(&self, key: &str) -> Result<bool> {
        let conn = self.store.lock();
        let sql = "DELETE FROM \"kv_store\" WHERE \"key\" = ?1";
        Ok(conn.execute(sql, params![key])? > 0)
    }

    /// The device token identifying this install to the sync API: returns the
    /// persisted token, or generates and stores one on first use — the exact
    /// behavior of the original `_getDeviceToken`.
    /// PORT-TODO: the original generator (`qa()`) may not be UUIDv4; swap the
    /// format if the recovered implementation says otherwise.
    pub fn device_token(&self) -> Result<String> {
        if let Some(token) = self.get(DEVICE_TOKEN_KEY)? {
            return Ok(token);
        }
        let token = Uuid::new_v4().to_string();
        self.set(DEVICE_TOKEN_KEY, &token)?;
        Ok(token)
    }
}

// ---------------------------------------------------------------------------
// Storable implementations — one per table.
// ---------------------------------------------------------------------------

impl Storable for Host {
    const TABLE: &'static str = "hosts";

    fn id(&self) -> &str {
        &self.id
    }

    fn insert(&self, conn: &Connection) -> rusqlite::Result<()> {
        let host_type = to_json_text(&self.host_type)?;
        let group_ids = to_json_text(&self.group_ids)?;
        let port_forwards = to_json_text(&self.port_forwardings)?;
        let snippet_ids = to_json_text(&self.snippet_ids)?;
        conn.execute(
            "INSERT INTO \"hosts\" (id, label, hostname, port, username, type, group_ids,
                 identity_id, key_id, keychain_id, port_forwards, snippet_ids, known_host_id,
                 created_at, updated_at, notes)
             VALUES (:id, :label, :hostname, :port, :username, :type, :group_ids,
                 :identity_id, :key_id, :keychain_id, :port_forwards, :snippet_ids,
                 :known_host_id, :created_at, :updated_at, :notes)",
            named_params! {
                ":id": self.id,
                ":label": self.label,
                ":hostname": self.hostname,
                ":port": self.port as i64,
                ":username": self.username,
                ":type": host_type,
                ":group_ids": group_ids,
                ":identity_id": self.identity_id,
                ":key_id": self.key_id,
                ":keychain_id": self.keychain_id,
                ":port_forwards": port_forwards,
                ":snippet_ids": snippet_ids,
                ":known_host_id": self.known_host_id,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at,
                ":notes": self.notes
            },
        )?;
        Ok(())
    }

    fn update_by_id(&self, conn: &Connection) -> rusqlite::Result<usize> {
        let host_type = to_json_text(&self.host_type)?;
        let group_ids = to_json_text(&self.group_ids)?;
        let port_forwards = to_json_text(&self.port_forwardings)?;
        let snippet_ids = to_json_text(&self.snippet_ids)?;
        conn.execute(
            "UPDATE \"hosts\" SET label = :label, hostname = :hostname, port = :port,
                 username = :username, type = :type, group_ids = :group_ids,
                 identity_id = :identity_id, key_id = :key_id, keychain_id = :keychain_id,
                 port_forwards = :port_forwards, snippet_ids = :snippet_ids,
                 known_host_id = :known_host_id, created_at = :created_at,
                 updated_at = :updated_at, notes = :notes
             WHERE id = :id",
            named_params! {
                ":id": self.id,
                ":label": self.label,
                ":hostname": self.hostname,
                ":port": self.port as i64,
                ":username": self.username,
                ":type": host_type,
                ":group_ids": group_ids,
                ":identity_id": self.identity_id,
                ":key_id": self.key_id,
                ":keychain_id": self.keychain_id,
                ":port_forwards": port_forwards,
                ":snippet_ids": snippet_ids,
                ":known_host_id": self.known_host_id,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at,
                ":notes": self.notes
            },
        )
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let host_type: String = row.get("type")?;
        let group_ids: String = row.get("group_ids")?;
        let port_forwards: String = row.get("port_forwards")?;
        let snippet_ids: String = row.get("snippet_ids")?;
        Ok(Host {
            id: row.get("id")?,
            label: row.get("label")?,
            hostname: row.get("hostname")?,
            port: port_from_sql(row.get("port")?)?,
            username: row.get("username")?,
            host_type: serde_json::from_str(&host_type).map_err(json_err)?,
            group_ids: serde_json::from_str(&group_ids).map_err(json_err)?,
            identity_id: row.get("identity_id")?,
            key_id: row.get("key_id")?,
            keychain_id: row.get("keychain_id")?,
            port_forwardings: serde_json::from_str(&port_forwards).map_err(json_err)?,
            snippet_ids: serde_json::from_str(&snippet_ids).map_err(json_err)?,
            known_host_id: row.get("known_host_id")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            notes: row.get("notes")?,
        })
    }
}

impl Storable for Identity {
    const TABLE: &'static str = "identities";

    fn id(&self) -> &str {
        &self.id
    }

    fn insert(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO \"identities\" (id, name, username, password, keychain_id, key_id,
                 key_passphrase_keychain_id, created_at, updated_at)
             VALUES (:id, :name, :username, :password, :keychain_id, :key_id,
                 :key_passphrase_keychain_id, :created_at, :updated_at)",
            named_params! {
                ":id": self.id,
                ":name": self.name,
                ":username": self.username,
                ":password": self.password,
                ":keychain_id": self.keychain_id,
                ":key_id": self.key_id,
                ":key_passphrase_keychain_id": self.key_passphrase_keychain_id,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )?;
        Ok(())
    }

    fn update_by_id(&self, conn: &Connection) -> rusqlite::Result<usize> {
        conn.execute(
            "UPDATE \"identities\" SET name = :name, username = :username, password = :password,
                 keychain_id = :keychain_id, key_id = :key_id,
                 key_passphrase_keychain_id = :key_passphrase_keychain_id,
                 created_at = :created_at, updated_at = :updated_at
             WHERE id = :id",
            named_params! {
                ":id": self.id,
                ":name": self.name,
                ":username": self.username,
                ":password": self.password,
                ":keychain_id": self.keychain_id,
                ":key_id": self.key_id,
                ":key_passphrase_keychain_id": self.key_passphrase_keychain_id,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Identity {
            id: row.get("id")?,
            name: row.get("name")?,
            username: row.get("username")?,
            password: row.get("password")?,
            keychain_id: row.get("keychain_id")?,
            key_id: row.get("key_id")?,
            key_passphrase_keychain_id: row.get("key_passphrase_keychain_id")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl Storable for Key {
    const TABLE: &'static str = "keys";

    fn id(&self) -> &str {
        &self.id
    }

    fn insert(&self, conn: &Connection) -> rusqlite::Result<()> {
        let key_type = to_json_text(&self.key_type)?;
        conn.execute(
            "INSERT INTO \"keys\" (id, name, key_type, private_key, public_key, keychain_id,
                 passphrase_hint, created_at, updated_at)
             VALUES (:id, :name, :key_type, :private_key, :public_key, :keychain_id,
                 :passphrase_hint, :created_at, :updated_at)",
            named_params! {
                ":id": self.id,
                ":name": self.name,
                ":key_type": key_type,
                ":private_key": self.private_key,
                ":public_key": self.public_key,
                ":keychain_id": self.keychain_id,
                ":passphrase_hint": self.passphrase_hint,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )?;
        Ok(())
    }

    fn update_by_id(&self, conn: &Connection) -> rusqlite::Result<usize> {
        let key_type = to_json_text(&self.key_type)?;
        conn.execute(
            "UPDATE \"keys\" SET name = :name, key_type = :key_type, private_key = :private_key,
                 public_key = :public_key, keychain_id = :keychain_id,
                 passphrase_hint = :passphrase_hint, created_at = :created_at,
                 updated_at = :updated_at
             WHERE id = :id",
            named_params! {
                ":id": self.id,
                ":name": self.name,
                ":key_type": key_type,
                ":private_key": self.private_key,
                ":public_key": self.public_key,
                ":keychain_id": self.keychain_id,
                ":passphrase_hint": self.passphrase_hint,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let key_type: String = row.get("key_type")?;
        Ok(Key {
            id: row.get("id")?,
            name: row.get("name")?,
            key_type: serde_json::from_str(&key_type).map_err(json_err)?,
            private_key: row.get("private_key")?,
            public_key: row.get("public_key")?,
            keychain_id: row.get("keychain_id")?,
            passphrase_hint: row.get("passphrase_hint")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl Storable for Keychain {
    const TABLE: &'static str = "keychains";

    fn id(&self) -> &str {
        &self.id
    }

    fn insert(&self, conn: &Connection) -> rusqlite::Result<()> {
        let entries = to_json_text(&self.entries)?;
        conn.execute(
            "INSERT INTO \"keychains\" (id, title, entries, created_at, updated_at)
             VALUES (:id, :title, :entries, :created_at, :updated_at)",
            named_params! {
                ":id": self.id,
                ":title": self.title,
                ":entries": entries,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )?;
        Ok(())
    }

    fn update_by_id(&self, conn: &Connection) -> rusqlite::Result<usize> {
        let entries = to_json_text(&self.entries)?;
        conn.execute(
            "UPDATE \"keychains\" SET title = :title, entries = :entries,
                 created_at = :created_at, updated_at = :updated_at
             WHERE id = :id",
            named_params! {
                ":id": self.id,
                ":title": self.title,
                ":entries": entries,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let entries: String = row.get("entries")?;
        Ok(Keychain {
            id: row.get("id")?,
            title: row.get("title")?,
            entries: serde_json::from_str(&entries).map_err(json_err)?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl Storable for KnownHost {
    const TABLE: &'static str = "known_hosts";

    fn id(&self) -> &str {
        &self.id
    }

    fn insert(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO \"known_hosts\" (id, host, port, public_key, created_at, updated_at)
             VALUES (:id, :host, :port, :public_key, :created_at, :updated_at)",
            named_params! {
                ":id": self.id,
                ":host": self.host,
                ":port": self.port as i64,
                ":public_key": self.public_key,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )?;
        Ok(())
    }

    fn update_by_id(&self, conn: &Connection) -> rusqlite::Result<usize> {
        conn.execute(
            "UPDATE \"known_hosts\" SET host = :host, port = :port, public_key = :public_key,
                 created_at = :created_at, updated_at = :updated_at
             WHERE id = :id",
            named_params! {
                ":id": self.id,
                ":host": self.host,
                ":port": self.port as i64,
                ":public_key": self.public_key,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(KnownHost {
            id: row.get("id")?,
            host: row.get("host")?,
            port: port_from_sql(row.get("port")?)?,
            public_key: row.get("public_key")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl Storable for Snippet {
    const TABLE: &'static str = "snippets";
    const DEFAULT_ORDER: &'static str = "sort_order ASC, created_at ASC, id ASC";

    fn id(&self) -> &str {
        &self.id
    }

    fn insert(&self, conn: &Connection) -> rusqlite::Result<()> {
        let host_ids = to_json_text(&self.host_ids)?;
        conn.execute(
            "INSERT INTO \"snippets\" (id, title, body, host_ids, command, sort_order,
                 created_at, updated_at)
             VALUES (:id, :title, :body, :host_ids, :command, :sort_order,
                 :created_at, :updated_at)",
            named_params! {
                ":id": self.id,
                ":title": self.title,
                ":body": self.body,
                ":host_ids": host_ids,
                ":command": self.command,
                ":sort_order": self.sort_order,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )?;
        Ok(())
    }

    fn update_by_id(&self, conn: &Connection) -> rusqlite::Result<usize> {
        let host_ids = to_json_text(&self.host_ids)?;
        conn.execute(
            "UPDATE \"snippets\" SET title = :title, body = :body, host_ids = :host_ids,
                 command = :command, sort_order = :sort_order, created_at = :created_at,
                 updated_at = :updated_at
             WHERE id = :id",
            named_params! {
                ":id": self.id,
                ":title": self.title,
                ":body": self.body,
                ":host_ids": host_ids,
                ":command": self.command,
                ":sort_order": self.sort_order,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let host_ids: String = row.get("host_ids")?;
        Ok(Snippet {
            id: row.get("id")?,
            title: row.get("title")?,
            body: row.get("body")?,
            host_ids: serde_json::from_str(&host_ids).map_err(json_err)?,
            command: row.get("command")?,
            sort_order: row.get("sort_order")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl Storable for Group {
    const TABLE: &'static str = "groups";
    const DEFAULT_ORDER: &'static str = "sort_order ASC, created_at ASC, id ASC";

    fn id(&self) -> &str {
        &self.id
    }

    fn insert(&self, conn: &Connection) -> rusqlite::Result<()> {
        let host_ids = to_json_text(&self.host_ids)?;
        conn.execute(
            "INSERT INTO \"groups\" (id, title, parent_id, host_ids, sort_order,
                 created_at, updated_at)
             VALUES (:id, :title, :parent_id, :host_ids, :sort_order,
                 :created_at, :updated_at)",
            named_params! {
                ":id": self.id,
                ":title": self.title,
                ":parent_id": self.parent_id,
                ":host_ids": host_ids,
                ":sort_order": self.sort_order,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )?;
        Ok(())
    }

    fn update_by_id(&self, conn: &Connection) -> rusqlite::Result<usize> {
        let host_ids = to_json_text(&self.host_ids)?;
        conn.execute(
            "UPDATE \"groups\" SET title = :title, parent_id = :parent_id,
                 host_ids = :host_ids, sort_order = :sort_order, created_at = :created_at,
                 updated_at = :updated_at
             WHERE id = :id",
            named_params! {
                ":id": self.id,
                ":title": self.title,
                ":parent_id": self.parent_id,
                ":host_ids": host_ids,
                ":sort_order": self.sort_order,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let host_ids: String = row.get("host_ids")?;
        Ok(Group {
            id: row.get("id")?,
            title: row.get("title")?,
            parent_id: row.get("parent_id")?,
            host_ids: serde_json::from_str(&host_ids).map_err(json_err)?,
            sort_order: row.get("sort_order")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl Storable for PortForwardingConfig {
    const TABLE: &'static str = "port_forwards";

    fn id(&self) -> &str {
        &self.id
    }

    fn insert(&self, conn: &Connection) -> rusqlite::Result<()> {
        let forward_type = to_json_text(&self.forward_type)?;
        conn.execute(
            "INSERT INTO \"port_forwards\" (id, type, listen_interface, listen_port,
                 destination_host, destination_port, host_id, bind_all, created_at, updated_at)
             VALUES (:id, :type, :listen_interface, :listen_port,
                 :destination_host, :destination_port, :host_id, :bind_all,
                 :created_at, :updated_at)",
            named_params! {
                ":id": self.id,
                ":type": forward_type,
                ":listen_interface": self.listen_interface,
                ":listen_port": self.listen_port as i64,
                ":destination_host": self.destination_host,
                ":destination_port": self.destination_port.map(i64::from),
                ":host_id": self.host_id,
                ":bind_all": self.bind_all as i64,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )?;
        Ok(())
    }

    fn update_by_id(&self, conn: &Connection) -> rusqlite::Result<usize> {
        let forward_type = to_json_text(&self.forward_type)?;
        conn.execute(
            "UPDATE \"port_forwards\" SET type = :type, listen_interface = :listen_interface,
                 listen_port = :listen_port, destination_host = :destination_host,
                 destination_port = :destination_port, host_id = :host_id, bind_all = :bind_all,
                 created_at = :created_at, updated_at = :updated_at
             WHERE id = :id",
            named_params! {
                ":id": self.id,
                ":type": forward_type,
                ":listen_interface": self.listen_interface,
                ":listen_port": self.listen_port as i64,
                ":destination_host": self.destination_host,
                ":destination_port": self.destination_port.map(i64::from),
                ":host_id": self.host_id,
                ":bind_all": self.bind_all as i64,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let forward_type: String = row.get("type")?;
        let destination_port: Option<i64> = row.get("destination_port")?;
        Ok(PortForwardingConfig {
            id: row.get("id")?,
            forward_type: serde_json::from_str(&forward_type).map_err(json_err)?,
            listen_interface: row.get("listen_interface")?,
            listen_port: port_from_sql(row.get("listen_port")?)?,
            destination_host: row.get("destination_host")?,
            destination_port: match destination_port {
                Some(raw) => Some(port_from_sql(raw)?),
                None => None,
            },
            host_id: row.get("host_id")?,
            bind_all: row.get("bind_all")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl Storable for Vault {
    const TABLE: &'static str = "vaults";

    fn id(&self) -> &str {
        &self.id
    }

    fn insert(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO \"vaults\" (id, title, is_default, created_at, updated_at)
             VALUES (:id, :title, :is_default, :created_at, :updated_at)",
            named_params! {
                ":id": self.id,
                ":title": self.title,
                ":is_default": self.is_default as i64,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )?;
        Ok(())
    }

    fn update_by_id(&self, conn: &Connection) -> rusqlite::Result<usize> {
        conn.execute(
            "UPDATE \"vaults\" SET title = :title, is_default = :is_default,
                 created_at = :created_at, updated_at = :updated_at
             WHERE id = :id",
            named_params! {
                ":id": self.id,
                ":title": self.title,
                ":is_default": self.is_default as i64,
                ":created_at": self.created_at,
                ":updated_at": self.updated_at
            },
        )
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Vault {
            id: row.get("id")?,
            title: row.get("title")?,
            is_default: row.get("is_default")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use termius_core::host::HostType;
    use termius_core::{ForwardType, KeyType, PortForwardingConfig, Snippet};

    use super::*;

    fn store() -> Store {
        Store::open(":memory:").expect("in-memory store")
    }

    #[test]
    fn host_round_trip_covers_create_get_list_update_delete() {
        let store = store();

        let mut host = Host::default();
        host.label = "web-1".to_owned();
        host.hostname = "10.0.0.1".to_owned();
        host.username = "root".to_owned();
        host.host_type = HostType::Ssh;
        host.group_ids = vec!["g1".to_owned(), "g2".to_owned()];
        host.snippet_ids = vec!["s1".to_owned()];
        host.notes = Some("production".to_owned());
        host.port_forwardings = vec![PortForwardingConfig {
            forward_type: ForwardType::Dynamic,
            listen_port: 1080,
            ..PortForwardingConfig::default()
        }];

        let created = store.hosts().create(&host).expect("create host");
        assert!(!created.id.is_empty(), "id must be auto-assigned");
        assert!(!created.created_at.is_empty(), "timestamps must be auto-filled");
        assert_eq!(created.updated_at, created.created_at);

        let fetched = store.hosts().get_by_id(&created.id).expect("get host");
        assert_eq!(fetched, created, "round-trip must be lossless");

        assert_eq!(store.hosts().list().expect("list hosts").len(), 1);

        let mut to_update = fetched.clone();
        to_update.label = "web-1-prod".to_owned();
        to_update.updated_at = String::new();
        let updated = store.hosts().update(&to_update).expect("update host");
        assert_eq!(updated.label, "web-1-prod");
        assert!(!updated.updated_at.is_empty(), "updatedAt refreshed on update");
        assert_eq!(
            store.hosts().get_by_id(&created.id).expect("get updated").label,
            "web-1-prod"
        );

        assert!(store.hosts().delete(&created.id).expect("delete host"));
        let err = store.hosts().get_by_id(&created.id).expect_err("deleted");
        assert!(err.is_not_found());
        assert!(!store.hosts().delete(&created.id).expect("delete again"));
    }

    #[test]
    fn identity_round_trip_preserves_password_and_bindings() {
        let store = store();

        let mut identity = Identity::default();
        identity.name = "deploy".to_owned();
        identity.username = "ubuntu".to_owned();
        identity.password = Some("s3cret-password".to_owned());
        identity.keychain_id = Some("kc-1".to_owned());
        identity.key_id = Some("key-1".to_owned());

        let created = store.identities().create(&identity).expect("create identity");
        let fetched = store
            .identities()
            .get_by_id(&created.id)
            .expect("get identity");
        assert_eq!(fetched, created);
        assert_eq!(fetched.password.as_deref(), Some("s3cret-password"));
        assert_eq!(fetched.key_id.as_deref(), Some("key-1"));
        assert_eq!(store.identities().list().expect("list").len(), 1);
    }

    #[test]
    fn snippet_round_trip_preserves_json_host_ids_and_sort_order() {
        let store = store();

        let mut snippet = Snippet::default();
        snippet.title = "uptime".to_owned();
        snippet.body = "uptime".to_owned();
        snippet.host_ids = vec!["host-a".to_owned()];
        snippet.command = Some("u".to_owned());
        snippet.sort_order = 7;

        let created = store.snippets().create(&snippet).expect("create snippet");
        let fetched = store.snippets().get_by_id(&created.id).expect("get snippet");
        assert_eq!(fetched, created);
        assert_eq!(fetched.host_ids, vec!["host-a".to_owned()]);
        assert_eq!(fetched.sort_order, 7);
    }

    #[test]
    fn all_remaining_record_types_round_trip() {
        let store = store();

        let vault = store.vaults().create(&Vault::default()).expect("vault");
        assert_eq!(store.vaults().get_by_id(&vault.id).expect("get vault"), vault);

        let mut keychain = Keychain::default();
        keychain.title = "Prod vault".to_owned();
        keychain.entries = vec![termius_core::keychain::KeychainEntry {
            owner_id: "key-1".to_owned(),
            secret: "passphrase".to_owned(),
        }];
        let keychain = store.keychains().create(&keychain).expect("keychain");
        assert_eq!(
            store.keychains().get_by_id(&keychain.id).expect("get keychain"),
            keychain
        );

        let mut key = Key::default();
        key.name = "id_ed25519".to_owned();
        key.key_type = KeyType::Ed25519;
        key.private_key = Some("-----BEGIN OPENSSH PRIVATE KEY-----".to_owned());
        key.public_key = Some("ssh-ed25519 AAAA...".to_owned());
        let key = store.keys().create(&key).expect("key");
        assert_eq!(store.keys().get_by_id(&key.id).expect("get key"), key);

        let mut known = KnownHost::default();
        known.host = "example.com".to_owned();
        known.port = 2222;
        known.public_key = "ssh-ed25519 AAAA...".to_owned();
        let known = store.known_hosts().create(&known).expect("known host");
        assert_eq!(
            store.known_hosts().get_by_id(&known.id).expect("get known host"),
            known
        );

        let root = store
            .groups()
            .create(&Group::default())
            .expect("root group");
        let mut child = Group::default();
        child.title = "prod".to_owned();
        child.parent_id = Some(root.id.clone());
        child.host_ids = vec!["h1".to_owned()];
        let child = store.groups().create(&child).expect("child group");
        assert_eq!(store.groups().get_by_id(&child.id).expect("get group"), child);
    }

    #[test]
    fn deleting_a_host_cascades_its_port_forwards() {
        let store = store();

        let host = store
            .hosts()
            .create(&Host::default())
            .expect("create host");
        let mut forward = PortForwardingConfig::default();
        forward.host_id = Some(host.id.clone());
        forward.listen_port = 8080;
        let forward = store.port_forwards().create(&forward).expect("forward");
        assert_eq!(
            store.port_forwards().get_by_id(&forward.id).expect("get forward"),
            forward
        );

        assert!(store.hosts().delete(&host.id).expect("delete host"));
        assert!(
            store.port_forwards().list().expect("list forwards").is_empty(),
            "port forwards must cascade with their host"
        );
    }

    #[test]
    fn list_in_group_filters_on_the_json_group_ids_column() {
        let store = store();

        let mut in_prod = Host::default();
        in_prod.label = "prod-1".to_owned();
        in_prod.group_ids = vec!["prod".to_owned()];
        let mut in_dev = Host::default();
        in_dev.label = "dev-1".to_owned();
        in_dev.group_ids = vec!["dev".to_owned()];
        store.hosts().create(&in_prod).expect("prod host");
        store.hosts().create(&in_dev).expect("dev host");

        let prod_hosts = store.hosts().list_in_group("prod").expect("filter");
        assert_eq!(prod_hosts.len(), 1);
        assert_eq!(prod_hosts[0].label, "prod-1");

        let none = store.hosts().list_in_group("missing").expect("filter");
        assert!(none.is_empty());
    }

    #[test]
    fn update_and_delete_report_missing_records() {
        let store = store();

        let mut host = Host::default();
        host.label = "ghost".to_owned();
        let err = store.hosts().update(&host).expect_err("update must fail");
        assert!(err.is_not_found());
        assert!(!store.hosts().delete("does-not-exist").expect("delete"));
    }

    #[test]
    fn settings_behave_like_local_storage() {
        let store = store();

        assert_eq!(store.settings().get("missing").expect("get"), None);
        store.settings().set("flag", "on").expect("set");
        store.settings().set("flag", "off").expect("overwrite");
        assert_eq!(
            store.settings().get("flag").expect("get"),
            Some("off".to_owned())
        );
        assert!(store.settings().delete("flag").expect("delete"));
        assert!(!store.settings().delete("flag").expect("delete again"));
        assert_eq!(store.settings().get("flag").expect("get"), None);

        let token = store.settings().device_token().expect("device token");
        assert!(!token.is_empty());
        assert_eq!(
            store.settings().device_token().expect("device token again"),
            token,
            "deviceToken must be stable across reads"
        );
    }
}
