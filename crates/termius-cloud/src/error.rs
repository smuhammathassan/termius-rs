//! `CloudError` — the crate-wide error type.
//!
//! Rust counterpart of the JS `CloudSyncError` (recovered-background/entry.js):
//! every cloud failure keeps its reason so callers can tell auth vs network vs
//! protocol vs permission problems apart.
//!
//! PORT-TODO: the JS error also carries `config` (the whole `cloud_config`,
//! including `client_secret`) and `type` (`error.name`). Carrying the config
//! through errors would leak credentials into logs, so failures are classified
//! into typed variants instead and the server's reason string is preserved in
//! the variant payload.

use thiserror::Error;

/// Longest error body kept verbatim — ARM can return whole HTML error pages.
const MAX_ERROR_BODY: usize = 512;

/// Convenience alias used across the crate.
pub type Result<T> = std::result::Result<T, CloudError>;

/// Cloud provider failure, classified by cause.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CloudError {
    /// The credential was rejected or lacks permission: HTTP 401 **and** 403
    /// (there is no separate `Forbidden` variant; the status is kept in the
    /// reason string), plus every failure from the AAD token endpoint
    /// (`invalid_client`, `invalid_scope`, … arrive as HTTP 400 there).
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// Any other non-success HTTP status. `body` keeps the server's reason
    /// (ARM `error.code`/`error.message` extracted when present, else the
    /// truncated raw body).
    #[error("http {status}: {body}")]
    Http { status: u16, body: String },

    /// Transport failure: connect/DNS/timeout/TLS, or a pager that would loop.
    #[error("network error: {0}")]
    Network(String),

    /// A response (or credential blob) could not be parsed as expected JSON.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Provider this port does not implement yet (AWS, GCP, DigitalOcean).
    #[error("unsupported cloud provider: {provider}")]
    Unsupported { provider: String },

    /// HTTP 404 — ARM resource (NIC, public IP, subscription) does not exist.
    #[error("not found: {0}")]
    NotFound(String),
}

impl CloudError {
    /// Build an [`CloudError::Unsupported`] for a provider stub.
    pub fn unsupported(provider: impl Into<String>) -> Self {
        CloudError::Unsupported {
            provider: provider.into(),
        }
    }

    /// Classify a failed ARM (management.azure.com) response.
    ///
    /// 401/403 → [`CloudError::Unauthorized`] (auth/permission), 404 →
    /// [`CloudError::NotFound`], everything else keeps `status` + reason in
    /// [`CloudError::Http`].
    pub fn from_status(status: u16, body: &str) -> Self {
        let reason = server_reason(body);
        match status {
            401 => CloudError::Unauthorized(format!("HTTP 401: {reason}")),
            403 => CloudError::Unauthorized(format!("HTTP 403: {reason}")),
            404 => CloudError::NotFound(reason),
            _ => CloudError::Http {
                status,
                body: reason,
            },
        }
    }

    /// Classify a failed response from the OAuth token endpoint. Per RFC 6749
    /// the endpoint reports auth failures as HTTP 400 with an
    /// `{"error": "...", "error_description": "..."}` body — those are treated
    /// as [`CloudError::Unauthorized`], not generic HTTP errors.
    pub fn from_token_endpoint(status: u16, body: &str) -> Self {
        let reason = server_reason(body);
        match status {
            400 | 401 | 403 => CloudError::Unauthorized(format!("HTTP {status}: {reason}")),
            404 => CloudError::NotFound(reason),
            _ => CloudError::Http {
                status,
                body: reason,
            },
        }
    }
}

impl From<reqwest::Error> for CloudError {
    fn from(err: reqwest::Error) -> Self {
        // reqwest error messages never include request headers, so the bearer
        // token cannot leak through this conversion.
        let text = err.to_string();
        if let Some(status) = err.status() {
            return CloudError::from_status(status.as_u16(), &text);
        }
        if err.is_decode() {
            CloudError::Serialization(text)
        } else {
            CloudError::Network(text)
        }
    }
}

impl From<serde_json::Error> for CloudError {
    fn from(err: serde_json::Error) -> Self {
        CloudError::Serialization(err.to_string())
    }
}

/// Best-effort human reason from an error body:
/// ARM `{"error":{"code","message"}}`, AAD `{"error","error_description"}`,
/// else the body itself (truncated). Never panics.
fn server_reason(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(error) = value.get("error") {
                match error {
                    // ARM shape: {"error":{"code":"AuthorizationFailed","message":"..."}}
                    serde_json::Value::Object(_) => {
                        let code = error.get("code").and_then(|c| c.as_str());
                        let message = error.get("message").and_then(|m| m.as_str());
                        match (code, message) {
                            (Some(c), Some(m)) => return truncate(&format!("{c}: {m}")),
                            (Some(c), None) => return truncate(c),
                            (None, Some(m)) => return truncate(m),
                            (None, None) => {}
                        }
                    }
                    // AAD shape: {"error":"invalid_client","error_description":"..."}
                    serde_json::Value::String(code) => {
                        let description = value
                            .get("error_description")
                            .and_then(|d| d.as_str())
                            .unwrap_or_default();
                        if description.is_empty() {
                            return truncate(code);
                        }
                        return truncate(&format!("{code}: {description}"));
                    }
                    _ => {}
                }
            }
            if let Some(description) = value.get("error_description").and_then(|d| d.as_str()) {
                return truncate(description);
            }
        }
    }
    truncate(trimmed)
}

/// Keep at most [`MAX_ERROR_BODY`] characters of an error body.
fn truncate(reason: &str) -> String {
    if reason.chars().count() <= MAX_ERROR_BODY {
        reason.to_string()
    } else {
        let mut out: String = reason.chars().take(MAX_ERROR_BODY).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_status_codes() {
        assert!(matches!(
            CloudError::from_status(401, ""),
            CloudError::Unauthorized(_)
        ));
        // 403 is a permission failure — folded into Unauthorized (no Forbidden
        // variant exists) with the status preserved in the reason.
        assert!(matches!(
            CloudError::from_status(403, ""),
            CloudError::Unauthorized(_)
        ));
        assert!(matches!(
            CloudError::from_status(404, "gone"),
            CloudError::NotFound(_)
        ));
        let http = CloudError::from_status(500, "boom");
        assert_eq!(
            http,
            CloudError::Http {
                status: 500,
                body: "boom".to_string()
            }
        );
        assert!(matches!(
            CloudError::from_status(429, "slow down"),
            CloudError::Http { status: 429, .. }
        ));
    }

    #[test]
    fn extracts_arm_error_message() {
        let body = r#"{"error":{"code":"AuthorizationFailed","message":"The client does not have authorization."}}"#;
        let err = CloudError::from_status(403, body);
        match err {
            CloudError::Unauthorized(reason) => {
                assert!(reason.contains("AuthorizationFailed"), "{reason}");
                assert!(reason.contains("does not have authorization"), "{reason}");
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn oauth_errors_from_token_endpoint_are_unauthorized() {
        // AAD reports bad credentials as HTTP 400 + error JSON.
        let body = r#"{"error":"invalid_client","error_description":"AADSTS7000222: invalid client secret."}"#;
        let err = CloudError::from_token_endpoint(400, body);
        match err {
            CloudError::Unauthorized(reason) => {
                assert!(reason.contains("invalid_client"), "{reason}");
                assert!(reason.contains("AADSTS7000222"), "{reason}");
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
        // Non-auth failures from the token endpoint still map to Http.
        assert!(matches!(
            CloudError::from_token_endpoint(503, "unavailable"),
            CloudError::Http { status: 503, .. }
        ));
    }

    #[test]
    fn truncates_long_bodies() {
        let long = "x".repeat(MAX_ERROR_BODY * 4);
        match CloudError::from_status(500, &long) {
            CloudError::Http { body, .. } => {
                assert!(body.chars().count() <= MAX_ERROR_BODY + 1); // + ellipsis
                assert!(body.ends_with('…'));
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn serde_errors_map_to_serialization() {
        let err = serde_json::from_str::<u64>("not a number").expect_err("must fail");
        let err = CloudError::from(err);
        assert!(matches!(err, CloudError::Serialization(_)));
    }

    #[test]
    fn unsupported_helper() {
        assert_eq!(
            CloudError::unsupported("aws"),
            CloudError::Unsupported {
                provider: "aws".to_string()
            }
        );
    }
}
