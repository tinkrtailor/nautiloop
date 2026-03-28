use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

/// Auth middleware: validates API key from `Authorization: Bearer <key>` header.
///
/// In V1, all authenticated users have full access (FR-14).
/// mTLS is handled at the ingress/load-balancer level, not in application code.
pub async fn auth_middleware(request: Request, next: Next) -> Result<Response, StatusCode> {
    // Extract API key from Authorization header
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let _api_key = &header[7..];
            // V1: Accept any non-empty API key.
            // Production: validate against stored keys.
            if _api_key.is_empty() {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(next.run(request).await)
        }
        _ => {
            // No auth header or wrong format
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}
