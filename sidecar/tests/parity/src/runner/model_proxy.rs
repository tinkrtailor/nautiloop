//! Model proxy category runner (FR-22 first block).
//!
//! Issues HTTP requests to both sidecars' model proxy ports (19090
//! for Go, 29090 for Rust) and captures:
//!
//! - HTTP status
//! - Subset of response headers (content-type)
//! - Response body
//! - Mock observations attributed to each side via source IP
//!
//! The 10 parity cases each set different input shapes via the
//! corpus JSON `input` field. This runner interprets the schema.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use serde::Deserialize;

use crate::compose::ports;
use crate::corpus::CorpusCase;
use crate::introspection;
use crate::result::{ObservedMockRequest, SideOutput};
use crate::runner::{CaseAssertion, CaseExecution, RunnerContext};

/// Input shape for a model_proxy case. Deserialized from `case.input`
/// with sensible defaults when fields are omitted.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct ModelProxyInput {
    /// Uppercase HTTP method, e.g. "GET".
    method: String,
    /// Path including leading `/openai/...` or `/anthropic/...`.
    path: String,
    /// Optional headers to send from the client to the sidecar. The
    /// sidecar may rewrite some of these (e.g. Authorization).
    headers: BTreeMap<String, String>,
    /// Optional request body.
    body: String,
    /// When set, mutate this file's contents between the first and
    /// second request of a credential-refresh case. Only the
    /// `openai_credential_refresh_per_request` case uses this.
    credential_refresh: Option<CredentialRefresh>,
}

impl Default for ModelProxyInput {
    fn default() -> Self {
        Self {
            method: "GET".to_string(),
            path: "/".to_string(),
            headers: BTreeMap::new(),
            body: String::new(),
            credential_refresh: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct CredentialRefresh {
    /// Mock credential files live under
    /// `sidecar/tests/parity/fixtures/{go,rust}-secrets/model-credentials/`.
    /// The runner writes `new_value` to the `openai` file on BOTH
    /// sides between requests. This exercises the sidecar's
    /// "read credentials fresh per request" behavior (FR-4 in the
    /// rust-sidecar spec).
    new_value: String,
}

/// Run a standard parity model_proxy case.
///
/// For non-refresh cases this is a simple request pair followed by an
/// introspection fetch. For `openai_credential_refresh_per_request`
/// the flow is:
///
/// 1. Issue request 1 with the original credentials.
/// 2. Reset the mocks so observations from request 1 do not
///    contaminate request 2.
/// 3. Write the mutated credential to BOTH `go-secrets/` and
///    `rust-secrets/`.
/// 4. Issue request 2 — both sidecars MUST re-read the file on every
///    request per FR-4 of the rust-sidecar spec.
/// 5. Fetch mock observations.
/// 6. Restore the original credentials (best-effort cleanup; the
///    fixture files are harness-only, but leaving them mutated would
///    trip `cargo test` on the next run and the committed-fixture
///    gates).
/// 7. Build an assertion that BOTH sides' upstream requests
///    (observed by mock-openai) carried
///    `Authorization: Bearer <new_value>`. The assertion is attached
///    to the `CaseExecution` alongside the normal diff so BOTH the
///    parity diff AND the "credential was actually refreshed"
///    invariant must hold for the case to pass.
///
/// This replaces the prior three-request dance that restored the
/// original credentials before the final observation fetch — that
/// flow observed the original credential in the last request pair
/// and would pass even if neither sidecar reloaded its file.
pub async fn run(case: &CorpusCase, ctx: &RunnerContext) -> Result<CaseExecution> {
    let input: ModelProxyInput = serde_json::from_value(case.input.clone())
        .with_context(|| format!("parsing input for case {}", case.name))?;

    if let Some(refresh) = input.credential_refresh.clone() {
        run_credential_refresh(&input, &refresh, ctx).await
    } else {
        run_standard(&input).await
    }
}

async fn run_standard(input: &ModelProxyInput) -> Result<CaseExecution> {
    let (mut go_out, mut rust_out) = issue_pair(input).await?;
    let (mut go_obs, mut rust_obs) = introspection::fetch_and_split().await?;
    go_out.mock_observations.append(&mut go_obs);
    rust_out.mock_observations.append(&mut rust_obs);
    Ok(CaseExecution::parity(go_out, rust_out))
}

async fn run_credential_refresh(
    input: &ModelProxyInput,
    refresh: &CredentialRefresh,
    ctx: &RunnerContext,
) -> Result<CaseExecution> {
    let go_secret_path = ctx
        .harness_dir
        .join("fixtures/go-secrets/model-credentials/openai");
    let rust_secret_path = ctx
        .harness_dir
        .join("fixtures/rust-secrets/model-credentials/openai");
    let go_original = std::fs::read_to_string(&go_secret_path).context("read go openai secret")?;
    let rust_original =
        std::fs::read_to_string(&rust_secret_path).context("read rust openai secret")?;

    // Step 1: request 1 with original credentials.
    let pair_one = issue_pair(input).await;

    // Step 2: reset mocks so request 2's observations stand alone.
    let reset_before_request_two = introspection::reset_all().await;

    // Step 3: write the mutated credential to BOTH sides.
    let write_go = std::fs::write(&go_secret_path, &refresh.new_value);
    let write_rust = std::fs::write(&rust_secret_path, &refresh.new_value);

    // Step 4: issue request 2 with the mutated credential ONLY if
    // the prior steps succeeded, so we can still restore the files
    // on the error paths below.
    let refreshed_result = match (&pair_one, &reset_before_request_two, &write_go, &write_rust) {
        (Ok(_), Ok(()), Ok(()), Ok(())) => {
            tokio::time::sleep(Duration::from_millis(1000)).await;
            Ok(issue_pair(input).await)
        }
        _ => Err(()),
    };

    // Step 5: fetch observations AFTER request 2 and BEFORE restore.
    let obs_result = if refreshed_result.is_ok() {
        Some(introspection::fetch_and_split().await)
    } else {
        None
    };

    // Step 6: restore original credentials no matter what.
    let _ = std::fs::write(&go_secret_path, go_original);
    let _ = std::fs::write(&rust_secret_path, rust_original);

    // Propagate any early error now that the files are restored.
    let (_go1, _rust1) = pair_one?;
    reset_before_request_two?;
    write_go.context("write go openai secret")?;
    write_rust.context("write rust openai secret")?;
    let pair_two_outer = refreshed_result.map_err(|()| {
        anyhow::anyhow!("credential refresh: precondition failed before request 2")
    })?;
    let (mut go_out, mut rust_out) = pair_two_outer?;
    let (mut go_obs, mut rust_obs) = obs_result
        .ok_or_else(|| anyhow::anyhow!("credential refresh: observations were not fetched"))??;

    // Step 7: build the assertion from the observations captured
    // for request 2 (the mutated credential should appear).
    let assertion = credential_refresh_assertion(&go_obs, &rust_obs, &refresh.new_value);

    go_out.mock_observations.append(&mut go_obs);
    rust_out.mock_observations.append(&mut rust_obs);
    Ok(CaseExecution::with_assertion(go_out, rust_out, assertion))
}

/// Inspect the mock-openai observations from a credential refresh
/// request 2 and assert that BOTH sides' upstream requests carried
/// `Authorization: Bearer <new_value>`.
///
/// Extracted as a pure function so the unit tests can exercise the
/// failure paths (no side saw it, only one side saw it, a side saw
/// the wrong value) without spinning up docker.
fn credential_refresh_assertion(
    go_obs: &[ObservedMockRequest],
    rust_obs: &[ObservedMockRequest],
    new_value: &str,
) -> CaseAssertion {
    let expected = format!("Bearer {new_value}");
    let go_saw = observed_bearer(go_obs, &expected);
    let rust_saw = observed_bearer(rust_obs, &expected);
    match (go_saw, rust_saw) {
        (true, true) => CaseAssertion::pass(format!(
            "both sidecars upstreamed the refreshed credential to mock-openai ({expected:?})"
        )),
        (true, false) => CaseAssertion::fail(format!(
            "credential refresh: go upstreamed {expected:?} but rust did NOT (rust did not reload the credential file per request)"
        )),
        (false, true) => CaseAssertion::fail(format!(
            "credential refresh: rust upstreamed {expected:?} but go did NOT (go did not reload the credential file per request)"
        )),
        (false, false) => CaseAssertion::fail(format!(
            "credential refresh: NEITHER side upstreamed {expected:?}; both sidecars cached the original credential in memory"
        )),
    }
}

/// Return true if any mock-openai observation carries an
/// Authorization header whose value equals `expected`.
///
/// Header name comparison is case-insensitive (per RFC 7230 §3.2 and
/// our own FR-19 normalization which lowercases observed header
/// names). Header value comparison is exact because the refreshed
/// credential is a literal string.
fn observed_bearer(obs: &[ObservedMockRequest], expected: &str) -> bool {
    obs.iter().filter(|o| o.mock == "mock-openai").any(|o| {
        o.headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == expected)
    })
}

/// Issue the request to both sidecars and return `(go, rust)`.
async fn issue_pair(input: &ModelProxyInput) -> Result<(SideOutput, SideOutput)> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .context("build model-proxy reqwest client")?;

    let go_url = format!("http://127.0.0.1:{}{}", ports::GO_MODEL, input.path);
    let rust_url = format!("http://127.0.0.1:{}{}", ports::RUST_MODEL, input.path);
    let go_fut = issue_one(&client, &go_url, &input.method, &input.headers, &input.body);
    let rust_fut = issue_one(
        &client,
        &rust_url,
        &input.method,
        &input.headers,
        &input.body,
    );
    let (go, rust) = tokio::try_join!(go_fut, rust_fut)?;
    Ok((go, rust))
}

async fn issue_one(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    headers: &BTreeMap<String, String>,
    body: &str,
) -> Result<SideOutput> {
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .with_context(|| format!("parsing HTTP method {method:?}"))?;
    let mut req = client.request(method, url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    if !body.is_empty() {
        req = req.body(body.to_string());
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("POST/GET {url}"))?;
    let status = resp.status().as_u16();
    let mut out_headers = BTreeMap::new();
    for (k, v) in resp.headers() {
        out_headers.insert(
            k.as_str().to_ascii_lowercase(),
            v.to_str().unwrap_or("").to_string(),
        );
    }
    let bytes = resp.bytes().await.context("read response body")?;
    let body_str = String::from_utf8_lossy(&bytes).to_string();
    Ok(SideOutput::http(status, out_headers, body_str))
}

/// Thresholds from FR-22. These may be widened in CI if noisy.
///
/// Exposed at module scope so unit tests can assert the verdict
/// logic against the real bounds without hardcoding duplicate
/// constants.
const SSE_RUST_MAX_MS: u128 = 250;
const SSE_GO_MIN_MS: u128 = 250;

/// SSE streaming divergence runner for FR-22 `divergence_sse_streaming_*`.
///
/// - `use_openai`: true for OpenAI path (`/openai/v1/chat/completions`),
///   false for Anthropic path (`/anthropic/v1/messages`).
///
/// The assertion is encoded explicitly via [`sse_assertion`]:
///
/// - Rust's first chunk MUST arrive within `SSE_RUST_MAX_MS`
///   (streaming). If not: FAIL with "Rust is buffered, expected
///   streaming".
/// - Go's first chunk MUST arrive at or after `SSE_GO_MIN_MS`
///   (buffered). If Go streamed: FAIL with "Go expected buffered
///   but streamed in X ms; file issue #66 regression is fixed?".
/// - Both conditions must hold: if only one holds, the case fails
///   with a message naming the specific violation.
///
/// The runner captures **every** chunk's wall-clock offset so the
/// report artifact can show the full streaming timeline, not just
/// the first chunk.
pub async fn run_sse_divergence(
    case: &CorpusCase,
    _ctx: &RunnerContext,
    use_openai: bool,
) -> Result<CaseExecution> {
    let input: ModelProxyInput = serde_json::from_value(case.input.clone())
        .with_context(|| format!("parsing input for case {}", case.name))?;

    let expected_path = if use_openai {
        "/openai/v1/chat/completions"
    } else {
        "/anthropic/v1/messages"
    };
    if input.path != expected_path {
        return Err(anyhow!(
            "sse divergence case {} expects path {expected_path}, got {}",
            case.name,
            input.path
        ));
    }

    let go_url = format!("http://127.0.0.1:{}{}", ports::GO_MODEL, input.path);
    let rust_url = format!("http://127.0.0.1:{}{}", ports::RUST_MODEL, input.path);
    let go_fut = stream_chunk_timestamps(&go_url, &input.method, &input.headers, &input.body);
    let rust_fut = stream_chunk_timestamps(&rust_url, &input.method, &input.headers, &input.body);
    let (go_chunks, rust_chunks) = tokio::try_join!(go_fut, rust_fut)?;

    let go_first = go_chunks
        .first()
        .copied()
        .ok_or_else(|| anyhow!("go sidecar produced no SSE chunks"))?;
    let rust_first = rust_chunks
        .first()
        .copied()
        .ok_or_else(|| anyhow!("rust sidecar produced no SSE chunks"))?;

    let assertion = sse_assertion(go_first, rust_first);

    let go_out = SideOutput {
        time_to_first_chunk_ms: Some(go_first),
        chunk_timestamps_ms: go_chunks,
        ..SideOutput::default()
    };
    let rust_out = SideOutput {
        time_to_first_chunk_ms: Some(rust_first),
        chunk_timestamps_ms: rust_chunks,
        ..SideOutput::default()
    };

    Ok(CaseExecution::with_assertion(go_out, rust_out, assertion))
}

/// Evaluate the SSE divergence assertion from measured first-chunk
/// wall-clock values. Extracted for unit testing.
///
/// The four outcomes are:
///
/// | rust | go  | verdict                                                  |
/// |------|-----|----------------------------------------------------------|
/// | ok   | ok  | PASS: divergence holds                                    |
/// | ok   | bad | FAIL: Go streamed too fast; issue #66 regression fixed?  |
/// | bad  | ok  | FAIL: Rust is buffered, expected streaming               |
/// | bad  | bad | FAIL: both sides fail their bounds                       |
fn sse_assertion(go_first_ms: u128, rust_first_ms: u128) -> CaseAssertion {
    let rust_ok = rust_first_ms < SSE_RUST_MAX_MS;
    let go_ok = go_first_ms >= SSE_GO_MIN_MS;
    match (rust_ok, go_ok) {
        (true, true) => CaseAssertion::pass(format!(
            "sse divergence holds: rust first chunk {rust_first_ms}ms < {SSE_RUST_MAX_MS}ms (streaming) AND go first chunk {go_first_ms}ms >= {SSE_GO_MIN_MS}ms (buffered)"
        )),
        (false, true) => CaseAssertion::fail(format!(
            "Rust is buffered, expected streaming: first_chunk_ms={rust_first_ms} NOT < {SSE_RUST_MAX_MS}. Go correctly buffered at {go_first_ms}ms."
        )),
        (true, false) => CaseAssertion::fail(format!(
            "Go expected buffered (>={SSE_GO_MIN_MS}ms) but streamed in {go_first_ms}ms; is issue #66 regression fixed? If so, convert this case to a parity case. Rust first chunk {rust_first_ms}ms was fine."
        )),
        (false, false) => CaseAssertion::fail(format!(
            "both sides failed their bounds: rust first chunk {rust_first_ms}ms (want <{SSE_RUST_MAX_MS}), go first chunk {go_first_ms}ms (want >={SSE_GO_MIN_MS})"
        )),
    }
}

/// Read the streaming response body and return the wall-clock
/// offsets (ms since send) of every non-empty chunk received until
/// upstream close.
async fn stream_chunk_timestamps(
    url: &str,
    method: &str,
    headers: &BTreeMap<String, String>,
    body: &str,
) -> Result<Vec<u128>> {
    let client = reqwest::Client::builder()
        .http1_only()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build streaming reqwest client")?;

    let method = reqwest::Method::from_bytes(method.as_bytes())
        .with_context(|| format!("parsing HTTP method {method:?}"))?;
    let mut req = client.request(method, url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    if !body.is_empty() {
        req = req.body(body.to_string());
    }

    let send_at = Instant::now();
    let resp = req.send().await.with_context(|| format!("POST {url}"))?;
    let mut stream = resp.bytes_stream();

    let mut timestamps: Vec<u128> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.context("reading SSE chunk")?;
        if !bytes.is_empty() {
            timestamps.push(send_at.elapsed().as_millis());
        }
    }
    if timestamps.is_empty() {
        return Err(anyhow!("SSE stream closed with no chunks from {url}"));
    }
    Ok(timestamps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        mock: &str,
        source_ip: &str,
        auth_header_value: Option<&str>,
    ) -> ObservedMockRequest {
        let mut headers = BTreeMap::new();
        if let Some(v) = auth_header_value {
            headers.insert("authorization".to_string(), v.to_string());
        }
        ObservedMockRequest {
            mock: mock.to_string(),
            method: "GET".to_string(),
            path: "/v1/models".to_string(),
            host_header: "api.openai.com".to_string(),
            headers,
            body_b64: String::new(),
            source_ip: source_ip.to_string(),
        }
    }

    // ---- SSE divergence assertion ----

    #[test]
    fn sse_assertion_passes_when_rust_streams_and_go_buffers() {
        let a = sse_assertion(320, 45);
        assert!(a.passed, "detail: {}", a.detail);
        assert!(a.detail.contains("divergence holds"));
    }

    #[test]
    fn sse_assertion_fails_when_rust_is_buffered() {
        // Rust first chunk > 250ms → buffered → FAIL.
        let a = sse_assertion(320, 260);
        assert!(!a.passed);
        assert!(a.detail.contains("Rust is buffered"));
    }

    #[test]
    fn sse_assertion_fails_when_go_streams_fast() {
        // Go first chunk < 250ms → streaming fast → issue #66 fixed → FAIL.
        let a = sse_assertion(42, 45);
        assert!(!a.passed);
        assert!(a.detail.contains("issue #66"));
    }

    #[test]
    fn sse_assertion_fails_when_both_sides_break_bounds() {
        let a = sse_assertion(42, 250);
        assert!(!a.passed);
        assert!(a.detail.contains("both sides failed"));
    }

    #[test]
    fn sse_assertion_edge_case_go_exactly_at_min() {
        // Go first chunk == SSE_GO_MIN_MS (250) → buffered → OK.
        let a = sse_assertion(SSE_GO_MIN_MS, 30);
        assert!(a.passed, "detail: {}", a.detail);
    }

    #[test]
    fn sse_assertion_edge_case_rust_exactly_at_max() {
        // Rust first chunk == SSE_RUST_MAX_MS (250) → NOT < 250 → FAIL.
        let a = sse_assertion(260, SSE_RUST_MAX_MS);
        assert!(!a.passed);
        assert!(a.detail.contains("Rust is buffered"));
    }

    // ---- Credential refresh assertion ----

    #[test]
    fn credential_refresh_assertion_passes_when_both_sides_upstream_new_value() {
        let go = vec![observation(
            "mock-openai",
            "100.64.0.20",
            Some("Bearer sk-test-openai-key-REFRESHED"),
        )];
        let rust = vec![observation(
            "mock-openai",
            "100.64.0.21",
            Some("Bearer sk-test-openai-key-REFRESHED"),
        )];
        let a = credential_refresh_assertion(&go, &rust, "sk-test-openai-key-REFRESHED");
        assert!(a.passed, "detail: {}", a.detail);
    }

    #[test]
    fn credential_refresh_assertion_fails_when_neither_side_upstreams_new_value() {
        let go = vec![observation(
            "mock-openai",
            "100.64.0.20",
            Some("Bearer sk-test-openai-key"),
        )];
        let rust = vec![observation(
            "mock-openai",
            "100.64.0.21",
            Some("Bearer sk-test-openai-key"),
        )];
        let a = credential_refresh_assertion(&go, &rust, "sk-test-openai-key-REFRESHED");
        assert!(!a.passed);
        assert!(a.detail.contains("NEITHER side"));
    }

    #[test]
    fn credential_refresh_assertion_fails_when_only_go_refreshes() {
        let go = vec![observation(
            "mock-openai",
            "100.64.0.20",
            Some("Bearer sk-test-openai-key-REFRESHED"),
        )];
        let rust = vec![observation(
            "mock-openai",
            "100.64.0.21",
            Some("Bearer sk-test-openai-key"),
        )];
        let a = credential_refresh_assertion(&go, &rust, "sk-test-openai-key-REFRESHED");
        assert!(!a.passed);
        assert!(a.detail.contains("go upstreamed"));
        assert!(a.detail.contains("rust did NOT"));
    }

    #[test]
    fn credential_refresh_assertion_fails_when_only_rust_refreshes() {
        let go = vec![observation(
            "mock-openai",
            "100.64.0.20",
            Some("Bearer sk-test-openai-key"),
        )];
        let rust = vec![observation(
            "mock-openai",
            "100.64.0.21",
            Some("Bearer sk-test-openai-key-REFRESHED"),
        )];
        let a = credential_refresh_assertion(&go, &rust, "sk-test-openai-key-REFRESHED");
        assert!(!a.passed);
        assert!(a.detail.contains("rust upstreamed"));
        assert!(a.detail.contains("go did NOT"));
    }

    #[test]
    fn credential_refresh_assertion_ignores_non_openai_observations() {
        // A mock-anthropic observation with the refreshed token
        // should NOT count — the case targets mock-openai only.
        let go = vec![observation(
            "mock-anthropic",
            "100.64.0.20",
            Some("Bearer sk-test-openai-key-REFRESHED"),
        )];
        let rust = vec![observation(
            "mock-anthropic",
            "100.64.0.21",
            Some("Bearer sk-test-openai-key-REFRESHED"),
        )];
        let a = credential_refresh_assertion(&go, &rust, "sk-test-openai-key-REFRESHED");
        assert!(!a.passed);
        assert!(a.detail.contains("NEITHER side"));
    }

    #[test]
    fn credential_refresh_assertion_ignores_missing_auth_header() {
        let go = vec![observation("mock-openai", "100.64.0.20", None)];
        let rust = vec![observation("mock-openai", "100.64.0.21", None)];
        let a = credential_refresh_assertion(&go, &rust, "sk-test-openai-key-REFRESHED");
        assert!(!a.passed);
    }

    // ---- Existing input parsing coverage ----

    #[test]
    fn model_proxy_input_defaults() {
        let input: ModelProxyInput = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(input.method, "GET");
        assert_eq!(input.path, "/");
        assert!(input.headers.is_empty());
        assert!(input.body.is_empty());
        assert!(input.credential_refresh.is_none());
    }

    #[test]
    fn model_proxy_input_full() {
        let input: ModelProxyInput = serde_json::from_value(serde_json::json!({
            "method": "POST",
            "path": "/openai/v1/chat/completions",
            "headers": {"authorization": "Bearer client-forged"},
            "body": "{}"
        }))
        .unwrap();
        assert_eq!(input.method, "POST");
        assert_eq!(input.path, "/openai/v1/chat/completions");
        assert_eq!(
            input.headers.get("authorization").map(String::as_str),
            Some("Bearer client-forged")
        );
    }
}
