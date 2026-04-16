# Health Endpoint JSON Response

## Summary

The `GET /health` endpoint currently returns HTTP 200 with an empty body, or 503 on DB failure. This spec adds a JSON body to both responses so callers get structured status information without parsing HTTP status codes alone.

## Functional Requirements

### FR-1: JSON response body

`GET /health` MUST return `Content-Type: application/json` and a JSON object:

```json
{"status": "ok", "version": "0.4.9"}
```

When healthy (DB reachable), `status` is `"ok"`. When unhealthy, the response is HTTP 503 with:

```json
{"status": "degraded", "version": "0.4.9"}
```

### FR-2: Version string

The version string MUST match the value in `control-plane/Cargo.toml` (`version = "x.y.z"`). Read it at compile time using `env!("CARGO_PKG_VERSION")`.

### FR-3: No auth required

`/health` is already unauthenticated. No change to auth middleware.

## Non-Functional Requirements

### NFR-1: No breaking change

Existing clients that ignore the response body continue to work. The HTTP status code semantics are unchanged.

### NFR-2: Test coverage

Add a unit test in `control-plane/src/api/mod.rs` that calls `GET /health` on the test router and asserts:
- Status 200
- `Content-Type: application/json`
- Body deserializes to `{"status": "ok", ...}`

## Implementation Notes

- Modify the `health` handler in `control-plane/src/api/mod.rs`
- Return `axum::Json(serde_json::json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")}))` on success
- For the error case, use `(StatusCode::SERVICE_UNAVAILABLE, axum::Json(...))` tuple response
- The handler signature will need to change from returning `StatusCode` to returning `impl IntoResponse`
