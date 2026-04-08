# Sidecar: Containerized Parity Test Harness

## Overview

Phase 4 of the Rust sidecar migration plan. Build a containerized parity test harness that runs the Go sidecar and the Rust sidecar side-by-side against the same hermetic inputs, diffs their outputs, and gates the cutover decision.

This is the single biggest remaining production-readiness blocker before phase 5 (K8s cutover).

## Baseline

Main at merge of PR #73 (`63d61e8`). The Rust sidecar is committed at `sidecar/` with 93 unit tests + 7 integration tests passing. The Go sidecar is still in the repo at `images/sidecar/main.go` — kept per the migration plan until phase 6. Both binaries compile and build their own Docker images.

## Problem Statement

After three review passes (codex v1, v2, v3) and one followup batch review (PR #73), the Rust sidecar compiles, passes clippy, passes 100+ tests, and has been adversarially reviewed to the point of diminishing returns. **What we still lack is evidence of behavior parity against the Go implementation under realistic conditions.**

Specifically:

- All current Rust tests exercise the Rust implementation in isolation. None compare its behavior against the Go binary side-by-side.
- The only manual validation we have is what the codex reviews surfaced, which are logical / static-analysis checks, not dynamic equivalence.
- Three intentional deviations from Go are documented in `specs/rust-sidecar.md` (SSRF fail-closed on DNS error, DNS rebinding resolve-once, bare-exec rejection). Without a diff harness, we cannot prove those are the *only* deviations.
- The cutover plan (phase 5) treats "production bake" as the main safety net. A one-week bake against a 3-binary-wire-protocol (HTTP, TLS, SSH) component is a thin safety net when an alternative — hermetic differential testing — is achievable today.

**The harness this spec builds is the difference between "we think it's behavior-identical" and "we have evidence it's behavior-identical."**

## Design grounding

The core harness design is already in `specs/rust-sidecar.md` under the `### Parity test harness (containerized, hermetic)` section. That section includes:

- Docker Compose layout with 5 services (sidecar-go, sidecar-rust, mock-openai, mock-anthropic, mock-github-ssh)
- Test CA bundle strategy via `NAUTILOOP_EXTRA_CA_BUNDLE` env var (Rust side) + `Dockerfile.go-with-test-ca` variant (Go side)
- DNS override via compose `extra_hosts` so real hostnames (`api.openai.com`, `github.com`) resolve to mock services
- Harness corpus: Model proxy, Egress, Git SSH, Health categories with specific inputs per category
- Comparison logic: HTTP status / body / log lines / exit codes, normalized per-test
- Acknowledged divergences: the 3 documented bug fixes + the CONNECT drain improvement + `/healthz` method parity
- Known limitations: concurrent log ordering under load, trailing-newline differences in error bodies, fatal error wording

**This spec implements that design.** Where the `rust-sidecar.md` design is ambiguous or underspecified, this spec makes the calls explicit.

## Dependencies

- **Requires:** PR #63 (rust sidecar merged), PR #73 (followups merged including `__test_utils` feature), PR #56 (Go sidecar health bind fix). All three are on main.
- **Enables:** phase 5 cutover with actual parity evidence. Unblocks retiring the Go sidecar.
- **Blocks:** nothing.

## Requirements

### Functional Requirements

#### Harness layout and services

- FR-1: The harness lives at `sidecar/tests/parity/` with this structure:
  ```
  sidecar/tests/parity/
  ├── docker-compose.yml         # orchestrates 5 services
  ├── Dockerfile.go-with-test-ca # Go sidecar + baked test CA bundle
  ├── fixtures/
  │   ├── test-ca/
  │   │   ├── ca.pem             # CA certificate (committed)
  │   │   └── ca.key             # CA private key (committed — test-only, never used in prod)
  │   ├── mock-openai/
  │   │   ├── server.py          # minimal HTTPS server serving /v1/models, /v1/chat/completions
  │   │   ├── cert.pem           # mock-openai.test cert signed by test-ca
  │   │   └── key.pem
  │   ├── mock-anthropic/
  │   │   ├── server.py          # same shape for /v1/messages
  │   │   ├── cert.pem
  │   │   └── key.pem
  │   ├── mock-github-ssh/
  │   │   ├── server.py          # python SSH server accepting git-upload-pack/git-receive-pack
  │   │   ├── host_key
  │   │   └── authorized_keys
  │   ├── go-secrets/
  │   │   ├── model-credentials/openai       # "sk-test-openai-key"
  │   │   ├── model-credentials/anthropic    # "sk-ant-test-key"
  │   │   ├── ssh-key/id_ed25519             # harness client key
  │   │   └── ssh-known-hosts/known_hosts    # includes mock-github-ssh host key
  │   └── rust-secrets/          # identical content, separate mount
  ├── corpus/
  │   └── *.json                 # one file per test case, see corpus design
  ├── src/
  │   └── main.rs                # the harness driver binary
  └── Cargo.toml                 # own crate: nautiloop-sidecar-parity-harness
  ```

- FR-2: `docker-compose.yml` shall define exactly 5 services:
  1. `sidecar-go` — Go binary built from `Dockerfile.go-with-test-ca`, exposes container ports 9090-9093 as host ports 19090-19093, mounts `go-secrets/` at `/secrets/`, sets `GIT_REPO_URL=git@github.com:test/repo.git`, `extra_hosts` maps `api.openai.com`, `api.anthropic.com`, `github.com` to the corresponding mock service IPs on the Docker network.
  2. `sidecar-rust` — Rust binary from `images/sidecar/Dockerfile`, exposes container ports 9090-9093 as host ports 29090-29093, mounts `rust-secrets/` at `/secrets/` AND `fixtures/test-ca/ca.pem` at `/test-ca/ca.pem`, sets `GIT_REPO_URL=git@github.com:test/repo.git` AND `NAUTILOOP_EXTRA_CA_BUNDLE=/test-ca/ca.pem`, same `extra_hosts`.
  3. `mock-openai` — serves `api.openai.com:443` on the Docker network. Python-based HTTPS server with cert signed by test-ca. Returns fixed, deterministic responses for `GET /v1/models`, `POST /v1/chat/completions`, `GET /v1/chat/completions/stream-sse`, and a default 404 for unknown paths. Logs every incoming request (method, path, Authorization header, Host header) to a mounted volume for harness inspection.
  4. `mock-anthropic` — same for `api.anthropic.com:443`. Routes: `POST /v1/messages` (both streaming and non-streaming).
  5. `mock-github-ssh` — Python SSH server on `github.com:22`. Accepts `git-upload-pack <path>` and `git-receive-pack <path>`. For `test/repo.git`, returns fixed pack data bytes and `ExitStatus(0)`. For any other path, returns an error and `ExitStatus(128)`. Logs exec commands and bytes received.

- FR-3: Both `sidecar-go` and `sidecar-rust` shall depend on all three mock services being healthy before starting, via Docker Compose `depends_on` with `condition: service_healthy`.

- FR-4: The `Dockerfile.go-with-test-ca` shall:
  1. Start from the production Go sidecar build (copy from existing `images/sidecar/Dockerfile` base or use a multi-stage approach to rebuild the Go binary)
  2. Copy `fixtures/test-ca/ca.pem` to `/etc/ssl/certs/ca-certificates.crt` (appended, not replaced, to preserve any default CAs the Go binary's `crypto/tls` needs)
  3. Produce a scratch image identical to production except for the CA bundle

  **Alternative:** if the production Go image base is `FROM scratch`, the Dockerfile simply copies the modified CA bundle directly. Document which approach is taken.

- FR-5: The Rust sidecar image (production `images/sidecar/Dockerfile`) is used AS-IS for the `sidecar-rust` service. The test CA is loaded at runtime via `NAUTILOOP_EXTRA_CA_BUNDLE`, not baked into the image. This exercises the production code path.

#### Mock services behavior

- FR-6: `mock-openai` shall respond to:
  - `GET /v1/models` → 200 with JSON body `{"data":[{"id":"gpt-4o-mini","object":"model"}]}`, Content-Type `application/json`
  - `POST /v1/chat/completions` (non-streaming) → 200 with a fixed JSON response containing a known string (e.g. `{"id":"chatcmpl-test","choices":[{"message":{"content":"pong"}}]}`)
  - `POST /v1/chat/completions` with header `Accept: text/event-stream` OR request body field `"stream": true` → 200 with `Content-Type: text/event-stream`, streams 3 SSE events with deterministic content, then `[DONE]` and closes. **Server must flush between events so the client receives them incrementally.** Required to test the model proxy's streaming pass-through (FR-6 in rust-sidecar.md).
  - Every other path → 404

  All responses record the incoming `Host` header, `Authorization` header, and request body to a log file for harness inspection.

- FR-7: `mock-anthropic` shall respond to:
  - `POST /v1/messages` → 200 with fixed JSON response. Non-streaming.
  - `POST /v1/messages` with request body field `"stream": true` → SSE streaming, same shape as mock-openai's streaming endpoint.
  - Every other path → 404

  All responses record incoming `Host`, `x-api-key`, `anthropic-version`, and request body.

- FR-8: `mock-github-ssh` shall be a minimal SSH server (Python `paramiko` is acceptable):
  - Accepts any client key (no auth — the harness key is static)
  - Recognizes `git-upload-pack` and `git-receive-pack` exec commands
  - For `git-upload-pack 'test/repo.git'` or `git-upload-pack test/repo.git`: write a fixed byte sequence (a valid minimal pack file or equivalent fixed payload) to the channel stdout, then send `ExitStatus(0)` and close
  - For `git-receive-pack 'test/repo.git'`: read bytes from channel stdin (the pack the client pushes), write a fixed acknowledgment, send `ExitStatus(0)`
  - For any other exec command or any other repo path: write an error message to stderr and send `ExitStatus(128)`
  - For `env`, `pty-req`, `subsystem`, `shell` requests: reject via channel failure
  - Logs every connection, exec command, and byte count to a file

- FR-9: Mock services shall have Docker healthchecks:
  - `mock-openai`, `mock-anthropic`: `curl -kf https://localhost:443/_healthz` returns 200 (serve a `/_healthz` endpoint)
  - `mock-github-ssh`: TCP connect check on port 22 via `nc -z localhost 22`

#### Harness driver

- FR-10: The harness driver is a new workspace crate at `sidecar/tests/parity/` named `nautiloop-sidecar-parity-harness`. It is NOT part of the `nautiloop-sidecar` crate — it's a separate member of the Cargo workspace, and only its own `cargo test` or `cargo run -p nautiloop-sidecar-parity-harness` invokes it.

- FR-11: The harness driver shall start the `docker-compose.yml` stack, wait for all services to be healthy (up to 60s total, fail with clear error if any service doesn't come up), run the test corpus against both sidecars in parallel, collect outputs, diff them, and report results.

- FR-12: The driver shall accept a `--stop` flag that tears down the compose stack after the run regardless of outcome. Default behavior is to leave the stack up if any test fails so the operator can inspect.

- FR-13: For each test case in the corpus, the driver shall:
  1. Reset any per-test mock service state (e.g., delete the mock's log file so only this test's requests are recorded)
  2. Issue the test input to BOTH sidecars in parallel (against host ports 19090-19093 for Go, 29090-29093 for Rust)
  3. Capture from each side: HTTP status code, response headers (subset), response body, emitted log lines (by reading docker logs for that container since the test started), SSH exit codes, SSH stderr
  4. Normalize the captures per the rules in FR-14
  5. Diff Go vs Rust side. If they match (after normalization), the test passes. If they don't, the test fails with a readable diff.
  6. For the 4 documented divergences (FR-15), the assertion is flipped: the test passes if Go and Rust differ in the documented direction, and fails if they match.

- FR-14: Normalization rules applied before comparing outputs:
  - Log lines: strip `timestamp` field entirely. All other fields (`level`, `message`, `destination`, `method`, `bytes_sent`, `bytes_recv`, `prefix`) are compared verbatim.
  - HTTP response headers: compare `Content-Type`, strip `Date`, `Server`, `Via`, and any other connection-specific headers
  - Response bodies: per-test config can specify fields to strip (for responses that contain timestamps or request IDs)
  - SSH stderr: trim trailing whitespace (Go `http.Error` appends a newline that Rust hyper does not; same convention applies to SSH error paths if any)

- FR-15: The driver shall assert DIVERGENCE (not parity) for these four documented fixes/improvements:
  1. **SSRF fail-closed on DNS error:** send a request through the egress proxy where DNS lookup fails (e.g. via a test-specific mock DNS response or by using a deliberately unresolvable hostname). Go → connects or errors with upstream 502; Rust → 502 with a log line indicating SSRF block on DNS error. Harness asserts Rust log contains the SSRF indicator AND HTTP response matches.
  2. **DNS rebinding resolve-once:** harder to test hermetically without a fake DNS. Acceptable scope: verify the Rust sidecar uses `SsrfConnector` and dials the resolved IP, by observing (via mock service logs) that the upstream `Host` header matches the hostname and the request arrived. This is a regression smoke, not a full rebinding simulation.
  3. **Bare-exec rejection:** send `git-upload-pack` and `git-receive-pack` with no path argument to both sidecars' SSH proxies. Go → proxies through (mock-github-ssh receives the exec and returns ExitStatus 128). Rust → rejects locally with exit_status(1). Harness asserts divergence: Go's exit is 128 (from mock), Rust's exit is 1 (from sidecar reject). Both non-zero, different codes.
  4. **CONNECT tunnel drain on SIGTERM:** send a CONNECT tunnel request, then SIGTERM the sidecar container while the tunnel is active. Rust should drain (up to 5s) before closing; Go should drop immediately. Harness asserts the Rust tunnel continues to receive bytes for up to 5s after SIGTERM, while the Go tunnel closes within 100ms.

#### Test corpus

- FR-16: The corpus lives in `sidecar/tests/parity/corpus/` as JSON files. Each file is one test case with fields:
  ```json
  {
    "name": "test_case_name",
    "category": "model_proxy" | "egress" | "git_ssh" | "health" | "divergence",
    "description": "human-readable",
    "input": { ... category-specific ... },
    "expected_parity": true | false,
    "divergence": null | { "go_should": "...", "rust_should": "..." },
    "normalize": { "body_strip_fields": ["id"], ... }
  }
  ```

- FR-17: The corpus MUST cover at minimum these cases (expand if the harness framework makes it cheap):

  **Model proxy parity (12 cases):**
  - GET /openai/v1/models
  - POST /openai/v1/chat/completions (non-streaming)
  - POST /openai/v1/chat/completions (SSE streaming, 3 events)
  - GET /anthropic/v1/messages
  - POST /anthropic/v1/messages (non-streaming)
  - POST /anthropic/v1/messages (streaming)
  - GET /openai (bare path) → maps to upstream /
  - GET /anthropic (bare path) → maps to upstream /
  - GET /unknown-route → 403 with exact Go error body
  - POST /openai/v1/chat/completions with client-supplied Authorization header → verify upstream receives the sidecar-injected Bearer, not client's
  - POST /anthropic/v1/messages with client-supplied x-api-key → verify upstream receives the sidecar-injected value
  - POST /anthropic/v1/messages with client-supplied anthropic-version → verify passthrough (not overwritten)

  **Egress parity (6 cases):**
  - CONNECT github.com:443 → tunnel, log destination = github.com:443
  - CONNECT github.com (no port) → tunnel, log destination = github.com:443 (synthesized)
  - GET http://mock-example/ via egress (mock-example is a Docker-network-only HTTP server, added as 6th mock service) → forwarded, log destination = mock-example (no port per Go URL.Host behavior)
  - GET http://mock-example:8080/foo → forwarded, log destination = mock-example:8080
  - GET http://mock-example/ with `Proxy-Connection: keep-alive` header → verify header is stripped on the outgoing request (mock-example logs the incoming headers)
  - GET http://mock-example/ returning 302 → verify sidecar does NOT follow the redirect (client sees the 302)

  **Git SSH parity (5 cases):**
  - git-upload-pack 'test/repo.git' → proxies to mock-github-ssh, receives fixed pack bytes, exit status 0
  - git-receive-pack 'test/repo.git' → proxies, pushes pack bytes, exit status 0
  - git-upload-pack 'wrong/repo.git' → rejected locally by sidecar (repo allowlist mismatch), exit status 1
  - Send `ls /etc` as exec → rejected locally, exit status 1
  - Send env request (before exec) → rejected via channel_failure, no exit status

  **Health parity (3 cases):**
  - GET /healthz immediately after container start → 503 with body `{"status":"starting"}`
  - Wait for ready, GET /healthz → 200 with body `{"status":"ok"}`
  - HEAD /healthz after ready → 200 (method parity per Go's mux.HandleFunc which accepts any method)

  **Documented divergences (4 cases, must fail IF identical):**
  - SSRF DNS error path (FR-15 item 1)
  - DNS rebinding smoke (FR-15 item 2)
  - Bare git-upload-pack (FR-15 item 3)
  - CONNECT drain on SIGTERM (FR-15 item 4)

- FR-18: The harness driver shall support filtering: `cargo run -p nautiloop-sidecar-parity-harness -- --category model_proxy` runs only the model_proxy cases. `--case test_case_name` runs only one case.

#### CI integration (minimal)

- FR-19: The harness shall NOT be wired into any GitHub Actions workflow in this spec. It runs only when explicitly invoked locally via `cargo run -p nautiloop-sidecar-parity-harness` (which starts the compose stack) or `docker compose -f sidecar/tests/parity/docker-compose.yml up --abort-on-container-exit` (manual debugging). CI wiring is deferred to a followup once the harness itself is stable.

- FR-20: A `sidecar/tests/parity/README.md` shall document:
  - Prerequisites (Docker Desktop, Rust stable)
  - How to build the required images (`docker compose build`)
  - How to run the full corpus (`cargo run -p nautiloop-sidecar-parity-harness`)
  - How to run a single category or case
  - Expected runtime (target: <5 minutes for the full corpus on a developer workstation)
  - How to inspect mock service logs when a test fails
  - How to add a new corpus case (template + example)

### Non-Functional Requirements

- NFR-1: The harness shall be runnable on any developer workstation with Docker Desktop and Rust stable installed. No other prerequisites.
- NFR-2: The full corpus run shall complete in under 5 minutes on a 2024-era laptop (modest bar; we're not optimizing for CI-scale throughput).
- NFR-3: The harness driver shall be `cargo clippy --workspace -- -D warnings` green, same bar as the rest of the workspace.
- NFR-4: The harness shall be hermetic — no network access outside the Docker Compose network during test runs. Both sidecars see `api.openai.com`, `api.anthropic.com`, `github.com` as Docker-internal IPs.
- NFR-5: Test case failures shall produce readable diffs with the exact fields that differ, not just "outputs don't match". The diff format should point at the corpus file name so the operator can fix a specific case.

### Security Requirements

- SR-1: The test CA private key (`fixtures/test-ca/ca.key`) is committed to the repo. It is marked clearly as test-only in the README and in the file header comment. Its only purpose is to sign certificates for mock HTTPS services that are unreachable outside the harness.
- SR-2: The harness client SSH key (`fixtures/go-secrets/ssh-key/id_ed25519` and `rust-secrets/` equivalent) is also committed. Same test-only justification.
- SR-3: The mock model credentials (e.g. `"sk-test-openai-key"`) are committed and are obviously non-production. They never leave the harness network.
- SR-4: The harness MUST NOT reuse any production certificate, key, or credential from anywhere in the repo. All harness secrets are freshly generated for this spec and committed. A comment at the top of each key file identifies it as harness-only.
- SR-5: The `NAUTILOOP_EXTRA_CA_BUNDLE` env var set on the `sidecar-rust` service is scoped to the harness docker-compose.yml only. No production K8s manifest sets this variable (SR-10 from the rust-sidecar spec).
- SR-6: `sidecar/scripts/lint-no-test-utils-in-prod.sh` (from PR #73) shall also be extended to check for `NAUTILOOP_EXTRA_CA_BUNDLE` references in CI workflow files, marking any such reference as a failure. Update the script in this spec.

## Architecture

### Crate structure

`sidecar/tests/parity/` becomes a new workspace member. Top-level `Cargo.toml` gets:

```toml
[workspace]
members = [
    "cli",
    "control-plane",
    "sidecar",
    "sidecar/tests/parity",  # NEW
]
```

The harness crate's own `Cargo.toml`:

```toml
[package]
name = "nautiloop-sidecar-parity-harness"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "nautiloop-sidecar-parity-harness"
path = "src/main.rs"

[dependencies]
tokio = { version = "1.40", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "rustls-tls", "stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
anyhow = "1"
thiserror = "1"
russh = { version = "0.60", default-features = false, features = ["ring"] }
russh-keys = "0.60"
tracing = "0.1"
tracing-subscriber = ">=0.3.20"
```

Note: the harness depends on `russh` because it needs to speak SSH client-side to `sidecar-go:9091` and `sidecar-rust:9091` to issue exec commands and capture exit statuses. `reqwest` is used for the HTTP side (both `:9090` model proxy and `:9092` egress logger). `reqwest` with `rustls-tls` + a custom root store loaded from `fixtures/test-ca/ca.pem` lets it trust the mock HTTPS services.

### Driver program structure

`sidecar/tests/parity/src/main.rs` roughly:

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let corpus = load_corpus("corpus/")?;
    let filtered = filter_corpus(&corpus, args.category, args.case);

    let compose = ComposeStack::up("docker-compose.yml").await?;
    compose.wait_healthy(Duration::from_secs(60)).await?;

    let mut results = Vec::new();
    for case in &filtered {
        let result = run_case(case).await;
        print_case_result(&result);
        results.push(result);
    }

    let summary = summarize(&results);
    print_summary(&summary);

    if args.stop || summary.all_passed {
        compose.down().await?;
    } else {
        println!("Stack left running for inspection; run `docker compose down` when done.");
    }

    if !summary.all_passed {
        std::process::exit(1);
    }
    Ok(())
}

async fn run_case(case: &TestCase) -> CaseResult {
    let go_future = exercise_sidecar(case, SidecarSide::Go);
    let rust_future = exercise_sidecar(case, SidecarSide::Rust);
    let (go_out, rust_out) = tokio::join!(go_future, rust_future);

    let go_norm = normalize(&go_out, &case.normalize);
    let rust_norm = normalize(&rust_out, &case.normalize);

    match case.expected_parity {
        true => compare_parity(&go_norm, &rust_norm),
        false => compare_divergence(&go_norm, &rust_norm, &case.divergence.unwrap()),
    }
}

async fn exercise_sidecar(case: &TestCase, side: SidecarSide) -> SidecarOutput {
    match case.category.as_str() {
        "model_proxy" => exercise_model_proxy(case, side).await,
        "egress" => exercise_egress(case, side).await,
        "git_ssh" => exercise_git_ssh(case, side).await,
        "health" => exercise_health(case, side).await,
        _ => SidecarOutput::error("unknown category"),
    }
}
```

Each `exercise_*` function uses the appropriate client library (`reqwest` for HTTP, `russh::client` for SSH) to send the case's input to the sidecar at its host-mapped port, capture the response + exit status + logs, and return a structured `SidecarOutput`.

Log capture uses `docker logs <container> --since <test_start_time>` executed via `tokio::process::Command`. Parse the log lines as JSON (FR-19/FR-26 schemas from the rust-sidecar spec), normalize per FR-14, and include in the `SidecarOutput` for comparison.

### Normalization helper

A single `normalize(output: &SidecarOutput, rules: &NormalizeRules) -> NormalizedOutput` function applies:

- Strip `timestamp` from all log line JSON objects
- Strip any body fields listed in `rules.body_strip_fields`
- Strip volatile HTTP response headers: `Date`, `Server`, `Via`, `X-Request-Id`, any header matching a per-case allowlist
- Trim trailing whitespace from all text bodies and stderr strings
- Sort log lines by (destination, method) tuple so order-dependence is removed (acknowledges the FR-14 concurrent-log-order limitation in the rust-sidecar spec)

### Comparison output format

On failure, the harness prints:

```
FAIL: test_case_name (model_proxy)
   Description: POST /openai/v1/chat/completions with streaming

   HTTP status: Go=200, Rust=200 (match)
   Response body (normalized):
      - chunk 0: match
      - chunk 1: DIFF
         Go:   "event: delta\ndata: {\"content\":\"pong\"}"
         Rust: "event: delta\ndata: {\"content\":\"pong\" }"   # trailing space
   Log lines:
      Go (4 lines): match
      Rust (4 lines): match

Corpus file: sidecar/tests/parity/corpus/openai_streaming.json
```

Clear enough for an operator to locate the case file and fix whichever side is wrong.

### Handling the CONNECT drain divergence test

FR-15 item 4 needs the harness to SIGTERM a running container mid-operation and observe behavior. Approach:

1. Establish a CONNECT tunnel to `github.com:443` via the sidecar egress
2. Begin sending bytes through the tunnel at a trickle (1 byte every 100ms via mock-github-ssh — we'll reuse the SSH server as a TCP target for this case)
3. Issue `docker kill --signal SIGTERM sidecar-go` (and separately for sidecar-rust)
4. Measure time until the tunnel bytes stop flowing
5. Assert: Go stops within 100ms of SIGTERM, Rust continues for 2-5 seconds (up to the 5s drain deadline), then stops
6. After this test, the containers are dead — the harness driver must recreate them before running any subsequent test cases. Keep this test LAST in the corpus ordering so the restart only happens once.

### Handling the DNS error path divergence test

FR-15 item 1: the harness needs to trigger a DNS error on the sidecar's upstream resolution, NOT the harness's own resolution.

Approach: add a 7th "mock service" — a custom DNS responder (`dnsmasq` or a small Python server) on the Docker network. Configure the compose stack so `sidecar-go` and `sidecar-rust` use it via `dns:` field. The responder is configured to return `SERVFAIL` for a specific hostname (e.g. `broken.test.docker`). The harness then asks each sidecar to CONNECT to `broken.test.docker:443` and observes:

- Go → logs an error about DNS lookup but still tries to dial (fail-open), resulting in an HTTP 502 from the dial failure
- Rust → logs an SSRF block entry AND returns HTTP 502 with a specific error body indicating the DNS fail-closed path

Assert that the log lines DIFFER in the documented direction.

This adds complexity but is the cleanest way to hermetically validate the fix. If the DNS mock proves too much scope for phase 4, it can be deferred to a followup and this specific divergence case marked as manually verified.

**Simpler alternative** (acceptable if DNS mock is too much scope): use a hostname that resolves inside both containers to a TCP port where nothing is listening. DNS succeeds, dial fails. Both sidecars should return 502. This doesn't test the DNS error path specifically but proves basic failure handling parity. Document the limitation in the corpus file for this case.

## Migration plan

Five commits on one branch `feat/sidecar-parity-harness`, in order:

### Commit 1 — scaffolding
- New workspace member at `sidecar/tests/parity/`
- `Cargo.toml` with the full dependency list
- Stub `src/main.rs` that just prints "harness not yet implemented"
- `sidecar/tests/parity/README.md` with the overview and usage placeholder
- Top-level `Cargo.toml` workspace member added
- `cargo build -p nautiloop-sidecar-parity-harness` green
- `cargo clippy -p nautiloop-sidecar-parity-harness --all-targets -- -D warnings` green

### Commit 2 — fixtures + mock services
- `fixtures/test-ca/{ca.pem,ca.key}` generated with openssl (self-signed, 10-year validity, test-only)
- `fixtures/mock-openai/{server.py,cert.pem,key.pem}` — Python HTTPS server with fixed responses per FR-6
- `fixtures/mock-anthropic/{server.py,cert.pem,key.pem}` — per FR-7
- `fixtures/mock-github-ssh/{server.py,host_key,authorized_keys}` — Python SSH server per FR-8
- `fixtures/go-secrets/` and `fixtures/rust-secrets/` directories with test credentials + known_hosts
- Each cert and key has a comment header marking it as test-only
- Mock service Dockerfiles (or inline in the compose YAML) documented

### Commit 3 — docker-compose.yml + Dockerfile.go-with-test-ca
- Full compose file with all 5 services wired
- `Dockerfile.go-with-test-ca` for the Go sidecar variant
- `docker compose up` brings the stack up; `docker compose ps` shows all 5 services as healthy
- `docker compose down` tears it down cleanly
- Manual smoke: `curl https://localhost:443/v1/models --cacert fixtures/test-ca/ca.pem` (pointing at mock-openai port-mapped to host) succeeds

### Commit 4 — corpus + harness driver
- `corpus/` populated with all cases from FR-17
- `src/main.rs` implements the driver per the Architecture section
- Parity tests pass, divergence tests pass (both as expected)
- README updated with actual usage commands

### Commit 5 — polish and followup tracking
- `sidecar/scripts/lint-no-test-utils-in-prod.sh` extended per SR-6 to check for `NAUTILOOP_EXTRA_CA_BUNDLE` in CI workflows
- README's "how to add a new corpus case" section filled in with a real template
- If any DNS-mock scope was deferred, followup issues filed and referenced in the README

## Test plan

### Harness self-checks

- `cargo build --workspace` green
- `cargo clippy --workspace --all-targets -- -D warnings` green
- `cargo test --workspace` — no regressions in the sidecar crate's unit/integration tests
- `cargo test -p nautiloop-sidecar-parity-harness` — any internal unit tests for the normalization, corpus loading, or compose orchestration helpers

### End-to-end runs

- `cargo run -p nautiloop-sidecar-parity-harness --release` — full corpus, all cases green
- Full corpus run under 5 minutes
- Forcing a known failure (e.g., modifying a Go binary response fixture) causes the harness to report a readable diff pointing at the correct corpus case
- Running with `--stop` on a failing run leaves the stack torn down
- Running without `--stop` on a failing run leaves the stack up; subsequent `docker compose down -f sidecar/tests/parity/docker-compose.yml` cleans up

### Divergence assertion checks

For each of the 4 documented divergences, verify that:
- If the Rust sidecar were "fixed" to match Go (the bug-compatible path), the corresponding test would FAIL (i.e., the divergence assertion is real, not a no-op)
- This can be verified manually by temporarily hacking the mock service or the assertion to check the other direction

### Manual smoke (out of harness)

- `docker compose up` by itself with the harness stack
- Manually hit `curl http://localhost:19090/openai/v1/models -H 'Authorization: Bearer anything'` — should return mock-openai's response with the sidecar-injected Bearer
- Manually ssh through the proxy: `ssh -p 19091 -i fixtures/go-secrets/ssh-key/id_ed25519 git@localhost git-upload-pack 'test/repo.git'` — should return mock pack bytes
- Same commands against ports 29090/29091 hit the Rust sidecar
- The two responses should be identical modulo normalization

## Security considerations

This spec introduces test infrastructure that ships test certificates and keys in the repo. Every piece is marked test-only and can never run against production hosts because the hostnames are overridden via `extra_hosts` inside a sandboxed Docker network.

### Non-negotiables

1. **No production certificates, keys, or credentials appear anywhere in `sidecar/tests/parity/fixtures/`.** Every file in `fixtures/` is freshly generated and committed with a test-only header comment.
2. **`NAUTILOOP_EXTRA_CA_BUNDLE` is only set on the `sidecar-rust` service in the harness compose file.** No production image or K8s manifest references this env var. SR-6's extended lint script enforces this.
3. **The test CA's trust boundary is the harness compose network.** The CA never signs certificates for real hostnames.
4. **The harness SSH key never authenticates against real GitHub.** It's only trusted by `mock-github-ssh`'s `authorized_keys`.

### New risks

1. **Operator confusion.** A developer sees `fixtures/test-ca/ca.key` and thinks it's a leaked production key. Mitigation: loud README comments + file header comments + a top-level `HARNESS.md` link from the main README.
2. **A rogue CI config enables `NAUTILOOP_EXTRA_CA_BUNDLE` in a prod build.** Mitigation: SR-6 lint script + grep + CI config review in code review.
3. **Mock services drift from real behavior.** If OpenAI changes their SSE wire format, the mock doesn't know and the harness still passes. This is a known limitation — the harness tests Go↔Rust parity, not correctness against live upstreams. Real upstream correctness is covered by the phase 5 manual smoke test.

## Out of scope

- **CI integration.** No GitHub Actions wiring in this spec. The harness runs locally only.
- **`cargo-deny` integration for the harness crate.** Inherits the workspace cargo-deny config; no changes needed.
- **Performance benchmarking.** Not a performance harness. Parity only.
- **Real upstream tests.** The harness is strictly hermetic. Real-API smoke testing is phase 5's job.
- **DNS rebinding full simulation.** The hermetic DNS mock for the rebinding test is optional per FR-15 item 2. A smoke-only assertion is acceptable if full simulation proves too much scope.
- **Retrofitting the harness to test any previous sidecar release.** The harness runs against whatever Go and Rust binaries are built from the current workspace. Historical regression testing is a separate concern.

## Open questions

1. **Is `paramiko` acceptable for the mock SSH server, or should it be a small Rust program using russh's server API?** Paramiko is simpler and proven; russh would dogfood our own dependency but adds implementation scope. Lean paramiko for phase 4; revisit in followup if the Python install becomes a friction point. Not blocking.
2. **Should the harness use `testcontainers-rs` instead of direct `docker compose` orchestration?** `testcontainers` gives nicer Rust ergonomics. `docker compose` gives better debuggability (`docker compose logs`, `docker compose ps`). Lean compose for visibility; testcontainers can be a followup refactor if useful. Not blocking.
3. **How to handle the mock-example HTTP service (referenced in FR-17 egress cases)?** Easiest: add a 6th mock service that's just `nginx:alpine` serving a static file. Or: use Python's `http.server`. Either works. Not blocking.
4. **Should the harness measure wall-clock time for the CONNECT drain test, or trust docker event timestamps?** Wall-clock is simpler but racey. Docker events are precise but require API access. Lean wall-clock with a generous tolerance (assert Rust > 2s, Go < 1s). Not blocking.

None are blocking. The spec is implementable as written.
