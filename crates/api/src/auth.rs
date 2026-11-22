//! API-Key-Authentifizierung für den Daemon.
//!
//! Angelehnt an das Referenz-`web/apikeys.py`: Schlüssel kommen per Header
//! `X-API-Key` oder `Authorization: Bearer <key>`. Ohne konfigurierte Keys ist
//! die API offen — dann lässt [`crate::serve`] aber nur einen localhost-Bind zu.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Konfigurierte API-Keys. Geklont pro Request (nur ein `Arc`-Bump).
#[derive(Clone)]
pub struct AuthConfig {
    keys: Arc<Vec<String>>,
}

impl AuthConfig {
    /// Übernimmt die Keys aus den geladenen Settings.
    pub fn from_settings(s: &vindex_core::config::Settings) -> Self {
        AuthConfig {
            keys: Arc::new(s.api_keys.clone()),
        }
    }

    /// True, wenn kein Key konfiguriert ist (API offen).
    pub fn is_open(&self) -> bool {
        self.keys.is_empty()
    }

    /// Prüft die Request-Header gegen die konfigurierten Keys.
    fn allows(&self, headers: &HeaderMap) -> bool {
        if self.keys.is_empty() {
            return true;
        }
        match extract_key(headers) {
            Some(provided) => self.keys.iter().any(|k| ct_eq(k, &provided)),
            None => false,
        }
    }
}

/// Zieht den Key aus `X-API-Key` oder `Authorization: Bearer <key>`.
fn extract_key(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    let auth = headers.get("authorization")?.to_str().ok()?;
    let token = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Längengebundener, konstant laufender Vergleich (vermeidet frühen Abbruch bei
/// gleicher Länge). Die Länge selbst wird nicht verschleiert.
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Middleware für `/api/*`: lässt den Request nur mit gültigem Key durch.
pub async fn require_api_key(State(auth): State<AuthConfig>, req: Request, next: Next) -> Response {
    if auth.allows(req.headers()) {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "ungültiger oder fehlender API-Key" })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(keys: &[&str]) -> AuthConfig {
        AuthConfig {
            keys: Arc::new(keys.iter().map(|k| k.to_string()).collect()),
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        use axum::http::HeaderName;
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    #[test]
    fn open_when_no_keys() {
        let auth = cfg(&[]);
        assert!(auth.is_open());
        assert!(auth.allows(&HeaderMap::new()));
    }

    #[test]
    fn rejects_missing_or_wrong_key() {
        let auth = cfg(&["secret-key"]);
        assert!(!auth.is_open());
        assert!(!auth.allows(&HeaderMap::new()));
        assert!(!auth.allows(&headers(&[("x-api-key", "nope")])));
    }

    #[test]
    fn accepts_valid_key_both_schemes() {
        let auth = cfg(&["abc", "def"]);
        assert!(auth.allows(&headers(&[("x-api-key", "def")])));
        assert!(auth.allows(&headers(&[("authorization", "Bearer abc")])));
        assert!(auth.allows(&headers(&[("authorization", "bearer abc")])));
    }

    #[test]
    fn ct_eq_matches_only_equal() {
        assert!(ct_eq("same", "same"));
        assert!(!ct_eq("short", "longer"));
        assert!(!ct_eq("aaaa", "aaab"));
    }
}
