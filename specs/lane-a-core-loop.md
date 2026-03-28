# Core Loop Engine, API Server, and CLI

> **Note:** Design doc originally proposed SQLite; eng review (2026-03-27) decided Postgres for concurrent access from split control plane. Design doc updated accordingly.

## Overview

The three binaries that compose the Nemo control plane: a loop engine that drives convergent loops via K8s Jobs, an API server that exposes REST endpoints for human interaction, and a CLI that wraps those endpoints. All three are Rust crates in a shared cargo workspace. See `docs/design.md` for full system context.

## Dependencies

- **Requires:** Postgres (via sqlx), k3s cluster, shared bare repo on PVC
- **Required by:** Terraform module (deploys these binaries), agent job image (receives dispatched jobs)

## Architecture

```
                        nemo CLI (engineer's Mac)
                              |
                              | HTTPS / mTLS
                              v
                    +-------------------+
                    |  API Server (k3s) |
                    |  (Deployment)     |
                    +--------+----------+
                             |
                   Postgres  |  pg_notify / polling
                             |
                    +--------v----------+
                    | Loop Engine (k3s) |
                    |  (Deployment)     |
                    +--------+----------+
                             |
                        kube-rs API
                             |
              +--------------+--------------+
              |              |              |
         K8s Job         K8s Job        K8s Job
        (implement)     (review)        (test)
```

The API server and loop engine are separate k3s Deployments. They share a Postgres database. The API server writes commands (submit, cancel, approve); the loop engine reads them and drives state transitions.

## Requirements

### Functional Requirements

- FR-1: The system shall provide a `ConvergentLoopDriver` that accepts a `LoopKind` enum (Harden or Implement) and dispatches stages accordingly
- FR-2: The system shall transition loops through states: PENDING -> HARDENING -> AWAITING_APPROVAL -> IMPLEMENTING -> TESTING -> REVIEWING -> CONVERGED | FAILED | CANCELLED, with PAUSED reachable from any active sub-state and AWAITING_REAUTH reachable from any state where a job is active (DISPATCHED or RUNNING)
- FR-3: The system shall gate implementation behind AWAITING_APPROVAL, requiring explicit `POST /approve/:id` (opt-out via `--auto-approve` on submit)
- FR-4: The system shall feed test failures back to the implement stage as structured feedback including at minimum: service name, test command, exit code, stdout (last 10KB), stderr (last 10KB)
- FR-5: The system shall create branches named `agent/{engineer}/{spec-slug}-{short-hash}` (short-hash = first 8 chars of SHA-256 of the original submitted spec content, making the branch name stable across harden rounds; supersedes design doc's loop-ID-based naming)
- FR-6: The system shall invoke headless agents: `claude -p --output-format stream-json` (implementer), `opencode run --format json` (reviewer)
- FR-7: The system shall support session continuation across rounds via `--resume` / `-s` flags
- FR-8: The system shall run `git fetch` per-job (not on a cron) to ensure freshness without stale-cache risk (supersedes design doc's cron-based fetch)
- FR-9: The system shall retry malformed verdict JSON 2x before marking the loop FAILED
- FR-10: The system shall transition to AWAITING_REAUTH when agent credentials expire mid-loop
- FR-11: The system shall track sub-states per stage: DISPATCHED / RUNNING / COMPLETED
- FR-12: The API server shall expose REST endpoints for submit, status, logs, cancel, approve, and inspect
- FR-13: The CLI shall provide commands: submit, status, logs, cancel, approve, inspect, resume, init, auth, config
- FR-14: V1: all authenticated users have full access to all loops (no per-user authorization)

### Non-Functional Requirements

- NFR-1: Loop engine reconciliation interval <= 5 seconds (time from state change to next action)
- NFR-2: API server response time < 200ms for status/logs endpoints (p99)
- NFR-3: Postgres connection pool: max 20 connections shared across API server and loop engine
- NFR-4: Loop engine must recover all in-progress loops on restart (crash recovery from Postgres state)
- NFR-5: CLI binary size < 15 MB (static Rust binary, cross-compiled for linux-amd64 and darwin-arm64)
- NFR-6: All API endpoints authenticated via API key or mTLS

## Behavior

### ConvergentLoop Trait

```rust
/// A stage in a convergent loop (e.g., Implement, Review, Audit, Revise, Test).
pub trait Stage: Send + Sync + 'static {
    /// The input this stage receives.
    type Input: Serialize + DeserializeOwned;
    /// The output this stage produces (written to DB as JSON).
    type Output: Serialize + DeserializeOwned;

    /// Human-readable name for logging ("implement", "review", "spec_audit").
    fn name(&self) -> &'static str;

    /// Build a K8s Job spec for this stage.
    fn job_spec(
        &self,
        ctx: &LoopContext,
        input: &Self::Input,
    ) -> Result<k8s_openapi::api::batch::v1::Job>;

    /// Parse the job's output artifacts into a typed Output.
    /// Called after the Job reaches Succeeded status.
    fn parse_output(
        &self,
        ctx: &LoopContext,
        job: &Job,
    ) -> Result<Self::Output>;

    /// Evaluate whether the loop should continue or converge.
    /// Returns Continue(feedback) or Converged.
    fn evaluate(
        &self,
        ctx: &LoopContext,
        output: &Self::Output,
    ) -> LoopDecision;
}

/// Shared context for all stages in a loop.
pub struct LoopContext {
    pub loop_id: Uuid,
    pub engineer: Engineer,
    pub spec_path: String,
    pub branch: String,
    pub current_sha: String,
    pub round: u32,
    pub max_rounds: u32,
    pub session_id: Option<String>,  // for --resume continuation
    pub feedback_path: Option<String>,
}

pub enum LoopDecision {
    Continue { feedback: serde_json::Value },
    Converged,
    Failed { reason: String },
}

/// Configuration for a single stage in a loop.
pub struct StageConfig {
    /// Human-readable name ("audit", "revise", "implement", "test", "review").
    pub name: &'static str,
    /// Model to invoke (e.g., "claude-opus-4", "gpt-5.4"). None for non-model stages (test).
    pub model: Option<String>,
    /// Path to the prompt template file.
    pub prompt_template: Option<String>,
    /// Stage timeout.
    pub timeout: Duration,
    /// Parse the job's output artifacts into a serde_json::Value.
    pub parse_output: fn(&LoopContext, &Job) -> Result<serde_json::Value>,
}

/// The two kinds of convergent loops. Each variant defines its own stage order.
pub enum LoopKind {
    Harden {
        audit: StageConfig,
        revise: StageConfig,
    },
    Implement {
        implement: StageConfig,
        test: StageConfig,
        review: StageConfig,
    },
}

/// The loop driver. Takes a LoopKind and matches on it to dispatch stages.
pub struct ConvergentLoopDriver {
    db: PgPool,
    kube: kube::Client,
    kind: LoopKind,
}

impl ConvergentLoopDriver {
    /// Run one tick of the loop: check current sub-state, dispatch or
    /// collect the next job, evaluate, and write the new state to Postgres.
    /// Matches on `self.kind` to determine the current stage and the next
    /// stage in sequence. No stringly-typed stage advancement.
    pub async fn tick(&self, loop_id: Uuid) -> Result<LoopState>;
}
```

See `docs/design.md` SS Two Convergent Loops for the pseudocode each loop follows.

### HardenLoop

Uses `LoopKind::Harden` with two alternating stages:

| Stage | Model | Input | Output |
|-------|-------|-------|--------|
| SpecAudit | reviewer (default: openai) | spec file path, branch | `AuditVerdict { clean: bool, issues: Vec<Issue> }` |
| SpecRevise | implementor (default: claude) | spec + audit issues | `ReviseOutput { updated_spec_path: String, sha: String }` |

Convergence: `AuditVerdict.clean == true`. Max rounds from `nemo.toml` `limits.max_rounds_harden` (default: 10).

Harden and implement loops share the same branch, named `agent/{engineer}/{spec-slug}-{short-hash}` per FR-5 (supersedes design doc's `spec/` prefix in pseudocode). Hardened spec commits land on the branch before implementation starts.

### ImplementLoop

Uses `LoopKind::Implement` with three stages per round:

| Stage | Model | Input | Output |
|-------|-------|-------|--------|
| Implement | implementor (default: claude) | spec + feedback file (if round > 1) | `ImplOutput { sha: String, affected_services: Vec<String> }` |
| Test | none (runs test commands) | affected_services from impl output | `TestOutput { passed: bool, failures: Vec<TestFailure> }` |
| Review | reviewer (default: openai) | spec + branch diff | `ReviewVerdict` (see schema below) |

If Test fails: loop feeds `TestFailure` items back as feedback to next Implement round (no Review dispatched).
If Review returns `clean: false`: loop feeds `verdict.issues` back as feedback to next Implement round.
Convergence: `ReviewVerdict.clean == true` AND `TestOutput.passed == true`. Max rounds from `nemo.toml` `limits.max_rounds_implement` (default: 15).

### State Machine

```
                              submit
                                |
                                v
                           +---------+
                           | PENDING |
                           +----+----+
                                |
               +----------------+----------------+
               | (--harden)                      | (no --harden)
               v                                 |
        +------------+                           |
   +--->| HARDENING  |                           |
   |    | [sub-state]|                           |
   |    +-----+------+                           |
   |          |                                  |
   |    audit clean?                             |
   |     no  / \ yes                             |
   |    +---+   +--------+                       |
   |    |       |        |                       |
   |    |       | harden_only?                   |
   |    |       | yes  / \ no                    |
   |    |       v     /   \                      v
   |    |  +-----------+   +-------------------+   +-------------------+
   +----+  | CONVERGED |   | AWAITING_APPROVAL |   | AWAITING_APPROVAL |
           |(harden)   |   | (if not auto)     |   | (if not auto)     |
           +-----------+   +--------+----------+   +--------+----------+
                                     |                       |
                                approve / auto-approve       |
                         |                       |
                         +----------+------------+
                                    |
                                    v
                            +---------------+
                       +--->| IMPLEMENTING  |
                       |    | [sub-state]   |
                       |    +-------+-------+
                       |            |
                       |            v
                       |    +---------------+
                       |    | TESTING       |
                       |    | [sub-state]   |
                       |    +-------+-------+
                       |            |
                       |     pass? / \ fail?
                       |          /   \
                       |         v     +-----> feedback to IMPLEMENTING
                       |    +---------------+
                       |    | REVIEWING     |
                       |    | [sub-state]   |
                       |    +-------+-------+
                       |            |
                       |     clean? / \ issues?
                       |           /   \
                       |          v     +-----> feedback to IMPLEMENTING
                       |   +-----------+
                       |   | CONVERGED |
                       |   +-----------+
                       |
                       |   (max rounds exceeded OR unrecoverable error)
                       |          |
                       |          v
                       |    +--------+
                       +--->| FAILED |
                            +--------+

Sub-states (apply to HARDENING, IMPLEMENTING, TESTING, REVIEWING):
  DISPATCHED ---> RUNNING ---> COMPLETED
  (Job created)  (Job active) (Job succeeded/failed)

Special states:
  PAUSED: entered when branch divergence is detected from any active sub-state
    (DISPATCHED or RUNNING). If divergence is detected while DISPATCHED, the
    pending Job is cancelled before transitioning.
    PAUSED ---> {previous stage}/DISPATCHED  (on `nemo resume`, same round)
    PAUSED ---> CANCELLED                    (on `nemo cancel`)

  AWAITING_REAUTH: entered when agent credentials expire mid-job, only from
    states where a job is active (DISPATCHED or RUNNING). Not reachable from
    AWAITING_APPROVAL (no job running, no auth needed).
    AWAITING_REAUTH responds to cancel. No automatic timeout in V1.
    TODO(V1.5): add configurable timeout for AWAITING_REAUTH.
    AWAITING_REAUTH ---> {previous stage}/DISPATCHED  (on re-auth via `nemo auth`, same round, re-dispatches the failed job)

  CANCELLED: terminal state (distinct from FAILED to distinguish user-initiated cancellation).

Special transitions:
  ANY active state ---> FAILED           (on unrecoverable error)
  ANY active state ---> CANCELLED        (on cancel)
  ANY active sub-state (DISPATCHED/RUNNING) ---> PAUSED  (on branch divergence; cancel pending Job if DISPATCHED)
  ANY active sub-state (DISPATCHED/RUNNING) ---> AWAITING_REAUTH  (on expired credentials; not from AWAITING_APPROVAL)
```

### Loop Engine Binary

**Startup:**
1. Connect to Postgres (sqlx pool, run migrations)
2. Initialize kube-rs client (in-cluster config)
3. Load all loops with state not in (CONVERGED, FAILED, CANCELLED) from DB
4. Start reconciliation loop (tick every loop, 5s interval)
5. Start K8s Job watcher (kube-rs `watcher()` on Jobs with label `app=nemo`)

**Reconciliation tick (per loop):**
1. Read current loop state + sub-state from Postgres
2. If sub-state == DISPATCHED: check Job status via kube-rs. If Running, update sub-state. If Succeeded/Failed, update.
3. If sub-state == COMPLETED: call `stage.parse_output()`, then `stage.evaluate()`. Write next state to DB.
4. If state == PENDING or sub-state needs new dispatch: call `stage.job_spec()`, create Job via kube-rs, set sub-state = DISPATCHED.
5. All state writes are transactional (single Postgres transaction per tick).

**K8s Job watch:**
- `kube::runtime::watcher` on `batch/v1/Job` with label selector `app=nemo`
- On Job status change: signals the reconciliation loop to wake up via channel/notify. The watcher does NOT write to Postgres directly.
- Only the reconciliation loop writes state transitions to Postgres. This eliminates race conditions between the watcher and tick.

**Per-stage timeouts:**
- implement: 30 min (default)
- review: 15 min (default)
- test: 30 min (default)
- spec-audit: 15 min (default)
- spec-revise: 15 min (default)
- Watchdog: no-output timeout of 15 min (kills job if no stdout/stderr for 15 min)
- All timeouts configurable in `nemo.toml` under `[timeouts]`

**Retry model:**
- Jobs table has `retry_count` (int, default 0) and `max_retries` (int, default 2) fields.
- On job failure (OOM, timeout, eviction): retry up to `max_retries` times with backoff (30s, 120s). Third failure marks the loop FAILED.
- On verdict parse failure: retry the review/audit job up to 2 times (same backoff).
- Retries do NOT increment the round counter. Backoff is per-retry, not per-round.
- After exhausting retries, mark loop FAILED with reason describing the failure mode.

**Verdict evaluation (FR-9):**
- After Review or Audit job completes, parse `.agent/review-verdict.json` (or `.agent/audit-verdict.json`) from the job's output PVC
- If JSON is malformed: increment `retry_count` on the job, re-dispatch the same stage (same inputs)
- If `retry_count >= max_retries`: mark loop FAILED with reason "Malformed verdict after {max_retries} retries"

### API Server Binary

**Startup:**
1. Connect to Postgres (shared pool)
2. Bind HTTP server (axum) on `:8080`
3. Apply auth middleware (API key from `Authorization: Bearer <key>` header, or mTLS client cert)

**Endpoints:**

#### `POST /submit`

Submit a spec for processing.

Request:
```json
{
  "spec_path": "specs/feature/invoice-cancel.md",
  "engineer": "alice",
  "harden": true,
  "harden_only": false,
  "auto_approve": false,
  "model_overrides": {
    "implementor": "claude-opus-4",
    "reviewer": "gpt-5.4"
  }
}
```

When `harden_only: true`, the loop terminates at CONVERGED with `phase: "harden"` after the spec passes audit, skipping AWAITING_APPROVAL and implementation entirely.

Response (201):
```json
{
  "loop_id": "a1b2c3d4-...",
  "branch": "agent/alice/invoice-cancel-a1b2c3d4",
  "state": "PENDING"
}
```

Behavior:
- Validate spec_path exists in the repo (git ls-tree on bare repo)
- Check no active loop exists for the computed branch name (409 if conflict)
- Insert loop row into Postgres with state=PENDING
- Return immediately; loop engine picks up on next reconciliation tick

#### `GET /status`

Query parameters: `?engineer=alice` (optional, defaults to authed user), `?team=true` (show all engineers).

Response (200):
```json
{
  "loops": [
    {
      "loop_id": "a1b2c3d4-...",
      "engineer": "alice",
      "spec_path": "specs/feature/invoice-cancel.md",
      "branch": "agent/alice/invoice-cancel-a1b2c3d4",
      "state": "IMPLEMENTING",
      "sub_state": "RUNNING",
      "round": 3,
      "created_at": "2026-03-27T10:00:00Z",
      "updated_at": "2026-03-27T10:32:00Z"
    }
  ]
}
```

#### `GET /logs/:id`

Stream logs for a loop via SSE (Server-Sent Events). Returns log lines from the current and past jobs for this loop. Streams in real-time while the loop is active; closes when CONVERGED, FAILED, or CANCELLED.

Query parameters: `?round=N` (optional, filter to round N), `?stage=implement` (optional, filter by stage name).

Response: `text/event-stream`
```
data: {"timestamp": "...", "stage": "implement", "round": 2, "line": "Editing src/invoice.rs..."}
data: {"timestamp": "...", "stage": "implement", "round": 2, "line": "Running cargo test..."}
```

Log source: structured log events persisted to Postgres. Pod logs are ephemeral and disappear after Job deletion, so the loop engine persists log events to a `log_events` table as they are collected. `GET /logs/:id` reads from Postgres, not from pod logs directly. During active jobs, the engine streams pod logs into Postgres in near-real-time; the API server tails from Postgres via SSE.

#### `DELETE /cancel/:id`

Cancel a running loop. Sets a `cancel_requested` flag in Postgres. The loop engine reads this flag on the next tick and:
1. Deletes the active K8s Job (if any) via kube-rs
2. Transitions state to CANCELLED with reason "Cancelled by user"

Response (200):
```json
{ "loop_id": "a1b2c3d4-...", "state": "CANCELLED", "reason": "Cancelled by user" }
```

#### `POST /approve/:id`

Approve a loop in AWAITING_APPROVAL state. Sets `approve_requested = true` in Postgres. The loop engine reads the flag on the next reconciliation tick and transitions to IMPLEMENTING/DISPATCHED.

Response (200):
```json
{ "loop_id": "a1b2c3d4-...", "state": "AWAITING_APPROVAL", "approve_requested": true }
```

The response returns the current state (still AWAITING_APPROVAL). The transition to IMPLEMENTING happens asynchronously via the loop engine.

Error (409) if loop is not in AWAITING_APPROVAL state.

#### `POST /resume/:id`

Resume a loop in PAUSED or AWAITING_REAUTH state. Sets `resume_requested = true` in Postgres. The loop engine reads the flag on the next reconciliation tick and re-dispatches the previously interrupted stage.

Response (200):
```json
{ "loop_id": "a1b2c3d4-...", "state": "PAUSED", "resume_requested": true }
```

The response returns the current state. The transition back to the active stage happens asynchronously via the loop engine.

Error (409) if loop is not in PAUSED or AWAITING_REAUTH state.

#### `GET /inspect/:user/:branch`

View detailed state of a loop by engineer and branch name. Returns the full loop record including all round history, verdicts, and feedback files.

Response (200):
```json
{
  "loop_id": "a1b2c3d4-...",
  "engineer": "alice",
  "branch": "agent/alice/invoice-cancel-a1b2c3d4",
  "state": "REVIEWING",
  "rounds": [
    {
      "round": 1,
      "implement": { "sha": "abc123", "affected_services": ["api"], "duration_s": 120 },
      "test": { "passed": false, "failures": ["api::invoice::test_cancel FAILED"] },
      "review": null
    },
    {
      "round": 2,
      "implement": { "sha": "def456", "affected_services": ["api"], "duration_s": 95 },
      "test": { "passed": true },
      "review": { "clean": false, "issues": 2, "summary": "Missing null check..." }
    }
  ]
}
```

### Communication: API Server to Loop Engine

The API server and loop engine share only Postgres. No direct RPC.

| Command | Mechanism | How engine reads it |
|---------|-----------|---------------------|
| Submit | INSERT into `loops` table | Engine's reconciliation tick picks up PENDING rows |
| Cancel | UPDATE `loops SET cancel_requested = true` | Engine checks flag each tick; deletes Job, sets CANCELLED |
| Approve | UPDATE `loops SET approve_requested = true` | Engine checks flag each tick; transitions to IMPLEMENTING, dispatches first impl Job |
| Resume | UPDATE `loops SET resume_requested = true` | Engine checks flag each tick; re-dispatches interrupted stage |
| Re-auth | UPDATE `engineer_credentials` table | Engine checks credential validity before dispatching |

Optional optimization: use Postgres `NOTIFY/LISTEN` to wake the engine immediately on writes, rather than waiting for the next 5s tick. The engine still reconciles on interval as the primary mechanism.

### CLI Binary

The CLI is a standalone Rust binary that calls the API server over HTTPS.

```
nemo <command> [options]

COMMANDS:
  submit <spec-path>        Submit a spec for processing
    --harden                Run hardening loop first
    --harden-only           Harden only, do not implement
    --auto-approve          Skip AWAITING_APPROVAL gate
    --model-impl <model>    Override implementor model
    --model-review <model>  Override reviewer model

  status                    Show your running loops
    --team                  Show all engineers' loops
    --json                  Output as JSON

  logs <loop-id>            Stream logs (follows until done)
    --round <n>             Show only round N
    --stage <stage>         Filter by stage (implement/test/review)

  cancel <loop-id>          Cancel a running loop

  approve <loop-id>         Approve a loop awaiting approval

  inspect <user>/<branch>   Show detailed loop state, round history, and verdicts

  resume <loop-id>          Resume a PAUSED or AWAITING_REAUTH loop

  init                      Scan monorepo, generate nemo.toml
    --force                 Overwrite existing nemo.toml

  auth                      Push local model credentials to cluster
    --claude                Push Claude credentials only
    --openai                Push OpenAI credentials only

  config                    Edit ~/.nemo/config.toml
    --set <key>=<value>     Set a config value
    --get <key>             Get a config value
```

Config resolution order (see `docs/design.md` SS Configuration Layers):
1. CLI flags (highest priority)
2. `~/.nemo/config.toml` (engineer)
3. `nemo.toml` in repo root (team)
4. Cluster defaults (lowest)

### Review Verdict Schema

Written by the review agent to `.agent/review-verdict.json` in the worktree:

```json
{
  "clean": false,
  "confidence": 0.85,
  "issues": [
    {
      "severity": "high",
      "category": "correctness",
      "file": "api/src/invoice.rs",
      "line": 42,
      "description": "Missing null check on customer_id before database lookup",
      "suggestion": "Add early return with 400 response if customer_id is null"
    }
  ],
  "summary": "Implementation covers the happy path but misses two edge cases in error handling.",
  "token_usage": { "input": 45000, "output": 3200 }
}
```

Fields:
- `clean` (bool, required): `true` means zero issues. This is the convergence signal.
- `confidence` (f64, 0.0-1.0, optional): Informational in V1. Used by V2 judge for multi-reviewer scoring. Omit or null if the model does not provide it.
- `issues` (array, required): Empty array when clean. Each issue has `severity` (critical/high/medium/low), `category` (correctness/security/performance/style), `file`, `line` (nullable), `description`, `suggestion`.
- `summary` (string, required): One-sentence overview for display in `nemo status`.
- `token_usage` (object, required): `input` and `output` token counts.

Validation: the loop engine validates this schema via serde deserialization. If deserialization fails, FR-9 retry logic applies.

### Audit Verdict Schema

Written by the audit agent to `.agent/audit-verdict.json` in the worktree:

```json
{
  "clean": false,
  "confidence": 0.9,
  "issues": [
    {
      "severity": "high",
      "category": "completeness",
      "description": "Missing error handling section for the cancel endpoint",
      "suggestion": "Add a section specifying behavior when cancellation is attempted on an already-cancelled invoice",
      "file": "specs/feature/invoice-cancel.md",
      "line": null
    }
  ],
  "summary": "Spec covers the happy path but omits two error-handling edge cases.",
  "token_usage": { "input": 32000, "output": 2100 }
}
```

Fields:
- `clean` (bool, required): `true` means zero issues. This is the convergence signal for the harden loop.
- `confidence` (f64, 0.0-1.0, optional): Informational in V1.
- `issues` (array, required): Empty array when clean. Each issue has `severity` (critical/high/medium/low), `category` (completeness/clarity/correctness/consistency), `description`, `suggestion`. `file` (string, optional) and `line` (int, optional) may reference spec locations but are not required for spec-level audits.
- `summary` (string, required): One-sentence overview.
- `token_usage` (object, required): `input` and `output` token counts.

Validation: same FR-9 retry logic as ReviewVerdict.

### Feedback File Schema

Written by the loop engine to `.agent/review-feedback-round-{N}.json` in the worktree before dispatching the next Implement job:

```json
{
  "round": 2,
  "source": "review",
  "issues": [
    {
      "severity": "high",
      "category": "correctness",
      "file": "api/src/invoice.rs",
      "line": 42,
      "description": "Missing null check on customer_id before database lookup",
      "suggestion": "Add early return with 400 response if customer_id is null"
    }
  ]
}
```

When the source is test failures:

```json
{
  "round": 2,
  "source": "test",
  "failures": [
    {
      "service": "api",
      "test_command": "cargo test -p api",
      "test_name": "api::invoice::test_cancel_already_cancelled",
      "exit_code": 101,
      "stdout": "thread 'test_cancel_already_cancelled' panicked at 'assertion failed: ...' (last 10KB)",
      "stderr": "error[E0425]: cannot find value `cancelled_at` in this scope (last 10KB)"
    }
  ]
}
```

The implement agent receives both the spec path and the feedback file path. The agent prompt includes: "Fix the following issues found in {source}: {issues/failures}". This is prompt injection into the agent, not spec mutation (see `docs/design.md` SS Implementation Loop Logic).

## Edge Cases

| Scenario | Expected Behavior |
|----------|-------------------|
| Submit spec that maps to an already-active branch | 409 Conflict: "Active loop exists for branch agent/alice/foo-abcd1234" |
| Engineer pushes manually to a loop's branch | Loop engine detects SHA mismatch on next dispatch. Transitions to PAUSED, notifies engineer: "Branch diverged. Resume or cancel?" |
| Two submits for the same spec by different engineers | Allowed: different branches (agent/alice/... vs agent/bob/...) |
| Cancel during AWAITING_APPROVAL | Immediate transition to CANCELLED (no Job to delete) |
| Approve a loop not in AWAITING_APPROVAL | 409: "Loop is in {current_state}, not AWAITING_APPROVAL" |
| `--auto-approve` with `--harden` | Hardening runs, then skips AWAITING_APPROVAL, implementation starts immediately |
| Loop engine crashes mid-tick | Postgres transaction uncommitted; on restart, loop resumes from last committed state. Jobs are idempotent (start from pinned SHA). |
| K8s Job OOM-killed | Retry per unified retry model: up to `max_retries` (default 2) with backoff (30s, 120s). Retries do not increment round counter. Third failure -> FAILED. See `docs/design.md` SS Failure Handling. |
| Agent produces no output files | Treat as job failure. Retry per OOM logic. |
| Credentials expire mid-job | Job exits with auth error code. Loop transitions to AWAITING_REAUTH. `nemo auth` re-pushes creds, engine resumes. |
| Max rounds exceeded | Create PR with status NEEDS_HUMAN_REVIEW and remaining issues attached. State -> FAILED with reason "Max rounds exceeded". |
| `nemo logs` on CONVERGED/CANCELLED loop | Return full historical logs from Postgres (all rounds). No SSE streaming (connection closes after last line). |
| Postgres connection lost | Loop engine retries with exponential backoff (1s, 2s, 4s... up to 60s). API server returns 503. |

## Error Handling

| Error | Code | Message | Recovery |
|-------|------|---------|----------|
| Spec file not found in repo | 404 | "Spec not found: {path}" | Check path, ensure committed to default branch |
| Active loop conflict on branch | 409 | "Active loop exists for branch {branch}" | Cancel existing loop first |
| Loop not found | 404 | "Loop not found: {id}" | Check loop ID via `nemo status` |
| Loop not in AWAITING_APPROVAL | 409 | "Cannot approve: loop is in {state}" | Wait for loop to reach approval gate |
| Engineer not registered | 401 | "Unknown engineer. Run `nemo auth` first" | Run `nemo auth` |
| Invalid API key | 401 | "Authentication failed" | Check API key in `~/.nemo/config.toml` |
| Malformed verdict JSON (retriable) | - | Internal: retries 2x then FAILED | Agent-side issue; check agent image/model |
| Expired model credentials | - | Loop -> AWAITING_REAUTH | Run `nemo auth --claude` or `--openai` |
| K8s API unreachable | 503 | "Cluster unavailable" | Check k3s health |
| Postgres unreachable | 503 | "Database unavailable" | Check Postgres pod |

## Out of Scope

- Web dashboard (V1.5, see `docs/design.md` SS Dashboard)
- DAG-based multi-implementer racing (V2+, see `docs/design.md` SS Future Vision)
- Cost tracking aggregation and surfacing (tracked in verdict schema, display is V1.5)
- Terraform provisioning module (separate spec)
- Agent job image and Dockerfile.nemo (separate spec)
- Auth sidecar and credential proxy architecture (separate spec)
- `opencode serve` persistent sidecar mode (V1 uses per-job process invocation)
- Multi-node k3s topology (V1 is single-node; code is node-count agnostic)

## Acceptance Criteria

- [ ] `ConvergentLoopDriver` with `LoopKind` enum compiles and correctly dispatches both Harden and Implement stage sequences
- [ ] Loop engine processes a loop from PENDING through CONVERGED when all stages succeed
- [ ] Loop engine processes a loop from PENDING through FAILED when max rounds exceeded
- [ ] AWAITING_APPROVAL gate blocks implementation until `POST /approve/:id` (unless auto-approve)
- [ ] Test failures produce a feedback file and re-dispatch Implement (no Review dispatched)
- [ ] Review issues produce a feedback file and re-dispatch Implement
- [ ] Malformed verdict JSON retries 2x then marks FAILED
- [ ] Expired credentials transition loop to AWAITING_REAUTH; `nemo auth` resumes it
- [ ] `POST /submit` returns 409 when branch already has an active loop
- [ ] `DELETE /cancel/:id` kills the active K8s Job and transitions to CANCELLED
- [ ] `GET /logs/:id` streams real-time SSE from active loop, returns full history for completed loop
- [ ] `GET /inspect/:user/:branch` returns full round history with verdicts
- [ ] Loop engine recovers all in-progress loops after crash/restart
- [ ] CLI `nemo submit --harden spec.md` triggers hardening then implementation pipeline
- [ ] Branch names follow `agent/{engineer}/{spec-slug}-{short-hash}` convention
- [ ] Session continuation works across rounds (agent resumes context, not cold start)

## Open Questions

- [x] ~~Should `GET /logs/:id` proxy raw pod logs or persist to Postgres?~~ Decision: persist structured log events to Postgres. Pod logs are ephemeral.
- [ ] Exact Postgres schema for the `loops`, `rounds`, `log_events`, and `engineer_credentials` tables (separate impl-plan or migration spec).
- [ ] How does `nemo auth` securely transport credentials? Options: direct K8s API (requires kubeconfig), API server relay endpoint, or SSH tunnel. Needs threat model.
