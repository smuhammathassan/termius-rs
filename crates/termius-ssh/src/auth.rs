//! auth — see crate docs.
//!
//! Maps `termius_core::AuthMethod` (the data the Termius UI persists) onto the
//! `russh` authentication surface. The original addon (`libtermius`) exposed
//! `auth(...)` with the same four shapes: none / password / OpenSSH-PEM
//! private key (+ optional passphrase) / ssh-agent.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use russh::client::{Handle, Handler};
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

/// Normalize a `russh` authentication outcome to a boolean.
///
/// PORT-TODO: the return type of `russh`'s `authenticate_*` methods changed
/// across releases (`bool` in older versions, `AuthResult` in newer ones).
/// Matching on the `Debug` rendering accepts either shape without pinning the
/// crate to one exact signature — `bool` prints `true`/`false` and
/// `AuthResult::Success` prints `Success`. If `russh` 0.64 exposes a typed
/// result, replace this helper with an exhaustive `match`.
fn outcome_succeeded(outcome: impl std::fmt::Debug) -> bool {
    let rendered = format!("{outcome:?}");
    rendered == "true" || rendered == "Success"
}

/// Finish an authentication attempt, preserving the failure reason.
fn check_outcome(method: &str, outcome: impl std::fmt::Debug) -> Result<()> {
    if outcome_succeeded(outcome) {
        debug!(method, "ssh authentication accepted");
        Ok(())
    } else {
        Err(SshError::auth(method, "server rejected credentials"))
    }
}

static TEMP_KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Materialize key material to a path `russh` can load.
///
/// Returns `(path, is_temporary)`; callers must delete the file when
/// `is_temporary` is `true`.
///
/// Termius stores the key *content* in its vault, while `russh`'s
/// `load_secret_key` reads from disk — so PEM blobs are written to a
/// per-process temp file with `0600` permissions, loaded, and removed before
/// authentication even starts.
///
/// PORT-TODO: if `russh` 0.64 exposes an in-memory parser (e.g.
/// `PrivateKey::from_openssh(...)`), drop the temp file entirely.
fn key_material_to_path(material: &str) -> Result<(PathBuf, bool)> {
    if !looks_like_private_key(material) {
        // Not a PEM blob — treat it as a path the user configured.
        return Ok((PathBuf::from(material), false));
    }

    for _ in 0..4 {
        let unique = TEMP_KEY_COUNTER.fetch_add(1, Ordering::Relaxed);
        // Rebuilt each iteration so a retry never nests a second component
        // onto a previously pushed path.
        let mut path = std::env::temp_dir();
        path.push(format!("termius-rs-key-{}-{unique}", std::process::id()));

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(material.as_bytes()).map_err(|e| {
                    let _ = std::fs::remove_file(&path);
                    SshError::auth("private key", format!("failed to stage private key: {e}"))
                })?;
                return Ok((path, true));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(SshError::auth(
                    "private key",
                    format!("failed to stage private key: {e}"),
                ));
            }
        }
    }
    Err(SshError::auth("private key", "failed to stage private key: temp name collision"))
}

/// ssh-agent authentication (`SSH_AUTH_SOCK`).
///
/// PORT-TODO: `russh`'s agent client (`russh::keys::agent::client::AgentClient`
/// + a `Handle::authenticate_agent`-style call, feature-gated in some
/// releases) is the intended wiring here; the exact 0.64 surface could not be
/// verified from this environment, so we fail with a precise, actionable
/// reason instead of guessing an API that may not compile. The env-var check
/// and error shape are the real Termius behavior (no agent → auth error).
async fn authenticate_agent(username: &str) -> Result<()> {
    let sock = std::env::var("SSH_AUTH_SOCK").map_err(|_| {
        SshError::auth("agent", "SSH_AUTH_SOCK is not set; no ssh-agent available")
    })?;
    Err(SshError::auth(
        "agent",
        format!("ssh-agent at {sock} is not wired into russh yet (PORT-TODO) for user {username}"),
    ))
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
            check_outcome("none", outcome)
        }
        AuthPlan::Password(password) => {
            let outcome = handle
                .authenticate_password(username, password.as_str())
                .await
                .map_err(|e| SshError::auth("password", e.to_string()))?;
            check_outcome("password", outcome)
        }
        AuthPlan::PrivateKey { material, passphrase } => {
            let (path, temporary) = key_material_to_path(&material)?;
            // PORT-TODO: `load_secret_key` is assumed synchronous
            // (`path, Option<String>`); verify against russh 0.64.
            let loaded = russh::keys::load_secret_key(path.clone(), passphrase);
            if temporary {
                let _ = std::fs::remove_file(&path);
            }
            let key = loaded.map_err(|e| {
                SshError::auth("private key", format!("failed to load private key: {e}"))
            })?;
            // PORT-TODO: `authenticate_publickey(username, &key)` — verify
            // whether russh 0.64 takes `&PrivateKey`, `Arc<PrivateKey>` or an
            // owned key.
            let outcome = handle
                .authenticate_publickey(username, &key)
                .await
                .map_err(|e| SshError::auth("private key", e.to_string()))?;
            check_outcome("private key", outcome)
        }
        AuthPlan::Agent => authenticate_agent(username).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    enum FakeOutcome {
        Success,
        Failure,
    }

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
    fn outcome_hedge_accepts_both_russh_shapes() {
        assert!(outcome_succeeded(true));
        assert!(!outcome_succeeded(false));
        assert!(outcome_succeeded(FakeOutcome::Success));
        assert!(!outcome_succeeded(FakeOutcome::Failure));
        assert!(!outcome_succeeded(()));
    }

    #[test]
    fn check_outcome_keeps_auth_reason() {
        let err = check_outcome("password", false).expect_err("false must be rejected");
        assert!(err.is_authentication());
        assert!(err.to_string().contains("password"));

        assert!(check_outcome("password", true).is_ok());
    }
}
