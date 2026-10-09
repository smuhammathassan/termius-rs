//! Serde wire models for the Azure surfaces Termius touches: the AAD OAuth
//! token endpoint, `@azure/arm-subscriptions`, `@azure/arm-compute` (virtual
//! machines) and `@azure/arm-network` (NIC / public IP) — enough to discover
//! VMs and resolve an address for each (see [`crate::azure`]).
//!
//! Grounded in the SDKs bundled with the Termius app
//! (`analysis/termius-extracted/node_modules/@azure/*/dist-esm/src/models/mappers.js`):
//!
//! * ARM nests resource properties under a `properties` object — the compute
//!   mapper serialises the VM network profile as `"properties.networkProfile"`,
//!   the network mapper as `"properties.privateIPAddress"` — so the models
//!   mirror that nesting instead of the flattened TypeScript view the JS SDK
//!   hands to app code.
//! * List envelopes are `{ "value": [...], "nextLink": "..." }`.
//! * OAuth token responses are `snake_case` (`access_token`, `expires_in`),
//!   unlike ARM's `camelCase`.
//! * A few ARM field names keep the acronym capitalised (`privateIPAddress`,
//!   `publicIPAddress`, `type`) — `rename_all = "camelCase"` would get those
//!   wrong, so they carry explicit renames.
//!
//! Everything is best-effort tolerant: fields the port does not use are
//! ignored on read (serde's default), and `Option`-typed fields are never
//! required. Shapes that are only partially modelled carry a `PORT-TODO`.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

// ---------------------------------------------------------------------------
// OAuth token endpoint (login.microsoftonline.com .../oauth2/v2.0/token)
// ---------------------------------------------------------------------------

/// Access-token response for the `client_credentials` grant.
///
/// Fields are intentionally `snake_case` (no `rename_all`): Azure AD emits
/// snake_case here, not ARM's camelCase.
///
/// The [`fmt::Debug`] impl redacts `access_token` — never log the raw token.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenResponse {
    /// The ARM bearer token. Treat as a secret.
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    /// Lifetime in seconds.
    #[serde(default, deserialize_with = "flexible_opt_i64")]
    pub expires_in: Option<i64>,
    /// Extended lifetime for retry scenarios (seconds).
    #[serde(default, deserialize_with = "flexible_opt_i64")]
    pub ext_expires_in: Option<i64>,
    /// Absolute expiry as epoch seconds (AAD v1 sends it, v2 omits it).
    #[serde(default, deserialize_with = "flexible_opt_i64")]
    pub expires_on: Option<i64>,
    /// Absolute start as epoch seconds (AAD v1 only).
    #[serde(default, deserialize_with = "flexible_opt_i64")]
    pub not_before: Option<i64>,
    #[serde(default)]
    pub scope: Option<String>,
    /// AAD v1 resource indicator (`https://management.azure.com/`).
    #[serde(default)]
    pub resource: Option<String>,
    // PORT-TODO: `client_credentials` never issues a refresh_token; modelled
    // only because other AAD flows do. The port never uses it.
    #[serde(default)]
    pub refresh_token: Option<String>,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenResponse")
            .field("access_token", &"<redacted>")
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

/// Tolerant `Option<i64>`: AAD sends epoch fields as either a JSON number or a
/// numeric string depending on the endpoint/flow; anything else (including
/// `null`) becomes `None` instead of failing the whole token parse.
fn flexible_opt_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FlexibleInt {
        Int(i64),
        Str(String),
        // PORT-TODO: catch-all keeps odd IdP payloads parseable; the value is
        // dropped rather than failing auth.
        Other(serde_json::Value),
    }

    Ok(match FlexibleInt::deserialize(deserializer)? {
        FlexibleInt::Int(value) => Some(value),
        FlexibleInt::Str(value) => value.trim().parse::<i64>().ok(),
        FlexibleInt::Other(_) => None,
    })
}

// ---------------------------------------------------------------------------
// Subscriptions (GET /subscriptions, @azure/arm-subscriptions)
// ---------------------------------------------------------------------------

/// One entry of the subscription list (`subscriptions.list()` in the JS SDK).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    /// ARM resource id: `/subscriptions/{subscriptionId}`.
    #[serde(default)]
    pub id: Option<String>,
    /// The id used for every per-subscription ARM call. The JS fetcher only
    /// keeps entries where this is set.
    #[serde(default)]
    pub subscription_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    /// `Enabled` | `Disabled` | `Warned`.
    #[serde(default)]
    pub state: Option<String>,
    // PORT-TODO: `subscriptionPolicies`, `authorizationSource` and the
    // `subscriptionId`-adjacent fields of newer api-versions are not modelled
    // (discovery only needs the id + display name).
}

/// ARM list envelope shared by every paged resource: `value` + `nextLink`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResponse<T> {
    pub value: Vec<T>,
    /// Absolute URL of the next page; absent on the last page.
    #[serde(default)]
    pub next_link: Option<String>,
}

/// `GET /subscriptions?api-version=2016-06-01`
pub type SubscriptionList = ListResponse<Subscription>;

// ---------------------------------------------------------------------------
// Compute — virtual machines (GET /subscriptions/{sub}/providers/Microsoft.Compute/virtualMachines)
// ---------------------------------------------------------------------------

/// `GET .../Microsoft.Compute/virtualMachines` page. Mirrors the JS SDK's
/// flattened `VirtualMachine` view (`id`/`name`/`location` at the top level,
/// everything else under `properties`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualMachine {
    /// Full ARM id: `/subscriptions/{sub}/resourceGroups/{rg}/providers/Microsoft.Compute/virtualMachines/{name}`.
    /// Termius stores this as `cloud_instance_id` for host dedupe.
    #[serde(default)]
    pub id: Option<String>,
    /// VM name — Termius' host `label`.
    #[serde(default)]
    pub name: Option<String>,
    /// Azure region (`eastus`, …).
    #[serde(default)]
    pub location: Option<String>,
    /// ARM type discriminator: `Microsoft.Compute/virtualMachines`.
    #[serde(rename = "type", default)]
    pub vm_type: Option<String>,
    #[serde(default)]
    pub properties: Option<VmProperties>,
    // PORT-TODO: `tags`, `identity`, `zones`, `resources` are ignored — the
    // Termius host projection does not consume them.
}

/// `properties` of a virtual machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmProperties {
    #[serde(default)]
    pub network_profile: Option<NetworkProfile>,
    #[serde(default)]
    pub storage_profile: Option<StorageProfile>,
    #[serde(default)]
    pub os_profile: Option<OsProfile>,
    #[serde(default)]
    pub provisioning_state: Option<String>,
    // PORT-TODO: `hardwareProfile` (vmSize), `diagnosticsProfile`,
    // `instanceView` (power state) not modelled — useful for richer host
    // notes/icons later.
}

/// `properties.networkProfile` — NICs attached to the VM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkProfile {
    #[serde(default)]
    pub network_interfaces: Vec<NetworkInterfaceReference>,
}

/// A NIC attachment; only the ARM `id` is needed (name + resource group are
/// parsed from it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterfaceReference {
    #[serde(default)]
    pub id: Option<String>,
}

/// `properties.storageProfile`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageProfile {
    #[serde(default)]
    pub os_disk: Option<OsDisk>,
    // PORT-TODO: `imageReference` (publisher/offer/sku) not modelled — useful
    // for per-distro username/icon hints later.
}

/// `properties.storageProfile.osDisk`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsDisk {
    /// `Linux` | `Windows` — Termius' host `os_name`.
    #[serde(default)]
    pub os_type: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// `properties.osProfile` — login hints for the projected host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsProfile {
    #[serde(default)]
    pub computer_name: Option<String>,
    /// The VM's admin user, when ARM returns it.
    // PORT-TODO: confirm `adminUsername` is present in *list* responses (it is
    // in single-VM GETs); the projection falls back to an OS-based hint when
    // absent. `adminPassword` is deliberately not modelled — ARM never returns
    // it and the port must never handle it.
    #[serde(default)]
    pub admin_username: Option<String>,
}

/// `GET /subscriptions/{sub}/providers/Microsoft.Compute/virtualMachines` page.
pub type VirtualMachineList = ListResponse<VirtualMachine>;

// ---------------------------------------------------------------------------
// Network — NIC + public IP (@azure/arm-network), used for address resolution
// ---------------------------------------------------------------------------

/// `GET .../Microsoft.Network/networkInterfaces/{name}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterface {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub properties: Option<NetworkInterfaceProperties>,
}

/// `properties` of a NIC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterfaceProperties {
    #[serde(default)]
    pub ip_configurations: Vec<IpConfiguration>,
    // PORT-TODO: `networkSecurityGroup`, `dnsSettings`, `virtualMachine`
    // back-reference not modelled.
}

/// One of the NIC's IP configurations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpConfiguration {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub properties: Option<IpConfigurationProperties>,
}

/// `properties` of an IP configuration.
///
/// NOTE: the two address fields keep ARM's acronym casing — plain
/// `camelCase` would produce `privateIpAddress`/`publicIpAddress`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpConfigurationProperties {
    /// The VM's private IP — the fallback hostname when no public IP exists.
    #[serde(rename = "privateIPAddress", default)]
    pub private_ip_address: Option<String>,
    /// `Static` | `Dynamic`.
    #[serde(rename = "privateIPAllocationMethod", default)]
    pub private_ip_allocation_method: Option<String>,
    /// Reference to the public IP resource, when one is attached.
    #[serde(rename = "publicIPAddress", default)]
    pub public_ip_address: Option<ResourceRef>,
    // PORT-TODO: `subnet`, `loadBalancerBackendAddressPools`, … not modelled.
}

/// Bare ARM resource reference (`{"id": "..."}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRef {
    #[serde(default)]
    pub id: Option<String>,
}

/// `GET .../Microsoft.Network/publicIPAddresses/{name}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicIpAddress {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub properties: Option<PublicIpAddressProperties>,
}

/// `properties` of a public IP resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicIpAddressProperties {
    /// The address Termius uses as the host's `hostname` (JS behaviour).
    #[serde(default)]
    pub ip_address: Option<String>,
    /// `Static` | `Dynamic` — wire name keeps the `IP` acronym capitalised.
    #[serde(rename = "publicIPAllocationMethod", default)]
    pub public_ip_allocation_method: Option<String>,
    // PORT-TODO: `dnsSettings.domainNameLabel`/`fqdn` (a nicer hostname than
    // the raw IP) and `idleTimeoutInMinutes` are not modelled yet.
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN_JSON: &str = r#"{
        "access_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.secret.signature",
        "token_type": "Bearer",
        "expires_in": 3599,
        "ext_expires_in": 3599,
        "expires_on": "1760003999",
        "not_before": "1760000400",
        "resource": "https://management.azure.com/"
    }"#;

    const SUBSCRIPTIONS_JSON: &str = r#"{
        "value": [
            {
                "id": "/subscriptions/11111111-1111-1111-1111-111111111111",
                "subscriptionId": "11111111-1111-1111-1111-111111111111",
                "displayName": "Pay-As-You-Go",
                "state": "Enabled"
            }
        ],
        "nextLink": "https://management.azure.com/subscriptions?api-version=2016-06-01&$skipToken=abc"
    }"#;

    const VM_JSON: &str = r#"{
        "id": "/subscriptions/11111111-1111-1111-1111-111111111111/resourceGroups/rg-demo/providers/Microsoft.Compute/virtualMachines/vm-web-1",
        "name": "vm-web-1",
        "type": "Microsoft.Compute/virtualMachines",
        "location": "eastus",
        "properties": {
            "provisioningState": "Succeeded",
            "osProfile": {
                "computerName": "vm-web-1",
                "adminUsername": "deploy"
            },
            "storageProfile": {
                "osDisk": {
                    "osType": "Linux",
                    "name": "vm-web-1_OsDisk_1"
                }
            },
            "networkProfile": {
                "networkInterfaces": [
                    {
                        "id": "/subscriptions/11111111-1111-1111-1111-111111111111/resourceGroups/rg-demo/providers/Microsoft.Network/networkInterfaces/vm-web-1-nic"
                    }
                ]
            }
        }
    }"#;

    const NIC_JSON: &str = r#"{
        "id": "/subscriptions/11111111-1111-1111-1111-111111111111/resourceGroups/rg-demo/providers/Microsoft.Network/networkInterfaces/vm-web-1-nic",
        "name": "vm-web-1-nic",
        "location": "eastus",
        "properties": {
            "ipConfigurations": [
                {
                    "id": "/subscriptions/11111111-1111-1111-1111-111111111111/resourceGroups/rg-demo/providers/Microsoft.Network/networkInterfaces/vm-web-1-nic/ipConfigurations/ipconfig1",
                    "name": "ipconfig1",
                    "properties": {
                        "privateIPAddress": "10.0.0.4",
                        "privateIPAllocationMethod": "Dynamic",
                        "publicIPAddress": {
                            "id": "/subscriptions/11111111-1111-1111-1111-111111111111/resourceGroups/rg-demo/providers/Microsoft.Network/publicIPAddresses/vm-web-1-ip"
                        }
                    }
                }
            ]
        }
    }"#;

    const PUBLIC_IP_JSON: &str = r#"{
        "id": "/subscriptions/11111111-1111-1111-1111-111111111111/resourceGroups/rg-demo/providers/Microsoft.Network/publicIPAddresses/vm-web-1-ip",
        "name": "vm-web-1-ip",
        "location": "eastus",
        "properties": {
            "ipAddress": "20.1.2.3",
            "publicIPAllocationMethod": "Static"
        }
    }"#;

    #[test]
    fn token_response_round_trip() {
        let token: TokenResponse =
            serde_json::from_str(TOKEN_JSON).expect("token payload should parse");
        assert_eq!(token.expires_in, Some(3599));
        assert_eq!(token.expires_on, Some(1760003999));
        assert!(token.access_token.starts_with("eyJ"));

        let encoded = serde_json::to_string(&token).expect("token should serialize");
        let decoded: TokenResponse = serde_json::from_str(&encoded).expect("re-parse");
        assert_eq!(token, decoded);
    }

    #[test]
    fn token_response_accepts_string_expires_in() {
        // Some IdPs send numeric fields as strings.
        let json = r#"{"access_token":"t","token_type":"Bearer","expires_in":"3599"}"#;
        let token: TokenResponse = serde_json::from_str(json).expect("should parse");
        assert_eq!(token.expires_in, Some(3599));

        let encoded = serde_json::to_string(&token).expect("serialize");
        let decoded: TokenResponse = serde_json::from_str(&encoded).expect("re-parse");
        assert_eq!(token, decoded);
    }

    #[test]
    fn token_response_tolerates_garbage_epoch_fields() {
        let json = r#"{"access_token":"t","expires_in":null,"expires_on":true}"#;
        let token: TokenResponse = serde_json::from_str(json).expect("should parse");
        assert_eq!(token.expires_in, None);
        assert_eq!(token.expires_on, None);
    }

    #[test]
    fn token_response_debug_redacts_access_token() {
        let token: TokenResponse =
            serde_json::from_str(TOKEN_JSON).expect("token payload should parse");
        let debugged = format!("{token:?}");
        assert!(!debugged.contains("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9"));
        assert!(debugged.contains("<redacted>"));
    }

    #[test]
    fn subscription_list_round_trip() {
        let page: SubscriptionList =
            serde_json::from_str(SUBSCRIPTIONS_JSON).expect("subscription page should parse");
        assert_eq!(page.value.len(), 1);
        let sub = &page.value[0];
        assert_eq!(
            sub.subscription_id.as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(sub.display_name.as_deref(), Some("Pay-As-You-Go"));
        assert!(page.next_link.as_deref().is_some_and(|l| l.contains("$skipToken")));

        let encoded = serde_json::to_string(&page).expect("serialize");
        let decoded: SubscriptionList = serde_json::from_str(&encoded).expect("re-parse");
        assert_eq!(page, decoded);
    }

    #[test]
    fn vm_list_round_trip_with_nested_properties() {
        let vm: VirtualMachine = serde_json::from_str(VM_JSON).expect("vm should parse");
        assert_eq!(vm.name.as_deref(), Some("vm-web-1"));
        assert_eq!(vm.vm_type.as_deref(), Some("Microsoft.Compute/virtualMachines"));
        assert_eq!(vm.location.as_deref(), Some("eastus"));

        let props = vm.properties.as_ref().expect("properties");
        let nic_id = props
            .network_profile
            .as_ref()
            .and_then(|n| n.network_interfaces.first())
            .and_then(|n| n.id.as_deref())
            .expect("nic id");
        assert!(nic_id.contains("/networkInterfaces/vm-web-1-nic"));
        assert_eq!(
            props
                .storage_profile
                .as_ref()
                .and_then(|s| s.os_disk.as_ref())
                .and_then(|d| d.os_type.as_deref()),
            Some("Linux")
        );
        assert_eq!(
            props
                .os_profile
                .as_ref()
                .and_then(|o| o.admin_username.as_deref()),
            Some("deploy")
        );

        // Round-trip through the list envelope the pager actually uses.
        let list = VirtualMachineList {
            value: vec![vm],
            next_link: None,
        };
        let encoded = serde_json::to_string(&list).expect("serialize");
        let decoded: VirtualMachineList = serde_json::from_str(&encoded).expect("re-parse");
        assert_eq!(list, decoded);
    }

    #[test]
    fn vm_without_properties_parses_with_defaults() {
        let vm: VirtualMachine =
            serde_json::from_str(r#"{"name":"bare","location":"westus"}"#).expect("parse");
        assert_eq!(vm.name.as_deref(), Some("bare"));
        assert!(vm.properties.is_none());
        assert!(vm.id.is_none());
    }

    #[test]
    fn network_interface_keeps_arm_acronym_casing() {
        let nic: NetworkInterface = serde_json::from_str(NIC_JSON).expect("nic should parse");
        let props = nic.properties.as_ref().expect("properties");
        let ip = props
            .ip_configurations
            .first()
            .and_then(|c| c.properties.as_ref())
            .expect("ip configuration");
        // The wire names are privateIPAddress/publicIPAddress — a plain
        // camelCase rename would have produced privateIpAddress and missed.
        assert_eq!(ip.private_ip_address.as_deref(), Some("10.0.0.4"));
        let public_ref = ip.public_ip_address.as_ref().expect("public ip ref");
        assert!(public_ref
            .id
            .as_deref()
            .is_some_and(|id| id.contains("/publicIPAddresses/vm-web-1-ip")));

        let encoded = serde_json::to_string(&nic).expect("serialize");
        let decoded: NetworkInterface = serde_json::from_str(&encoded).expect("re-parse");
        assert_eq!(nic, decoded);
    }

    #[test]
    fn public_ip_address_parses_ip() {
        let pip: PublicIpAddress =
            serde_json::from_str(PUBLIC_IP_JSON).expect("public ip should parse");
        let props = pip.properties.as_ref().expect("properties");
        assert_eq!(props.ip_address.as_deref(), Some("20.1.2.3"));
        assert_eq!(props.public_ip_allocation_method.as_deref(), Some("Static"));

        let encoded = serde_json::to_string(&pip).expect("serialize");
        let decoded: PublicIpAddress = serde_json::from_str(&encoded).expect("re-parse");
        assert_eq!(pip, decoded);
    }
}
