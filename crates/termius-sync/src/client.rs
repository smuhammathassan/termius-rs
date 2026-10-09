//! `SyncClient` — a typed async client for the Termius cloud API.
//!
//! Reconstructed from the recovered Electron sources:
//!
//! * base URL `https://api.termius.com` (`chunk_ffe.js::ffe()`; the app also
//!   honours a custom backend URL — see [`SyncClientBuilder::base_url`]),
//! * endpoint table `chunk_Pt.js`,
//! * HTTP client behaviour (`b1e`/`x1e` classes in `entry.js`): JSON content
//!   type, `X-DEVICE-APP-VERSION` / `X-DEVICE-PLATFORM` headers, auth header,
//!   `last_synced__gte` pull query, `last_synced` on push, 401/403/490 event
//!   classification, 400→pull-repair-retry on push,
//! * sign-in flow (`auth.login()`): `POST /api/v3.3/auth/device/login/` with
//!   a hex-SHA256 password and the persisted `deviceToken` device record.
//!
//! The token is held in memory on the client and attached to every request;
//! it is never written to logs or error messages (`Debug` is redacted).

use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{de::DeserializeOwned, Serialize};
use termius_core::{Host, Keychain, Snippet};

use crate::error::SyncError;
use crate::models::{
    BrandConfig, BulkAccount, DeviceInfo, DeviceRecord, SignInRequest, SignInResponse,
    SyncPullResponse, SyncPushRequest, WireGroup, WireHost, WireKeychain, WireKnownHost,
    WireSnippet,
};

/// Production API host (recovered from `chunk_ffe.js`).
pub const DEFAULT_BASE_URL: &str = "https://api.termius.com";

/// Default per-request timeout applied by [`SyncClientBuilder`].
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Auth scheme
// ---------------------------------------------------------------------------

/// Scheme prefix for the `Authorization` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenScheme {
    /// `Authorization: Bearer <token>` — the scheme this port uses.
    Bearer,
    /// `Authorization: DeviceToken <token>` — what the recovered JS client
    /// actually sends (`LU` map in `entry.js`: `token → "Token"`,
    /// `temp → "TempToken"`, `device → "DeviceToken"`, default `device`).
    ///
    /// // PORT-TODO: verify which prefix the live API accepts and, if it is
    /// // `DeviceToken`, flip the builder default to this variant.
    Device,
    /// `Authorization: Token <token>`.
    Token,
    /// `Authorization: TempToken <token>` (temporary/2FA token).
    TempToken,
}

impl TokenScheme {
    pub fn as_str(self) -> &'static str {
        match self {
            TokenScheme::Bearer => "Bearer",
            TokenScheme::Device => "DeviceToken",
            TokenScheme::Token => "Token",
            TokenScheme::TempToken => "TempToken",
        }
    }
}

/// Build the `Authorization` header value. Pure so it can be unit-tested
/// without a client or network.
pub fn authorization_value(scheme: TokenScheme, token: &str) -> String {
    format!("{} {}", scheme.as_str(), token)
}

/// Join a base URL and an absolute API path without doubling or dropping
/// slashes.
pub fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{base}{path}")
}

// ---------------------------------------------------------------------------
// Endpoints (recovered from chunk_Pt.js unless noted)
// ---------------------------------------------------------------------------

/// API paths, verbatim from the app's endpoint table.
pub mod endpoints {
    /// Bulk sync: `GET` (pull, `?last_synced__gte=<rfc3339>`) and `POST`
    /// (push, `{<set>: [...], delete_sets: {...}, last_synced}`).
    pub const SYNC: &str = "/api/v4/terminal/sync/";
    /// Personal-data-only bulk sync (no team data).
    pub const SYNC_PERSONAL_BULK: &str = "/api/v3/terminal/bulk/";
    /// `POST` device login — the sign-in endpoint.
    pub const DEVICE_LOGIN: &str = "/api/v3.3/auth/device/login/";
    /// `DELETE` the current device session.
    pub const DEVICE_LOGOUT: &str = "/api/v3/auth/device/logout/current/";
    /// `PUT` refresh the current device record.
    pub const UPDATE_DEVICE: &str = "/api/v3/user/device/current/";
    /// `GET` list registered devices.
    pub const DEVICES: &str = "/api/v3/user/device/";
    /// `GET` full account/team/plan snapshot.
    pub const BULK_ACCOUNT: &str = "/api/v4/bulk/account/";
    /// `GET` profile.
    pub const USER_PROFILE: &str = "/api/v3/account/profile/";
    /// White-label theming config.
    ///
    /// // PORT-TODO: no brand endpoint exists in the recovered sources —
    /// // path reconstructed to give `pull_brand()` a faithful-looking home.
    pub const BRAND: &str = "/api/v4/terminal/brand/";
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Configures and builds a [`SyncClient`].
#[derive(Debug, Clone)]
pub struct SyncClientBuilder {
    base_url: String,
    client_version: String,
    platform: String,
    token_scheme: TokenScheme,
    device: DeviceInfo,
    timeout: Duration,
}

impl Default for SyncClientBuilder {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            // Mirrors `X-DEVICE-APP-VERSION` (`client_version` in the JS
            // client); the port reports its own version here.
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            // Mirrors the JS constructor call: `platform: "Desktop"`.
            platform: "Desktop".to_string(),
            token_scheme: TokenScheme::Bearer,
            device: DeviceInfo::default(),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl SyncClientBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the API host (the app ships prod/dev/local presets:
    /// `https://api.termius.com`, `https://dev.api.termius.com`,
    /// `http://api.termius.localhost`).
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn client_version(mut self, version: impl Into<String>) -> Self {
        self.client_version = version.into();
        self
    }

    pub fn platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = platform.into();
        self
    }

    pub fn token_scheme(mut self, scheme: TokenScheme) -> Self {
        self.token_scheme = scheme;
        self
    }

    pub fn device(mut self, device: DeviceInfo) -> Self {
        self.device = device;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn build(self) -> Result<SyncClient, SyncError> {
        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|err| SyncError::Network(format!("failed to build http client: {err}")))?;
        Ok(SyncClient {
            inner: Arc::new(Inner {
                http,
                base_url: self.base_url,
                client_version: self.client_version,
                platform: self.platform,
                token_scheme: self.token_scheme,
                token: RwLock::new(None),
                device: RwLock::new(self.device),
                last_synced: RwLock::new(None),
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

struct Inner {
    http: reqwest::Client,
    base_url: String,
    client_version: String,
    platform: String,
    token_scheme: TokenScheme,
    /// Bearer token returned by sign-in. Never logged.
    token: RwLock<Option<String>>,
    /// Persisted `deviceToken` descriptor. Never logged.
    device: RwLock<DeviceInfo>,
    /// RFC 3339 instant of the last successful sync (from `resp.now`).
    last_synced: RwLock<Option<String>>,
}

/// Typed Termius cloud sync client. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct SyncClient {
    inner: Arc<Inner>,
}

impl fmt::Debug for SyncClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Deliberately redacted: never print the bearer or device token.
        f.debug_struct("SyncClient")
            .field("base_url", &self.inner.base_url)
            .field("token_scheme", &self.inner.token_scheme)
            .field("has_token", &self.has_token())
            .field("has_device_token", &!self.device_info().token.is_empty())
            .finish()
    }
}

impl SyncClient {
    /// Start a builder (preferred: allows customising base URL, scheme, …).
    pub fn builder() -> SyncClientBuilder {
        SyncClientBuilder::default()
    }

    /// Client pointed at `base_url` with all other defaults.
    pub fn new(base_url: impl Into<String>) -> Result<Self, SyncError> {
        SyncClientBuilder::default().base_url(base_url).build()
    }

    // -- state accessors ---------------------------------------------------

    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    pub fn token_scheme(&self) -> TokenScheme {
        self.inner.token_scheme
    }

    /// Store the bearer token (e.g. restored from secure storage).
    pub fn set_token(&self, token: Option<String>) {
        *write_locked(&self.inner.token) = token;
    }

    /// The current bearer token, if any. Never log this value.
    pub fn token(&self) -> Option<String> {
        read_locked(&self.inner.token)
    }

    pub fn has_token(&self) -> bool {
        self.token().is_some()
    }

    /// Drop the bearer token (401 handling / logout).
    pub fn clear_token(&self) {
        self.set_token(None);
    }

    /// Replace the device descriptor (typically from persisted storage).
    pub fn set_device(&self, device: DeviceInfo) {
        *write_locked(&self.inner.device) = device;
    }

    /// Store only the device token, keeping the rest of the descriptor.
    pub fn set_device_token(&self, token: impl Into<String>) {
        write_locked(&self.inner.device).token = token.into();
    }

    /// The device descriptor (its `token` is the persisted `deviceToken`).
    pub fn device_info(&self) -> DeviceInfo {
        read_locked(&self.inner.device)
    }

    /// RFC 3339 instant of the last successful sync, if known.
    pub fn last_synced(&self) -> Option<String> {
        read_locked(&self.inner.last_synced)
    }

    /// Restore (or clear) the last-sync instant — the recovered app keeps
    /// this in its settings store and replays it on startup.
    pub fn set_last_synced(&self, last_synced: Option<String>) {
        *write_locked(&self.inner.last_synced) = last_synced;
    }

    /// `Authorization` header value for the current token/scheme, if a token
    /// is stored. Pure — unit-tested without network.
    pub fn authorization_header_value(&self) -> Option<String> {
        let token = self.token()?;
        if token.is_empty() {
            return None;
        }
        Some(authorization_value(self.inner.token_scheme, &token))
    }

    // -- auth --------------------------------------------------------------

    /// Sign in with email + password.
    ///
    /// Sends `POST /api/v3.3/auth/device/login/` with the password hashed
    /// exactly like the recovered app (`hex(sha256(password))`) and the
    /// current device descriptor (a UUIDv4 `deviceToken` is generated on
    /// first use — read it back via [`SyncClient::device_info`] to persist
    /// it). On success the returned bearer token is stored on the client.
    pub async fn signin(&self, email: &str, password: &str) -> Result<SignInResponse, SyncError> {
        let device = self.ensure_device();
        let request = SignInRequest::new(email, password, device);
        self.signin_with(request).await
    }

    /// Sign in with a fully-built request (2FA / SSO callers construct
    /// [`SignInRequest`] themselves). Stores the returned bearer token.
    pub async fn signin_with(
        &self,
        request: SignInRequest,
    ) -> Result<SignInResponse, SyncError> {
        let response: SignInResponse = self
            .post_json(endpoints::DEVICE_LOGIN, &request, false)
            .await?;
        let token = response.credentials.token.clone();
        if !token.is_empty() {
            self.set_token(Some(token));
        }
        Ok(response)
    }

    /// Register/refresh this device with the API
    /// (`PUT /api/v3/user/device/current/`) and remember `device_token` for
    /// future sign-ins.
    ///
    /// Requires an authenticated client (call [`SyncClient::signin`]
    /// first); the recovered app only sends device info *inside* the login
    /// payload before that.
    pub async fn register_device(&self, device_token: &str) -> Result<DeviceRecord, SyncError> {
        self.set_device_token(device_token);
        let device = self.device_info();
        self.put_json(endpoints::UPDATE_DEVICE, &device).await
    }

    /// End the current device session (`DELETE /api/v3/auth/device/logout/current/`)
    /// and drop the local bearer token. Errors from the server are surfaced
    /// but the token is cleared either way.
    pub async fn logout(&self) -> Result<serde_json::Value, SyncError> {
        let result = self
            .request(endpoints::DEVICE_LOGOUT, Verb::Delete, None, false)
            .await;
        self.clear_token();
        result
    }

    /// Full account snapshot: `GET /api/v4/bulk/account/`.
    pub async fn bulk_account(&self) -> Result<BulkAccount, SyncError> {
        self.get_json(endpoints::BULK_ACCOUNT, None).await
    }

    // -- sync core ---------------------------------------------------------

    /// Pull changed records: `GET /api/v4/terminal/sync/` with
    /// `?last_synced__gte=<since>` when `since` (RFC 3339) is given.
    ///
    /// Advances the tracked last-sync instant from `resp.now`.
    pub async fn pull(&self, since: Option<&str>) -> Result<SyncPullResponse, SyncError> {
        let response: SyncPullResponse = self
            .get_json(endpoints::SYNC, since.map(|s| ("last_synced__gte", s)))
            .await?;
        self.note_sync(&response);
        Ok(response)
    }

    /// Push local changes: `POST /api/v4/terminal/sync/`.
    ///
    /// `last_synced` is filled from the tracked instant when unset. An
    /// empty request skips the POST and performs a pull instead — the same
    /// fallback the JS saga takes when there is nothing to push. A 400 from
    /// this endpoint is classified as [`SyncError::Conflict`] (the JS saga's
    /// pull → repair → retry signal).
    pub async fn push(&self, request: SyncPushRequest) -> Result<SyncPullResponse, SyncError> {
        if request.is_empty() {
            return self.pull(request.last_synced.as_deref()).await;
        }
        let mut request = request;
        if request.last_synced.is_none() {
            request.last_synced = self.last_synced();
        }
        let response: SyncPullResponse = self
            .post_json(endpoints::SYNC, &request, true)
            .await?;
        self.note_sync(&response);
        Ok(response)
    }

    // -- per-set CRUD surface ---------------------------------------------

    /// Pull hosts changed since `since` (RFC 3339) or all hosts when `None`.
    pub async fn pull_hosts(&self, since: Option<&str>) -> Result<Vec<WireHost>, SyncError> {
        Ok(self.pull(since).await?.host_set.unwrap_or_default())
    }

    /// Push a single host (upsert by `id`, insert when empty).
    pub async fn push_host(&self, host: &Host) -> Result<SyncPullResponse, SyncError> {
        self.push(SyncPushRequest::with_hosts(vec![WireHost::from(host)]))
            .await
    }

    /// Pull every snippet.
    ///
    /// // PORT-TODO: the sync endpoint is bulk — use [`SyncClient::pull`]
    /// // with a `since` filter when the caller has one.
    pub async fn pull_snippets(&self) -> Result<Vec<WireSnippet>, SyncError> {
        Ok(self.pull(None).await?.snippet_set.unwrap_or_default())
    }

    /// Push a single snippet.
    pub async fn push_snippet(&self, snippet: &Snippet) -> Result<SyncPullResponse, SyncError> {
        self.push(SyncPushRequest::with_snippets(vec![WireSnippet::from(
            snippet,
        )]))
        .await
    }

    /// Pull every group.
    pub async fn pull_groups(&self) -> Result<Vec<WireGroup>, SyncError> {
        Ok(self.pull(None).await?.group_set.unwrap_or_default())
    }

    /// Pull every keychain.
    ///
    /// // PORT-TODO: `keychain_set` is unverified — see
    /// // [`crate::models::SET_KEYCHAINS`].
    pub async fn pull_keychains(&self) -> Result<Vec<WireKeychain>, SyncError> {
        Ok(self.pull(None).await?.keychain_set.unwrap_or_default())
    }

    /// Push a single keychain. // PORT-TODO: see [`SyncClient::pull_keychains`].
    pub async fn push_keychain(
        &self,
        keychain: &Keychain,
    ) -> Result<SyncPullResponse, SyncError> {
        self.push(SyncPushRequest::with_keychains(vec![WireKeychain::from(
            keychain,
        )]))
        .await
    }

    /// Pull every known-host fingerprint (`knownhost_set`).
    pub async fn pull_known_hosts(&self) -> Result<Vec<WireKnownHost>, SyncError> {
        Ok(self.pull(None).await?.knownhost_set.unwrap_or_default())
    }

    /// Pull the white-label theming config.
    ///
    /// // PORT-TODO: endpoint and payload are reconstructed — no brand
    /// // API exists in the recovered sources.
    pub async fn pull_brand(&self) -> Result<BrandConfig, SyncError> {
        self.get_json(endpoints::BRAND, None).await
    }

    // -- internals ---------------------------------------------------------

    /// Device descriptor for outbound payloads, generating + remembering a
    /// UUIDv4 `deviceToken` on first use (mirrors `Device._getDeviceToken`
    /// in `entry.js`: read storage, else `uuid()` and persist).
    fn ensure_device(&self) -> DeviceInfo {
        let mut device = write_locked(&self.inner.device);
        if device.token.is_empty() {
            device.token = uuid::Uuid::new_v4().to_string();
        }
        device.clone()
    }

    /// Record the sync instant (`resp.now`, else server-local now) — the JS
    /// saga stores `resp.now ?? Date.now()` as the next `last_synced`.
    fn note_sync(&self, response: &SyncPullResponse) {
        let instant = response
            .now
            .clone()
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        *write_locked(&self.inner.last_synced) = Some(instant);
    }

    /// Attach the standard headers (auth + device metadata).
    fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let builder = builder
            .header("X-DEVICE-APP-VERSION", self.inner.client_version.as_str())
            .header("X-DEVICE-PLATFORM", self.inner.platform.as_str());
        match self.authorization_header_value() {
            Some(value) => builder.header("Authorization", value),
            None => builder,
        }
    }

    async fn get_json<T: DeserializeOwned + Default>(
        &self,
        path: &str,
        query: Option<(&str, &str)>,
    ) -> Result<T, SyncError> {
        let mut builder = self.inner.http.get(join_url(&self.inner.base_url, path));
        if let Some((key, value)) = query {
            builder = builder.query(&[(key, value)]);
        }
        self.request_with(builder, Verb::Get, path, false).await
    }

    async fn post_json<B: Serialize, T: DeserializeOwned + Default>(
        &self,
        path: &str,
        body: &B,
        is_push: bool,
    ) -> Result<T, SyncError> {
        let builder = self.inner.http.post(join_url(&self.inner.base_url, path));
        self.request_with(builder.json(body), Verb::Post, path, is_push)
            .await
    }

    async fn put_json<B: Serialize, T: DeserializeOwned + Default>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, SyncError> {
        let builder = self.inner.http.put(join_url(&self.inner.base_url, path));
        self.request_with(builder.json(body), Verb::Put, path, false)
            .await
    }

    /// Bodyless request (used by `logout`).
    async fn request(
        &self,
        path: &str,
        verb: Verb,
        query: Option<(&str, &str)>,
        is_push: bool,
    ) -> Result<serde_json::Value, SyncError> {
        let mut builder = match verb {
            Verb::Get => self.inner.http.get(join_url(&self.inner.base_url, path)),
            Verb::Post => self.inner.http.post(join_url(&self.inner.base_url, path)),
            Verb::Put => self.inner.http.put(join_url(&self.inner.base_url, path)),
            Verb::Delete => self.inner.http.delete(join_url(&self.inner.base_url, path)),
        };
        if let Some((key, value)) = query {
            builder = builder.query(&[(key, value)]);
        }
        self.request_with(builder, verb, path, is_push).await
    }

    /// Shared pipeline: headers → send → classify → decode.
    async fn request_with<T: DeserializeOwned + Default>(
        &self,
        builder: reqwest::RequestBuilder,
        verb: Verb,
        path: &str,
        is_push: bool,
    ) -> Result<T, SyncError> {
        let text = self
            .exec(self.apply_auth(builder), verb, path, is_push)
            .await?;
        Self::decode(&text, path)
    }

    /// Send a request and return the response body text on 2xx, or a
    /// classified [`SyncError`]. Never logs tokens.
    async fn exec(
        &self,
        builder: reqwest::RequestBuilder,
        verb: Verb,
        path: &str,
        is_push: bool,
    ) -> Result<String, SyncError> {
        let response = builder
            .send()
            .await
            .map_err(|err| SyncError::Network(err.to_string()))?;

        let status = response.status();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());

        if status.is_success() {
            tracing::debug!(verb = %verb.as_str(), path = %path, status = %status, "sync request ok");
            return response
                .text()
                .await
                .map_err(|err| SyncError::Network(err.to_string()));
        }

        tracing::warn!(verb = %verb.as_str(), path = %path, status = %status, "sync request failed");
        let body = response.text().await.unwrap_or_default();
        if is_push {
            return Err(SyncError::classify_push(status.as_u16(), &body, retry_after));
        }
        Err(SyncError::classify(status.as_u16(), &body, retry_after))
    }

    /// Decode a response body; empty bodies (204 etc.) yield `T::default()`.
    fn decode<T: DeserializeOwned + Default>(text: &str, path: &str) -> Result<T, SyncError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(T::default());
        }
        serde_json::from_str(trimmed)
            .map_err(|err| SyncError::Serialization(format!("decode response from {path}: {err}")))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Verb {
    Get,
    Post,
    Put,
    Delete,
}

impl Verb {
    fn as_str(self) -> &'static str {
        match self {
            Verb::Get => "GET",
            Verb::Post => "POST",
            Verb::Put => "PUT",
            Verb::Delete => "DELETE",
        }
    }
}

/// Read a clone of a locked value, tolerating a poisoned lock (the data
/// itself is never poisoned — we only panic-free paths here).
fn read_locked<T: Clone>(lock: &RwLock<T>) -> T {
    lock.read()
        .map(|guard| guard.clone())
        .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
}

/// Take the write guard, tolerating a poisoned lock.
fn write_locked<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_header_value_is_pure_and_correct() {
        assert_eq!(
            authorization_value(TokenScheme::Bearer, "tok-123"),
            "Bearer tok-123"
        );
        assert_eq!(
            authorization_value(TokenScheme::Device, "tok-123"),
            "DeviceToken tok-123"
        );
        assert_eq!(authorization_value(TokenScheme::Token, "t"), "Token t");
        assert_eq!(
            authorization_value(TokenScheme::TempToken, "t"),
            "TempToken t"
        );
        assert_eq!(TokenScheme::Bearer.as_str(), "Bearer");
    }

    #[test]
    fn client_builds_and_manages_tokens() {
        let client = SyncClient::new(DEFAULT_BASE_URL).expect("build client");
        assert_eq!(client.base_url(), DEFAULT_BASE_URL);
        assert_eq!(client.token_scheme(), TokenScheme::Bearer);
        assert!(client.token().is_none());
        assert!(!client.has_token());
        assert!(client.authorization_header_value().is_none());

        client.set_token(Some("secret-token".to_string()));
        assert!(client.has_token());
        assert_eq!(
            client.authorization_header_value().as_deref(),
            Some("Bearer secret-token")
        );

        client.clear_token();
        assert!(client.token().is_none());

        // an empty stored token must not produce a header
        client.set_token(Some(String::new()));
        assert!(client.authorization_header_value().is_none());
    }

    #[test]
    fn client_never_leaks_token_in_debug_output() {
        let client = SyncClient::new(DEFAULT_BASE_URL).expect("build client");
        client.set_token(Some("super-secret-token".to_string()));
        client.set_device_token("device-secret");
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("super-secret-token"));
        assert!(!rendered.contains("device-secret"));
        assert!(rendered.contains("has_token: true"));
    }

    #[test]
    fn device_token_is_generated_on_first_use_and_persisted_by_caller() {
        let client = SyncClient::new(DEFAULT_BASE_URL).expect("build client");
        let first = client.device_info().token;
        assert!(!first.is_empty());

        client.set_device_token("known-token");
        assert_eq!(client.device_info().token, "known-token");
        // still stable on later reads
        assert_eq!(client.device_info().token, "known-token");
    }

    #[test]
    fn builder_overrides_apply() {
        let client = SyncClient::builder()
            .base_url("https://dev.api.termius.com/")
            .client_version("10.1.3")
            .platform("Desktop")
            .token_scheme(TokenScheme::Device)
            .build()
            .expect("build client");
        assert_eq!(client.base_url(), "https://dev.api.termius.com/");
        assert_eq!(client.token_scheme(), TokenScheme::Device);
        client.set_token(Some("t".into()));
        assert_eq!(
            client.authorization_header_value().as_deref(),
            Some("DeviceToken t")
        );
    }

    #[test]
    fn joins_urls_without_double_slashes() {
        assert_eq!(
            join_url("https://api.termius.com", "/api/v4/terminal/sync/"),
            "https://api.termius.com/api/v4/terminal/sync/"
        );
        assert_eq!(
            join_url("https://api.termius.com/", "/api/v4/bulk/account/"),
            "https://api.termius.com/api/v4/bulk/account/"
        );
        assert_eq!(
            join_url("https://api.termius.com", "api/v3/account/profile/"),
            "https://api.termius.com/api/v3/account/profile/"
        );
        assert_eq!(join_url("http://x//", "//a"), "http://x//a");
    }

    #[test]
    fn endpoint_paths_are_recovered_verbatim() {
        assert_eq!(endpoints::SYNC, "/api/v4/terminal/sync/");
        assert_eq!(endpoints::DEVICE_LOGIN, "/api/v3.3/auth/device/login/");
        assert_eq!(endpoints::UPDATE_DEVICE, "/api/v3/user/device/current/");
        assert_eq!(endpoints::DEVICE_LOGOUT, "/api/v3/auth/device/logout/current/");
        assert_eq!(endpoints::BULK_ACCOUNT, "/api/v4/bulk/account/");
        assert_eq!(endpoints::USER_PROFILE, "/api/v3/account/profile/");
    }

    #[test]
    fn last_synced_round_trips_through_client_state() {
        let client = SyncClient::new(DEFAULT_BASE_URL).expect("build client");
        assert!(client.last_synced().is_none());
        client.set_last_synced(Some("2026-01-01T00:00:00Z".to_string()));
        assert_eq!(
            client.last_synced().as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        client.set_last_synced(None);
        assert!(client.last_synced().is_none());
    }
}
