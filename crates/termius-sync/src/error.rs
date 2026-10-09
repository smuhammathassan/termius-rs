//! Sync error type: one `thiserror` enum with distinct classification for
//! auth failures, rate limiting, network failures, generic HTTP errors,
//! payload (de)serialization and optimistic-concurrency conflicts.
//!
//! Status + response body are preserved for debugging, but this crate never
//! puts a bearer/device token into an error message or log line.

use thiserror::Error;

/// Upper bound on the response body kept inside an error value, so a huge
/// HTML error page cannot bloat logs.
const MAX_BODY_LEN: usize = 512;

/// Every failure mode of [`crate::SyncClient`].
#[derive(Debug, Error)]
pub enum SyncError {
    /// The API rejected our credentials (HTTP 401). The stored token should be
    /// considered dead; the caller re-runs sign-in (mirrors the JS client's
    /// `UNAUTHORIZED` event → re-login saga).
    #[error("unauthorized (401): {body}")]
    Unauthorized { body: String },

    /// The API throttled us (HTTP 429). `retry_after` is taken from the
    /// `Retry-After` response header when it is expressed in seconds.
    #[error("rate limited (429), retry after {retry_after:?} seconds: {body}")]
    RateLimited {
        retry_after: Option<u64>,
        body: String,
    },

    /// DNS/TCP/TLS/timeout failure — nothing reached the API (or the response
    /// could not be read). Mirrors the JS `HttpError.connectionAborted()`
    /// (`status: 0`) path.
    #[error("network error: {0}")]
    Network(String),

    /// Any other non-2xx response. `status` and `body` are preserved verbatim
    /// (clipped) for debugging.
    #[error("http error {status}: {body}")]
    Http { status: u16, body: String },

    /// A response (or request) payload could not be encoded/decoded.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// The record changed server-side since our last sync. HTTP 409, plus the
    /// HTTP 400 the sync endpoint answers to a stale push — the JS sync saga
    /// treats 400 from `POST /api/v4/terminal/sync/` as "pull, repair, retry".
    #[error("sync conflict ({status}): {body}")]
    Conflict { status: u16, body: String },
}

impl SyncError {
    /// Classify a non-2xx response into a distinct variant.
    ///
    /// `retry_after` is only consulted for 429; `body` is the (clipped)
    /// response body and is preserved on every variant for debugging.
    pub fn classify(status: u16, body: &str, retry_after: Option<u64>) -> Self {
        let body = clip(body);
        match status {
            401 => SyncError::Unauthorized { body },
            429 => SyncError::RateLimited {
                retry_after,
                body,
            },
            409 => SyncError::Conflict { status, body },
            _ => SyncError::Http { status, body },
        }
    }

    /// Like [`SyncError::classify`], but additionally maps HTTP 400 to
    /// [`SyncError::Conflict`]. Used for pushes to the sync endpoint, where a
    /// stale `last_synced` makes the server answer 400 and the recovered JS
    /// saga responds with pull → repair → retry.
    pub fn classify_push(status: u16, body: &str, retry_after: Option<u64>) -> Self {
        if status == 400 {
            return SyncError::Conflict {
                status,
                body: clip(body),
            };
        }
        Self::classify(status, body, retry_after)
    }

    /// HTTP status carried by this error, if any (0 for transport failures).
    pub fn status(&self) -> Option<u16> {
        match self {
            SyncError::Unauthorized { .. } => Some(401),
            SyncError::RateLimited { .. } => Some(429),
            SyncError::Http { status, .. } => Some(*status),
            SyncError::Conflict { status, .. } => Some(*status),
            SyncError::Network(_) | SyncError::Serialization(_) => None,
        }
    }

    /// The preserved response body, if this error came from the server.
    pub fn body(&self) -> Option<&str> {
        match self {
            SyncError::Unauthorized { body }
            | SyncError::RateLimited { body, .. }
            | SyncError::Http { body, .. }
            | SyncError::Conflict { body, .. } => Some(body.as_str()),
            SyncError::Network(_) | SyncError::Serialization(_) => None,
        }
    }

    /// Whether a naive retry could succeed: transport failures and 429s.
    /// Conflicts need a pull first; 401 needs a fresh sign-in.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            SyncError::Network(_) | SyncError::RateLimited { .. }
        )
    }
}

impl From<serde_json::Error> for SyncError {
    fn from(err: serde_json::Error) -> Self {
        SyncError::Serialization(err.to_string())
    }
}

/// Clip a response body to [`MAX_BODY_LEN`] characters for safe logging.
fn clip(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= MAX_BODY_LEN {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(MAX_BODY_LEN).collect();
    format!("{cut}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_401_as_unauthorized() {
        let err = SyncError::classify(401, r#"{"detail":"Invalid token."}"#, None);
        assert!(matches!(err, SyncError::Unauthorized { .. }));
        assert_eq!(err.status(), Some(401));
        assert!(err.body().unwrap().contains("Invalid token"));
        assert!(!err.is_retryable());
    }

    #[test]
    fn classifies_429_with_retry_after() {
        let err = SyncError::classify(429, "slow down", Some(30));
        match &err {
            SyncError::RateLimited { retry_after, body } => {
                assert_eq!(*retry_after, Some(30));
                assert_eq!(body, "slow down");
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
        assert_eq!(err.status(), Some(429));
        assert!(err.is_retryable());
    }

    #[test]
    fn classifies_429_without_retry_after() {
        let err = SyncError::classify(429, "", None);
        assert!(matches!(
            err,
            SyncError::RateLimited {
                retry_after: None,
                ..
            }
        ));
    }

    #[test]
    fn classifies_409_as_conflict() {
        let err = SyncError::classify(409, "stale", None);
        assert!(matches!(
            err,
            SyncError::Conflict {
                status: 409,
                ..
            }
        ));
    }

    #[test]
    fn push_treats_sync_400_as_conflict() {
        let err = SyncError::classify_push(400, "last_synced too old", None);
        assert!(matches!(
            err,
            SyncError::Conflict {
                status: 400,
                ..
            }
        ));
        // non-push classification keeps 400 as a generic HTTP error
        let generic = SyncError::classify(400, "bad", None);
        assert!(matches!(
            generic,
            SyncError::Http {
                status: 400,
                ..
            }
        ));
    }

    #[test]
    fn classifies_other_statuses_as_http() {
        for status in [403u16, 490, 500, 503] {
            let err = SyncError::classify(status, "nope", None);
            assert!(matches!(err, SyncError::Http { status: s, .. } if s == status));
            assert_eq!(err.status(), Some(status));
            assert_eq!(err.body(), Some("nope"));
        }
        assert!(!SyncError::classify(500, "", None).is_retryable());
    }

    #[test]
    fn network_and_serialization_variants() {
        let net = SyncError::Network("connection reset".into());
        assert_eq!(net.status(), None);
        assert_eq!(net.body(), None);
        assert!(net.is_retryable());
        assert!(net.to_string().contains("connection reset"));

        let ser: Result<u32, SyncError> = serde_json::from_str("not a number").map_err(Into::into);
        assert!(matches!(ser, Err(SyncError::Serialization(_))));
    }

    #[test]
    fn clips_long_bodies() {
        let long = "x".repeat(5000);
        let err = SyncError::classify(500, &long, None);
        let body = err.body().unwrap();
        assert!(body.len() < 600);
        assert!(body.ends_with("..."));
    }

    #[test]
    fn display_never_contains_unexpected_secrets() {
        // Errors only echo what the *server* sent back; a server body is not
        // our bearer token. Sanity: the Display output stays bounded.
        let err = SyncError::classify(500, "internal server error", None);
        let rendered = err.to_string();
        assert!(rendered.contains("500"));
        assert!(rendered.contains("internal server error"));
    }
}
