/// Constant-time byte comparison to prevent timing side-channel attacks.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Extract a cookie value by name from the Cookie header.
pub fn extract_cookie_value(
    headers: &axum::http::HeaderMap,
    name: &str,
) -> Option<String> {
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|pair| {
                let pair = pair.trim();
                let (k, v) = pair.split_once('=')?;
                if k.trim() == name {
                    Some(v.trim().to_string())
                } else {
                    None
                }
            })
        })
}

/// Extract API key from Authorization: Bearer <key> header.
pub fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|header| {
            if header.len() > 7 && header[..7].eq_ignore_ascii_case("bearer ") {
                let key = &header[7..];
                if key.is_empty() {
                    None
                } else {
                    Some(key.to_string())
                }
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_extract_cookie_value() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "cookie",
            "nautiloop_api_key=secret123; nautiloop_engineer=alice"
                .parse()
                .unwrap(),
        );
        assert_eq!(
            extract_cookie_value(&headers, "nautiloop_api_key"),
            Some("secret123".to_string())
        );
        assert_eq!(
            extract_cookie_value(&headers, "nautiloop_engineer"),
            Some("alice".to_string())
        );
        assert_eq!(extract_cookie_value(&headers, "missing"), None);
    }

    #[test]
    fn test_extract_bearer() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer mykey".parse().unwrap());
        assert_eq!(extract_bearer(&headers), Some("mykey".to_string()));
    }

    #[test]
    fn test_extract_bearer_empty() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer ".parse().unwrap());
        assert_eq!(extract_bearer(&headers), None);
    }
}
