# Implementation Plan: Core Loop Engine, API Server, and CLI

**Spec:** `specs/lane-a-core-loop.md`
**Status:** IN_PROGRESS
**Branch:** `feat/core-loop-engine`

## Overview

Greenfield implementation of Cargo workspace with three crates: `control-plane` (lib+bin), `cli` (bin). The control-plane binary runs both the API server and loop engine as async tasks. The CLI is a standalone binary calling the API.

## Steps

### Step 1: Cargo Workspace Scaffold
**Status:** PENDING
- Create root `Cargo.toml` workspace with `control-plane` and `cli` members
- Create `control-plane/Cargo.toml` with dependencies: axum, sqlx (postgres), kube, k8s-openapi, tokio, serde, serde_json, thiserror, uuid, chrono, tracing, tracing-subscriber, sha2, clap
- Create `cli/Cargo.toml` with dependencies: reqwest, clap, serde, serde_json, tokio, tracing
- Minimal `main.rs` for both crates that compiles
- Verify: `cargo check --workspace`

### Step 2: Domain Types and Error Handling
**Status:** PENDING
- `control-plane/src/lib.rs` - crate root with module declarations
- `control-plane/src/error.rs` - `NemoError` enum using thiserror
- `control-plane/src/types.rs` - Core types: `LoopState`, `SubState`, `LoopKind`, `LoopDecision`, `LoopContext`, `StageConfig`
- `control-plane/src/types/verdict.rs` - `ReviewVerdict`, `AuditVerdict`, `Issue`, `TestFailure`, `FeedbackFile`
- `control-plane/src/types/api.rs` - Request/response types for all API endpoints
- All types derive Serialize/Deserialize where appropriate

### Step 3: Database Layer (sqlx + migrations)
**Status:** PENDING
- `control-plane/migrations/` - SQL migration files for loops, rounds, log_events, engineer_credentials tables
- `control-plane/src/state/mod.rs` - Database trait `StateStore` and Postgres implementation
- `control-plane/src/state/postgres.rs` - `PgStateStore` implementing all DB operations
- Operations: create_loop, get_loop, update_state, create_round, get_rounds, append_log, get_logs, set_flag (cancel/approve/resume), get_active_loops, credential operations
- All queries use sqlx with compile-time checking where possible (offline mode for CI)

### Step 4: Loop Engine Core (Stage trait + ConvergentLoopDriver)
**Status:** PENDING
- `control-plane/src/loop_engine/mod.rs` - Module root
- `control-plane/src/loop_engine/stage.rs` - `Stage` trait definition
- `control-plane/src/loop_engine/driver.rs` - `ConvergentLoopDriver` with `tick()` method
- `control-plane/src/loop_engine/stages/harden.rs` - SpecAudit and SpecRevise stage implementations
- `control-plane/src/loop_engine/stages/implement.rs` - Implement, Test, Review stage implementations
- `control-plane/src/loop_engine/reconciler.rs` - Reconciliation loop (5s interval, per-loop tick)
- `control-plane/src/loop_engine/watcher.rs` - K8s Job watcher using kube::runtime::watcher
- Implements: state machine transitions, sub-state tracking, retry model, verdict parsing, feedback file generation
- Branch naming: `agent/{engineer}/{spec-slug}-{short-hash}` per FR-5

### Step 5: K8s Job Dispatch
**Status:** PENDING
- `control-plane/src/k8s/mod.rs` - K8s client wrapper trait `JobDispatcher`
- `control-plane/src/k8s/client.rs` - Real kube-rs implementation
- `control-plane/src/k8s/job_builder.rs` - Job spec construction from StageConfig
- Operations: create_job, delete_job, get_job_status, watch_jobs
- Job labels: `app=nemo`, `loop-id`, `stage`, `round`

### Step 6: Git Operations
**Status:** PENDING
- `control-plane/src/git/mod.rs` - Git operations trait `GitOperations`
- `control-plane/src/git/bare.rs` - Implementation for bare repo operations
- Operations: validate_spec_exists (ls-tree), create_branch, get_current_sha, detect_divergence, fetch
- Branch naming helper: `agent/{engineer}/{spec-slug}-{short-hash}`

### Step 7: Config Loading
**Status:** PENDING
- `control-plane/src/config/mod.rs` - Config types and loading
- `NemoConfig` from `nemo.toml` (repo-level): limits, timeouts, model defaults
- `EngineerConfig` from `~/.nemo/config.toml`: API key, server URL, engineer name
- `ClusterConfig` from environment: Postgres URL, K8s namespace
- Config resolution: CLI flags > engineer config > repo config > cluster defaults

### Step 8: API Server (axum)
**Status:** PENDING
- `control-plane/src/api/mod.rs` - Router setup
- `control-plane/src/api/handlers.rs` - Endpoint handler functions
- `control-plane/src/api/auth.rs` - Auth middleware (API key / mTLS)
- `control-plane/src/api/sse.rs` - SSE log streaming for GET /logs/:id
- Endpoints: POST /submit, GET /status, GET /logs/:id, DELETE /cancel/:id, POST /approve/:id, POST /resume/:id, GET /inspect/:user/:branch
- Error responses per spec error table

### Step 9: Control Plane Binary
**Status:** PENDING
- `control-plane/src/main.rs` - Binary entry point
- Starts: Postgres pool, kube-rs client, API server (axum), loop engine reconciler, K8s job watcher
- Graceful shutdown on SIGTERM

### Step 10: CLI Binary
**Status:** PENDING
- `cli/src/main.rs` - CLI entry point with clap
- `cli/src/commands/` - One module per command: submit, status, logs, cancel, approve, inspect, resume, init, auth, config
- `cli/src/client.rs` - HTTP client wrapper for API calls
- `cli/src/config.rs` - Config loading (~/.nemo/config.toml)
- SSE streaming for `nemo logs`

### Step 11: Unit Tests
**Status:** PENDING
- State machine transition tests (all valid transitions, invalid transition rejection)
- ConvergentLoopDriver tick tests with mock StateStore and JobDispatcher
- Verdict parsing tests (valid, malformed, retry logic)
- Feedback file generation tests
- API handler tests with mock StateStore
- Branch naming tests
- Config resolution tests

### Step 12: Integration Tests
**Status:** PENDING
- Full loop happy path: PENDING -> CONVERGED (with mock K8s)
- Full loop failure path: max rounds exceeded -> FAILED
- Cancel mid-loop test
- Approve gate test
- Crash recovery test (restart with in-progress loops)
- API endpoint integration tests

## Blockers

None identified. All requirements are clear from the spec.

## Learnings

(captured during implementation)
