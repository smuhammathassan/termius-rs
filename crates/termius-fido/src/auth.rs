//! CTAP2 authenticator operations plus the WebAuthn data types shared with
//! the SSH and UI crates.
//!
//! In the Electron app the hot path is `@termius/libfido2`'s native `sign` /
//! `generate` calls (OpenSSH `ssh-sk` semantics over C `libfido2`); the CTAP2
//! `getAssertion` / `makeCredential` operations modelled here are their
//! WebAuthn equivalents. No crypto is faked: every operation currently
//! returns [`FidoError::NotImplemented`] with the reason, and the structs are
//! real serde types the rest of the workspace can store and exchange today.

use serde::{Deserialize, Serialize};

use crate::device::FidoDevice;
use crate::error::{FidoError, Result};

/// WebAuthn relying party (`rp`) — the scope a credential is bound to.
///
/// For SSH `sk-*` keys the rp id corresponds to the credential's
/// `application` string (e.g. `ssh:` / `ssh:hostname`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelyingParty {
    /// `rp.id` — host/credential scope.
    pub id: String,
    /// `rp.name` — human-readable name shown by authenticator prompts.
    pub name: String,
}

impl RelyingParty {
    /// Build a relying party from its id and display name.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self { id: id.into(), name: name.into() }
    }
}

/// WebAuthn `user` account handle the credential is registered to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserHandle {
    /// User id bytes (base64 on the wire).
    #[serde(with = "crate::serde_util")]
    pub id: Vec<u8>,
    /// Login-style user name (`user.name`).
    pub name: String,
    /// Human-readable name (`user.displayName`).
    pub display_name: String,
}

/// COSE signature algorithm a credential is bound to
/// (SSH `sk-*` keys support `ed25519` and `es256`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialAlgorithm {
    /// ECDSA w/ SHA-256 over P-256 (COSE `-7`, `sk-ecdsa-sha2-nistp256@openssh.com`).
    Es256,
    /// EdDSA (COSE `-8`, `sk-ssh-ed25519@openssh.com`).
    EdDsa,
    /// RSASSA-PKCS1-v1_5 w/ SHA-256 (COSE `-257`).
    Rs256,
}

/// Result of [`FidoAuthenticator::make_credential`] — a registered,
/// resident-capable credential (WebAuthn `PublicKeyCredential` shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicKeyCredential {
    /// Credential id / `keyHandle` (base64 on the wire).
    #[serde(with = "crate::serde_util")]
    pub credential_id: Vec<u8>,
    /// Raw encoded public key (COSE_Key; base64 on the wire).
    #[serde(with = "crate::serde_util")]
    pub public_key: Vec<u8>,
    /// Relying party the credential is bound to.
    pub relying_party: RelyingParty,
    /// Algorithm the credential signs with.
    pub algorithm: CredentialAlgorithm,
    /// Signature counter at creation time.
    pub counter: u32,
}

/// Result of [`FidoAuthenticator::assert`] — proof of credential possession.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assertion {
    /// Credential that produced the assertion (base64 on the wire).
    #[serde(with = "crate::serde_util")]
    pub credential_id: Vec<u8>,
    /// CTAP2 `authData` bytes (base64 on the wire).
    #[serde(with = "crate::serde_util")]
    pub authenticator_data: Vec<u8>,
    /// Signature over `authenticatorData || challenge` (base64 on the wire).
    #[serde(with = "crate::serde_util")]
    pub signature: Vec<u8>,
    /// Signature counter at assertion time (cloning detection).
    pub counter: u32,
}

/// FIDO2 operations used by the SSH unlock / UI wizard flows.
///
/// The API is stable; the hardware backend is deferred. Construct with
/// [`FidoAuthenticator::new`] for "any available key", or
/// [`FidoAuthenticator::with_device`] to pin a specific device.
#[derive(Debug, Clone, Default)]
pub struct FidoAuthenticator {
    /// Specific key to talk to; `None` means "first available key".
    device: Option<FidoDevice>,
}

impl FidoAuthenticator {
    /// Authenticator that will use the first available key.
    pub fn new() -> Self {
        Self::default()
    }

    /// Authenticator pinned to one specific security key.
    pub fn with_device(device: FidoDevice) -> Self {
        Self { device: Some(device) }
    }

    /// The device this authenticator is pinned to, if any.
    pub fn device(&self) -> Option<&FidoDevice> {
        self.device.as_ref()
    }

    /// Whether a usable security key is available on this machine.
    pub fn is_available(&self) -> bool {
        FidoDevice::is_available()
    }

    /// CTAP2 `getAssertion` — prove possession of a credential scoped to
    /// `relying_party`, signing `challenge`.
    ///
    /// PORT-TODO: implement over the real backend — open the device,
    /// hash/forward the challenge, run `getAssertion` with `allowList`
    /// (and PIN/UV when `DeviceInfo::pin_required`), and map user-cancel
    /// to `FidoError::Cancelled`, keepalive/transport failures to
    /// `FidoError::Io`, CTAP2 status codes to `FidoError::Protocol`.
    pub fn assert(&self, relying_party: &RelyingParty, challenge: &[u8]) -> Result<Assertion> {
        // PORT-TODO: real CTAP2 getAssertion over the device transport (allowList,
        // PIN/UV, user-cancel -> FidoError::Cancelled, status -> FidoError::Protocol).
        Err(FidoError::not_implemented(format!(
            "CTAP2 getAssertion for rp '{}' ({}-byte challenge): no CTAP2/HID \
             backend linked yet — see PORT-TODO in termius-fido::device",
            relying_party.id,
            challenge.len()
        )))
    }

    /// CTAP2 `makeCredential` — register a new credential for
    /// `relying_party` under `user`, keyed to `challenge`.
    ///
    /// PORT-TODO: implement over the real backend — resident-key
    /// enrollment, PIN/UV prompt, algorithm selection (Ed25519/ES256),
    /// and translate the addon's `generate`/`loadResidentKeys`/
    /// `loadSKMetadata` behaviour for SSH `sk-*` key parity.
    pub fn make_credential(
        &self,
        relying_party: &RelyingParty,
        user: &UserHandle,
        challenge: &[u8],
    ) -> Result<PublicKeyCredential> {
        // PORT-TODO: real CTAP2 makeCredential (resident key, PIN/UV, Ed25519/ES256)
        // + parity with the addon's generate/loadResidentKeys/loadSKMetadata.
        Err(FidoError::not_implemented(format!(
            "CTAP2 makeCredential for rp '{}' user '{}' ({}-byte challenge): \
             no CTAP2/HID backend linked yet — see PORT-TODO in \
             termius-fido::device",
            relying_party.id,
            user.name,
            challenge.len()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_assertion() -> Assertion {
        Assertion {
            credential_id: vec![1, 2, 3, 4],
            authenticator_data: b"authdata".to_vec(),
            signature: b"sig-bytes".to_vec(),
            counter: 7,
        }
    }

    #[test]
    fn assert_stub_is_explicitly_not_implemented() {
        let auth = FidoAuthenticator::new();
        let rp = RelyingParty::new("example.com", "Example");
        let err = auth.assert(&rp, b"0123456789abcdef").expect_err("backend deferred");
        assert!(err.is_not_implemented());
        let message = err.to_string();
        assert!(message.contains("getAssertion"), "message: {message}");
        assert!(message.contains("example.com"), "message: {message}");
    }

    #[test]
    fn make_credential_stub_is_explicitly_not_implemented() {
        let auth = FidoAuthenticator::new();
        let rp = RelyingParty::new("example.com", "Example");
        let user = UserHandle { id: vec![9], name: "alice".to_string(), display_name: "Alice".to_string() };
        let err = auth.make_credential(&rp, &user, b"challenge").expect_err("backend deferred");
        assert!(err.is_not_implemented());
        let message = err.to_string();
        assert!(message.contains("makeCredential"), "message: {message}");
        assert!(message.contains("alice"), "message: {message}");
    }

    #[test]
    fn new_authenticator_has_no_pinned_device() {
        assert!(FidoAuthenticator::new().device().is_none());
        assert!(!FidoAuthenticator::new().is_available());
    }

    #[test]
    fn assertion_json_round_trip_is_base64_and_camel_case() {
        let assertion = sample_assertion();
        let json = serde_json::to_string(&assertion).expect("serialize");
        // [1, 2, 3, 4] -> "AQIDBA==" (JS `Buffer#toString("base64")` compatible).
        assert!(json.contains("\"credentialId\":\"AQIDBA==\""), "json: {json}");
        assert!(json.contains("\"authenticatorData\":"), "json: {json}");
        assert!(json.contains("\"counter\":7"), "json: {json}");

        let back: Assertion = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, assertion);
        assert_eq!(back.signature, b"sig-bytes".to_vec());
    }

    #[test]
    fn public_key_credential_round_trip_preserves_nested_relying_party() {
        let credential = PublicKeyCredential {
            credential_id: vec![0xAA, 0xBB],
            public_key: b"raw-cose-key".to_vec(),
            relying_party: RelyingParty::new("ssh.example.com", "Termius SSH ID"),
            algorithm: CredentialAlgorithm::EdDsa,
            counter: 1,
        };
        let json = serde_json::to_string(&credential).expect("serialize");
        assert!(json.contains("\"relyingParty\":"), "json: {json}");
        assert!(json.contains("\"algorithm\":\"eddsa\""), "json: {json}");

        let back: PublicKeyCredential = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, credential);
        assert_eq!(back.relying_party.id, "ssh.example.com");
    }

    #[test]
    fn user_handle_serializes_display_name_camel_case() {
        let user = UserHandle {
            id: b"user-id".to_vec(),
            name: "alice@example.com".to_string(),
            display_name: "Alice".to_string(),
        };
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&user).expect("serialize"))
                .expect("parse");
        assert_eq!(value["displayName"], "Alice");
        assert_eq!(value["name"], "alice@example.com");
        assert_eq!(value["id"], "dXNlci1pZA=="); // "user-id" in base64
    }
}
