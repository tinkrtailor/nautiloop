# Lane B: Infrastructure Layer

## Overview

Postgres schema, git operations, and config loading for the Nemo control plane. These are the foundational modules that every other component depends on: the loop engine writes state to Postgres, dispatches jobs against git worktrees, and reads merged configuration to determine model preferences and limits.

> **Eng review (2026-03-27) decided Postgres over SQLite. Design doc updated.**

## Dependencies

- **Requires:** [Design doc](../docs/design.md) (architecture, resource model, configuration layers)
- **Required by:** Loop engine, job scheduler, API server, CLI

## Requirements

### Functional Requirements

#### Postgres Schema

- FR-1: The `loops` table shall store all loop state including phase (HARDEN/IMPLEMENT), stage, sub-state, round counter, current SHA, `harden_only` flag, and the engineer who owns it. When harden converges: if `harden_only`, status transitions to `converged` and phase stays `harden`; if not `harden_only`, status stays `running` and phase transitions to `implement` (after engineer approval).
- FR-2: The `jobs` table shall store every K8s job dispatched, linked to its parent loop, with status, timing, verdict JSON, and token usage.
- FR-3: The `engineers` table shall store registered engineers with their git identity, model preferences, and concurrency limits.
- FR-4: The `egress_logs` table shall store all outbound network traffic logged by the auth sidecar, linked to the originating job.
- FR-5: All schema changes shall be managed via `sqlx migrate` with sequential, timestamped migration files checked into the repo.
- FR-6: The schema shall enforce referential integrity: jobs reference loops, loops reference engineers, egress_logs reference jobs.

#### Git Operations

- FR-7: `BareRepo::fetch_and_resolve()` shall acquire the worktree mutex, run `git fetch --prune`, resolve the target ref to a SHA, and hold the mutex until the worktree is created. This serializes fetch WITH worktree creation to prevent race conditions where a ref moves between fetch and worktree creation.
- FR-8: `BareRepo::create_worktree()` shall create a worktree from the bare repo at a specified SHA (not a branch ref), returning the worktree path. The caller must resolve the SHA before calling this (via `fetch_and_resolve`).
- FR-9: `BareRepo::delete_worktree()` shall remove a worktree and run `git worktree prune`.
- FR-10: All `fetch_and_resolve` + `create_worktree` and `delete_worktree` calls shall be serialized through a `tokio::sync::Mutex` to prevent concurrent `git worktree` lock contention and fetch/worktree race conditions.
- FR-11: Branch creation shall follow the pattern `agent/{engineer}/{spec-slug}-{short-hash}` where `short-hash` is the first 8 hex chars of SHA-256 of the ORIGINAL spec file content at submission time, making branch names immutable across harden rounds. (Note: design doc examples are being updated to include hash suffix.)
- FR-12: `BareRepo::detect_divergence()` shall compare the local branch tip SHA against the remote tracking branch and classify the result into three variants: `RemoteAhead` (engineer pushed additional commits, fast-forward possible), `ForceDeviated` (histories diverged due to force push), or `LocalAhead` (normal agent operation, not a divergence).
- FR-13: On `RemoteAhead`: pause (status becomes `paused_remote_ahead`), notify engineer. On `ForceDeviated`: pause (status becomes `paused_force_deviated`), notify engineer. On `LocalAhead`: normal operation, no action. On `RemoteGone`: treat as `ForceDeviated`, pause.
- FR-13a: `paused_remote_ahead` state machine: `nemo resume <loop-id>` fast-forwards to remote SHA (no work lost), re-dispatches current stage. `nemo cancel <loop-id>` transitions to `cancelled`. No other transitions valid.
- FR-13b: `paused_force_deviated` state machine: `nemo resume --force <loop-id>` shows what commits will be discarded, then resets to remote SHA. Without `--force`, the command is rejected with an explanation of data loss. `nemo cancel <loop-id>` transitions to `cancelled`. No other transitions valid.

#### Config Loading

- FR-14: Config shall merge three layers in order: cluster (lowest priority) -> repo (`nemo.toml`) -> engineer (`~/.nemo/config.toml`, highest priority).
- FR-15: `nemo.toml` shall be parsed from the monorepo root using the `toml` crate. On the control plane, it is loaded from the worktree at job dispatch time (the bare repo contains the file). Missing file is not an error at the cluster level (bare cluster config suffices); missing file IS an error at the repo level for `nemo submit`.
- FR-16: Engineer config at `~/.nemo/config.toml` shall be optional. Missing file means no overrides.
- FR-17: Cluster config shall be read from a K8s ConfigMap mounted as a file, or from environment variables prefixed with `NEMO_CLUSTER_`.
- FR-18: Model resolution order: engineer override > repo default > cluster default. If no model is configured at any layer, fail with an explicit error naming which role (implementor/reviewer) is unconfigured.
- FR-19: `nemo init` shall scan the monorepo root for build system markers (`Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`, `build.sbt`, `foundry.toml`, `composer.json`, `Makefile`) and generate a `nemo.toml` with auto-detected `[services.*]` entries.
- FR-20: Engineer limits (e.g., `max_parallel_loops`) shall be capped by cluster limits. If an engineer sets `max_parallel_loops = 10` but the cluster cap is 5, the effective value is 5.

### Non-Functional Requirements

- NFR-1: Postgres queries on `loops` and `jobs` tables shall complete in < 5ms for single-row lookups by primary key (indexed).
- NFR-2: `create_worktree` shall complete in < 2s for repos up to 5 GB bare size, on the target CCX43 NVMe disk with warm filesystem cache.
- NFR-3: `git fetch` shall time out after 120s. Fetch failure shall not crash the control plane; the job is retried with backoff.
- NFR-4: Config parsing shall fail fast on startup with clear error messages naming the exact field and layer that failed validation.
- NFR-5: All Postgres operations shall use connection pooling (sqlx `PgPool`, max 10 connections for V1).
- NFR-6: Schema migrations shall be forward-only. No down migrations. Breaking changes require a new migration that transforms data.

## Behavior

### Postgres Schema Detail

```sql
-- 001_initial_schema.sql

CREATE TYPE loop_phase AS ENUM ('harden', 'implement');
CREATE TYPE loop_stage AS ENUM (
    'spec_audit', 'spec_revise',           -- harden phase
    'implementing', 'testing', 'reviewing' -- implement phase
);
CREATE TYPE loop_status AS ENUM (
    'running', 'converged', 'failed',
    'max_rounds_exceeded', 'paused_remote_ahead', 'paused_force_deviated', 'cancelled'
);
CREATE TYPE job_status AS ENUM (
    'pending', 'running', 'succeeded', 'failed'
);

CREATE TABLE engineers (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL UNIQUE,
    email       TEXT NOT NULL UNIQUE,
    model_preferences JSONB NOT NULL DEFAULT '{}',
    max_parallel_loops INTEGER NOT NULL DEFAULT 5,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE loops (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    engineer_id UUID NOT NULL REFERENCES engineers(id),
    spec_path   TEXT NOT NULL,
    branch      TEXT NOT NULL UNIQUE,
    phase       loop_phase NOT NULL,
    stage       loop_stage NOT NULL,
    status      loop_status NOT NULL DEFAULT 'running',
    harden_only BOOLEAN NOT NULL DEFAULT false,
    round       INTEGER NOT NULL DEFAULT 0,
    sha         TEXT NOT NULL,  -- current branch tip; set to base branch tip at loop creation
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_loops_engineer_id ON loops(engineer_id);
CREATE INDEX idx_loops_status ON loops(status);
CREATE INDEX idx_loops_branch ON loops(branch);
CREATE INDEX idx_loops_engineer_status ON loops(engineer_id, status);

CREATE TABLE jobs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    loop_id         UUID NOT NULL REFERENCES loops(id) ON DELETE CASCADE,
    stage           loop_stage NOT NULL,
    round           INTEGER NOT NULL,
    k8s_job_name    TEXT NOT NULL UNIQUE,
    status          job_status NOT NULL DEFAULT 'pending',
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    verdict_json    JSONB,        -- review/audit verdict, NULL for non-review jobs
    token_usage     JSONB,        -- {"input": N, "output": N}
    exit_code       INTEGER,
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_jobs_loop_id ON jobs(loop_id);
CREATE INDEX idx_jobs_status ON jobs(status);
CREATE INDEX idx_jobs_k8s_job_name ON jobs(k8s_job_name);

CREATE TABLE egress_logs (
    id          BIGSERIAL PRIMARY KEY,
    job_id      UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    timestamp   TIMESTAMPTZ NOT NULL DEFAULT now(),
    destination TEXT NOT NULL,     -- hostname or IP
    bytes       BIGINT NOT NULL,
    method      TEXT NOT NULL      -- HTTP method or 'TCP'
);

CREATE INDEX idx_egress_logs_job_id ON egress_logs(job_id);
CREATE INDEX idx_egress_logs_timestamp ON egress_logs(timestamp);

-- Auto-update updated_at on row modification
CREATE OR REPLACE FUNCTION update_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_loops_updated_at
    BEFORE UPDATE ON loops
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

CREATE TRIGGER trg_jobs_updated_at
    BEFORE UPDATE ON jobs
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

-- Retention: egress_logs older than 30 days auto-pruned by a scheduled task.
-- Implemented as a pg_cron job or control-plane scheduled task:
--   DELETE FROM egress_logs WHERE timestamp < now() - interval '30 days';
```

#### Column Design Rationale

- `loops.branch` is `UNIQUE` because only one active loop may exist per branch (design doc: "Submitting a spec that maps to an existing active branch is rejected").
- `loops.sha` is `NOT NULL`. When a loop is created, the branch is created from the base branch tip, and `sha` is set to that SHA immediately.
- `jobs.verdict_json` is JSONB (not a separate table) because the verdict schema is owned by the agent image and may evolve. Structured querying of verdicts is not a V1 requirement.
- `jobs.k8s_job_name` is `UNIQUE` to enable idempotent reconciliation: if the control plane restarts, it can match running K8s jobs back to DB rows.
- `engineers.model_preferences` is JSONB (`{"implementor": "claude-opus-4", "reviewer": "gpt-5.4"}`) because model names are free-form strings that change frequently.
- `egress_logs` uses `BIGSERIAL` because it is append-only, high-volume, and never updated.

#### Migration Strategy

Migrations live in `control-plane/migrations/` as `{timestamp}_{description}.sql` files. The control plane binary runs `sqlx::migrate!()` embedded at compile time on startup. This means:

1. `cargo sqlx prepare` generates offline query metadata (checked into repo).
2. On control plane boot, pending migrations run automatically before the API server or loop engine starts.
3. No separate migration binary or manual step.

### Git Operations Module

```
control-plane/src/git/
    mod.rs          -- pub mod bare_repo; pub mod branch;
    bare_repo.rs    -- BareRepo struct
    branch.rs       -- branch naming, divergence detection
```

#### BareRepo Struct

```rust
pub struct BareRepo {
    path: PathBuf,              // e.g., /data/bare-repo.git
    remote_url: String,
    worktree_mutex: tokio::sync::Mutex<()>,
}
```

**Lifecycle of a job's git operations:**

1. Loop engine calls `bare_repo.fetch_and_resolve(branch)` -- acquires mutex, runs `git fetch --prune`, resolves branch ref to a SHA, holds mutex
2. Loop engine calls `bare_repo.create_worktree(sha)` -- creates worktree at the resolved SHA (mutex still held), then releases mutex
3. K8s job runs inside the worktree
4. On job completion, loop engine calls `bare_repo.delete_worktree(path)` -- acquires mutex, runs `git worktree remove --force`, then `git worktree prune`, releases mutex

**Why fetch and worktree creation share the mutex:** Without this, a concurrent fetch could update the ref between our fetch and worktree creation, causing the worktree to check out a different SHA than intended. By holding the mutex across fetch + SHA resolution + worktree creation, we guarantee the worktree is created at the exact SHA we resolved.

**Why mutex instead of async semaphore:** `git worktree add` takes a file lock on the bare repo (`.git/worktrees/`). Concurrent calls block at the filesystem level anyway. The mutex makes the serialization explicit and avoidable (no spawning N processes that all block on the same file lock).

#### Branch Naming

```rust
pub fn branch_name(engineer: &str, spec_path: &str, original_spec_content: &[u8]) -> String {
    let slug = spec_slug(spec_path);       // "invoice-cancel" from "specs/billing/invoice-cancel.md"
    let hash = short_hash(original_spec_content); // first 8 hex chars of SHA-256(original spec file content at submission time)
    format!("agent/{engineer}/{slug}-{hash}")
}
```

The short-hash is computed from the ORIGINAL submitted spec file content (at submission time), making branch names immutable across harden rounds. The hash disambiguates when two specs produce the same slug (unlikely but possible across categories). Note: design doc examples are being updated to include hash suffix.

#### Divergence Detection

Called by the loop engine before dispatching each job:

```rust
pub enum DivergenceResult {
    /// Normal operation: agent committed, local is ahead of remote. Not a divergence.
    LocalAhead,
    /// Engineer pushed additional commits. Fast-forward is possible, no work lost.
    /// Always pauses (status → paused_remote_ahead). Engineer decides via `nemo resume`.
    RemoteAhead { local_sha: String, remote_sha: String },
    /// Histories diverged (force push or rebase). Resuming discards local commits.
    /// Always pauses (status → paused_force_deviated). Requires `nemo resume --force`.
    ForceDeviated { local_sha: String, remote_sha: String },
    /// Branch deleted on remote.
    RemoteGone,
}

impl BareRepo {
    pub async fn detect_divergence(&self, branch: &str) -> Result<DivergenceResult>;
}
```

Detection method: compare `refs/heads/{branch}` against `refs/remotes/origin/{branch}` after fetch. Use `git merge-base --is-ancestor` to classify:
- If local is ancestor of remote: `RemoteAhead` (fast-forward possible).
- If remote is ancestor of local: `LocalAhead` (normal agent operation).
- If neither is ancestor: `ForceDeviated` (histories diverged).
- If the remote ref doesn't exist: `RemoteGone`.

On `RemoteAhead`: set `loops.status = 'paused_remote_ahead'`, write the SHA mismatch to the loop record. The API exposes this so the CLI can show "Engineer pushed new commits. `nemo resume <loop-id>` to fast-forward or `nemo cancel <loop-id>`."

On `ForceDeviated`: set `loops.status = 'paused_force_deviated'`, write the SHA mismatch to the loop record. The API exposes this so the CLI can show "Branch histories diverged. `nemo resume --force <loop-id>` (discards agent work) or `nemo cancel <loop-id>`."

On `RemoteGone`: treat as `ForceDeviated` (someone deleted the branch). Same pause behavior.

**paused_remote_ahead resume flow:**
- `nemo resume <loop-id>`: re-fetches, fast-forwards `loops.sha` to current remote branch tip (no agent work is lost), re-dispatches the current stage. Transitions to `running`.
- `nemo cancel <loop-id>`: transitions to `cancelled`.
- No other transitions are valid from `paused_remote_ahead`.

**paused_force_deviated resume flow:**
- `nemo resume --force <loop-id>`: shows which local commits will be discarded, then re-fetches and resets `loops.sha` to current remote branch tip, re-dispatches the current stage. Transitions to `running`. Without `--force`, the command is rejected with an explanation of what will be lost.
- `nemo cancel <loop-id>`: transitions to `cancelled`.
- No other transitions are valid from `paused_force_deviated`.

### Config Loading Module

```
control-plane/src/config/
    mod.rs          -- pub mod cluster; pub mod repo; pub mod engineer; pub mod merged;
    cluster.rs      -- ClusterConfig
    repo.rs         -- RepoConfig (nemo.toml)
    engineer.rs     -- EngineerConfig (~/.nemo/config.toml)
    merged.rs       -- MergedConfig, merge logic
```

#### Structs

```rust
#[derive(Deserialize)]
pub struct ClusterConfig {
    pub node_size: Option<String>,
    pub provider: Option<String>,
    pub domain: String,
    pub default_implementor: Option<String>,
    pub default_reviewer: Option<String>,
    pub max_parallel_loops_cap: Option<u32>,  // hard ceiling per engineer
    pub max_cluster_jobs: Option<u32>,        // hard ceiling cluster-wide; enforced by loop engine via SELECT COUNT(*) FOR UPDATE within a transaction before dispatching (dispatch lock prevents TOCTOU)
}

#[derive(Deserialize)]
pub struct RepoConfig {
    pub repo: RepoMeta,        // name, default_branch
    pub models: Option<ModelConfig>,
    pub limits: Option<LimitsConfig>,
    pub services: HashMap<String, ServiceConfig>,
}

#[derive(Deserialize)]
pub struct EngineerConfig {
    pub identity: Option<IdentityConfig>,   // name, email
    pub models: Option<ModelConfig>,
    pub limits: Option<LimitsConfig>,
}

pub struct MergedConfig {
    pub implementor_model: String,
    pub reviewer_model: String,
    pub max_parallel_loops: u32,
    pub max_rounds_harden: u32,
    pub max_rounds_implement: u32,
    pub services: HashMap<String, ServiceConfig>,
    // ... other merged fields
}
```

#### Merge Algorithm

```rust
impl MergedConfig {
    pub fn merge(
        cluster: &ClusterConfig,
        repo: &RepoConfig,
        engineer: Option<&EngineerConfig>,
    ) -> Result<Self, ConfigError>;
}
```

For each scalar field, take the highest-priority non-None value. For limits, apply `min(engineer_value, cluster_cap)`. If a required field (like `implementor_model`) is None at all three layers, return `ConfigError::MissingField { field, role }`.

**Collection merge rules:**
- `services` HashMap: deep merge. Repo defines services; engineer cannot override existing service configs, only add new services. If engineer defines a service with the same key as one already defined in the repo config, it is silently ignored (repo wins for services).
- `models`: last-writer-wins. Engineer overrides repo, repo overrides cluster.

**Model preferences authority:** `~/.nemo/config.toml` is authoritative for model preferences. The `engineers` table stores a JSONB cache that is synced on `nemo auth`. On conflict, the config file wins.

#### Cluster Config Loading

Two sources, checked in order:

1. File at path `$NEMO_CLUSTER_CONFIG` (K8s ConfigMap mounted as a file, e.g., `/etc/nemo/cluster.toml`)
2. Environment variables: `NEMO_CLUSTER_DOMAIN`, `NEMO_CLUSTER_DEFAULT_IMPLEMENTOR`, etc.

If the file exists, it takes precedence. Environment variables fill in any fields the file doesn't set.

#### Service Detection (`nemo init`)

Scan rules (each produces a `ServiceConfig` entry):

| Marker File | Service Type | Default Test Command |
|---|---|---|
| `Cargo.toml` | rust | `cargo test` |
| `package.json` | node | `npm test` |
| `go.mod` | go | `go test ./...` |
| `pyproject.toml` | python | `pytest` |
| `build.sbt` | jvm | `sbt test` |
| `foundry.toml` | solidity | `forge test` |
| `composer.json` | php | `composer test` |
| `Makefile` (alone) | generic | `make test` |

Scan depth: configurable via `nemo init --depth N` (default 2, i.e., monorepo root + one level of subdirectories). Each directory containing a marker becomes a service. The service name is the directory name (or the repo name if the marker is at root). Warns when zero services are detected.

`nemo init` writes the generated `nemo.toml` to stdout and prompts the engineer to review before writing to disk. It never overwrites an existing `nemo.toml` without `--force`.

## Edge Cases

| Scenario | Expected Behavior |
|---|---|
| Bare repo path does not exist on startup | Control plane fails to start with clear error: "Bare repo not found at {path}. Run initial clone first." |
| `git fetch` fails (network error, auth failure) | Job dispatch is retried with backoff (30s, 120s). After 3 failures, loop is marked `failed` with `error_message = "fetch failed: {reason}"`. |
| Disk full during `create_worktree` | `git worktree add` returns non-zero. Control plane logs the error, marks the job `failed`, retries once after 60s (in case temp files were cleaned). On second failure, loop fails. |
| Bare repo corruption (bad objects, broken refs) | Detected by non-zero exit from git commands. Control plane logs error and marks loop `failed`. Recovery: manual re-clone of bare repo (out of scope for V1 auto-recovery). |
| Two loops submitted for the same spec by the same engineer | Second submission rejected: `loops.branch` UNIQUE constraint prevents duplicate. API returns 409 Conflict with message "Active loop already exists for branch {branch}". |
| Engineer pushes to agent branch during active loop (fast-forward) | Detected as `RemoteAhead` by `detect_divergence()`. Loop paused (`paused_remote_ahead`). Engineer uses `nemo resume <loop-id>` to fast-forward (no work lost). |
| Engineer force-pushes to agent branch during active loop | Detected as `ForceDeviated` by `detect_divergence()`. Loop paused (`paused_force_deviated`). Engineer must `nemo resume --force <loop-id>` (discards agent commits, accepts remote state) or `nemo cancel <loop-id>`. |
| `nemo.toml` has unknown fields | `toml` deserialization with `#[serde(deny_unknown_fields)]` returns a parse error naming the unknown field. This catches typos early. |
| `nemo.toml` references a service path that doesn't exist | Validated at config load time. Error: "Service '{name}' path '{path}' does not exist in the repo." |
| Engineer config sets model to empty string | Treated as None (not set). The merge algorithm skips empty strings. |
| Postgres connection lost during loop execution | `sqlx` PgPool retries connections automatically. If the pool is exhausted for > 30s, pending DB operations fail and the loop engine logs the error. Loops resume from last known state when the connection recovers (state is already persisted). |
| Worktree mutex held while control plane receives SIGTERM | `tokio::sync::Mutex` is dropped on process exit. The lock file left by `git worktree add` is cleaned up by `git worktree prune` on next startup. |
| Migration fails mid-apply | `sqlx migrate` runs each migration in a transaction. Failed migration rolls back. Control plane refuses to start until the migration issue is resolved manually. |

## Error Handling

| Error | Detection | Response |
|---|---|---|
| Postgres connection refused on startup | `PgPool::connect` returns error | Control plane exits with code 1 and message "Cannot connect to Postgres at {url}" |
| Migration version conflict (two developers add same timestamp) | `sqlx migrate` detects duplicate | Startup fails. Developer must renumber the migration. |
| `git worktree add` returns non-zero | Exit code check after `Command::new("git")` | Release mutex, return `GitError::WorktreeCreateFailed { stderr }` |
| TOML parse error in any config layer | `toml::from_str` returns error | Return `ConfigError::ParseFailed { layer, path, detail }` with the exact line and column |
| Branch name collision (two specs produce same slug-hash) | `loops.branch` UNIQUE constraint | API returns 409. Astronomically unlikely with 8-char hex hash (4 billion combinations) but handled. |
| Worktree path already exists (stale from crash) | `git worktree add` fails | Delete stale path, run `git worktree prune`, retry once. If retry fails, return error. |

## Out of Scope

- Automatic bare repo re-clone on corruption (V2)
- Down migrations / schema rollback (forward-only by policy)
- Multi-cluster config federation
- Git LFS support
- Partial clone / shallow clone optimizations
- Config hot-reload without control plane restart (V2)
- Postgres replication or HA (single-node V1)
- `nemo init` for polyglot monorepos with nested build systems beyond the configured depth

## Acceptance Criteria

- [ ] `sqlx migrate run` applies all migrations to a fresh Postgres 15+ database without errors
- [ ] `cargo sqlx prepare --check` passes in CI (offline query verification)
- [ ] `BareRepo::fetch_and_resolve()` acquires mutex, pulls new commits from remote, resolves ref to SHA, and holds mutex until worktree creation completes
- [ ] `BareRepo::create_worktree()` returns a valid worktree path checked out at the specified SHA (not a branch ref)
- [ ] `BareRepo::delete_worktree()` removes the worktree directory and cleans up `.git/worktrees` metadata
- [ ] Concurrent `create_worktree` calls are serialized (second call waits for first to complete, no git lock errors)
- [ ] Branch names match pattern `agent/{engineer}/{slug}-{hash}` for all valid inputs
- [ ] `detect_divergence()` returns `RemoteAhead` when engineer fast-forward-pushed, `ForceDeviated` when histories diverged, and `LocalAhead` for normal agent operation
- [ ] `RemoteAhead` sets status to `paused_remote_ahead`; `ForceDeviated` sets status to `paused_force_deviated`
- [ ] `nemo resume <loop-id>` fast-forwards on `paused_remote_ahead` (no `--force` required)
- [ ] `nemo resume --force <loop-id>` required for `paused_force_deviated`; without `--force`, command is rejected with explanation of data loss
- [ ] `nemo resume` on `paused_force_deviated` without `--force` shows which commits will be discarded
- [ ] `ON DELETE CASCADE` propagates from loops to jobs and from jobs to egress_logs
- [ ] `egress_logs` retention: records older than 30 days are pruned by scheduled task
- [ ] `updated_at` triggers fire on row updates for both loops and jobs tables
- [ ] `max_cluster_jobs` in ClusterConfig is enforced via `SELECT COUNT(*) ... FOR UPDATE` within a transaction (dispatch lock prevents TOCTOU)
- [ ] When `harden_only` loop converges harden phase, status transitions to `converged` (phase stays `harden`)
- [ ] When non-`harden_only` loop converges harden phase, status stays `running` and phase transitions to `implement` (after approval)
- [ ] `MergedConfig::merge()` correctly applies three-layer override: engineer > repo > cluster
- [ ] `MergedConfig::merge()` silently ignores engineer-defined services that collide with repo-defined service keys (repo wins)
- [ ] `MergedConfig::merge()` caps engineer `max_parallel_loops` at cluster `max_parallel_loops_cap`
- [ ] `MergedConfig::merge()` returns `ConfigError::MissingField` when no model is configured for a required role
- [ ] `nemo init` detects at least `Cargo.toml`, `package.json`, and `go.mod` in a test monorepo and generates correct `[services.*]` TOML
- [ ] `nemo init` refuses to overwrite existing `nemo.toml` without `--force`
- [ ] Control plane starts successfully with only cluster config (no repo or engineer config loaded at boot)
- [ ] Control plane refuses to start if Postgres is unreachable or migrations fail

## Open Questions

- [x] Should `egress_logs` be stored in Postgres or shipped to a separate log sink (e.g., file-based, rotated)? **Decision: Postgres, with 30-day retention.** `egress_logs` rows older than 30 days are auto-pruned by a scheduled task. `ON DELETE CASCADE` from jobs ensures cleanup on loop deletion.
- [ ] Should the `loops` table track `affected_services` (JSONB array) to enable filtering loops by service on the dashboard? Not needed for V1 loop execution but useful for `nemo status --service api`.
