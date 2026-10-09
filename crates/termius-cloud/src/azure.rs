//! Azure integration: ARM discovery (auth → subscriptions → virtual machines)
//! projected into [`termius_core::Host`].
//!
//! This is a partial port of the `@azure/*` surface the Termius background
//! process uses (recovered-background/entry.js, class `yst` + the
//! `fbe(cloud_config)` factory):
//!
//! 1. **Auth** — Termius stores `tenant_id`/`client_id`/`client_secret` in
//!    `cloud_config` (`cloudType: "azure"`) and passes them to `@azure/identity`'s
//!    `ClientSecretCredential`. Here that is [`AzureCredential::ServicePrincipal`],
//!    which performs the OAuth2 `client_credentials` flow against
//!    `login.microsoftonline.com/{tenant}/oauth2/v2.0/token` with scope
//!    `https://management.azure.com/.default` and caches the bearer token
//!    until shortly before it expires. [`AzureCredential::StaticToken`] covers
//!    the "bring your own token" path (managed identity, `az` CLI, a token file).
//! 2. **Subscriptions** — `GET /subscriptions?api-version=2016-06-01`, following
//!    `nextLink` (the JS `SubscriptionClient.subscriptions.list()` paged
//!    iterator).
//! 3. **Virtual machines** — per subscription,
//!    `GET /subscriptions/{sub}/providers/Microsoft.Compute/virtualMachines?api-version=2022-08-01`
//!    (the JS `ComputeManagementClient.virtualMachines.listAll()`), then
//!    address resolution through `@azure/arm-network`: NIC → (private IP,
//!    public IP resource) → public address. JS behaviour (`getVmConfig`) is to
//!    use the public IP; the private IP is modelled as the fallback.
//! 4. **Projection** — [`project_vm`] builds a [`CloudHost`]: label = VM
//!    name, hostname = resolved address, username = the VM's admin user hint,
//!    plus the `cloud_instance_*` metadata Termius dedupes hosts on.
//!
//! Secret hygiene: tokens and client secrets never appear in `Debug` output,
//! error payloads, or (this crate logs nothing at all) any log line.

use std::fmt;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use termius_core::Host;

use crate::error::{CloudError, Result};
use crate::models::{
    ListResponse, NetworkInterface, PublicIpAddress, Subscription, TokenResponse, VirtualMachine,
};

/// ARM endpoint of the public cloud.
pub const AZURE_MANAGEMENT_ROOT: &str = "https://management.azure.com";
/// AAD authority used by `@azure/identity`'s `ClientSecretCredential`.
pub const AZURE_AUTHORITY_HOST: &str = "https://login.microsoftonline.com";
/// OAuth scope for ARM on the public cloud.
pub const AZURE_MANAGEMENT_SCOPE: &str = "https://management.azure.com/.default";
/// Hostname used when no address could be resolved for a VM. Termius' JS drops
/// such VMs entirely; the port keeps them so the user can fill the address in
/// (filter on this constant if drop-on-unresolved is preferred).
pub const UNRESOLVED_HOSTNAME: &str = "<unresolved-ip>";

// API versions pinned to the SDKs bundled with the Termius app
// (analysis/termius-extracted/node_modules/@azure/*/dist-esm/src/models/parameters.js):
// arm-subscriptions 5.1.0 → 2016-06-01, arm-compute 19.2.0 → 2022-08-01,
// arm-network 30.2.0 → 2022-09-01.
const SUBSCRIPTIONS_API_VERSION: &str = "2016-06-01";
const COMPUTE_API_VERSION: &str = "2022-08-01";
const NETWORK_API_VERSION: &str = "2022-09-01";

/// Refresh the cached token this long before `expires_in` elapses.
const TOKEN_EXPIRY_SKEW: Duration = Duration::from_secs(60);
/// Lifetime assumed when a token response omits `expires_in`.
const DEFAULT_TOKEN_TTL_SECS: i64 = 3600;
/// Guard against a misbehaving/malformed `nextLink` loop.
const MAX_PAGES: usize = 1024;

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// Credential an [`AzureClient`] authenticates with.
#[derive(Clone)]
pub enum AzureCredential {
    /// AAD service principal — Termius' `cloud_config` triple
    /// (`tenant_id`, `client_id`, `client_secret`), the JS `ClientSecretCredential`.
    ServicePrincipal {
        tenant_id: String,
        client_id: String,
        /// Secret — never logged, never printed by `Debug`.
        client_secret: String,
    },
    /// A pre-minted ARM bearer token: managed-identity endpoint output, `az
    /// account get-access-token`, a token file, …
    // PORT-TODO: `@azure/identity`'s `DefaultAzureCredential` chain (managed
    // identity IMDS, environment, CLI, workload identity) — only raw-token
    // passthrough is ported; callers obtain the token themselves for now.
    StaticToken(String),
}

impl AzureCredential {
    /// Mirror of the JS `getUserIdentity(config)`: a service principal is only
    /// usable when tenant id, client id **and** client secret are all present
    /// and non-blank — otherwise `None` (JS then yields no hosts at all).
    pub fn service_principal(
        tenant_id: &str,
        client_id: &str,
        client_secret: &str,
    ) -> Option<Self> {
        let present = |value: &str| !value.trim().is_empty();
        if present(tenant_id) && present(client_id) && present(client_secret) {
            Some(AzureCredential::ServicePrincipal {
                tenant_id: tenant_id.to_string(),
                client_id: client_id.to_string(),
                client_secret: client_secret.to_string(),
            })
        } else {
            None
        }
    }
}

impl fmt::Debug for AzureCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Deliberately redacted: Debug must never carry a secret.
        match self {
            AzureCredential::ServicePrincipal {
                tenant_id,
                client_id,
                ..
            } => f
                .debug_struct("ServicePrincipal")
                .field("tenant_id", tenant_id)
                .field("client_id", client_id)
                .field("client_secret", &"<redacted>")
                .finish(),
            AzureCredential::StaticToken(_) => f
                .debug_struct("StaticToken")
                .field("access_token", &"<redacted>")
                .finish(),
        }
    }
}

/// Cached bearer token. `expires_at: None` means "no known expiry" (static
/// tokens). `Debug` is redacted.
#[derive(Clone)]
struct CachedToken {
    token: String,
    expires_at: Option<Instant>,
}

impl CachedToken {
    fn is_fresh(&self) -> bool {
        self.expires_at
            .map(|deadline| Instant::now() < deadline)
            .unwrap_or(true)
    }
}

impl fmt::Debug for CachedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedToken")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at.map(|_| "<set>"))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Address resolution + host projection
// ---------------------------------------------------------------------------

/// Which address of a VM to use as the host `hostname`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IpPreference {
    /// Public IP first, private IP as fallback — Termius' JS behaviour (the
    /// AWS config exposes this as `ipAddressType: "public"`).
    #[default]
    PreferPublic,
    /// Private IP first (VPN/VNet-only discovery).
    PreferPrivate,
}

/// Addresses resolved for one VM. Either may be `None`; [`VmAddresses::hostname`]
/// then falls back to [`UNRESOLVED_HOSTNAME`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VmAddresses {
    pub public_ip: Option<String>,
    pub private_ip: Option<String>,
}

impl VmAddresses {
    /// `true` when at least one usable address was found.
    pub fn is_resolved(&self) -> bool {
        let usable = |value: &Option<String>| {
            value.as_deref().map(str::trim).is_some_and(|v| !v.is_empty())
        };
        usable(&self.public_ip) || usable(&self.private_ip)
    }

    /// The hostname for this VM under `preference`, honouring the other
    /// address as fallback, else [`UNRESOLVED_HOSTNAME`].
    pub fn hostname(&self, preference: IpPreference) -> String {
        let (primary, fallback) = match preference {
            IpPreference::PreferPublic => (&self.public_ip, &self.private_ip),
            IpPreference::PreferPrivate => (&self.private_ip, &self.public_ip),
        };
        for candidate in [primary, fallback] {
            if let Some(value) = candidate.as_deref() {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
        UNRESOLVED_HOSTNAME.to_string()
    }
}

/// A discovered cloud VM projected to a Termius host, plus the cloud metadata
/// Termius keeps *on the host entity* (`cloud_instance_id`,
/// `cloud_instance_type`, `os_name`) that [`termius_core::Host`] does not
/// model yet.
///
/// PORT-TODO: `termius-core` (owned by another wave) should grow these three
/// fields; until then callers must persist them alongside the host (the JS
/// `resolveHosts` saga matches existing hosts by `cloud_instance_id`).
#[derive(Debug, Clone, PartialEq)]
pub struct CloudHost {
    /// The saved-host entry (label / hostname / username / port 22 / SSH).
    pub host: Host,
    /// Full ARM id of the VM — Termius' `cloud_instance_id`.
    pub cloud_instance_id: String,
    /// `"azure"` — Termius' `cloud_instance_type`.
    pub cloud_instance_type: String,
    /// OS type from the OS disk (`Linux` / `Windows`) — Termius' `os_name`.
    pub os_name: String,
}

/// Project an ARM VM (already resolved to [`VmAddresses`]) into a
/// [`CloudHost`]. Mirrors the JS host construction in `getVms`: label = VM
/// name, address = resolved IP, `cloud_instance_id = vm.id`,
/// `os_name = storageProfile.osDisk.osType`.
pub fn project_vm(
    vm: &VirtualMachine,
    addresses: &VmAddresses,
    preference: IpPreference,
) -> CloudHost {
    // PORT-TODO: `id`/`created_at`/`updated_at` stay empty — storage assigns
    // the RecordId and timestamps on insert (this crate has no uuid/clock dep
    // and the JS saga writes through the same repositories).
    let mut host = Host::default(); // port 22, HostType::Ssh, empty groups/ids
    host.label = vm_name(vm);
    host.hostname = addresses.hostname(preference);
    host.username = username_hint(vm);

    CloudHost {
        host,
        cloud_instance_id: vm.id.clone().unwrap_or_default(),
        cloud_instance_type: crate::CloudProvider::Azure.as_str().to_string(),
        os_name: os_type(vm).unwrap_or_default(),
    }
}

/// VM name; falls back to the last ARM id segment when `name` is absent.
fn vm_name(vm: &VirtualMachine) -> String {
    if let Some(name) = vm.name.as_deref() {
        if !name.trim().is_empty() {
            return name.trim().to_string();
        }
    }
    vm.id.as_deref()
        .and_then(arm_last_segment)
        .unwrap_or_default()
        .to_string()
}

fn os_type(vm: &VirtualMachine) -> Option<String> {
    vm.properties
        .as_ref()?
        .storage_profile
        .as_ref()?
        .os_disk
        .as_ref()?
        .os_type
        .clone()
}

/// Login-user hint for the projected host: the VM's admin username when ARM
/// returns it, else an OS-based default.
///
/// PORT-TODO: the JS port never sets a username (`ssh_config` starts empty);
/// per-image defaults (Ubuntu image → `ubuntu`, …) can be refined once an OS
/// image catalog is ported.
fn username_hint(vm: &VirtualMachine) -> String {
    if let Some(user) = vm
        .properties
        .as_ref()
        .and_then(|p| p.os_profile.as_ref())
        .and_then(|o| o.admin_username.as_deref())
    {
        let trimmed = user.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if os_type(vm).is_some_and(|os| os.eq_ignore_ascii_case("windows")) {
        // Windows has no ARM-default admin name — the portal forces a choice.
        String::new()
    } else {
        // Azure CLI/portal default admin placeholder for Linux images.
        "azureuser".to_string()
    }
}

// ---------------------------------------------------------------------------
// ARM resource-id helpers (JS: `ta(id.split("/"))` / `id.split("/")[4]`)
// ---------------------------------------------------------------------------

/// Non-empty segments of an ARM resource id
/// (`/subscriptions/S/resourceGroups/R/providers/...` →
/// `["subscriptions", "S", "resourceGroups", "R", ...]`).
fn arm_segments(id: &str) -> Vec<&str> {
    id.split('/').filter(|segment| !segment.is_empty()).collect()
}

/// Last non-empty segment — the resource name (JS `ta(...)`).
pub fn arm_last_segment(id: &str) -> Option<&str> {
    arm_segments(id).last().copied()
}

/// Segment following `resourceGroups` — the resource group (JS `split("/")[4]`).
pub fn arm_resource_group(id: &str) -> Option<&str> {
    let segments = arm_segments(id);
    segments
        .iter()
        .position(|segment| segment.eq_ignore_ascii_case("resourceGroups"))
        .and_then(|index| segments.get(index + 1))
        .copied()
}

/// Segment following `subscriptions` — the subscription id.
pub fn arm_subscription_id(id: &str) -> Option<&str> {
    let segments = arm_segments(id);
    segments
        .iter()
        .position(|segment| segment.eq_ignore_ascii_case("subscriptions"))
        .and_then(|index| segments.get(index + 1))
        .copied()
}

// ---------------------------------------------------------------------------
// URL / body encoding
// ---------------------------------------------------------------------------

fn subscription_url(root: &str) -> String {
    format!(
        "{}/subscriptions?api-version={}",
        root.trim_end_matches('/'),
        SUBSCRIPTIONS_API_VERSION
    )
}

fn vm_list_url(root: &str, subscription_id: &str) -> String {
    format!(
        "{}/subscriptions/{}/providers/Microsoft.Compute/virtualMachines?api-version={}",
        root.trim_end_matches('/'),
        encode_path_segment(subscription_id),
        COMPUTE_API_VERSION
    )
}

fn nic_url(root: &str, subscription_id: &str, resource_group: &str, nic_name: &str) -> String {
    format!(
        "{}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.Network/networkInterfaces/{}?api-version={}",
        root.trim_end_matches('/'),
        encode_path_segment(subscription_id),
        encode_path_segment(resource_group),
        encode_path_segment(nic_name),
        NETWORK_API_VERSION
    )
}

fn public_ip_url(root: &str, subscription_id: &str, resource_group: &str, ip_name: &str) -> String {
    format!(
        "{}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.Network/publicIPAddresses/{}?api-version={}",
        root.trim_end_matches('/'),
        encode_path_segment(subscription_id),
        encode_path_segment(resource_group),
        encode_path_segment(ip_name),
        NETWORK_API_VERSION
    )
}

fn token_url(authority: &str, tenant_id: &str) -> String {
    format!(
        "{}/{}/oauth2/v2.0/token",
        authority.trim_end_matches('/'),
        encode_path_segment(tenant_id)
    )
}

/// Percent-encode a path segment (RFC 3986 unreserved characters kept as-is).
fn encode_path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `application/x-www-form-urlencoded` body encoding for the token request
/// (WHATWG rules: unreserved + `*` kept, space → `+`, everything else
/// `%XX`). Written by hand so the client secret is encoded correctly without
/// pulling in an extra dependency for one request.
fn form_urlencode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", form_component(key), form_component(value)))
        .collect::<Vec<String>>()
        .join("&")
}

fn form_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// ARM client: token acquisition + discovery calls + host projection.
pub struct AzureClient {
    http: reqwest::Client,
    credential: AzureCredential,
    authority: String,
    management_root: String,
    scope: String,
    ip_preference: IpPreference,
    /// Never read while holding across an await except in [`Self::access_token`].
    cached_token: tokio::sync::Mutex<Option<CachedToken>>,
}

impl AzureClient {
    /// Build a client with the public-cloud defaults.
    pub fn new(credential: AzureCredential) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(format!("termius-rs/{}", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http,
            credential,
            authority: AZURE_AUTHORITY_HOST.to_string(),
            management_root: AZURE_MANAGEMENT_ROOT.to_string(),
            scope: AZURE_MANAGEMENT_SCOPE.to_string(),
            ip_preference: IpPreference::default(),
            cached_token: tokio::sync::Mutex::new(None),
        })
    }

    /// PORT-TODO: national-cloud presets (US Gov, China, Germany) change
    /// authority + management root + scope together; only manual overrides
    /// exist. `@azure/identity` also honours `AZURE_AUTHORITY_HOST` — not read
    /// here yet.
    pub fn with_authority(mut self, authority: impl Into<String>) -> Self {
        self.authority = authority.into();
        self
    }

    /// Override the ARM endpoint (national clouds / emulators like Azurite).
    pub fn with_management_root(mut self, root: impl Into<String>) -> Self {
        self.management_root = root.into();
        self
    }

    /// Override the OAuth scope (must match `with_management_root`).
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    /// Which address wins when a VM resolves to both (default: public first).
    pub fn with_ip_preference(mut self, preference: IpPreference) -> Self {
        self.ip_preference = preference;
        self
    }

    /// The configured address preference.
    pub fn ip_preference(&self) -> IpPreference {
        self.ip_preference
    }

    /// Obtain (and cache) an ARM bearer token.
    ///
    /// The token is a secret — callers must not log it or put it in errors.
    pub async fn access_token(&self) -> Result<String> {
        {
            let guard = self.cached_token.lock().await;
            if let Some(cached) = guard.as_ref() {
                if cached.is_fresh() {
                    return Ok(cached.token.clone());
                }
            }
        }
        let minted = self.mint_token().await?;
        let token = minted.token.clone();
        *self.cached_token.lock().await = Some(minted);
        Ok(token)
    }

    /// OAuth `client_credentials` exchange (or static-token passthrough).
    async fn mint_token(&self) -> Result<CachedToken> {
        match &self.credential {
            AzureCredential::StaticToken(token) => Ok(CachedToken {
                token: token.clone(),
                expires_at: None,
            }),
            AzureCredential::ServicePrincipal {
                tenant_id,
                client_id,
                client_secret,
            } => {
                let body = form_urlencode(&[
                    ("grant_type", "client_credentials"),
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                    ("scope", self.scope.as_str()),
                ]);
                let response = self
                    .http
                    .post(token_url(&self.authority, tenant_id))
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .header("Accept", "application/json")
                    .body(body)
                    .send()
                    .await?;
                let status = response.status();
                if !status.is_success() {
                    // The failure body carries AAD's `error`/`error_description`
                    // (never a token) — safe to turn into a reason.
                    let text = response.text().await.unwrap_or_default();
                    return Err(CloudError::from_token_endpoint(status.as_u16(), &text));
                }
                let token: TokenResponse = response.json().await?;
                let ttl = token
                    .expires_in
                    .filter(|secs| *secs > 0)
                    .unwrap_or(DEFAULT_TOKEN_TTL_SECS);
                let skew = TOKEN_EXPIRY_SKEW.as_secs() as i64;
                let usable = ttl.saturating_sub(skew).max(0) as u64;
                Ok(CachedToken {
                    token: token.access_token,
                    expires_at: Some(Instant::now() + Duration::from_secs(usable)),
                })
            }
        }
    }

    /// GET a JSON resource with a fresh bearer token; non-success statuses are
    /// classified via [`CloudError::from_status`].
    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let token = self.access_token().await?;
        let response = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json")
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(CloudError::from_status(status.as_u16(), &body));
        }
        Ok(response.json::<T>().await?)
    }

    /// Follow `nextLink` until the pager ends (the JS SDK's paged iterator).
    async fn get_all_pages<T: DeserializeOwned>(&self, first_url: &str, what: &str) -> Result<Vec<T>> {
        let mut url = first_url.to_string();
        let mut out: Vec<T> = Vec::new();
        let mut pages = 0usize;
        loop {
            pages += 1;
            if pages > MAX_PAGES {
                return Err(CloudError::Network(format!(
                    "stopped following nextLink after {MAX_PAGES} pages while listing {what}"
                )));
            }
            let page: ListResponse<T> = self.get_json(&url).await?;
            out.extend(page.value);
            match page.next_link {
                Some(next) if !next.is_empty() => {
                    // ARM normally returns an absolute nextLink; older
                    // api-versions can return a rooted path.
                    url = if next.starts_with("http://") || next.starts_with("https://") {
                        next
                    } else if let Some(rest) = next.strip_prefix('/') {
                        format!("{}{rest}", self.management_root.trim_end_matches('/'))
                    } else {
                        next
                    };
                }
                _ => break,
            }
        }
        Ok(out)
    }

    /// List every subscription visible to the credential
    /// (`SubscriptionClient.subscriptions.list()`).
    pub async fn list_subscriptions(&self) -> Result<Vec<Subscription>> {
        self.get_all_pages(&subscription_url(&self.management_root), "subscriptions")
            .await
    }

    /// List the VMs of one subscription
    /// (`ComputeManagementClient.virtualMachines.listAll()`).
    pub async fn list_virtual_machines(&self, subscription_id: &str) -> Result<Vec<VirtualMachine>> {
        self.get_all_pages(
            &vm_list_url(&self.management_root, subscription_id),
            "virtual machines",
        )
        .await
    }

    /// Resolve a VM's addresses through `@azure/arm-network`: NIC → private IP
    /// and (when attached) the public IP resource's address.
    ///
    /// Mirrors the JS `getVmConfig`: parse the first NIC id out of
    /// `networkProfile`, GET it, take the first IP configuration that has a
    /// `publicIPAddress`, then GET that public IP. The private IP (which the JS
    /// code ignores) is captured as the fallback.
    ///
    /// A VM without a NIC id yields empty addresses rather than an error — the
    /// JS code returns `{}` there (PORT-TODO: the JS then *drops* the VM; this
    /// port keeps it with [`UNRESOLVED_HOSTNAME`]).
    pub async fn resolve_vm_addresses(&self, vm: &VirtualMachine) -> Result<VmAddresses> {
        let nic_ref = vm
            .properties
            .as_ref()
            .and_then(|p| p.network_profile.as_ref())
            .and_then(|np| np.network_interfaces.first())
            .and_then(|nic| nic.id.as_deref());
        let Some(nic_id) = nic_ref else {
            return Ok(VmAddresses::default());
        };
        let Some(nic_name) = arm_last_segment(nic_id) else {
            return Ok(VmAddresses::default());
        };
        // PORT-TODO: the JS looks the NIC up with the *VM's* resource group
        // (`vm.id.split("/")[4]`); the NIC id carries its own subscription +
        // resource group, which is correct even when they differ.
        let (subscription, resource_group) = match (
            arm_subscription_id(nic_id),
            arm_resource_group(nic_id),
        ) {
            (Some(subscription), Some(resource_group)) => (subscription, resource_group),
            _ => return Ok(VmAddresses::default()),
        };

        let nic: NetworkInterface = self
            .get_json(&nic_url(
                &self.management_root,
                subscription,
                resource_group,
                nic_name,
            ))
            .await?;

        let mut private_ip: Option<String> = None;
        let mut public_ip_ref: Option<String> = None;
        for configuration in nic.properties.map(|p| p.ip_configurations).unwrap_or_default() {
            let Some(properties) = configuration.properties else {
                continue;
            };
            if private_ip.is_none() {
                private_ip = properties.private_ip_address.filter(non_empty);
            }
            if public_ip_ref.is_none() {
                public_ip_ref = properties
                    .public_ip_address
                    .and_then(|reference| reference.id)
                    .filter(non_empty);
            }
        }

        // JS: `networkInterfaces.get` … `publicIPAddresses.get(...).ipAddress`.
        let public_ip = match public_ip_ref {
            Some(ip_id) => match (
                arm_subscription_id(&ip_id),
                arm_resource_group(&ip_id),
                arm_last_segment(&ip_id),
            ) {
                (Some(ip_sub), Some(ip_group), Some(ip_name)) => {
                    let address: PublicIpAddress = self
                        .get_json(&public_ip_url(
                            &self.management_root,
                            ip_sub,
                            ip_group,
                            ip_name,
                        ))
                        .await?;
                    address
                        .properties
                        .and_then(|props| props.ip_address)
                        .filter(non_empty)
                }
                _ => None,
            },
            None => None,
        };

        Ok(VmAddresses {
            public_ip,
            private_ip,
        })
    }

    /// Resolve + project one VM in one call (honours the client's
    /// [`IpPreference`]).
    pub async fn resolve_vm_host(&self, vm: &VirtualMachine) -> Result<CloudHost> {
        let addresses = self.resolve_vm_addresses(vm).await?;
        Ok(project_vm(vm, &addresses, self.ip_preference))
    }

    /// The full Termius cloud sync: subscriptions → per-subscription VMs →
    /// address resolution → [`CloudHost`]s (`yst.fetch(config)`).
    ///
    /// 1:1 with the JS behaviour, including propagating any ARM failure so the
    /// caller can surface it (JS wraps it in `CloudSyncError`), and the
    /// JS `getUserIdentity` guard — with no credential there is no client at
    /// all, so an empty credential simply means no [`AzureClient`] is built.
    ///
    /// PORT-TODO: the JS attaches returned hosts to the selected group in the
    /// `resolveHosts` saga (grouping happens above this layer); the JS also
    /// drops VMs whose public IP could not be resolved, whereas this keeps
    /// them with [`UNRESOLVED_HOSTNAME`].
    pub async fn fetch_hosts(&self) -> Result<Vec<CloudHost>> {
        let subscriptions = self.list_subscriptions().await?;
        let mut hosts = Vec::new();
        for subscription in subscriptions {
            // JS: `if (a.subscriptionId) push(...)`.
            let Some(subscription_id) = subscription
                .subscription_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            for vm in self.list_virtual_machines(subscription_id).await? {
                hosts.push(self.resolve_vm_host(&vm).await?);
            }
        }
        Ok(hosts)
    }
}

impl fmt::Debug for AzureClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Credential's Debug is already redacted; token state is reported as a
        // boolean so neither can leak.
        let token_cached = self
            .cached_token
            .try_lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false);
        f.debug_struct("AzureClient")
            .field("credential", &self.credential)
            .field("authority", &self.authority)
            .field("management_root", &self.management_root)
            .field("ip_preference", &self.ip_preference)
            .field("token_cached", &token_cached)
            .finish()
    }
}

/// `Option<String>` filter: drop whitespace-only values.
fn non_empty(value: &String) -> bool {
    !value.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{OsDisk, StorageProfile};
    use termius_core::host::HostType;

    const VM_JSON: &str = r#"{
        "id": "/subscriptions/11111111-1111-1111-1111-111111111111/resourceGroups/rg-demo/providers/Microsoft.Compute/virtualMachines/vm-web-1",
        "name": "vm-web-1",
        "type": "Microsoft.Compute/virtualMachines",
        "location": "eastus",
        "properties": {
            "osProfile": {"computerName": "vm-web-1", "adminUsername": "deploy"},
            "storageProfile": {"osDisk": {"osType": "Linux", "name": "osdisk"}},
            "networkProfile": {"networkInterfaces": [{
                "id": "/subscriptions/11111111-1111-1111-1111-111111111111/resourceGroups/rg-demo/providers/Microsoft.Network/networkInterfaces/vm-web-1-nic"
            }]}
        }
    }"#;

    fn sample_vm() -> VirtualMachine {
        serde_json::from_str(VM_JSON).expect("sample vm should parse")
    }

    // -- id helpers ---------------------------------------------------------

    #[test]
    fn arm_id_helpers_parse_js_equivalents() {
        let nic_id = "/subscriptions/11111111-1111-1111-1111-111111111111/resourceGroups/rg-demo/providers/Microsoft.Network/networkInterfaces/vm-web-1-nic";
        // JS: `ta(id.split("/"))`
        assert_eq!(arm_last_segment(nic_id), Some("vm-web-1-nic"));
        // JS: `id.split("/")[4]`
        assert_eq!(arm_resource_group(nic_id), Some("rg-demo"));
        assert_eq!(
            arm_subscription_id(nic_id),
            Some("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(arm_resource_group("/subscriptions/x"), None);
        assert_eq!(arm_last_segment(""), None);
    }

    // -- encoders -----------------------------------------------------------

    #[test]
    fn form_urlencode_escapes_secrets() {
        assert_eq!(
            form_urlencode(&[
                ("grant_type", "client_credentials"),
                ("client_secret", "p@ss +&=/wör")
            ]),
            "grant_type=client_credentials&client_secret=p%40ss+%2B%26%3D%2Fw%C3%B6r"
        );
        // A token request body must never contain raw separators.
        let body = form_urlencode(&[("client_secret", "a&b=c")]);
        assert_eq!(body, "client_secret=a%26b%3Dc");
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(encode_path_segment("rg demo/x"), "rg%20demo%2Fx");
        assert_eq!(encode_path_segment("11111111-1111-1111-1111-111111111111"), "11111111-1111-1111-1111-111111111111");
    }

    #[test]
    fn urls_carry_the_pinned_api_versions() {
        assert_eq!(
            subscription_url("https://management.azure.com"),
            "https://management.azure.com/subscriptions?api-version=2016-06-01"
        );
        assert!(vm_list_url("https://management.azure.com/", "sub-1")
            .ends_with("Microsoft.Compute/virtualMachines?api-version=2022-08-01"));
        assert!(nic_url("https://management.azure.com", "sub-1", "rg", "nic")
            .contains("Microsoft.Network/networkInterfaces/nic?api-version=2022-09-01"));
        assert!(public_ip_url("https://management.azure.com", "sub-1", "rg", "ip")
            .contains("Microsoft.Network/publicIPAddresses/ip?api-version=2022-09-01"));
        assert_eq!(
            token_url("https://login.microsoftonline.com/", "tenant-x"),
            "https://login.microsoftonline.com/tenant-x/oauth2/v2.0/token"
        );
    }

    // -- credentials --------------------------------------------------------

    #[test]
    fn partial_service_principal_is_rejected() {
        assert!(AzureCredential::service_principal("", "c", "s").is_none());
        assert!(AzureCredential::service_principal("t", "", "s").is_none());
        assert!(AzureCredential::service_principal("t", "c", " ").is_none());
        assert!(AzureCredential::service_principal("t", "c", "s").is_some());
    }

    #[test]
    fn credential_debug_redacts_secret() {
        let credential =
            AzureCredential::service_principal("tenant", "client", "top-secret-value")
                .expect("complete config");
        let debugged = format!("{credential:?}");
        assert!(!debugged.contains("top-secret-value"));
        assert!(debugged.contains("<redacted>"));

        let static_credential = AzureCredential::StaticToken("raw-token".to_string());
        let debugged = format!("{static_credential:?}");
        assert!(!debugged.contains("raw-token"));
    }

    // -- projection ---------------------------------------------------------

    #[test]
    fn projects_public_ip_preferred() {
        let vm = sample_vm();
        let addresses = VmAddresses {
            public_ip: Some("20.1.2.3".to_string()),
            private_ip: Some("10.0.0.4".to_string()),
        };
        let cloud_host = project_vm(&vm, &addresses, IpPreference::PreferPublic);
        let host = &cloud_host.host;
        assert_eq!(host.label, "vm-web-1");
        assert_eq!(host.hostname, "20.1.2.3");
        assert_eq!(host.username, "deploy");
        assert_eq!(host.port, 22);
        assert_eq!(host.host_type, HostType::Ssh);
        assert!(host.notes.is_none());
        assert_eq!(
            cloud_host.cloud_instance_id,
            "/subscriptions/11111111-1111-1111-1111-111111111111/resourceGroups/rg-demo/providers/Microsoft.Compute/virtualMachines/vm-web-1"
        );
        assert_eq!(cloud_host.cloud_instance_type, "azure");
        assert_eq!(cloud_host.os_name, "Linux");
    }

    #[test]
    fn projects_private_ip_when_public_missing() {
        let vm = sample_vm();
        let addresses = VmAddresses {
            public_ip: None,
            private_ip: Some("10.0.0.4".to_string()),
        };
        let cloud_host = project_vm(&vm, &addresses, IpPreference::PreferPublic);
        assert_eq!(cloud_host.host.hostname, "10.0.0.4");
        assert!(addresses.is_resolved());

        // PreferPrivate flips a dual-address VM to its private IP.
        let dual = VmAddresses {
            public_ip: Some("20.1.2.3".to_string()),
            private_ip: Some("10.0.0.4".to_string()),
        };
        let cloud_host = project_vm(&vm, &dual, IpPreference::PreferPrivate);
        assert_eq!(cloud_host.host.hostname, "10.0.0.4");
    }

    #[test]
    fn projects_placeholder_when_unresolved() {
        let vm = sample_vm();
        let addresses = VmAddresses::default();
        assert!(!addresses.is_resolved());
        let cloud_host = project_vm(&vm, &addresses, IpPreference::PreferPublic);
        // Placeholder must be non-empty so Host::validate() still passes.
        assert_eq!(cloud_host.host.hostname, UNRESOLVED_HOSTNAME);
        assert!(cloud_host.host.validate().is_ok());
    }

    #[test]
    fn username_hint_falls_back_per_os() {
        let addresses = VmAddresses {
            public_ip: Some("20.1.2.3".to_string()),
            private_ip: None,
        };

        // adminUsername from osProfile wins.
        let vm = sample_vm();
        let cloud_host = project_vm(&vm, &addresses, IpPreference::PreferPublic);
        assert_eq!(cloud_host.host.username, "deploy");

        // No adminUsername, Linux → Azure's default admin placeholder.
        let mut vm = sample_vm();
        vm.properties.as_mut().expect("properties").os_profile = None;
        let cloud_host = project_vm(&vm, &addresses, IpPreference::PreferPublic);
        assert_eq!(cloud_host.host.username, "azureuser");

        // Windows has no ARM default admin name → empty hint.
        let mut vm = sample_vm();
        let properties = vm.properties.as_mut().expect("properties");
        properties.storage_profile = Some(StorageProfile {
            os_disk: Some(OsDisk {
                os_type: Some("Windows".to_string()),
                name: None,
            }),
        });
        if let Some(os_profile) = properties.os_profile.as_mut() {
            os_profile.admin_username = None;
        }
        let cloud_host = project_vm(&vm, &addresses, IpPreference::PreferPublic);
        assert_eq!(cloud_host.host.username, "");
    }

    #[test]
    fn label_falls_back_to_arm_id_segment() {
        let mut vm = sample_vm();
        vm.name = None;
        let cloud_host = project_vm(&vm, &VmAddresses::default(), IpPreference::PreferPublic);
        assert_eq!(cloud_host.host.label, "vm-web-1");
    }

    // -- auth ---------------------------------------------------------------

    #[tokio::test]
    async fn static_token_is_served_without_network() {
        let client =
            AzureClient::new(AzureCredential::StaticToken("tok-123".to_string())).expect("client");
        assert_eq!(client.access_token().await.expect("token"), "tok-123");
        // Second call hits the cache — still no network involved.
        assert_eq!(client.access_token().await.expect("token"), "tok-123");
        let debugged = format!("{client:?}");
        assert!(!debugged.contains("tok-123"));
    }
}
