//! Serde wire models for the Termius cloud API.
//!
//! The backend (`https://api.termius.com`) is a Django/DRF service: payload
//! keys are `snake_case`, records travel grouped into named *sets*
//! (`host_set`, `snippet_set`, …) and are pushed/pulled through the bulk sync
//! endpoint `GET|POST /api/v4/terminal/sync/` (endpoint table recovered from
//! `analysis/recovered-background/chunk_Pt.js`, set names from
//! `chunk_sfe.js`, record field lists from the model classes in
//! `entry.js`).
//!
//! Two properties of the real wire format are modelled as pass-through rather
//! than typed fields:
//!
//! * **Relations** — a relation field is a bare id string when this client
//!   pushes and a nested object when the server pulls (see [`Relation`]).
//!   The recovered JS converts nested relation objects back to ids right
//!   before the request (`Spt`/`y[A] = H.id` in `entry.js`).
//! * **End-to-end encryption** — the recovered client runs every record
//!   through `encryptDataForBackend` before pushing and
//!   `decryptDataFromBackend` after pulling: schema *v3* encrypts the
//!   `cryptoFields` individually (for hosts: `address`, `label`,
//!   `cloud_instance_id`, `cloud_instance_type`), schema *v5* replaces the
//!   record with a single `content` envelope plus the `commonFields`
//!   (`id`, `local_id`, `updated_at`, `is_shared`, `encrypted_with`).
//!   This crate transports that envelope opaquely: unknown keys — including
//!   `content` — survive round-trips via each model's `extra` map.
//!   // PORT-TODO: the per-user cryptor (PBKDF2 → RNCryptor/sodium, key
//!   // stored in the OS keychain by termius-storage) must be applied by the
//!   // sync orchestration layer before push / after pull. Conversions below
//!   // operate on the *decrypted logical* shape of a record.
//!
//! Everything not typed here is preserved verbatim in `#[serde(flatten)]
//! extra`, so pulls never lose fields the server adds later.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use termius_core::host::HostType;
use termius_core::keychain::KeychainEntry;
use termius_core::{Group, Host, Keychain, KnownHost, Snippet};

// ---------------------------------------------------------------------------
// Sync set names (recovered verbatim from chunk_sfe.js `ns` table)
// ---------------------------------------------------------------------------

/// Hosts (`entry.js` model `fmt`, repository `hosts`).
pub const SET_HOSTS: &str = "host_set";
/// Snippets (`gmt` / `snippets`).
pub const SET_SNIPPETS: &str = "snippet_set";
/// Groups (`emt` / `groups`).
pub const SET_GROUPS: &str = "group_set";
/// Known hosts (`Tmt` / `known_hosts`).
pub const SET_KNOWN_HOSTS: &str = "knownhost_set";
/// Keychains.
///
/// // PORT-TODO: v10.1.3 has **no** keychain set in the recovered `ns`
/// // table — credentials sync through `identity_set` (username/password)
/// // and `sshkeycrypt_set` (key passphrases). This name is reconstructed so
/// // the port has a keychain bucket; confirm against the live API before
/// // shipping, or map keychains onto `identity_set`.
pub const SET_KEYCHAINS: &str = "keychain_set";
/// Container key for deleted record ids inside a push payload.
pub const SET_DELETES: &str = "delete_sets";
/// Container key the server may answer with instead of `delete_sets`
/// (the recovered JS accepts `delete_sets ?? deleted_sets`).
pub const SET_DELETED: &str = "deleted_sets";

// ---------------------------------------------------------------------------
// Relations
// ---------------------------------------------------------------------------

/// A relation field on the wire: either a bare id string (outgoing push) or a
/// nested record object (incoming pull). See the module docs.
///
/// `Serialize`/`Deserialize` are hand-written (equivalent to serde's
/// `untagged` representation) so the nested form composes with the
/// `#[serde(flatten)]` extra-field maps on the record types.
#[derive(Debug, Clone, PartialEq)]
pub enum Relation<T> {
    Id(String),
    Nested(Box<T>),
}

impl<T: Serialize> Serialize for Relation<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Relation::Id(id) => serializer.serialize_str(id),
            Relation::Nested(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Relation<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(id) => Ok(Relation::Id(id)),
            other => match T::deserialize(other) {
                Ok(nested) => Ok(Relation::Nested(Box::new(nested))),
                Err(err) => Err(<D::Error as serde::de::Error>::custom(err.to_string())),
            },
        }
    }
}

impl<T> Relation<T> {
    /// Wrap a record as a nested relation.
    pub fn nested(value: T) -> Self {
        Relation::Nested(Box::new(value))
    }

    /// The bare id form, when the wire carried a plain string.
    pub fn as_id_str(&self) -> Option<&str> {
        match self {
            Relation::Id(id) => Some(id.as_str()),
            Relation::Nested(_) => None,
        }
    }

    /// The nested record form, when the wire carried an object.
    pub fn as_nested(&self) -> Option<&T> {
        match self {
            Relation::Nested(value) => Some(value.as_ref()),
            Relation::Id(_) => None,
        }
    }
}

impl<T> Default for Relation<T> {
    fn default() -> Self {
        Relation::Id(String::new())
    }
}

/// Wire types that carry their own record id, so a nested relation can be
/// collapsed back to an id (mirrors `y[A] = H.id` in the recovered JS).
pub trait HasWireId {
    fn wire_id(&self) -> &str;
}

impl<T: HasWireId> Relation<T> {
    /// The referenced record id, resolving nested form as well. `None` when
    /// the id is missing/empty (a not-yet-pushed local record).
    pub fn id(&self) -> Option<String> {
        let id = match self {
            Relation::Id(id) => id.as_str(),
            Relation::Nested(value) => value.wire_id(),
        };
        if id.is_empty() {
            None
        } else {
            Some(id.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Host
// ---------------------------------------------------------------------------

/// Host record (`host_set`).
///
/// Recovered field lists (model `fmt` in `entry.js`):
/// crypto: `address`, `label`, `cloud_instance_id`, `cloud_instance_type`;
/// plain: `backspace`, `ip_version`, `os_name`;
/// common: `id`, `local_id`, `updated_at`, `is_shared`, `encrypted_with`,
/// `credentials_mode`; relations: `group`, `ssh_config`, `telnet_config`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireHost {
    /// Server record id (empty for a not-yet-pushed record).
    #[serde(default)]
    pub id: String,
    /// Client-side correlation id; kept on push only while `id` is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<Value>,
    /// // PORT-TODO: encrypted on the wire (v3 field / v5 `content`).
    #[serde(default)]
    pub label: String,
    /// DNS name or IP. // PORT-TODO: encrypted on the wire (see `label`).
    #[serde(default)]
    pub address: String,
    /// Plain field, e.g. `linux` / `osx` / `windows`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_name: Option<String>,
    /// Plain field: backspace handling mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backspace: Option<bool>,
    /// Plain field: `ipv4` / `ipv6` / null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_version: Option<String>,
    // PORT-TODO: encrypted on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_instance_type: Option<String>,
    /// Owning group (id on push, nested record on pull).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<Relation<WireGroup>>,
    /// Connection config — carries `port` and the bound identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_config: Option<Relation<WireSshConfig>>,
    /// Present ⇒ this is a telnet host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telnet_config: Option<Relation<WireTelnetConfig>>,
    /// Common field: team credential handling (`owner`/`team`/null).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials_mode: Option<String>,
    /// // PORT-TODO: not in the recovered `commonFields`; keep only if the
    /// // live API actually returns it (server sets `updated_at` itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Common field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Common field: record is shared through a team vault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_shared: Option<bool>,
    /// Common field: vault id the record is encrypted with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_with: Option<String>,
    /// Unknown/unsupported keys, preserved verbatim (incl. v5 `content`).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HasWireId for WireHost {
    fn wire_id(&self) -> &str {
        &self.id
    }
}

impl From<WireHost> for Host {
    fn from(w: WireHost) -> Self {
        // Resolve what we can from the (possibly nested) ssh_config relation
        // before moving out of `w`.
        let cfg = w.ssh_config.as_ref().and_then(Relation::as_nested);
        let port = cfg.and_then(|c| c.port).unwrap_or(22);
        let use_mosh = cfg.and_then(|c| c.use_mosh).unwrap_or(false);
        let identity_rel = cfg.and_then(|c| c.identity.as_ref());
        let identity_id = identity_rel.and_then(Relation::id);
        let username = identity_rel
            .and_then(Relation::as_nested)
            .and_then(|i| i.username.clone())
            .unwrap_or_default();
        let group_ids: Vec<String> = w
            .group
            .as_ref()
            .and_then(Relation::id)
            .into_iter()
            .collect();
        let host_type = if w.telnet_config.is_some() {
            HostType::Telnet
        } else if use_mosh {
            HostType::Mosh
        } else {
            HostType::Ssh
            // PORT-TODO: `Local` / `Serial` hosts are not distinguishable on
            // the wire from the recovered model; join with the local storage
            // record (or an `os_name` heuristic) in the orchestration layer.
        };

        let WireHost {
            id,
            label,
            address,
            created_at,
            updated_at,
            ..
        } = w;

        Host {
            id,
            label,
            hostname: address,
            port,
            username,
            host_type,
            group_ids,
            identity_id,
            key_id: None,
            // PORT-TODO: `keychain_id` comes from `sshkeycrypt_set`
            // (`identity.ssh_key` → key → passphrase) — join in the
            // orchestration layer.
            keychain_id: None,
            // PORT-TODO: port forwards live in `pfrule_set`, snippets in
            // `hostsnippet_set`, pinned fingerprints in `knownhost_set`.
            port_forwardings: Vec::new(),
            snippet_ids: Vec::new(),
            known_host_id: None,
            created_at,
            updated_at,
            notes: None,
        }
    }
}

impl From<&Host> for WireHost {
    fn from(h: &Host) -> Self {
        let identity = if let Some(identity_id) = &h.identity_id {
            Some(Relation::Id(identity_id.clone()))
        } else if !h.username.is_empty() {
            Some(Relation::nested(WireIdentity {
                username: Some(h.username.clone()),
                ..Default::default()
            }))
        } else {
            None
        };

        let is_telnet = matches!(h.host_type, HostType::Telnet);
        // // PORT-TODO: on push the server expects relation *ids* here; a
        // // nested object with an empty `id` only works for the initial
        // // create where the server assigns one. The orchestration layer
        // // should swap in the stored `ssh_config_set` id.
        let ssh_config = (!is_telnet).then(|| {
            Relation::nested(WireSshConfig {
                port: Some(h.port),
                use_mosh: matches!(h.host_type, HostType::Mosh).then_some(true),
                identity,
                ..Default::default()
            })
        });
        let telnet_config = is_telnet.then(|| {
            Relation::nested(WireTelnetConfig {
                port: Some(h.port),
                ..Default::default()
            })
        });

        WireHost {
            id: h.id.clone(),
            label: h.label.clone(),
            address: h.hostname.clone(),
            group: h.group_ids.first().map(|id| Relation::Id(id.clone())),
            ssh_config,
            telnet_config,
            created_at: non_empty(&h.created_at),
            updated_at: non_empty(&h.updated_at),
            ..Default::default()
        }
    }
}

impl From<Host> for WireHost {
    fn from(h: Host) -> Self {
        WireHost::from(&h)
    }
}

// ---------------------------------------------------------------------------
// Group
// ---------------------------------------------------------------------------

/// Group record (`group_set`). Recovered (model `emt`): crypto `label`;
/// common + `sharing_mode`, `credentials_mode`; relations `parent_group`,
/// `ssh_config`, `telnet_config`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireGroup {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<Value>,
    /// // PORT-TODO: encrypted on the wire (v3/v5).
    #[serde(default)]
    pub label: String,
    /// Parent group for nesting (id or nested record).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_group: Option<Relation<WireGroup>>,
    /// Optional default connection config inherited by hosts in the group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_config: Option<Relation<WireSshConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telnet_config: Option<Relation<WireTelnetConfig>>,
    /// Common field: team sharing mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharing_mode: Option<String>,
    /// Common field: team credential mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_shared: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_with: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HasWireId for WireGroup {
    fn wire_id(&self) -> &str {
        &self.id
    }
}

impl From<WireGroup> for Group {
    fn from(w: WireGroup) -> Self {
        let parent_id = w.parent_group.as_ref().and_then(Relation::id);
        let WireGroup {
            id,
            label,
            created_at,
            updated_at,
            ..
        } = w;
        Group {
            id,
            title: label,
            parent_id,
            // PORT-TODO: `host_ids` is a reverse index of `host_set`
            // (`host.group`); fill it in the orchestration layer.
            host_ids: Vec::new(),
            // PORT-TODO: no `sort_order` in the recovered wire model —
            // Termius appears to order groups by `created_at`.
            sort_order: 0,
            created_at,
            updated_at,
        }
    }
}

impl From<&Group> for WireGroup {
    fn from(g: &Group) -> Self {
        WireGroup {
            id: g.id.clone(),
            label: g.title.clone(),
            parent_group: g.parent_id.clone().map(Relation::Id),
            created_at: non_empty(&g.created_at),
            updated_at: non_empty(&g.updated_at),
            ..Default::default()
        }
    }
}

impl From<Group> for WireGroup {
    fn from(g: Group) -> Self {
        WireGroup::from(&g)
    }
}

// ---------------------------------------------------------------------------
// Snippet
// ---------------------------------------------------------------------------

/// Snippet record (`snippet_set`). Recovered (model `gmt`): crypto `label`,
/// `script`; relation `package`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireSnippet {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<Value>,
    /// // PORT-TODO: encrypted on the wire.
    #[serde(default)]
    pub label: String,
    /// Command body. // PORT-TODO: encrypted on the wire.
    #[serde(default)]
    pub script: String,
    /// Snippet package this belongs to (relation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<Relation<WireSnippetPackage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_shared: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_with: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HasWireId for WireSnippet {
    fn wire_id(&self) -> &str {
        &self.id
    }
}

impl From<WireSnippet> for Snippet {
    fn from(w: WireSnippet) -> Self {
        let WireSnippet {
            id,
            label,
            script,
            created_at,
            updated_at,
            ..
        } = w;
        Snippet {
            id,
            title: label,
            body: script,
            // PORT-TODO: host bindings live in `hostsnippet_set`, and
            // `command`/`sort_order` are not part of the recovered wire
            // model (local-only in Termius) — join/keep locally.
            host_ids: Vec::new(),
            command: None,
            sort_order: 0,
            created_at,
            updated_at,
        }
    }
}

impl From<&Snippet> for WireSnippet {
    fn from(s: &Snippet) -> Self {
        WireSnippet {
            id: s.id.clone(),
            label: s.title.clone(),
            script: s.body.clone(),
            created_at: non_empty(&s.created_at),
            updated_at: non_empty(&s.updated_at),
            ..Default::default()
        }
    }
}

impl From<Snippet> for WireSnippet {
    fn from(s: Snippet) -> Self {
        WireSnippet::from(&s)
    }
}

/// Snippet package relation target (`package_set`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireSnippetPackage {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HasWireId for WireSnippetPackage {
    fn wire_id(&self) -> &str {
        &self.id
    }
}

// ---------------------------------------------------------------------------
// Known host
// ---------------------------------------------------------------------------

/// Known-host fingerprint (`knownhost_set`). Recovered (model `Tmt`):
/// crypto `hostnames`, `key`; plus plain `marker` / `comment`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireKnownHost {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<Value>,
    /// Marker used to reconcile duplicates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    /// Host patterns this key was observed on.
    ///
    /// Encrypted on the wire, so the shape differs between schemas: v3
    /// carries a string (`"host:22"` or comma separated), v5 the parsed
    /// array inside `content`. Modelled as free-form JSON and normalised by
    /// [`KnownHost::from`] conversions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostnames: Option<Value>,
    /// OpenSSH public key (also encrypted on the wire).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Free-form comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_shared: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_with: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HasWireId for WireKnownHost {
    fn wire_id(&self) -> &str {
        &self.id
    }
}

impl From<WireKnownHost> for KnownHost {
    fn from(w: WireKnownHost) -> Self {
        let (host, port) = split_host_port(w.hostnames.as_ref());
        KnownHost {
            id: w.id,
            host,
            port,
            public_key: w.key.unwrap_or_default(),
            created_at: w.created_at.unwrap_or_default(),
            updated_at: w.updated_at.unwrap_or_default(),
        }
    }
}

impl From<&KnownHost> for WireKnownHost {
    fn from(k: &KnownHost) -> Self {
        WireKnownHost {
            id: k.id.clone(),
            hostnames: Some(Value::String(format!("{}:{}", k.host, k.port))),
            key: Some(k.public_key.clone()),
            created_at: non_empty(&k.created_at),
            updated_at: non_empty(&k.updated_at),
            ..Default::default()
        }
    }
}

impl From<KnownHost> for WireKnownHost {
    fn from(k: KnownHost) -> Self {
        WireKnownHost::from(&k)
    }
}

/// Pull `(host, port)` out of the wire's `hostnames` value, accepting the
/// v3 string form (`"host:22"`, comma separated) and the v5 array form.
fn split_host_port(hostnames: Option<&Value>) -> (String, u16) {
    let raw = match hostnames {
        Some(Value::String(s)) => s
            .split(',')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .find_map(|item| item.as_str())
            .unwrap_or_default()
            .trim()
            .to_string(),
        Some(Value::Object(map)) => map
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    };

    if let Some((host, port)) = raw.rsplit_once(':') {
        if let Ok(port) = port.parse::<u16>() {
            return (host.to_string(), port);
        }
    }
    (raw, 22)
}

// ---------------------------------------------------------------------------
// Keychain
// ---------------------------------------------------------------------------

/// Keychain record (`keychain_set`).
///
/// // PORT-TODO: no keychain set exists in the recovered v10.1.3 sources —
/// // this shape is reconstructed from `termius_core::Keychain` so the port
/// // has a bucket to sync. Verify against the live API (or model keychains
/// // as `identity_set` rows) before relying on it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireKeychain {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<Value>,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<WireKeychainEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_shared: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_with: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HasWireId for WireKeychain {
    fn wire_id(&self) -> &str {
        &self.id
    }
}

/// One secret slot inside a wire keychain. Never logged.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireKeychainEntry {
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub secret: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl From<WireKeychain> for Keychain {
    fn from(w: WireKeychain) -> Self {
        let WireKeychain {
            id,
            label,
            entries,
            created_at,
            updated_at,
            ..
        } = w;
        Keychain {
            id,
            title: label,
            entries: entries
                .unwrap_or_default()
                .into_iter()
                .map(|e| KeychainEntry {
                    owner_id: e.owner_id,
                    secret: e.secret,
                })
                .collect(),
            created_at,
            updated_at,
        }
    }
}

impl From<&Keychain> for WireKeychain {
    fn from(k: &Keychain) -> Self {
        WireKeychain {
            id: k.id.clone(),
            label: k.title.clone(),
            entries: Some(
                k.entries
                    .iter()
                    .map(|e| WireKeychainEntry {
                        owner_id: e.owner_id.clone(),
                        secret: e.secret.clone(),
                        ..Default::default()
                    })
                    .collect(),
            ),
            created_at: non_empty(&k.created_at),
            updated_at: non_empty(&k.updated_at),
            ..Default::default()
        }
    }
}

impl From<Keychain> for WireKeychain {
    fn from(k: Keychain) -> Self {
        WireKeychain::from(&k)
    }
}

// ---------------------------------------------------------------------------
// Config/identity relation targets
// ---------------------------------------------------------------------------

/// `sshconfig_set` record. `port` is a *plain* field on the wire (recovered
/// `plainFields` of model `Gpt`), so it survives without decryption — this
/// is where a host's connection port actually lives.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireSshConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<Value>,
    /// Connection port (plain on the wire).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Bound identity (relation); supplies the login username.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<Relation<WireIdentity>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_forwarding: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_blink: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_forward_ports: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive_packages: Option<i64>,
    /// Plain field; `true` marks the host as a Mosh host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_mosh: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_ssh_key: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict_host_key_check: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    /// Encrypted field (v3) — transported opaquely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_variables: Option<String>,
    /// Sodium-encrypted field — transported opaquely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mosh_server_command: Option<String>,
    /// Snippet run on connect (relation; kept as free-form JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_snippet: Option<Value>,
    /// Proxy command (relation; kept as free-form JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxycommand: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_shared: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_with: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HasWireId for WireSshConfig {
    fn wire_id(&self) -> &str {
        &self.id
    }
}

/// `telnetconfig_set` record.
///
/// // PORT-TODO: field list not recovered — only the relation name
/// // (`telnet_config`) and its set membership are evidenced.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireTelnetConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<Relation<WireIdentity>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_shared: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_with: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HasWireId for WireTelnetConfig {
    fn wire_id(&self) -> &str {
        &self.id
    }
}

/// `identity_set` record. Recovered (model `Mpt`): crypto `label`,
/// `password`, `username`; plain `is_visible`; common + `sshid_mode`.
///
/// // PORT-TODO: `password`/`ssh_key`/`hardware_key` are intentionally
/// // untyped here — the sync crate must not carry plaintext secrets beyond
/// // what it needs to build a host's username.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireIdentity {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<Value>,
    /// Login username (encrypted on the wire).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sshid_mode: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_shared: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_with: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HasWireId for WireIdentity {
    fn wire_id(&self) -> &str {
        &self.id
    }
}

// ---------------------------------------------------------------------------
// Sync envelope
// ---------------------------------------------------------------------------

/// `delete_sets`: set name → deleted record ids.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DeleteSets {
    #[serde(flatten)]
    pub sets: Map<String, Vec<String>>,
}

impl DeleteSets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `id` as deleted from `set`.
    pub fn insert(&mut self, set: impl Into<String>, id: impl Into<String>) {
        self.sets.entry(set.into()).or_default().push(id.into());
    }

    /// Deleted ids for `set`, if any.
    pub fn get(&self, set: &str) -> Option<&Vec<String>> {
        self.sets.get(set)
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }
}

/// Outgoing push body for `POST /api/v4/terminal/sync/`.
///
/// The recovered saga (`generateSyncRequest` in `entry.js`) builds exactly
/// this: one array per set plus a `delete_sets` bucket, and stamps
/// `last_synced` with the client's last-sync timestamp.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SyncPushRequest {
    /// Client's last successful sync instant (RFC 3339). Filled from the
    /// tracked value by [`crate::SyncClient::push`] when not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_sets: Option<DeleteSets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_set: Option<Vec<WireHost>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet_set: Option<Vec<WireSnippet>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_set: Option<Vec<WireGroup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knownhost_set: Option<Vec<WireKnownHost>>,
    /// // PORT-TODO: set name unverified, see [`SET_KEYCHAINS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keychain_set: Option<Vec<WireKeychain>>,
    /// Any other set the app supports (identities, keys, port forwards, …)
    /// passed through untouched.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SyncPushRequest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_hosts(hosts: Vec<WireHost>) -> Self {
        Self {
            host_set: Some(hosts),
            ..Default::default()
        }
    }

    pub fn with_snippets(snippets: Vec<WireSnippet>) -> Self {
        Self {
            snippet_set: Some(snippets),
            ..Default::default()
        }
    }

    pub fn with_groups(groups: Vec<WireGroup>) -> Self {
        Self {
            group_set: Some(groups),
            ..Default::default()
        }
    }

    pub fn with_keychains(keychains: Vec<WireKeychain>) -> Self {
        Self {
            keychain_set: Some(keychains),
            ..Default::default()
        }
    }

    /// True when the request carries no sets to push and no deletions (the
    /// JS saga then skips the POST and issues a plain pull instead).
    /// `last_synced` on its own does not count as content.
    pub fn is_empty(&self) -> bool {
        self.delete_sets.as_ref().map_or(true, DeleteSets::is_empty)
            && self.host_set.is_none()
            && self.snippet_set.is_none()
            && self.group_set.is_none()
            && self.knownhost_set.is_none()
            && self.keychain_set.is_none()
            && self.extra.is_empty()
    }
}

/// Response of `GET|POST /api/v4/terminal/sync/`.
///
/// Bare arrays per set (verified: the JS saga maps `resp[set]` directly),
/// deletion buckets under `delete_sets` or `deleted_sets`, `now` as the
/// server instant used to seed the next `last_synced`, plus whatever else
/// the server adds (preserved in `extra`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SyncPullResponse {
    /// Server-side instant of this sync — store it as the next
    /// `last_synced` (the JS saga stores `resp.now ?? Date.now()`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_sets: Option<DeleteSets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_sets: Option<DeleteSets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_set: Option<Vec<WireHost>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet_set: Option<Vec<WireSnippet>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_set: Option<Vec<WireGroup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knownhost_set: Option<Vec<WireKnownHost>>,
    /// // PORT-TODO: set name unverified, see [`SET_KEYCHAINS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keychain_set: Option<Vec<WireKeychain>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SyncPullResponse {
    /// Deleted host ids, checking both bucket spellings the server uses.
    pub fn deleted_hosts(&self) -> Vec<String> {
        self.deletion_bucket(SET_HOSTS)
    }

    /// Deleted record ids for `set`, checking both bucket spellings.
    pub fn deletion_bucket(&self, set: &str) -> Vec<String> {
        let from_delete = self.delete_sets.as_ref().and_then(|d| d.get(set));
        let from_deleted = self.deleted_sets.as_ref().and_then(|d| d.get(set));
        from_delete
            .or(from_deleted)
            .cloned()
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Auth / account / device
// ---------------------------------------------------------------------------

/// Payload of `POST /api/v3.3/auth/device/login/` (recovered
/// `auth.login()` fallback path in `entry.js`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignInRequest {
    pub email: String,
    /// Hex SHA-256 of the plaintext password — see [`hash_password`].
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firebase_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authy_token: Option<String>,
    /// Device descriptor; `token` is the persisted `deviceToken`.
    pub device: DeviceInfo,
}

impl SignInRequest {
    /// Build the request the recovered HTTP client sends for a plain
    /// email/password sign-in (no 2FA, no SSO).
    pub fn new(email: impl Into<String>, password: &str, device: DeviceInfo) -> Self {
        SignInRequest {
            email: email.into(),
            password: hash_password(password),
            firebase_token: None,
            authy_token: None,
            device,
        }
    }
}

/// Device descriptor sent with sign-in / `PUT /api/v3/user/device/current/`.
///
/// Mirrors `Device.toJSON()` in `entry.js`; `token` is the value the app
/// persists under the `deviceToken` storage key (a UUIDv4 generated on
/// first run).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Device token (UUIDv4). Never logged.
    pub token: String,
    pub app_version: String,
    pub os_version: String,
    /// Machine/host name.
    pub name: String,
    #[serde(default)]
    pub sub_name: String,
    #[serde(default = "default_mobile_type")]
    pub mobile_type: String,
}

fn default_mobile_type() -> String {
    "Desktop".to_string()
}

impl Default for DeviceInfo {
    fn default() -> Self {
        DeviceInfo {
            token: uuid::Uuid::new_v4().to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os_version: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            // PORT-TODO: the JS fills the machine hostname here (Electron
            // `os.hostname()`); no hostname API without an extra dep —
            // callers should set it from `termius-app`'s system info.
            name: String::new(),
            sub_name: String::new(),
            mobile_type: default_mobile_type(),
        }
    }
}

/// `credentials` block of the sign-in response.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Credentials {
    /// Bearer token stored on the client for subsequent requests.
    #[serde(default)]
    pub token: String,
    /// Password-derived encryption salt (base64) for the record cryptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salt: Option<String>,
    /// HMAC salt for the record cryptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hmac_salt: Option<String>,
    /// The user's personal keyset. // PORT-TODO: consumed by the crypto
    /// layer (termius-storage / integration wave), not interpreted here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_keyset: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Account/plan/team snapshot returned by the API (`bulk_account` on
/// sign-in, or `GET /api/v4/bulk/account/`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BulkAccount {
    pub account: AccountInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_subscription: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_subscription: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trial: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devices: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `account` object. Field list recovered from the reducer defaults (`yz`)
/// in `entry.js`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AccountInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pro_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub two_factor_auth: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_email_confirmed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_sso: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tax_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Server instant embedded in account payloads (`account.now`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    /// `{ encryption_schema: "v3" | "v5", … }` — decides how records are
    /// encrypted for the sync endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature_toggles: Option<Value>,
    /// e.g. `{ websocket_sync: bool }` — gates the WS sync transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorized_features: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_period: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expired_screen_type: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need_to_update_subscription: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_subscribed_to_marketing_emails: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `team` object of the account snapshot.
///
/// // PORT-TODO: only the fields the app reads are typed (`id`,
/// // `is_owner`, `owner`, `owner_name`); the full shape was not recovered.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TeamInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_owner: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Response of `POST /api/v3.3/auth/device/login/`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SignInResponse {
    pub credentials: Credentials,
    pub bulk_account: BulkAccount,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Device record returned by `PUT /api/v3/user/device/current/`.
///
/// // PORT-TODO: exact response shape not recovered — every field is
/// // optional and unknown keys land in `extra` so parsing never fails on
/// // a server-side addition.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DeviceRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// ---------------------------------------------------------------------------
// Brand (white-label theming)
// ---------------------------------------------------------------------------

/// White-label / theming configuration.
///
/// // PORT-TODO: no brand endpoint or payload exists anywhere in the
/// // recovered sources (the only `brand` hits are zod's `.brand()` and
/// // `navigator.userAgentData.brands`). `GET /api/v4/terminal/brand/` is a
/// // reconstruction; every field is optional and unknown keys round-trip
/// // through `extra`, so a differently shaped live response still parses.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BrandConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Product/brand name shown in the UI chrome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_url: Option<String>,
    /// Anything else the server sends is preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Lowercase hex SHA-256 of `password` — the exact transformation the
/// recovered app applies to the password before the HTTP device-login
/// request (`NS.generateHash` → `sjcl.hash.sha256` + hex codec).
pub fn hash_password(password: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
}

/// `Some(s)` for non-empty strings, `None` otherwise (keeps `skip_serializing_if`
/// effective for core types whose timestamps default to `""`).
fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hash_password_matches_sha256_vectors() {
        assert_eq!(
            hash_password(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash_password("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // lowercase hex, 64 chars
        let h = hash_password("s3cret");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }

    #[test]
    fn wire_host_round_trips_into_core_host() {
        let json = r#"{
            "id": "h-1",
            "local_id": 12,
            "label": "web-1",
            "address": "10.0.0.5",
            "os_name": "linux",
            "backspace": true,
            "ip_version": "ipv4",
            "group": "g-1",
            "ssh_config": {
                "id": "sc-1",
                "port": 2222,
                "use_mosh": false,
                "identity": {"id": "id-1", "username": "deploy"}
            },
            "credentials_mode": null,
            "updated_at": "2026-01-02T03:04:05Z",
            "is_shared": false,
            "encrypted_with": null
        }"#;

        let wire: WireHost = serde_json::from_str(json).expect("wire host");
        let host = Host::from(wire);

        assert_eq!(host.id, "h-1");
        assert_eq!(host.label, "web-1");
        assert_eq!(host.hostname, "10.0.0.5");
        assert_eq!(host.port, 2222);
        assert_eq!(host.username, "deploy");
        assert_eq!(host.identity_id.as_deref(), Some("id-1"));
        assert_eq!(host.group_ids, vec!["g-1".to_string()]);
        assert_eq!(host.host_type, HostType::Ssh);
        assert_eq!(host.updated_at, "2026-01-02T03:04:05Z");

        // core → wire keeps the essential fields and stays serializable
        let back = WireHost::from(&host);
        let value = serde_json::to_value(&back).expect("serialize wire host");
        assert_eq!(value["label"], "web-1");
        assert_eq!(value["address"], "10.0.0.5");
        assert_eq!(value["group"], "g-1");
        assert_eq!(value["ssh_config"]["port"], 2222);
        assert_eq!(value["ssh_config"]["identity"], "id-1");

        // and it parses back into the same core record — except `username`,
        // which lives in `identity_set` once the identity is referenced by id
        // (the join happens in the orchestration layer, see the PORT-TODOs)
        let reparsed: WireHost = serde_json::from_value(value).expect("reparse");
        let host2 = Host::from(reparsed);
        assert_eq!(host2.id, host.id);
        assert_eq!(host2.label, host.label);
        assert_eq!(host2.hostname, host.hostname);
        assert_eq!(host2.port, host.port);
        assert_eq!(host2.identity_id, host.identity_id);
        assert_eq!(host2.group_ids, host.group_ids);
        assert_eq!(host2.host_type, host.host_type);
        assert_eq!(host2.updated_at, host.updated_at);
        assert_eq!(host2.username, "");
    }

    #[test]
    fn host_protocol_is_inferred_from_relations() {
        // telnet host
        let wire: WireHost =
            serde_json::from_str(r#"{"id":"t1","telnet_config":{"id":"tc1","port":23}}"#)
                .expect("telnet host");
        let host = Host::from(wire);
        assert_eq!(host.host_type, HostType::Telnet);
        assert_eq!(host.port, 23);

        // mosh host (ssh_config.use_mosh)
        let wire: WireHost =
            serde_json::from_str(r#"{"id":"m1","ssh_config":{"port":22,"use_mosh":true}}"#)
                .expect("mosh host");
        let host = Host::from(wire);
        assert_eq!(host.host_type, HostType::Mosh);
        assert_eq!(host.port, 22);

        // no config at all → safe defaults
        let wire: WireHost = serde_json::from_str(r#"{"id":"n1"}"#).expect("bare host");
        let host = Host::from(wire);
        assert_eq!(host.host_type, HostType::Ssh);
        assert_eq!(host.port, 22);
        assert_eq!(host.username, "");
        assert!(host.group_ids.is_empty());

        // core → wire: mosh flag rides on ssh_config, telnet gets its own
        let mut h = Host::default();
        h.host_type = HostType::Mosh;
        let value = serde_json::to_value(WireHost::from(&h)).expect("serialize");
        assert_eq!(value["ssh_config"]["use_mosh"], true);

        let mut h = Host::default();
        h.host_type = HostType::Telnet;
        h.port = 2022;
        let value = serde_json::to_value(WireHost::from(&h)).expect("serialize");
        assert_eq!(value["telnet_config"]["port"], 2022);
        assert!(value.get("ssh_config").is_none());
    }

    #[test]
    fn wire_host_username_without_identity() {
        let mut h = Host::default();
        h.username = "root".into();
        let value = serde_json::to_value(WireHost::from(&h)).expect("serialize");
        assert_eq!(value["ssh_config"]["identity"]["username"], "root");
        // no identity id → nested identity, not an id string
        assert!(value["ssh_config"]["identity"].is_object());
    }

    #[test]
    fn wire_group_round_trips_parent_relation() {
        // nested parent (pull shape)
        let json = r#"{
            "id": "g-1",
            "label": "Prod",
            "parent_group": {"id": "g-0", "label": "Root"},
            "sharing_mode": "read",
            "credentials_mode": null,
            "updated_at": "2026-01-01T00:00:00Z",
            "is_shared": false
        }"#;
        let wire: WireGroup = serde_json::from_str(json).expect("wire group");
        let group = Group::from(wire);
        assert_eq!(group.id, "g-1");
        assert_eq!(group.title, "Prod");
        assert_eq!(group.parent_id.as_deref(), Some("g-0"));

        // bare-id parent (push shape)
        let wire: WireGroup =
            serde_json::from_str(r#"{"id":"g-2","label":"X","parent_group":"g-9"}"#)
                .expect("wire group");
        assert_eq!(Group::from(wire).parent_id.as_deref(), Some("g-9"));

        // core → wire serialises the parent as a bare id
        let value = serde_json::to_value(WireGroup::from(&group)).expect("serialize");
        assert_eq!(value["parent_group"], "g-0");
        assert_eq!(value["label"], "Prod");

        let reparsed: WireGroup = serde_json::from_value(value).expect("reparse");
        assert_eq!(Group::from(reparsed), group);
    }

    #[test]
    fn wire_snippet_round_trips() {
        let json = r#"{
            "id": "s-1",
            "label": "uptime",
            "script": "uptime -p",
            "updated_at": "2026-02-02T00:00:00Z"
        }"#;
        let wire: WireSnippet = serde_json::from_str(json).expect("wire snippet");
        let snippet = Snippet::from(wire);
        assert_eq!(snippet.id, "s-1");
        assert_eq!(snippet.title, "uptime");
        assert_eq!(snippet.body, "uptime -p");

        let value = serde_json::to_value(WireSnippet::from(&snippet)).expect("serialize");
        assert_eq!(value["script"], "uptime -p");
        assert_eq!(value["label"], "uptime");

        let reparsed: WireSnippet = serde_json::from_value(value).expect("reparse");
        assert_eq!(Snippet::from(reparsed), snippet);
    }

    #[test]
    fn wire_known_host_accepts_string_and_array_hostnames() {
        // v3 shape: plain string with port
        let wire: WireKnownHost =
            serde_json::from_str(r#"{"id":"k1","hostnames":"example.com:2222","key":"ssh-ed25519 AAA"}"#)
                .expect("wire known host");
        let kh = KnownHost::from(wire);
        assert_eq!(kh.host, "example.com");
        assert_eq!(kh.port, 2222);
        assert_eq!(kh.public_key, "ssh-ed25519 AAA");

        // v5 shape: parsed array
        let wire: WireKnownHost =
            serde_json::from_str(r#"{"id":"k2","hostnames":["db.internal:22"],"key":"k"}"#)
                .expect("wire known host");
        let kh = KnownHost::from(wire);
        assert_eq!(kh.host, "db.internal");
        assert_eq!(kh.port, 22);

        // no port anywhere → default 22
        let wire: WireKnownHost =
            serde_json::from_str(r#"{"id":"k3","hostnames":["plain.host"],"key":"k"}"#)
                .expect("wire known host");
        let kh = KnownHost::from(wire);
        assert_eq!(kh.host, "plain.host");
        assert_eq!(kh.port, 22);

        // core → wire emits the canonical "host:port" string form
        let value = serde_json::to_value(WireKnownHost::from(&kh)).expect("serialize");
        assert_eq!(value["hostnames"], "plain.host:22");
    }

    #[test]
    fn wire_keychain_round_trips_entries() {
        let json = r#"{
            "id": "kc-1",
            "label": "Prod secrets",
            "entries": [{"owner_id": "key-1", "secret": "hunter2"}],
            "updated_at": "2026-03-03T00:00:00Z"
        }"#;
        let wire: WireKeychain = serde_json::from_str(json).expect("wire keychain");
        let kc = Keychain::from(wire);
        assert_eq!(kc.id, "kc-1");
        assert_eq!(kc.title, "Prod secrets");
        assert_eq!(kc.entries.len(), 1);
        assert_eq!(kc.entries[0].owner_id, "key-1");
        assert_eq!(kc.entries[0].secret, "hunter2");

        let value = serde_json::to_value(WireKeychain::from(&kc)).expect("serialize");
        let reparsed: WireKeychain = serde_json::from_value(value).expect("reparse");
        assert_eq!(Keychain::from(reparsed), kc);
    }

    #[test]
    fn unknown_fields_survive_round_trips() {
        let json = r#"{
            "id": "h-9",
            "label": "future",
            "address": "1.2.3.4",
            "content": {"blob": "v5-ciphertext"},
            "some_new_server_field": 42
        }"#;
        let wire: WireHost = serde_json::from_str(json).expect("wire host");
        assert_eq!(wire.extra.get("some_new_server_field"), Some(&json!(42)));
        // v5 ciphertext envelope rides along untouched
        assert!(wire.extra.get("content").is_some());

        let value = serde_json::to_value(&wire).expect("serialize");
        assert_eq!(value["some_new_server_field"], 42);
        assert!(value.get("content").is_some());
    }

    #[test]
    fn sync_push_request_serialises_sets_and_last_synced() {
        let host = WireHost {
            id: "h-1".into(),
            label: "web".into(),
            address: "10.0.0.1".into(),
            ..Default::default()
        };
        let mut req = SyncPushRequest::with_hosts(vec![host]);
        req.last_synced = Some("2026-01-01T00:00:00Z".to_string());
        let mut deletes = DeleteSets::new();
        deletes.insert(SET_HOSTS, "h-9");
        req.delete_sets = Some(deletes);

        let value = serde_json::to_value(&req).expect("serialize push");
        assert_eq!(value["last_synced"], "2026-01-01T00:00:00Z");
        assert_eq!(value[SET_HOSTS][0]["label"], "web");
        assert_eq!(value["delete_sets"][SET_HOSTS][0], "h-9");

        // empty request stays empty
        assert!(SyncPushRequest::new().is_empty());
        assert!(!req.is_empty());

        let back: SyncPushRequest = serde_json::from_value(value).expect("reparse push");
        assert_eq!(back.last_synced.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(
            back.delete_sets.and_then(|d| d.get(SET_HOSTS).cloned()),
            Some(vec!["h-9".to_string()])
        );
    }

    #[test]
    fn sync_pull_response_parses_sets_deletions_and_extras() {
        let json = r#"{
            "now": "2026-04-04T00:00:00Z",
            "host_set": [{"id": "h-1", "label": "a", "address": "1.1.1.1"}],
            "snippet_set": [{"id": "s-1", "label": "l", "script": "ls"}],
            "group_set": [{"id": "g-1", "label": "root"}],
            "knownhost_set": [{"id": "k-1", "hostnames": "h:22", "key": "k"}],
            "delete_sets": {"host_set": ["h-7"]},
            "identity_set": [{"id": "i-1"}]
        }"#;
        let resp: SyncPullResponse = serde_json::from_str(json).expect("pull response");
        assert_eq!(resp.now.as_deref(), Some("2026-04-04T00:00:00Z"));
        assert_eq!(resp.host_set.as_ref().map(Vec::len), Some(1));
        assert_eq!(resp.snippet_set.as_ref().map(Vec::len), Some(1));
        assert_eq!(resp.group_set.as_ref().map(Vec::len), Some(1));
        assert_eq!(resp.knownhost_set.as_ref().map(Vec::len), Some(1));
        assert_eq!(resp.deleted_hosts(), vec!["h-7".to_string()]);
        // untouched sets land in `extra`
        assert!(resp.extra.get("identity_set").is_some());

        // serialising keeps the passthrough set
        let value = serde_json::to_value(&resp).expect("serialize");
        assert!(value.get("identity_set").is_some());
        assert!(value.get("now").is_some());

        // the alternate `deleted_sets` spelling is honoured too
        let json = r#"{"deleted_sets": {"group_set": ["g-2"]}}"#;
        let resp: SyncPullResponse = serde_json::from_str(json).expect("pull response");
        assert_eq!(resp.deletion_bucket(SET_GROUPS), vec!["g-2".to_string()]);
        assert!(resp.deleted_hosts().is_empty());
    }

    #[test]
    fn sign_in_response_parses_credentials_and_account() {
        let json = r#"{
            "credentials": {
                "token": "tok-1",
                "salt": "c2FsdA==",
                "hmac_salt": "aG1hYw==",
                "personal_keyset": {"id": "pk-1"}
            },
            "bulk_account": {
                "account": {
                    "id": 1,
                    "user_id": 7,
                    "email": "dev@example.com",
                    "plan_type": "Pro",
                    "user_type": "individual",
                    "pro_mode": false,
                    "two_factor_auth": true,
                    "has_sso": false,
                    "feature_toggles": {"encryption_schema": "v5"},
                    "authorized_features": {"websocket_sync": true}
                },
                "team": {"id": 3, "is_owner": true},
                "trial": null
            }
        }"#;
        let resp: SignInResponse = serde_json::from_str(json).expect("sign-in response");
        assert_eq!(resp.credentials.token, "tok-1");
        assert_eq!(resp.credentials.salt.as_deref(), Some("c2FsdA=="));
        assert!(resp.credentials.personal_keyset.is_some());
        assert_eq!(resp.bulk_account.account.email.as_deref(), Some("dev@example.com"));
        assert_eq!(resp.bulk_account.account.plan_type.as_deref(), Some("Pro"));
        assert_eq!(resp.bulk_account.account.two_factor_auth, Some(true));
        assert_eq!(
            resp.bulk_account.account.feature_toggles,
            Some(json!({"encryption_schema": "v5"}))
        );
        let team = resp.bulk_account.team.expect("team");
        assert_eq!(team.is_owner, Some(true));
    }

    #[test]
    fn sign_in_request_hashes_password_and_omits_absent_optionals() {
        let device = DeviceInfo {
            token: "dev-token".into(),
            app_version: "10.1.3".into(),
            os_version: "macOS 15.0".into(),
            name: "mbp".into(),
            sub_name: String::new(),
            mobile_type: "Desktop".into(),
        };
        let req = SignInRequest::new("dev@example.com", "pw", device);
        let value = serde_json::to_value(&req).expect("serialize");
        assert_eq!(value["email"], "dev@example.com");
        assert_eq!(value["password"], hash_password("pw"));
        assert!(value.get("firebase_token").is_none());
        assert!(value.get("authy_token").is_none());
        assert_eq!(value["device"]["token"], "dev-token");
        assert_eq!(value["device"]["mobile_type"], "Desktop");
    }

    #[test]
    fn device_info_defaults_are_complete() {
        let device = DeviceInfo::default();
        assert!(!device.token.is_empty());
        assert!(!device.app_version.is_empty());
        assert_eq!(device.mobile_type, "Desktop");
        let value = serde_json::to_value(&device).expect("serialize");
        for key in ["token", "app_version", "os_version", "name", "sub_name", "mobile_type"] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn brand_config_tolerates_unknown_shape() {
        let json = r#"{"name": "ACME", "brand_color": "#123456", "banner": {"img": "x"}}"#;
        let brand: BrandConfig = serde_json::from_str(json).expect("brand");
        assert_eq!(brand.name.as_deref(), Some("ACME"));
        assert_eq!(brand.extra.get("brand_color"), Some(&json!("#123456")));
        let value = serde_json::to_value(&brand).expect("serialize");
        assert_eq!(value["brand_color"], "#123456");
        assert_eq!(value["banner"]["img"], "x");
    }
}
