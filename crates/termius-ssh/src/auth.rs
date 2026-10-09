//! auth — see crate docs.
//!
//! Maps `termius_core::AuthMethod` (the data the Termius UI persists) onto the
//! `russh` authentication surface. The original addon (`libtermius`) exposed
//! `auth(...)` with the same four shapes: none / password / OpenSSH-PEM
//! private key (+ optional passphrase) / ssh-agent.

use std::sync::Arc;

use russh::client::{AuthResult, Handle, Handler};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg};
use termius_core::AuthMethod;
use tracing::{debug, instrument};

use crate::error::{Result, SshError};

/// How a connection intends to authenticate, derived from
/// [`AuthMethod::None`], [`AuthMethod::Password`],
/// [`AuthMethod::PrivateKey`] and [`AuthMethod::Agent`].
///
/// Kept as its own type so the mapping (and its edge cases) can be unit
/// tested without a live connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthPlan {
    /// Try the "none" authentication method (servers that allow passwordless
    /// logins grant it on the first request).
    None,
    Password(String),
    /// Private key material (PEM / OpenSSH) with an optional passphrase.
    PrivateKey { material: String, passphrase: Option<String> },
    /// Sign via the OS ssh-agent reachable at `SSH_AUTH_SOCK`.
    Agent,
}

/// Pure mapping from the domain model onto the engine's auth plan.
pub fn plan(auth: &AuthMethod) -> AuthPlan {
    match auth {
        AuthMethod::None => AuthPlan::None,
        AuthMethod::Password(password) => AuthPlan::Password(password.clone()),
        AuthMethod::PrivateKey { private_key, passphrase } => AuthPlan::PrivateKey {
            material: private_key.clone(),
            passphrase: passphrase.clone(),
        },
        AuthMethod::Agent => AuthPlan::Agent,
    }
}

/// `true` when the string looks like PEM / OpenSSH private key material
/// rather than a filesystem path.
pub fn looks_like_private_key(material: &str) -> bool {
    material.contains("PRIVATE KEY")
}

/// Load a private key from either inline PEM/OpenSSH material or a path.
///
/// Termius stores the key *content* in its vault; russh 0.64 exposes
/// `russh::keys::decode_secret_key(&str, Option<&str>)` for in-memory
/// parsing, so PEM blobs are decoded directly (no temp file) and paths go
/// through `russh::keys::load_secret_key`.
fn load_key(material: &str, passphrase: Option<&String>) -> Result<russh::keys::PrivateKey> {
    let passphrase = passphrase.map(String::as_str);
    if looks_like_private_key(material) {
        russh::keys::decode_secret_key(material, passphrase)
            .map_err(|e| SshError::auth("private key", format!("failed to load private key: {e}")))
    } else {
        russh::keys::load_secret_key(material, passphrase)
            .map_err(|e| SshError::auth("private key", format!("failed to load private key: {e}")))
    }
}

/// Pick the RSA hash algorithm the server supports (`server-sig-algs`);
/// `None` means "no extension info — try and hope". Non-RSA keys ignore the
/// hash and russh maps `None` to the legacy `ssh-rsa` for them.
async fn rsa_hash_alg<H: Handler>(handle: &Handle<H>) -> Option<HashAlg> {
    handle
        .best_supported_rsa_hash()
        .await
        .map(|best| best.unwrap_or(None))
        .unwrap_or(None)
}

/// Finish an authentication attempt, preserving the failure reason.
///
/// (russh 0.64: `authenticate_*` resolves to `russh::client::AuthResult`,
/// which carries `success()`; transport failures are separate `Err`s.)
fn check_outcome(method: &str, outcome: &AuthResult) -> Result<()> {
    if outcome.success() {
        debug!(method, "ssh authentication accepted");
        Ok(())
    } else {
        Err(SshError::auth(method, "server rejected credentials"))
    }
}

/// ssh-agent authentication (`SSH_AUTH_SOCK`).
///
/// russh 0.64 ships an agent client (`russh::keys::agent::client::AgentClient`,
/// with `connect_env()` on unix) implementing `russh::auth::Signer`, wired into
/// the handshake through `Handle::authenticate_publickey_with`. We enumerate
/// the agent's identities and try each until one is accepted.
async fn authenticate_agent<H: Handler>(handle: &mut Handle<H>, username: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use russh::keys::agent::client::AgentClient;

        let mut agent = AgentClient::connect_env().await.map_err(|e| {
            SshError::auth("agent", format!("failed to connect to ssh-agent: {e}"))
        })?;
        let identities = agent.request_identities().await.map_err(|e| {
            SshError::auth("agent", format!("failed to list ssh-agent identities: {e}"))
        })?;
        if identities.is_empty() {
            return Err(SshError::auth("agent", "ssh-agent offered no identities"));
        }
        let mut last_failure = None;
        for identity in &identities {
            let public_key = identity.public_key().into_owned();
            let hash_alg = if matches!(
                public_key.algorithm(),
                russh::keys::Algorithm::Rsa { .. }
            ) {
                rsa_hash_alg(handle).await
            } else {
                None
            };
            match handle
                .authenticate_publickey_with(username, public_key, hash_alg, &mut agent)
                .await
            {
                Ok(outcome) => {
                    if outcome.success() {
                        debug!(method = "agent", "ssh authentication accepted");
                        return Ok(());
                    }
                    last_failure = Some("server rejected the agent identity");
                }
                Err(e) => return Err(SshError::auth("agent", e.to_string())),
            }
        }
        Err(SshError::auth(
            "agent",
            last_failure.unwrap_or("server rejected all agent identities"),
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = handle;
        let sock = std::env::var("SSH_AUTH_SOCK").ok();
        Err(SshError::auth(
            "agent",
            format!(
                "ssh-agent authentication is not wired up on this platform yet (PORT-TODO){}",
                sock.map(|s| format!(" (agent socket: {s})")).unwrap_or_default()
            ),
        ))
    }
}

/// Run the SSH user authentication exchange for the given [`AuthMethod`].
///
/// Every failure keeps its phase: a transport error while authenticating is
/// reported as an auth failure of that method (with the underlying reason),
/// a refusal by the server is `SshError::Authentication`.
#[instrument(skip_all, fields(username = %username, method = ?auth))]
pub async fn authenticate<H: Handler>(
    handle: &mut Handle<H>,
    username: &str,
    auth: &AuthMethod,
) -> Result<()> {
    match plan(auth) {
        AuthPlan::None => {
            let outcome = handle
                .authenticate_none(username)
                .await
                .map_err(|e| SshError::auth("none", e.to_string()))?;
            check_outcome("none", &outcome)
        }
        AuthPlan::Password(password) => {
            let outcome = handle
                .authenticate_password(username, password.as_str())
                .await
                .map_err(|e| SshError::auth("password", e.to_string()))?;
            check_outcome("password", &outcome)
        }
        AuthPlan::PrivateKey { material, passphrase } => {
            let key = load_key(&material, passphrase.as_ref())?;
            // russh 0.64: `authenticate_publickey` takes a
            // `PrivateKeyWithHashAlg` (Arc'd key + optional RSA hash alg).
            let hash_alg = if matches!(key.algorithm(), russh::keys::Algorithm::Rsa { .. }) {
                rsa_hash_alg(handle).await
            } else {
                None
            };
            let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
            let outcome = handle
                .authenticate_publickey(username, key)
                .await
                .map_err(|e| SshError::auth("private key", e.to_string()))?;
            check_outcome("private key", &outcome)
        }
        AuthPlan::Agent => authenticate_agent(handle, username).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::MethodSet;

    #[test]
    fn maps_every_auth_method() {
        assert_eq!(plan(&AuthMethod::None), AuthPlan::None);
        assert_eq!(
            plan(&AuthMethod::Password("s3cret".into())),
            AuthPlan::Password("s3cret".into())
        );
        assert_eq!(
            plan(&AuthMethod::PrivateKey {
                private_key: "MIGH...".into(),
                passphrase: Some("pp".into()),
            }),
            AuthPlan::PrivateKey {
                material: "MIGH...".into(),
                passphrase: Some("pp".into()),
            }
        );
        assert_eq!(
            plan(&AuthMethod::PrivateKey { private_key: "p".into(), passphrase: None }),
            AuthPlan::PrivateKey { material: "p".into(), passphrase: None }
        );
        assert_eq!(plan(&AuthMethod::Agent), AuthPlan::Agent);
    }

    #[test]
    fn distinguishes_pem_material_from_paths() {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXkta2V5\n-----END OPENSSH PRIVATE KEY-----";
        assert!(looks_like_private_key(pem));
        assert!(looks_like_private_key("-----BEGIN RSA PRIVATE KEY-----\n..."));
        assert!(!looks_like_private_key("/Users/dev/.ssh/id_ed25519"));
        assert!(!looks_like_private_key("~/.ssh/id_rsa"));
    }

    #[test]
    fn check_outcome_keeps_auth_reason() {
        let err = check_outcome(
            "password",
            &AuthResult::Failure {
                remaining_methods: MethodSet::empty(),
                partial_success: false,
            },
        )
        .expect_err("failure must be rejected");
        assert!(err.is_authentication());
        assert!(err.to_string().contains("password"));

        assert!(check_outcome("password", &AuthResult::Success).is_ok());
    }
}
