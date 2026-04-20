use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

/// Extension type carrying the API key for dashboard auth.
/// Injected as a tower Extension layer so the middleware can access it
/// without requiring `State` extraction (which needs `from_fn_with_state`).
#[derive(Clone)]
pub struct DashboardApiKey(pub String);

/// Dashboard auth middleware: checks for `nautiloop_api_key` cookie OR Bearer header.
/// Unauthenticated requests to `/dashboard/*` (except `/dashboard/login` and
/// `/dashboard/static/*`) are redirected to `/dashboard/login`.
///
/// Reads the expected API key from `AppState.api_key` via request extensions.
pub async fn dashboard_auth_middleware(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = request.uri().path().to_string();

    // Skip auth for login page and static assets
    if path == "/dashboard/login" || path.starts_with("/dashboard/static/") {
        return Ok(next.run(request).await);
    }

    // Extract expected key from DashboardApiKey extension (set by the router layer)
    // or fall back to NAUTILOOP_API_KEY env var for backwards compatibility.
    let expected_key = request
        .extensions()
        .get::<DashboardApiKey>()
        .map(|k| k.0.clone())
        .or_else(|| std::env::var("NAUTILOOP_API_KEY").ok())
        .ok_or_else(|| {
            tracing::error!("NAUTILOOP_API_KEY not configured");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Check Bearer header first (for API calls)
    let bearer_valid = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| {
            if h.len() > 7 && h[..7].eq_ignore_ascii_case("bearer ") {
                Some(&h[7..])
            } else {
                None
            }
        })
        .is_some_and(|key| constant_time_eq(key.as_bytes(), expected_key.as_bytes()));

    if bearer_valid {
        return Ok(next.run(request).await);
    }

    // Check cookie
    let cookie_valid = extract_cookie_value(request.headers(), "nautiloop_api_key")
        .is_some_and(|key| constant_time_eq(key.as_bytes(), expected_key.as_bytes()));

    if cookie_valid {
        return Ok(next.run(request).await);
    }

    // Redirect to login for HTML requests, 401 for API/JSON requests
    let accepts_html = request
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"));

    if accepts_html {
        Ok(Redirect::to("/dashboard/login").into_response())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Extract a cookie value by name from the Cookie header.
pub fn extract_cookie_value<'a>(
    headers: &'a axum::http::HeaderMap,
    name: &str,
) -> Option<&'a str> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            for cookie in cookies.split(';') {
                let cookie = cookie.trim();
                if let Some(value) = cookie.strip_prefix(name).and_then(|v| v.strip_prefix('=')) {
                    return Some(value);
                }
            }
            None
        })
}

/// Validate an API key against the expected key. Accepts the expected key directly
/// rather than reading from the environment.
pub fn validate_api_key_against(key: &str, expected: &str) -> bool {
    if key.is_empty() || expected.is_empty() {
        return false;
    }
    constant_time_eq(key.as_bytes(), expected.as_bytes())
}

/// Constant-time byte comparison to prevent timing side-channel attacks.
/// Both values are hashed to a fixed-length digest before comparison,
/// preventing length leakage via early return on length mismatch.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    // Hash both values to a fixed 8-byte output so the comparison
    // is always over equal-length data — no early return on length difference.
    let hash_a = {
        let mut h = DefaultHasher::new();
        a.hash(&mut h);
        h.finish()
    };
    let hash_b = {
        let mut h = DefaultHasher::new();
        b.hash(&mut h);
        h.finish()
    };

    let a_bytes = hash_a.to_le_bytes();
    let b_bytes = hash_b.to_le_bytes();

    let mut diff = 0u8;
    for (x, y) in a_bytes.iter().zip(b_bytes.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn test_extract_cookie_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "foo=bar; nautiloop_api_key=test123; baz=qux".parse().unwrap(),
        );
        assert_eq!(
            extract_cookie_value(&headers, "nautiloop_api_key"),
            Some("test123")
        );
        assert_eq!(extract_cookie_value(&headers, "foo"), Some("bar"));
        assert_eq!(extract_cookie_value(&headers, "missing"), None);
    }

    #[test]
    fn test_extract_cookie_empty() {
        let headers = HeaderMap::new();
        assert_eq!(extract_cookie_value(&headers, "nautiloop_api_key"), None);
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        // Different lengths still compared safely (no early return on length)
        assert!(!constant_time_eq(b"short", b"longer-string"));
    }

    #[test]
    fn test_validate_api_key_against() {
        assert!(validate_api_key_against("test-key", "test-key"));
        assert!(!validate_api_key_against("wrong", "test-key"));
        assert!(!validate_api_key_against("", "test-key"));
        assert!(!validate_api_key_against("test-key", ""));
    }
}
