# Implementation Plan: Sidecar Containerized Parity Test Harness

**Spec:** `specs/sidecar-parity-harness.md`
**Branch:** `sidecar-parity-harness` (existing, do not recreate)
**Status:** In Progress
**Created:** 2026-04-08

## Environment notes & deferrals

The Docker daemon is not available in this agent sandbox (`/var/run/docker.sock`
is missing; the `docker` client works but cannot connect). This means:

- `docker compose build` / `up` / `down` cannot run locally in the sandbox.
- The harness driver cannot execute an end-to-end run here.
- Python mock smoke (FR-10 manual curl check) cannot run here.

What can run locally and is therefore the in-sandbox gate for completion:

- `cargo build --workspace` (including the new harness crate).
- `cargo clippy --workspace --all-targets -- -D warnings` (new crate included).
- `cargo test --workspace` (new crate ships unit tests for every pure driver
  helper: normalization, diffing, corpus loader, subnet whitelist, host-port
  resolution).
- `cargo clippy --workspace --all-targets --features nautiloop-sidecar/__test_utils -- -D warnings`.
- `cargo test --workspace --features nautiloop-sidecar/__test_utils` (7 ssh
  integration tests).
- `bash sidecar/scripts/lint-no-test-utils-in-prod.sh` (extended per FR-28).

The first real end-to-end run happens in CI when `.github/workflows/parity.yml`
fires on the PR. Per the spec itself (Dependencies section on Rust streaming):
"the harness is where this trust becomes evidence. First harness run is the
verification." CI is the documented verification environment. This is not a
blocker — it matches the spec's own expectation.

## Codebase Analysis

### Existing Implementations Found

| Component                          | Location                                                  | Status          |
| ---------------------------------- | --------------------------------------------------------- | --------------- |
| Rust sidecar (source of truth)     | `sidecar/src/*.rs`                                        | Complete (v0.2.10) |
| Rust sidecar Docker image          | `images/sidecar/Dockerfile`                               | Production ready |
| Go sidecar source                  | `images/sidecar/main.go` + `go.mod` + `go.sum`            | Present (phase 6 retires it) |
| Go sidecar Dockerfile              | NONE — must be resurrected per FR-4                       | Missing         |
| Sidecar `NAUTILOOP_EXTRA_CA_BUNDLE` support | `sidecar/src/tls.rs` (`build_client_config`)        | Complete        |
| Sidecar SSRF allowlist (CGNAT)     | `sidecar/src/ssrf.rs:94-99` (comment confirms not blocked) | Complete        |
| Go SSRF blocklist (RFC1918 only)   | `images/sidecar/main.go:43-48`                            | Complete        |
| `__test_utils` feature             | `sidecar/Cargo.toml` + `sidecar/tests/git_ssh_proxy_e2e.rs` | Complete (PR #73) |
| `lint-no-test-utils-in-prod.sh`    | `sidecar/scripts/lint-no-test-utils-in-prod.sh`           | Exists, needs FR-28 extension |
| Workspace members                  | `Cargo.toml` (cli, control-plane, sidecar)                | Needs new member |
| CI workflows                       | `.github/workflows/{pages,release}.yml`                   | `ci.yml` + `parity.yml` missing |

### Verification of key spec claims

- **CGNAT not blocked by either sidecar.** Confirmed:
  - Rust: `sidecar/src/ssrf.rs:94-99` explicit comment ("we DO NOT block 100.64.0.0/10 or TEST-NET ranges").
  - Go: `images/sidecar/main.go:43-48` only blocks `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, `127.0.0.0/8`.
- **Rust TLS accepts extra CA via `NAUTILOOP_EXTRA_CA_BUNDLE`.** Confirmed in
  `sidecar/src/tls.rs:50-98`. Rustls appends (not replaces) the webpki-roots
  default.
- **Rust hyper body streaming.** `sidecar/src/model_proxy.rs:268-278`
  uses `body.boxed()` forwarded directly to `Response::builder().body(...)`.
  This is the SSE fix path the harness will verify wall-clock.
- **Rust bare-exec rejected.** `sidecar/src/git_ssh_proxy.rs:parse_exec`
  returns `MissingRepoPath` error which maps to exit status 1 locally
  (no upstream reached). `divergence_bare_exec_*` cases rely on this.
- **Go bare-exec NOT rejected.** `images/sidecar/main.go` only validates
  `len(parts) == 2` — bare exec falls through and reaches upstream (where
  paramiko replies with exit 128).
- **Existing integration test `git_ssh_proxy_e2e.rs`** uses russh client
  and the `__test_utils` override addr. The harness driver will use russh
  identically for its `git_ssh` case category.
- **Workspace edition is 2024** (`Cargo.toml`). The harness crate MUST use
  edition 2024 as well to match workspace style.

### Patterns to Follow

| Pattern                             | Location                                   | Description                               |
| ----------------------------------- | ------------------------------------------ | ----------------------------------------- |
| thiserror for error enums           | Every sidecar source file                  | No anyhow in library code; harness binary may use anyhow |
| rustls config builder               | `sidecar/src/tls.rs:build_client_config`   | Append to webpki-roots default store       |
| Frozen PEM constants for tests      | `sidecar/src/tls.rs:144-153`               | Inline CA certs as `&'static str`         |
| russh client integration test       | `sidecar/tests/git_ssh_proxy_e2e.rs`       | Agent connects as `git` via russh client   |
| Graceful tokio shutdown             | `sidecar/src/main.rs`                      | watch channels + `tokio::select!`         |

### Files to Modify

| File                                                | Change                                                 |
| --------------------------------------------------- | ------------------------------------------------------ |
| `Cargo.toml` (workspace root)                       | Add `sidecar/tests/parity` to `[workspace] members`    |
| `sidecar/scripts/lint-no-test-utils-in-prod.sh`     | Extend per FR-28 — also block `NAUTILOOP_EXTRA_CA_BUNDLE` outside allowlist |

### Files to Create

**Harness crate:**

| File                                                                   | Purpose                                              |
| ---------------------------------------------------------------------- | ---------------------------------------------------- |
| `sidecar/tests/parity/Cargo.toml`                                      | New crate `nautiloop-sidecar-parity-harness` binary  |
| `sidecar/tests/parity/README.md`                                       | Usage docs, CGNAT rationale, manual smoke recipe     |
| `sidecar/tests/parity/src/main.rs`                                     | Binary entrypoint: CLI, orchestration, summary      |
| `sidecar/tests/parity/src/args.rs`                                     | `clap` args: `--category`, `--case`, `--stop`, `--no-rebuild`, `--subnet` |
| `sidecar/tests/parity/src/subnet.rs`                                   | FR-29 subnet whitelist validator (pure, unit-tested) |
| `sidecar/tests/parity/src/compose.rs`                                  | `docker compose build/up/down` wrappers + Drop guard |
| `sidecar/tests/parity/src/health_probe.rs`                             | Mock + sidecar health polling with timeouts          |
| `sidecar/tests/parity/src/corpus.rs`                                   | Corpus JSON schema + loader + validator              |
| `sidecar/tests/parity/src/normalize.rs`                                | FR-19 normalization (pure, unit-tested)              |
| `sidecar/tests/parity/src/diff.rs`                                     | Diff engine comparing Go vs Rust results             |
| `sidecar/tests/parity/src/introspection.rs`                            | Mock log `/__harness/logs` + `/__harness/reset` client |
| `sidecar/tests/parity/src/tls_client.rs`                               | rustls client config with the test CA loaded        |
| `sidecar/tests/parity/src/runner/mod.rs`                               | Dispatch on `category` field                         |
| `sidecar/tests/parity/src/runner/model_proxy.rs`                       | Model proxy cases + wall-clock streaming measurement |
| `sidecar/tests/parity/src/runner/egress.rs`                            | Egress HTTP + CONNECT + origin-form raw TCP          |
| `sidecar/tests/parity/src/runner/git_ssh.rs`                           | russh client, 5 parity + 2 bare-exec divergence     |
| `sidecar/tests/parity/src/runner/health.rs`                            | `GET /healthz` + `HEAD /healthz`                     |
| `sidecar/tests/parity/src/runner/divergence_drain.rs`                  | `docker kill --signal SIGTERM` + tunnel byte timing |
| `sidecar/tests/parity/src/result.rs`                                   | `CaseResult` + summary types                        |
| `sidecar/tests/parity/src/report.rs`                                   | Stdout progress + `harness-run.log` dump            |

**Docker compose + docker files:**

| File                                                                         | Purpose                                             |
| ---------------------------------------------------------------------------- | --------------------------------------------------- |
| `sidecar/tests/parity/docker-compose.yml`                                    | Service definitions + `parity-net` bridge          |
| `sidecar/tests/parity/Dockerfile.go-sidecar`                                 | Resurrect Go build from `images/sidecar/main.go`   |
| `sidecar/tests/parity/Dockerfile.go-with-test-ca`                            | Go + appended test CA in `/etc/ssl/certs/ca-certificates.crt` |

**Mock services:**

| File                                                                                      | Purpose                                        |
| ----------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `sidecar/tests/parity/fixtures/mock-openai/server.py`                                     | hypercorn ASGI app (HTTPS :443, HTTP :80, introspection :9999) |
| `sidecar/tests/parity/fixtures/mock-openai/Dockerfile`                                    | `python:3.12-slim` + hypercorn                 |
| `sidecar/tests/parity/fixtures/mock-openai/cert.pem`                                      | TLS cert SAN = api.openai.com                 |
| `sidecar/tests/parity/fixtures/mock-openai/key.pem`                                       | TLS key                                       |
| `sidecar/tests/parity/fixtures/mock-anthropic/server.py`                                  | Same shape with anthropic handlers            |
| `sidecar/tests/parity/fixtures/mock-anthropic/Dockerfile`                                 |                                                |
| `sidecar/tests/parity/fixtures/mock-anthropic/cert.pem`                                   | SAN = api.anthropic.com                       |
| `sidecar/tests/parity/fixtures/mock-anthropic/key.pem`                                    |                                                |
| `sidecar/tests/parity/fixtures/mock-github-ssh/server.py`                                 | paramiko SSH server + HTTP introspection      |
| `sidecar/tests/parity/fixtures/mock-github-ssh/Dockerfile`                                |                                                |
| `sidecar/tests/parity/fixtures/mock-github-ssh/host_key`                                  | Ed25519 host key                              |
| `sidecar/tests/parity/fixtures/mock-github-ssh/authorized_keys`                           | Trusts harness client key                     |
| `sidecar/tests/parity/fixtures/mock-example-http/server.py`                               | Plain HTTP mock with /foo, /redirect          |
| `sidecar/tests/parity/fixtures/mock-example-http/Dockerfile`                              |                                                |
| `sidecar/tests/parity/fixtures/mock-tcp-echo/server.py`                                   | Raw TCP echo + healthcheck                    |
| `sidecar/tests/parity/fixtures/mock-tcp-echo/Dockerfile`                                  |                                                |
| `sidecar/tests/parity/fixtures/test-ca/ca.pem`                                            | Test CA cert (committed)                      |
| `sidecar/tests/parity/fixtures/test-ca/ca.key`                                            | Test CA private key (committed, test-only)    |
| `sidecar/tests/parity/fixtures/test-ca/README.md`                                         | Loud test-only warning                        |
| `sidecar/tests/parity/fixtures/test-ca/regenerate-test-ca.sh`                             | Regeneration script per SR-7                  |
| `sidecar/tests/parity/fixtures/go-secrets/model-credentials/openai`                       | `sk-test-openai-key`                          |
| `sidecar/tests/parity/fixtures/go-secrets/model-credentials/anthropic`                    | `sk-ant-test-key`                             |
| `sidecar/tests/parity/fixtures/go-secrets/ssh-key/id_ed25519`                             | Harness client key                            |
| `sidecar/tests/parity/fixtures/go-secrets/ssh-known-hosts/known_hosts`                    | Trusts mock-github-ssh                        |
| `sidecar/tests/parity/fixtures/rust-secrets/...`                                          | Identical layout, symlink avoided (separate mounts) |

**Test corpus (one file per case):**

| File                                                                 | Case                                                      |
| -------------------------------------------------------------------- | --------------------------------------------------------- |
| `sidecar/tests/parity/corpus/openai_get_v1_models.json`              | model_proxy                                               |
| `...openai_post_chat_completions_nonstream.json`                     | model_proxy                                               |
| `...anthropic_post_v1_messages_nonstream.json`                       | model_proxy                                               |
| `...openai_bare_prefix.json`                                         | model_proxy                                               |
| `...anthropic_bare_prefix.json`                                      | model_proxy                                               |
| `...unknown_route_returns_403.json`                                  | model_proxy                                               |
| `...openai_client_auth_header_overwritten.json`                      | model_proxy                                               |
| `...anthropic_client_api_key_overwritten.json`                       | model_proxy                                               |
| `...anthropic_client_version_passthrough.json`                       | model_proxy                                               |
| `...openai_credential_refresh_per_request.json`                      | model_proxy                                               |
| `...egress_connect_egress_target.json`                               | egress                                                    |
| `...egress_connect_egress_target_no_port.json`                       | egress                                                    |
| `...egress_http_get_example.json`                                    | egress                                                    |
| `...egress_http_get_example_with_port.json`                          | egress                                                    |
| `...egress_http_origin_form_repair.json`                             | egress (raw TCP)                                          |
| `...egress_http_strips_proxy_connection.json`                        | egress                                                    |
| `...egress_http_no_redirect_follow.json`                             | egress                                                    |
| `...egress_dns_error_both_fail_502.json`                             | egress                                                    |
| `...ssh_upload_pack_matching_repo.json`                              | git_ssh                                                   |
| `...ssh_receive_pack_matching_repo.json`                             | git_ssh                                                   |
| `...ssh_wrong_repo_path_rejected_locally.json`                       | git_ssh                                                   |
| `...ssh_rejects_non_git_exec.json`                                   | git_ssh                                                   |
| `...ssh_rejects_env_request.json`                                    | git_ssh                                                   |
| `...healthz_post_ready_returns_200.json`                             | health                                                    |
| `...healthz_head_method_parity.json`                                 | health                                                    |
| `...divergence_sse_streaming_openai.json`                            | divergence                                                |
| `...divergence_sse_streaming_anthropic.json`                         | divergence                                                |
| `...divergence_bare_exec_upload_pack_rejection.json`                 | divergence                                                |
| `...divergence_bare_exec_receive_pack_rejection.json`                | divergence                                                |
| `...divergence_connect_drain_on_sigterm.json`                        | divergence (order_hint=last)                              |

**CI:**

| File                            | Purpose                                             |
| ------------------------------- | --------------------------------------------------- |
| `.github/workflows/ci.yml`      | FR-24: `rust-checks` + `rust-checks-with-test-utils` + `prod-leak-lint` |
| `.github/workflows/parity.yml`  | FR-24/FR-25: `parity-harness` job, 10min timeout   |

### Risks & Considerations

1. **No local Docker.** Mitigated by scoping the in-sandbox gate to cargo
   checks + lint script. CI validates the end-to-end path.
2. **Test CA PEM is committed.** Required by spec. Mitigation: loud header in
   `ca.key`, README warning, path under `tests/parity/fixtures/`, FR-28 lint
   prevents the env var leaking to production files.
3. **Workspace edition 2024.** New crate must use edition 2024. Fixed in plan.
4. **`__test_utils` feature for workspace-wide harness build.** The harness
   crate does NOT need `__test_utils` — it connects as an agent client to
   BOTH real sidecar binaries running in containers, the test override
   escape hatch is not used. This keeps the harness clean.
5. **Mock services and committed TLS cert expiry.** Using 10-year `-days 3650`
   per SR-8.
6. **Paramiko flush timing.** FR-10 requires manual curl smoke. Documented
   in README; first CI run is the automated verification.
7. **SSH key material committed.** Test-only, mock server host key and harness
   client key both live under `fixtures/`. Only used to talk to the paramiko
   mock on the hermetic parity-net. Never used outside the harness.

## Plan

Priority-ordered steps. Work sequentially. Each step commits on green.

### Step 1: Workspace member + harness crate skeleton + subnet validator

**Why this first:** Foundation. Without the crate, nothing compiles.
Subnet validator is a pure, unit-testable module we can fully lock down
before touching Docker infrastructure.

**Files:**
- `Cargo.toml` (add member)
- `sidecar/tests/parity/Cargo.toml`
- `sidecar/tests/parity/src/main.rs` (minimal binary that calls into
  a module, not a stub — wires args parsing + subnet validation + prints
  a helpful "no cases run, use --case / --category" message if `--dry-run`
  is set; real orchestration lands in Step 3)
- `sidecar/tests/parity/src/args.rs`
- `sidecar/tests/parity/src/subnet.rs`
- `sidecar/tests/parity/src/corpus.rs` (schema + loader, no runner yet)
- `sidecar/tests/parity/src/normalize.rs` (pure functions + tests)
- `sidecar/tests/parity/src/diff.rs` (pure functions + tests)
- `sidecar/tests/parity/src/result.rs`
- `sidecar/tests/parity/src/report.rs`
- `sidecar/tests/parity/src/tls_client.rs`
- `sidecar/tests/parity/README.md`

**Approach:**
- Declare the crate with `edition = "2024"` to match workspace.
- Dependency versions match the spec snippet in "Crate structure".
- Subnet validator: `ipnet::Ipv4Net::contains(&inner)` loop over the four
  safe constants. Exposes `fn validate_subnet_whitelist(s: &str) -> Result<Ipv4Net>`.
- Normalization: pure functions taking borrowed inputs, returning owned
  `String` with normalized content. Each rule from FR-19 gets a function.
- Diff: compare two `NormalizedCaseOutput` structs and report field-level
  diffs with a human readable pointer back at the case file.
- Corpus loader: `serde_json` deserialization of the FR-21 schema,
  validation that at most one case has `order_hint: "last"`.
- TLS client: identical pattern to `sidecar/src/tls.rs` — build rustls
  `ClientConfig` that appends test CA to `webpki-roots`.
- README: document CGNAT rationale, how to run, what requires Docker,
  the manual smoke curl commands.

**Tests:**
- `subnet::tests` — whitelist cases pass, non-whitelist fail, malformed
  CIDR fails, `--subnet 100.64.0.0/10` passes (equality case).
- `normalize::tests` — each normalization rule individually.
- `diff::tests` — `normalize` + `diff` on synthetic inputs catches injected
  deltas.
- `corpus::tests` — schema parse of a minimal fixture, `order_hint` panic.

**Depends on:** nothing
**Blocks:** all other steps

**Commit:** `feat(sidecar): scaffold parity harness crate with subnet whitelist`

### Step 2: Fixtures (test CA + per-mock certs + go/rust secrets + ssh keys)

**Why this second:** Mock services and docker files need cert material to
compile/build. All fixture bytes can be generated now once and committed.

**Files:**
- `sidecar/tests/parity/fixtures/test-ca/ca.pem`
- `sidecar/tests/parity/fixtures/test-ca/ca.key`
- `sidecar/tests/parity/fixtures/test-ca/README.md`
- `sidecar/tests/parity/fixtures/test-ca/regenerate-test-ca.sh`
- `sidecar/tests/parity/fixtures/mock-openai/{cert,key}.pem`
- `sidecar/tests/parity/fixtures/mock-anthropic/{cert,key}.pem`
- `sidecar/tests/parity/fixtures/mock-github-ssh/host_key`
- `sidecar/tests/parity/fixtures/mock-github-ssh/host_key.pub`
- `sidecar/tests/parity/fixtures/mock-github-ssh/authorized_keys`
- `sidecar/tests/parity/fixtures/go-secrets/model-credentials/openai`
- `sidecar/tests/parity/fixtures/go-secrets/model-credentials/anthropic`
- `sidecar/tests/parity/fixtures/go-secrets/ssh-key/id_ed25519`
- `sidecar/tests/parity/fixtures/go-secrets/ssh-key/id_ed25519.pub`
- `sidecar/tests/parity/fixtures/go-secrets/ssh-known-hosts/known_hosts`
- `sidecar/tests/parity/fixtures/rust-secrets/...` (identical shape)
- Each file above gets loud "test-only" headers where binary format permits.

**Approach:**
- Use `openssl` to generate CA + mock certs per SR-7.
- Use `ssh-keygen -t ed25519` to generate harness client key + host key.
- `known_hosts`: add the mock-github-ssh host key with hostname `github.com`
  AND `100.64.0.12` — matches FR-2 `extra_hosts` and the host key presented
  by mock-github-ssh.
- Populate both `go-secrets/` and `rust-secrets/` identically. Two separate
  mounts keeps the concurrent-write regression case (`openai_credential_refresh_per_request`)
  decoupled between sidecars.
- `regenerate-test-ca.sh` is idempotent, uses exactly the openssl invocation
  from SR-7.
- Test CA SAN: add DNS:api.openai.com to mock-openai cert, DNS:api.anthropic.com
  to mock-anthropic cert.

**Tests:** None (fixture-only step). Validation is implicit: Step 4's
mock services will fail to start if certs are wrong, and Step 5's driver
will fail SSH auth if keys are wrong.

**Depends on:** Step 1 (fixtures live inside the new crate tree)
**Blocks:** Steps 4, 5

**Commit:** `chore(sidecar): add parity harness test fixtures (CA, certs, ssh keys)`

### Step 3: docker-compose + go sidecar Dockerfiles + compose helpers

**Why this third:** Compose file and Dockerfiles are small and need to
reference fixtures (Step 2). The ComposeStack/ComposeGuard wrappers live
in the Rust driver and are unit-testable (the subprocess runner can be
covered by testing argument construction without actually spawning).

**Files:**
- `sidecar/tests/parity/docker-compose.yml`
- `sidecar/tests/parity/Dockerfile.go-sidecar`
- `sidecar/tests/parity/Dockerfile.go-with-test-ca`
- `sidecar/tests/parity/src/compose.rs`
- `sidecar/tests/parity/src/health_probe.rs`

**Approach:**
- Compose file: 7 services per FR-2, custom bridge network `parity-net`
  with `subnet: ${PARITY_NET_SUBNET:-100.64.0.0/24}`, `extra_hosts` on
  both sidecars, port publishing matching FR-2 exactly.
- Both sidecars use `depends_on` with `condition: service_healthy` on
  every mock (FR-3). Sidecars themselves have no Docker healthcheck.
- Per-container `mem_limit: 512m` (NFR-8).
- `Dockerfile.go-sidecar`: multi-stage, golang:1.22-alpine builder, scratch
  runtime. **Context must be the repo root** so the COPY can reach
  `images/sidecar/main.go`. Compose `build.context` is `../../..` relative
  to the dockerfile.
- `Dockerfile.go-with-test-ca`: builds golang stage same as above plus
  an alpine ca-builder stage that appends the test CA pem to
  `/etc/ssl/certs/ca-certificates.crt`, then copies the augmented bundle
  into scratch. Must reference the test CA via a build context path
  relative to the compose context, which must include the fixtures tree.
  Simplest: use `sidecar/tests/parity/` as the context for this image
  and COPY the main.go from `../../../images/sidecar/main.go`. BUT Docker
  does not allow parent-directory COPY. **Resolution:** set the compose
  `build.context` to the repo root for this image and use `build.dockerfile`
  set to `sidecar/tests/parity/Dockerfile.go-with-test-ca`, then COPY
  both `images/sidecar/main.go` and `sidecar/tests/parity/fixtures/test-ca/ca.pem`
  from that top-level context. Same approach for `Dockerfile.go-sidecar`.
- `compose.rs`: thin wrapper over `std::process::Command::new("docker")`
  building `compose build`, `compose up -d`, `compose down -v --remove-orphans`
  invocations. The wrapper sets `PARITY_NET_SUBNET` env before spawning,
  uses a working directory of the parity harness dir, and captures stderr.
  Exposes `fn build()`, `fn up()`, `fn down()`.
- `ComposeGuard`: RAII type. `Drop::drop` runs `docker compose down -v
  --remove-orphans` synchronously unless `disarm()` was called. Used as
  the NFR-7 safety net.
- `health_probe.rs`: `wait_mock_health(timeout)` polls the five mock
  health endpoints (four HTTP `/_healthz`, one TCP connect) with 200ms
  backoff until 60s. `wait_sidecar_ready(timeout)` polls both `/healthz`
  endpoints until 200 with 200ms backoff up to 30s.

**Tests:**
- `compose::tests::builds_expected_cli_args` — constructs a ComposeStack,
  inspects the debug-formatted command args for each method. Verifies
  `-f docker-compose.yml`, `down -v --remove-orphans`, etc. This catches
  silent regressions without spawning docker.
- `compose::tests::guard_disarm_suppresses_down` — uses a mock command
  runner trait so Drop's behavior can be observed without spawning docker.
- `health_probe::tests::deadline_expires_returns_error` — point at an
  unbindable port and assert the error wraps the mock name in the message.

**Depends on:** Steps 1, 2
**Blocks:** Steps 4, 5, 6

**Commit:** `feat(sidecar): add parity compose stack, go dockerfiles, compose runner`

### Step 4: Mock services (python hypercorn + paramiko) + introspection API

**Why this fourth:** Mock services are independent of the Rust driver's
runner logic but required for compose to come up. Each has its own
Dockerfile. Dependencies pinned per FR-10.

**Files:**
- `sidecar/tests/parity/fixtures/mock-openai/{Dockerfile,server.py,pyproject.toml}`
- `sidecar/tests/parity/fixtures/mock-anthropic/{Dockerfile,server.py}`
- `sidecar/tests/parity/fixtures/mock-github-ssh/{Dockerfile,server.py}`
- `sidecar/tests/parity/fixtures/mock-example-http/{Dockerfile,server.py}`
- `sidecar/tests/parity/fixtures/mock-tcp-echo/{Dockerfile,server.py}`

**Approach:**
- All HTTP mocks: Starlette ASGI app served by hypercorn, binding
  `0.0.0.0:443` (HTTPS with the committed test CA signed cert),
  `0.0.0.0:80` (plain HTTP for healthz), `0.0.0.0:9999` (plain HTTP
  introspection). 3 separate hypercorn binds.
- Introspection store: thread-safe in-memory list of request records;
  `/__harness/logs` returns JSON, `/__harness/reset` clears.
  Healthz `/_healthz` explicitly NOT logged (FR-13).
- SSE cases: `/v1/chat/completions` (openai) and `/v1/messages`
  (anthropic) detect `stream: true` in JSON body and return
  `Content-Type: text/event-stream` with 3 events spaced 100ms apart
  via `async def body(): yield ...; await asyncio.sleep(0.1); ...`,
  followed by `data: [DONE]\n\n`. hypercorn flushes on each yield.
- Origin IP recording: Starlette request `.client.host` gives the source
  IP needed to attribute requests Go-vs-Rust (FR-18).
- mock-github-ssh: paramiko `ServerInterface` implementation, accepts
  any client key (matches FR-9), exec command parser matches FR-9
  exactly (git-upload-pack / git-receive-pack with `test/repo.git`
  (with or without quotes), reject everything else with exit 128,
  `env`/`pty-req`/`subsystem`/`shell`/`x11-req` reject by returning
  False). Plus introspection HTTP server on port 9999 (in separate
  thread with `http.server.ThreadingHTTPServer`, custom handler). Plus
  raw-TCP health listener on port 2200 that accepts and immediately
  closes the connection.
- mock-example-http: binds both `:80` and `:8080` with identical
  handlers per FR-11.
- mock-tcp-echo: `asyncio.start_server` on `0.0.0.0:443` with a handler
  that echoes every chunk back until client closes. No healthz — the
  docker healthcheck uses a TCP connect via `nc -z 127.0.0.1 443`.

**Tests:** None at this step (Python code, exercised in CI).

**Depends on:** Steps 1, 2, 3
**Blocks:** Step 5, 6

**Commit:** `feat(sidecar): add parity mock services (openai/anthropic/ssh/http/tcp-echo)`

### Step 5: Corpus + driver runners + introspection client + tls client

**Why this fifth:** Real implementation of each case category. This
is the largest step; it completes the harness logic.

**Files:**
- `sidecar/tests/parity/corpus/*.json` (all 30 cases)
- `sidecar/tests/parity/src/introspection.rs`
- `sidecar/tests/parity/src/runner/mod.rs`
- `sidecar/tests/parity/src/runner/model_proxy.rs`
- `sidecar/tests/parity/src/runner/egress.rs`
- `sidecar/tests/parity/src/runner/git_ssh.rs`
- `sidecar/tests/parity/src/runner/health.rs`
- `sidecar/tests/parity/src/runner/divergence_drain.rs`
- `sidecar/tests/parity/src/main.rs` (wire full orchestration)

**Approach:**
- Corpus JSON: fill in every case with deterministic inputs and
  expected outputs. For divergence cases, populate `expected_parity:
  false` and the `divergence` object.
- introspection client: `reqwest` calls to host-published ports
  49990-49993, `/__harness/logs` returns a Vec<ObservedRequest>, split
  by source IP for Go/Rust attribution.
- model_proxy runner: `reqwest::Client` with the test-CA-augmented
  rustls config (for whatever upstream uses the CA), targeted at
  `http://localhost:19090` (Go) and `http://localhost:29090` (Rust).
  For each non-streaming case, issue to both in parallel, compare.
  For divergence_sse_streaming cases, use reqwest `stream()` and
  timestamp every chunk arrival; assert first-chunk-time bounds
  (Rust < 200ms, Go >= 250ms).
- egress runner: most cases use `reqwest` configured with `proxy(
  http://localhost:190[9]2)` (Go) and `19092 / 29092` (Rust) for
  the CONNECT + absolute-form HTTP cases. The origin-form repair
  case uses raw `tokio::net::TcpStream` because reqwest can't send a
  malformed proxy request. DNS error case sends GET to
  `http://deliberately-unresolvable.invalid/`.
- git_ssh runner: `russh::client` connecting to
  `127.0.0.1:19091` (Go) and `127.0.0.1:29091` (Rust). Uses the committed
  harness client key from `go-secrets/ssh-key/id_ed25519`. Opens session,
  sends exec, captures exit status and stderr. Maps to the 5 git_ssh
  parity cases + 2 divergence_bare_exec cases.
- health runner: `reqwest` GET /healthz and a raw TCP HEAD request
  (reqwest supports HEAD natively, use that).
- divergence_drain runner: establishes CONNECT via egress port,
  starts byte trickle (1 byte per 100ms) in a background task, after
  500ms calls `docker kill --signal SIGTERM sidecar-go` then
  `sidecar-rust` and timestamps first observation of tunnel closure
  on each side. Asserts Go closes within 200ms, Rust continues 2-5s.
- main.rs orchestration: follows the FR-17 sequence exactly with
  explicit log lines at each phase.

**Tests:**
- `introspection::tests::log_attribution_by_source_ip` — synthesize a
  JSON log response, assert filter-by-source-ip.
- `runner::model_proxy::tests::first_chunk_assertion_bounds_logic` —
  pure function that takes a Vec<(Duration, Bytes)> and returns
  pass/fail for Rust/Go divergence assertion. Unit test it.
- `runner::mod::tests::dispatches_on_category` — given a case with
  each category, call `runner::dispatch_category(&case)` and assert
  it doesn't panic. The actual network work is skipped via a mock
  `RunnerContext` with fake ports.

**Depends on:** Steps 1, 2, 3, 4
**Blocks:** Step 6

**Commit:** `feat(sidecar): add parity test corpus and runner modules`

### Step 6: CI workflows + FR-28 lint extension

**Why this sixth:** CI is the primary automated gate for the end-to-end
path given the sandbox Docker absence. The lint extension must ship
together so FR-28 is enforced from day one.

**Files:**
- `.github/workflows/ci.yml`
- `.github/workflows/parity.yml`
- `sidecar/scripts/lint-no-test-utils-in-prod.sh` (extend per FR-28)

**Approach:**
- `ci.yml`: three jobs per FR-24 point 1-3. No path filter. Ubuntu-24.04,
  rust-stable action, cache cargo target. The `rust-checks-with-test-utils`
  job runs `cargo test --workspace --features nautiloop-sidecar/__test_utils`.
- `parity.yml`: one `parity-harness` job. Path-filtered
  `on.pull_request.paths`/`on.push.paths` match `sidecar/**`,
  `images/sidecar/**`, `specs/sidecar-*.md`. Timeout 10 minutes per FR-25.
  Steps: checkout, install docker (ubuntu runners already have it),
  install rust stable, `cargo run -p nautiloop-sidecar-parity-harness
  --release -- --stop`. On failure: upload `sidecar/tests/parity/harness-run.log`
  and `docker compose -f sidecar/tests/parity/docker-compose.yml logs` output
  as artifacts.
- Lint script extension: add a second check for `NAUTILOOP_EXTRA_CA_BUNDLE`
  references outside `sidecar/tests/parity/docker-compose.yml` and
  `.github/workflows/parity.yml`. Use `git grep -l` with pathspecs.
  Bail out with clear error naming the stray references.

**Tests:**
- Run `bash sidecar/scripts/lint-no-test-utils-in-prod.sh` locally —
  should pass (no stray refs).
- Verify exit code 0.

**Depends on:** Steps 1-5
**Blocks:** Step 7

**Commit:** `ci(sidecar): add parity + rust-checks workflows, extend no-leak lint`

### Step 7: Final polish, README, status flip, cargo verifies

**Why this seventh:** Finalize the impl-plan, verify baseline still green,
document any learnings.

**Files:**
- `sidecar/tests/parity/README.md` (finalize)
- `specs/sidecar-parity-harness-impl-plan.md` (mark complete, progress log)
- `.claude/learnings.md` (add discoveries)

**Approach:** Run the full local gate suite (clippy, test, lint script),
capture output, flip the plan's `Status` to `Complete`, add entries to
`Progress Log`, capture learnings.

**Tests:**
- `cargo build --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy --workspace --all-targets --features nautiloop-sidecar/__test_utils -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --features nautiloop-sidecar/__test_utils`
- `bash sidecar/scripts/lint-no-test-utils-in-prod.sh`

**Depends on:** Steps 1-6

**Commit:** `docs(sidecar): mark parity harness implementation complete`

## Acceptance Criteria Status

Mapped from spec Requirements section.

| Criterion                                                           | Status |
| ------------------------------------------------------------------- | ------ |
| FR-1: Harness layout                                                | ⬜     |
| FR-2: docker-compose with 7 services + CGNAT bridge                 | ⬜     |
| FR-3: depends_on healthcheck gating                                 | ⬜     |
| FR-4: Dockerfile.go-sidecar                                         | ⬜     |
| FR-5: Dockerfile.go-with-test-ca                                    | ⬜     |
| FR-6: Rust sidecar image reused AS-IS                               | ⬜     |
| FR-7: mock-openai endpoints + SSE flush                             | ⬜     |
| FR-8: mock-anthropic endpoints + SSE flush                          | ⬜     |
| FR-9: mock-github-ssh paramiko semantics                            | ⬜     |
| FR-10: Python pinning + manual curl smoke recipe                    | ⬜     |
| FR-11: mock-example dual port + handlers                            | ⬜     |
| FR-12: mock-tcp-echo raw TCP                                        | ⬜     |
| FR-13: Introspection API on :9999                                   | ⬜     |
| FR-14: SSH mock introspection via threaded HTTP server              | ⬜     |
| FR-15: Harness crate at sidecar/tests/parity/                       | ⬜     |
| FR-16: Image freshness enforced by default                          | ⬜     |
| FR-17: Driver lifecycle sequence                                    | ⬜     |
| FR-18: Per-test run flow (reset + parallel + source_ip attribution) | ⬜     |
| FR-19: Normalization rules                                          | ⬜     |
| FR-20: Filtering flags                                              | ⬜     |
| FR-21: Corpus schema + one file per case                            | ⬜     |
| FR-22: All 30 corpus cases                                          | ⬜     |
| FR-23: Scope reductions documented                                  | ⬜     |
| FR-24: Two CI workflows                                             | ⬜     |
| FR-25: 10-minute parity job timeout                                 | ⬜     |
| FR-26: Artifact uploads on failure                                  | ⬜     |
| FR-27: cargo-deny inheritance                                       | ⬜     |
| FR-28: Lint script extension                                        | ⬜     |
| FR-29: Subnet whitelist override                                    | ⬜     |
| NFR-1: Runnable on dev workstation with Docker                      | ⬜     |
| NFR-2: Warm run < 5 min                                             | ⬜     |
| NFR-3: Clippy clean                                                 | ⬜     |
| NFR-4: Hermetic                                                     | ⬜     |
| NFR-5: Diffs name the case JSON                                     | ⬜     |
| NFR-6: Determinism                                                  | ⬜     |
| NFR-7: Drop guard teardown                                          | ⬜     |
| NFR-8: 512MB memory bound                                           | ⬜     |
| NFR-9: Full artifact dump on failure                                | ⬜     |
| SR-1 through SR-9                                                   | ⬜     |

## Progress Log

| Date       | Step | Status  | Notes                                     |
| ---------- | ---- | ------- | ----------------------------------------- |
| 2026-04-08 | —    | Started | Plan created, baseline cargo checks pass  |

## Learnings

(to be filled during execution)

## Bugs Found

(to be filled during execution)

## Blockers / Notes

- Sandbox has no Docker daemon. Full harness end-to-end runs can only be
  executed in CI. In-sandbox verification is cargo + lint.
