//! Cloud provider integrations (Azure/AWS/GCP VM discovery), mirroring the
//! `@azure/*` SDK usage of the Termius Electron app (v10.1.3).
//!
//! Ported from the background process' cloud sync (recovered-background/
//! entry.js): the `fbe(cloud_config)` factory dispatches on
//! `cloud_config.cloudType` (`"azure"`, `"aws"`, `"DigitalOcean"`) to a
//! fetcher that returns `{ hosts: [...] }`, which the `resolveHosts` saga then
//! merges into the host list (matched by `cloud_instance_id`).
//!
//! * **Azure — implemented** ([`azure`]): service-principal (or raw-token)
//!   auth against `login.microsoftonline.com`, subscription listing, per-
//!   subscription VM listing via the ARM compute API, NIC/public-IP address
//!   resolution, and projection into [`termius_core::Host`]. Wire models live
//!   in [`models`], errors in [`error`].
//! * **AWS / GCP / DigitalOcean — stubs**: entry points return
//!   [`CloudError::Unsupported`] (see [`list_aws_hosts`] & friends).
//!
//! ```text
//! crates/termius-cloud/src/
//! ├── lib.rs     — module tree, CloudProvider, unsupported stubs
//! ├── error.rs   — CloudError (thiserror)
//! ├── models.rs  — serde wire models (token, subscriptions, VMs, NIC, public IP)
//! └── azure.rs   — AzureClient: auth → subscriptions → VMs → Host projection
//! ```

pub mod azure;
pub mod error;
pub mod models;

pub use azure::{
    arm_last_segment, arm_resource_group, arm_subscription_id, project_vm, AzureClient,
    AzureCredential, CloudHost, IpPreference, VmAddresses, AZURE_AUTHORITY_HOST,
    AZURE_MANAGEMENT_ROOT, AZURE_MANAGEMENT_SCOPE, UNRESOLVED_HOSTNAME,
};
pub use error::{CloudError, Result};

/// Cloud providers Termius' `cloud_config` dispatches on (`cloudType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudProvider {
    Azure,
    Aws,
    Gcp,
    DigitalOcean,
}

impl CloudProvider {
    /// The value Termius stores in `cloud_config.cloudType`.
    pub const fn as_str(self) -> &'static str {
        match self {
            CloudProvider::Azure => "azure",
            CloudProvider::Aws => "aws",
            // PORT-TODO: the JS factory `fbe` handles aws/DigitalOcean/azure
            // only — "gcp" is a placeholder name for when GCP lands.
            CloudProvider::Gcp => "gcp",
            CloudProvider::DigitalOcean => "DigitalOcean",
        }
    }
}

impl std::fmt::Display for CloudProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// AWS EC2 discovery — **not ported**. Termius drives this through the AWS
/// SDK from its `Wve` config (`accessKeyId`/`secretAccessKey`/`region`,
/// `serviceType: "ec2"`); see the `fbe` factory.
///
/// Always returns [`CloudError::Unsupported`].
pub async fn list_aws_hosts() -> Result<Vec<CloudHost>> {
    Err(CloudError::unsupported(CloudProvider::Aws.as_str()))
}

/// GCP Compute Engine discovery — **not ported** (the JS app bundles no GCP
/// cloud config either). Always returns [`CloudError::Unsupported`].
pub async fn list_gcp_hosts() -> Result<Vec<CloudHost>> {
    Err(CloudError::unsupported(CloudProvider::Gcp.as_str()))
}

/// DigitalOcean droplet discovery — **not ported**. The JS app has a `Vve`
/// cloud config (`cloudType: "DigitalOcean"`, single `token`) but its fetcher
/// is not among the recovered surfaces. Always returns
/// [`CloudError::Unsupported`].
pub async fn list_digitalocean_hosts() -> Result<Vec<CloudHost>> {
    Err(CloudError::unsupported(
        CloudProvider::DigitalOcean.as_str(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_strings_match_js_cloud_types() {
        assert_eq!(CloudProvider::Azure.as_str(), "azure");
        assert_eq!(CloudProvider::Aws.as_str(), "aws");
        assert_eq!(CloudProvider::DigitalOcean.as_str(), "DigitalOcean");
        assert_eq!(format!("{}", CloudProvider::Azure), "azure");
    }

    #[tokio::test]
    async fn unported_providers_report_unsupported() {
        assert!(matches!(
            list_aws_hosts().await,
            Err(CloudError::Unsupported { provider }) if provider == "aws"
        ));
        assert!(matches!(
            list_gcp_hosts().await,
            Err(CloudError::Unsupported { provider }) if provider == "gcp"
        ));
        assert!(matches!(
            list_digitalocean_hosts().await,
            Err(CloudError::Unsupported { provider }) if provider == "DigitalOcean"
        ));
    }
}
