# Nautiloop Architecture & Lifecycle Diagrams

This document contains detailed ASCII art diagrams covering every major subsystem,
data flow, and lifecycle in Nautiloop. It is the visual companion to `docs/design.md`
and the three specs (`lane-a-core-loop.md`, `lane-b-infrastructure.md`,
`lane-c-agent-runtime.md`).

---

## 1. System Architecture Overview

```
                          +---------------------------+
                          |   Engineer's Machine      |
                          |                           |
                          |   nemo CLI (Rust binary)  |
                          |   ~/.nemo/config.toml     |
                          |   ~/.claude/ (credentials)|
                          +------------+--------------+
                                       |
                                       | HTTPS (API key auth)
                                       |
           ============================|==============================
           |           k3s Cluster (Hetzner CCX43)                   |
           |                           |                             |
           |  Namespace: nautiloop-system   |                             |
           |  +------------------------v-----------------------+     |
           |  |            API Server (Deployment)             |     |
           |  |            axum on :8080                       |     |
           |  |            REST: submit, status, logs,         |     |
           |  |                  cancel, approve, inspect      |     |
           |  +------------------------+-----------------------+     |
           |                           |                             |
           |                     reads | writes                      |
           |                           |                             |
           |  +------------------------v-----------------------+     |
           |  |              Postgres 16 (Pod + PVC)           |     |
           |  |              loops, jobs, engineers,           |     |
           |  |              egress_logs, log_events           |     |
           |  +------------------------+-----------------------+     |
           |                           |                             |
           |                     reads | writes                      |
           |                           |                             |
           |  +------------------------v-----------------------+     |
           |  |          Loop Engine (Deployment)              |     |
           |  |          Reconciliation tick (5s)              |     |
           |  |          K8s Job watcher (kube-rs)             |     |
           |  |          State machine driver                  |     |
           |  +----+-----------+-----------+----------+--------+     |
           |       |           |           |          |              |
           |       |       kube-rs API     |          |              |
           |       |           |           |          |              |
           |  +----v---+  +----v---+  +----v---+  +--v-----+        |
           |  |Bare Repo|  |       |  |       |  |        |        |
           |  |  (PVC)  |  |       |  |       |  |        |        |
           |  | 100 Gi  |  |       |  |       |  |        |        |
           |  +----+----+  |       |  |       |  |        |        |
           |       |       |       |  |       |  |        |        |
           |  Namespace: nautiloop-jobs |  |       |  |        |        |
           |  +----v-----------v---v--v-------v--v--------v--+     |
           |  |                                              |     |
           |  |    Agent Job Pods (K8s Jobs)                 |     |
           |  |                                              |     |
           |  |  +----------+  +----------+  +----------+   |     |
           |  |  | impl job |  | test job |  |review job|   |     |
           |  |  | (claude) |  | (runner) |  | (openai) |   |     |
           |  |  +----------+  +----------+  +----------+   |     |
           |  |                                              |     |
           |  +----------------------------------------------+     |
           |                                                       |
           =============================|===========================
                                        |
                        +---------------+---------------+
                        |               |               |
               +--------v---+  +--------v---+  +--------v--------+
               | Git Remote |  | Anthropic  |  | OpenAI          |
               | (GitHub)   |  | API        |  | API             |
               |            |  | api.       |  | api.openai.com  |
               |            |  | anthropic. |  |                 |
               +------------+  | com        |  +-----------------+
                               +------------+
```

**Key network flows:**

- `nemo CLI` --> `API Server`: HTTPS with API key auth (submit, status, cancel, approve)
- `API Server` <--> `Postgres`: SQL reads/writes (shared state)
- `Loop Engine` <--> `Postgres`: SQL reads/writes (state machine transitions)
- `Loop Engine` --> `K8s API`: Job create/delete/watch (kube-rs)
- `Agent Pods` --> `localhost sidecar` --> `Model APIs`: auth-injected model calls
- `Agent Pods` --> `localhost sidecar` --> `Git Remote`: SSH-proxied git push
- `Bare Repo PVC`: mounted into agent pods as worktree source

---

## 2. Job Pod Internal Architecture

```
+===========================================================================+
|  K8s Job Pod: nautiloop-a3f2b1c9-implement-r2                                 |
|  Labels: nautiloop.dev/loop-id, nautiloop.dev/stage, nautiloop.dev/engineer,            |
|          nautiloop.dev/round                                                   |
|  restartPolicy: Never                                                     |
|                                                                           |
|  +----------------------------------+  +-------------------------------+  |
|  |  AGENT CONTAINER                 |  |  AUTH SIDECAR CONTAINER       |  |
|  |  (claude-code OR opencode)       |  |  (Go static binary, ~10 MB)  |  |
|  |                                  |  |                               |  |
|  |  User: 1000 (non-root)          |  |  Ports:                       |  |
|  |  readOnlyRootFilesystem: true    |  |   :9090  Model API proxy     |  |
|  |                                  |  |   :9091  Git SSH proxy       |  |
|  |  Env vars:                       |  |   :9092  Egress logger       |  |
|  |   STAGE, SPEC_PATH, BRANCH,     |  |   :9093  /healthz (K8s only) |  |
|  |   SHA, MODEL, ROUND, LOOP_ID,   |  |                               |  |
|  |   SESSION_ID, FEEDBACK_PATH,    |  |  Reads on each request:       |  |
|  |   MAX_ROUNDS,                    |  |   /secrets/model-credentials  |  |
|  |   GIT_AUTHOR_NAME/EMAIL,        |  |   /secrets/ssh-key            |  |
|  |   (Claude: session auth, no     |  |                               |  |
|  |    ANTHROPIC_BASE_URL needed)    |  |  Writes:                      |  |
|  |   OPENAI_BASE_URL=              |  |   /tmp/shared/ready           |  |
|  |     http://localhost:9090/openai,|  |   (readiness signal)          |  |
|  |   HTTP_PROXY=                    |  |                               |  |
|  |     http://localhost:9092,       |  |  Egress logger:               |  |
|  |   GIT_SSH_COMMAND=              |  |   Logs all outbound traffic   |  |
|  |     localhost:9091 wrapper       |  |   JSON-lines to stdout        |  |
|  |                                  |  |   Does NOT block/filter       |  |
|  |  Entrypoint:                     |  |                               |  |
|  |   1. Poll /tmp/shared/ready     |  |  On SIGTERM:                  |  |
|  |      (100ms, 30s timeout)        |  |   5s drain, then exit        |  |
|  |   2. Load prompt template        |  |                               |  |
|  |   3. Inject variables            |  +-------------------------------+  |
|  |   4. exec claude/opencode        |  |                               |  |
|  |   5. Write /output/result.json   |  |  VOLUME MOUNTS (sidecar):    |  |
|  |      + stdout                    |  |                               |  |
|  |                                  |  |  /secrets/model-credentials   |  |
|  +----------------------------------+  |    <-- K8s Secret             |  |
|  |                                  |  |       (nautiloop-creds-{engineer}) |  |
|  |  VOLUME MOUNTS (agent):         |  |                               |  |
|  |                                  |  |  /secrets/ssh-key             |  |
|  |  /work       <-- Bare repo PVC  |  |    <-- K8s Secret             |  |
|  |                  (worktree)      |  |                               |  |
|  |  /sessions   <-- Session PVC    |  |  /tmp/shared                  |  |
|  |                  (cross-round)   |  |    <-- emptyDir (shared)      |  |
|  |  /specs      <-- ConfigMap/PVC  |  |                               |  |
|  |  /output     <-- emptyDir       |  +-------------------------------+  |
|  |  /tmp/shared <-- emptyDir       |                                     |
|  |                  (shared w/      |                                     |
|  |                   sidecar)       |                                     |
|  |  /tmp        <-- emptyDir       |                                     |
|  |                  (writable tmp)  |                                     |
|  |                                  |                                     |
|  |  NO /secrets mount!             |                                     |
|  +----------------------------------+                                     |
|                                                                           |
+===========================================================================+

NETWORK FLOW:

  Agent container                    Auth sidecar                  External
  +------------+                     +------------+                +--------+
  |            |  model API call     |            |  authenticated |        |
  | claude -p  | --- direct ------> |            | ------------> | api.   |
  |            |  (session auth)     |            |  HTTPS        | anthro |
  |            |                     |            |               | pic.com|
  |            |  git push           |            |  SSH w/ key   |        |
  |            | ---- :9091 ------> | SSH proxy  | ------------> | github |
  |            |  localhost          | inject key |               | .com   |
  |            |                     |            |               |        |
  |            |  any HTTP           |            |  proxied      |        |
  |            | ---- :9092 ------> | egress log | ------------> | any    |
  |            |  localhost          | (passthru) |               | host   |
  +------------+                     +------------+                +--------+

NETWORKPOLICY (nautiloop-jobs namespace):

  +------------------+     +------------------+
  | Agent container  |     | Auth sidecar     |
  |                  |     |                  |
  | Egress ALLOWED:  |     | Egress ALLOWED:  |
  |  - 127.0.0.1/32  |     |  - 0.0.0.0/0    |
  |    (localhost     |     |    (all)         |
  |     only)         |     |                  |
  |                  |     | + kube-dns :53   |
  | Egress DENIED:   |     |                  |
  |  - everything    |     |                  |
  |    else          |     |                  |
  +------------------+     +------------------+

  Result: Agent MUST go through sidecar.
  Agent cannot reach the internet directly.
  Agent never sees raw credentials.
```

---

## 3. Full Loop Lifecycle (Harden + Implement)

```
Engineer                API Server          Postgres            Loop Engine              K8s / Agent Pods
  |                        |                   |                    |                        |
  | nemo submit            |                   |                    |                        |
  |  --harden spec.md      |                   |                    |                        |
  |----------------------->|                   |                    |                        |
  |                        | INSERT loop       |                    |                        |
  |                        | state=PENDING     |                    |                        |
  |                        | harden=true       |                    |                        |
  |                        |------------------>|                    |                        |
  |  201 {loop_id, branch} |                   |                    |                        |
  |<-----------------------|                   |                    |                        |
  |                        |                   |                    |                        |
  |                        |                   |  reconciliation    |                        |
  |                        |                   |  tick (<=5s)       |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    |                        |
  |                        |                   | read PENDING loop  |                        |
  |                        |                   |------------------->|                        |
  |                        |                   |                    |                        |
  .                        .                   .  HARDENING         .                        .
  .                        .                   .  HARDEN ROUND 1    .                        .
  |                        |                   |                    |                        |
  |                        |                   | set stage=         |                        |
  |                        |                   | SPEC_AUDIT         |                        |
  |                        |                   | sub=DISPATCHED     |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    | create Job              |
  |                        |                   |                    | (audit, openai)         |
  |                        |                   |                    |----------------------->|
  |                        |                   |                    |                        |
  |                        |                   |                    |  Job watcher: RUNNING   |
  |                        |                   |                    |<-----------------------|
  |                        |                   | sub=RUNNING        |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    |                        |
  |                        |                   |                    |  Job watcher: SUCCEEDED |
  |                        |                   |                    |<-----------------------|
  |                        |                   | sub=COMPLETED      |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    |                        |
  |                        |                   |                    | parse audit-verdict.json|
  |                        |                   |                    | verdict.clean == false  |
  |                        |                   |                    |                        |
  |                        |                   | set stage=         |                        |
  |                        |                   | SPEC_REVISE        |                        |
  |                        |                   | sub=DISPATCHED     |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    | create Job              |
  |                        |                   |                    | (revise, claude)        |
  |                        |                   |                    |----------------------->|
  |                        |                   |                    |           ...           |
  |                        |                   |                    |  Job watcher: SUCCEEDED |
  |                        |                   |                    |<-----------------------|
  |                        |                   | sub=COMPLETED      |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    |                        |
  |                        |                   |                    | evaluate: loop back     |
  |                        |                   |                    | to SPEC_AUDIT round 2   |
  .                        .                   .                    .                        .
  .                        .    ... rounds repeat until audit       .                        .
  .                        .        verdict.clean == true ...       .                        .
  |                        |                   |                    |                        |
  |                        |                   |                    | audit clean! converge   |
  |                        |                   | state=             |                        |
  |                        |                   | AWAITING_APPROVAL  |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    |                        |
  | nemo approve <id>      |                   |                    |                        |
  |----------------------->|                   |                    |                        |
  |                        | UPDATE            |                    |                        |
  |                        | approve_requested |                    |                        |
  |                        | = true            |                    |                        |
  |                        |------------------>|                    |                        |
  |  200 OK                |                   |                    |                        |
  |<-----------------------|                   |                    |                        |
  |                        |                   |                    |                        |
  |                        |                   |  reconciliation    |                        |
  |                        |                   |  tick              |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    |                        |
  .                        .                   .  IMPLEMENTING      .                        .
  .                        .                   .  IMPL ROUND 1      .                        .
  |                        |                   |                    |                        |
  |                        |                   | set stage=         |                        |
  |                        |                   | IMPLEMENTING       |                        |
  |                        |                   | sub=DISPATCHED     |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    | create Job              |
  |                        |                   |                    | (implement, claude)     |
  |                        |                   |                    |----------------------->|
  |                        |                   |                    |           ...           |
  |                        |                   |                    |  SUCCEEDED              |
  |                        |                   |                    |<-----------------------|
  |                        |                   | sub=COMPLETED      |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    |                        |
  |                        |                   | set stage=TESTING  |                        |
  |                        |                   | sub=DISPATCHED     |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    | create Job (test)       |
  |                        |                   |                    |----------------------->|
  |                        |                   |                    |           ...           |
  |                        |                   |                    |  SUCCEEDED              |
  |                        |                   |                    |<-----------------------|
  |                        |                   | sub=COMPLETED      |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    |                        |
  |                        |                   |                    | tests passed            |
  |                        |                   |                    |                        |
  |                        |                   | set stage=         |                        |
  |                        |                   | REVIEWING          |                        |
  |                        |                   | sub=DISPATCHED     |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    | create Job              |
  |                        |                   |                    | (review, openai)        |
  |                        |                   |                    |----------------------->|
  |                        |                   |                    |           ...           |
  |                        |                   |                    |  SUCCEEDED              |
  |                        |                   |                    |<-----------------------|
  |                        |                   | sub=COMPLETED      |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    |                        |
  |                        |                   |                    | verdict.clean == false  |
  |                        |                   |                    | write feedback file     |
  .                        .                   .                    .                        .
  .                        .                   .  IMPL ROUND 2      .                        .
  |                        |                   |                    |                        |
  |                        |                   | round=2, stage=    |                        |
  |                        |                   | IMPLEMENTING       |                        |
  |                        |                   | sub=DISPATCHED     |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    | create Job              |
  |                        |                   |                    | (impl + feedback,       |
  |                        |                   |                    |  --resume SESSION_ID)   |
  |                        |                   |                    |----------------------->|
  |                        |                   |                    |           ...           |
  |                        |                   |                    |  (impl -> test -> review|
  |                        |                   |                    |   all pass/clean)       |
  |                        |                   |                    |                        |
  |                        |                   |                    | verdict.clean == true!  |
  |                        |                   |                    | create PR               |
  |                        |                   | state=CONVERGED    |                        |
  |                        |                   |<-------------------|                        |
  |                        |                   |                    |                        |
  | nemo status            |                   |                    |                        |
  |----------------------->|                   |                    |                        |
  |  CONVERGED, PR #42     |                   |                    |                        |
  |<-----------------------|                   |                    |                        |
```

### QA Stage (deferred)

Deferred v2 work: runs acceptance-criteria verification after review-clean, before CONVERGED. Gated by `[qa] enabled = true` in nemo.toml. See `specs/qa-stage.md`.

---

## 4. State Machine Diagram

```
                                     submit
                                       |
                                       v
                                 +-----------+
                                 |  PENDING  |
                                 +-----+-----+
                                       |
                      +----------------+----------------+
                      | (--harden)                      | (no --harden)
                      v                                 v
               +--------------+                  +--------------+
          +--->|  HARDENING   |                  |  AWAITING_   |<---+
          |    |              |                  |  APPROVAL    |    |
          |    | Sub-states:  |                  | (if not auto)|    |
          |    | .DISPATCHED  |                  +--------------+    |
          |    | .RUNNING     |                        |            |
          |    | .COMPLETED   |                   approve /         |
          |    +------+-------+                   auto-approve      |
          |           |                                |            |
          |     audit clean?                           |            |
          |      no / \ yes                            |            |
          |     /     \                                |            |
          +----+       +-->  AWAITING_APPROVAL  -------+            |
          (loop)            (if not auto-approve)                   |
                                                                    |
                            +---------------------------------------+
                            |
                            v
                     +---------------+
                +--->| IMPLEMENTING  |<---------+<---------+
                |    | .DISPATCHED   |          |          |
                |    | .RUNNING      |          |          |
                |    | .COMPLETED    |          |          |
                |    +-------+-------+          |          |
                |            |                  |          |
                |            v                  |          |
                |    +---------------+          |          |
                |    |   TESTING     |          |          |
                |    | .DISPATCHED   |          |          |
                |    | .RUNNING      |          |          |
                |    | .COMPLETED    |          |          |
                |    +-------+-------+          |          |
                |            |                  |          |
                |      pass? | fail?            |          |
                |           / \                 |          |
                |          /   \                |          |
                |         v     +-- feedback ---+          |
                |    +---------------+    (test failures   |
                |    |  REVIEWING    |     skip review)    |
                |    | .DISPATCHED   |                     |
                |    | .RUNNING      |                     |
                |    | .COMPLETED    |                     |
                |    +-------+-------+                     |
                |            |                             |
                |      clean? | issues?                    |
                |            / \                           |
                |           /   \                          |
                |          v     +---- feedback -----------+
                |                      (review issues)
                |   +-----------+
                |   | CONVERGED |  <-- terminal (PR created)
                |   +-----------+
                |
                |    (max rounds exceeded)
                |            |
                |            v
                |      +--------+
                +----->| FAILED |  <-- terminal (unrecoverable)
                       +--------+

  ============================================================
  INTERRUPT STATES (reachable from ANY active/running state):
  ============================================================

  ANY active state ----[cancel requested]----> +------------+
                                               | CANCELLED  |  <-- terminal
                                               +------------+

  ANY RUNNING sub-state ----[branch diverged]----> +--------+
                                                   | PAUSED |
                                                   +---+----+
                                                       |
                                          nemo resume / \ nemo cancel
                                                     /   \
                                                    v     v
                                          {prev stage}  CANCELLED
                                          /DISPATCHED

  ANY active state ----[creds expired]----> +------------------+
                                            | AWAITING_REAUTH  |
                                            +--------+---------+
                                                     |
                                                nemo auth
                                                     |
                                                     v
                                           {prev stage}/DISPATCHED

  FAILED ----[nemo extend --add N]----> {failed_from_state}/DISPATCHED
             (only when failed_from_state is set; resumes at the
              stage where the loop failed, e.g. IMPLEMENTING or REVIEWING)

  NOTE: LoopRecord carries two resume-tracking fields:
    - reauth_from_state: which stage the loop was in when it entered
      AWAITING_REAUTH, enabling correct resume to the interrupted stage.
    - failed_from_state: which stage the loop was in when it entered
      FAILED, enabling `nemo extend` to resume at the right point.

  ============================================================
  SUB-STATE TRANSITIONS (within each stage):
  ============================================================

      +------------+     Job pod      +----------+    Job exits     +-----------+
      | DISPATCHED | --- started ---> |  RUNNING | --- 0 or !0 --> | COMPLETED |
      | (Job       |                  | (Job     |                 | (Job      |
      |  created)  |                  |  active) |                 |  done)    |
      +------------+                  +----------+                 +-----------+

  ============================================================
  TERMINAL STATES:    CONVERGED    FAILED    CANCELLED
  ============================================================
```

---

## 5. Orchestrator Judge

The orchestrator judge is an LLM call at loop transition points that decides `continue | exit_clean | exit_escalate | exit_fail` when the reviewer's verdict is ambiguous or the loop is churning.

**Where it runs:** in-process from the loop engine, reusing the sidecar model proxy. It is NOT a separate pod — the loop engine makes a direct model API call through the existing proxy infrastructure.

**When it fires:**

- On review-clean-but-with-medium+-severity issues remaining
- On `round >= max_rounds` with recurring findings across rounds
- On audit ambiguity (reviewer verdict contradicts its own prior round)

**Data it reads:**

- Full spec text
- Round history (all prior verdicts and findings)
- Current verdict and finding list
- Recurring-finding analysis (findings that appeared in 2+ consecutive rounds)

**Storage:** every judge decision is persisted to the `judge_decisions` table:

| Column | Type | Description |
|--------|------|-------------|
| `loop_id` | UUID | Foreign key to loops |
| `round` | int | Round number when judge fired |
| `phase` | text | `implement` or `harden` |
| `trigger` | text | Why the judge was invoked (e.g. `recurring_findings`, `ambiguous_verdict`) |
| `input_json` | jsonb | Snapshot of data sent to the judge |
| `decision` | text | One of `continue`, `exit_clean`, `exit_escalate`, `exit_fail` |
| `confidence` | float | Model-reported confidence (0.0–1.0) |
| `reasoning` | text | Model-generated explanation |
| `hint` | text | Optional hint passed to the next agent round |
| `duration_ms` | int | Wall-clock time for the judge call |
| `created_at` | timestamptz | When the decision was recorded |
| `loop_final_state` | text | Terminal state of the loop (backfilled when loop ends) |
| `loop_terminated_at` | timestamptz | When the loop reached a terminal state (backfilled) |

**Future:** judge decisions provide a training signal for a resident fine-tuned judge model in v2.

---

## 6. Dashboard

The dashboard is a web UI served by the existing axum control plane at `/dashboard/*`. No new process, no new binary — the same server that handles API requests renders the dashboard.

**Rendering:** server-rendered HTML via `maud` (the workspace uses `maud = "0.26"`). A single embedded JS file polls `/dashboard/state` every 5 seconds for live updates.

**Auth model:** uses the existing API key. Browser sessions get an HttpOnly cookie after initial API-key auth. Programmatic access uses Bearer token (same API key).

**Features:**

- Card grid: one card per active loop with status, round, elapsed time, and cost
- Loop detail: rounds table with per-round diff viewer and live log tail
- Feed: terminal events stream (convergences, failures, cancellations)
- Per-spec history: all loops run against a given spec, with outcome and duration
- Stats deep-dive: `/dashboard/stats?window=7d` shows cost breakdown, convergence rate, engineer activity
- Kill switch: cancel any running loop from the UI
- Fleet summary header: total active loops, queue depth, cluster utilization

**Security:** the dashboard inherits the deployment's security posture. On the Hetzner terraform module, that means Tailscale-only access by default. In local dev, the dashboard binds to localhost. Never expose the dashboard to the public internet without fronting auth (oauth2-proxy or similar).

**No new database:** all aggregates are computed on-demand from the existing `loops` and `rounds` tables, with a 60-second cache on the stats endpoint.

---

## 7. Pluggable Cache

One PVC `nautiloop-cache` mounted at `/cache` on implement and revise pods. The cache is shared across all loops and rounds — a warm cache from one loop benefits the next.

**Env-var passthrough:** the `[cache.env]` table in `nemo.toml` becomes pod environment variables verbatim. The control plane does not interpret these values or have per-backend code. Any tool that respects environment variables for cache location works out of the box.

**Covered tools:**

- sccache (default for Rust workspaces) — `SCCACHE_DIR=/cache/sccache`, `RUSTC_WRAPPER=sccache`
- ccache, npm, pnpm, yarn, bun, pip, poetry, uv, turbo, go, gradle
- Any tool that wants a writable directory and an env var to point at it

**Operational:** `nemo cache show` prints the resolved env-var mapping, disk usage, and recent hit-rate stats.

**Terraform:** `cache_volume_size` variable (default 50 GiB) controls the PVC size.

---

## 8. Observability

`nemo ps` lists running agent pods with status, resource usage, and current stage. It queries the k8s API directly and joins with loop state from Postgres to show a unified view.

`/pod-introspect/:id` returns live pod details including a logs tail, resource metrics, and the current round. This endpoint is used by both the dashboard (for the loop detail view) and the CLI (`nemo ps --detail`). It requires `pods/exec` permission on the `nautiloop-jobs` namespace — the terraform module provisions this by default.

---

## 9. Control Plane Communication

```
                 +-------------------+          +-------------------+
                 |    API Server     |          |   Loop Engine     |
                 |    (Deployment)   |          |   (Deployment)    |
                 +--------+----------+          +---------+---------+
                          |                               |
                          |      NO DIRECT RPC            |
                          |      Postgres is the          |
                          |      ONLY shared medium       |
                          |                               |
           +--------------v-------------------------------v--------------+
           |                        Postgres                             |
           |                                                             |
           |  +-------------------------------------------------------+  |
           |  |  loops table                                          |  |
           |  |                                                       |  |
           |  |  id | engineer_id | state | sub_state | round | sha  |  |
           |  |  cancel_requested (bool)                              |  |
           |  |  approve_requested (bool)                             |  |
           |  +-------------------------------------------------------+  |
           |                                                             |
           |  +-------------------------------------------------------+  |
           |  |  jobs table                                           |  |
           |  |                                                       |  |
           |  |  id | loop_id | stage | round | k8s_job_name |       |  |
           |  |  status | verdict_json | token_usage                  |  |
           |  +-------------------------------------------------------+  |
           |                                                             |
           |  +-------------------------------------------------------+  |
           |  |  log_events table                                     |  |
           |  |                                                       |  |
           |  |  id | loop_id | timestamp | stage | round | line     |  |
           |  +-------------------------------------------------------+  |
           |                                                             |
           +-------------------------------------------------------------+

  WRITE PATTERNS:

  API Server WRITES:                       Loop Engine WRITES:
  +-------------------------------------+  +-------------------------------------+
  | INSERT INTO loops (submit)          |  | UPDATE loops SET state, sub_state,  |
  | UPDATE loops SET                    |  |   round, sha (state transitions)    |
  |   cancel_requested = true (cancel)  |  | INSERT INTO jobs (job dispatch)     |
  | UPDATE loops SET                    |  | UPDATE jobs SET status, verdict,    |
  |   approve_requested = true (approve)|  |   completed_at (job completion)     |
  | UPDATE engineer_credentials         |  | INSERT INTO log_events (streaming)  |
  |   (nemo auth)                       |  |                                     |
  +-------------------------------------+  +-------------------------------------+

  API Server READS:                        Loop Engine READS:
  +-------------------------------------+  +-------------------------------------+
  | SELECT FROM loops (status queries)  |  | SELECT FROM loops WHERE state       |
  | SELECT FROM jobs (inspect)          |  |   NOT IN (converged, failed,        |
  | SELECT FROM log_events (logs SSE)   |  |   cancelled) -- reconciliation      |
  |                                     |  | SELECT cancel_requested,            |
  |                                     |  |   approve_requested -- each tick    |
  |                                     |  | SELECT FROM engineer_credentials    |
  |                                     |  |   -- before dispatch                |
  +-------------------------------------+  +-------------------------------------+

  RECONCILIATION LOOP DETAIL:

  +------------------------------------------------------------------+
  |  Loop Engine main loop                                           |
  |                                                                  |
  |  loop {                                                          |
  |      select! {                                                   |
  |          _ = ticker.tick() => {         // every 5s              |
  |              for loop in active_loops:                            |
  |                  reconcile(loop)                                  |
  |          }                                                       |
  |          event = job_watcher.recv() => { // K8s Job status change|
  |              // Wake up reconciliation immediately               |
  |              // Watcher does NOT write to Postgres               |
  |              // It only signals: "something changed, re-check"   |
  |              notify_reconciler(event.loop_id)                    |
  |          }                                                       |
  |      }                                                           |
  |  }                                                               |
  |                                                                  |
  |  fn reconcile(loop):                                             |
  |      1. Read state + sub-state from Postgres                     |
  |      2. Check cancel_requested / approve_requested flags         |
  |      3. If DISPATCHED: check K8s Job status via kube-rs          |
  |      4. If COMPLETED: parse output, evaluate, write next state   |
  |      5. If needs dispatch: create K8s Job, set DISPATCHED        |
  |      6. ALL state writes in single Postgres transaction          |
  +------------------------------------------------------------------+

  OPTIONAL OPTIMIZATION:

  API Server ----> pg_notify('loop_update', loop_id) ----> Loop Engine
                   (wakes engine immediately instead of waiting for 5s tick)
```

---

## 10. Config Resolution Flow

```
  LAYER 1 (lowest priority)            LAYER 2 (team)                LAYER 3 (highest priority)
  Cluster Config                       Repo Config                   Engineer Config

  Source: K8s ConfigMap                Source: nemo.toml             Source: ~/.nemo/config.toml
  /etc/nautiloop/cluster.toml              (monorepo root, checked in)   (per-machine, not checked in)
  + NAUTILOOP_CLUSTER_* env vars

  +---------------------------+       +---------------------------+  +---------------------------+
  | [cluster]                 |       | [repo]                    |  | [identity]                |
  |   node_size = "ccx43"    |       |   name = "cleared"        |  |   name = "Alice"          |
  |   provider = "hetzner"   |       |   default_branch = "main" |  |   email = "alice@..."     |
  |   domain = "nautiloop.internal|       |                           |  |                           |
  |   default_implementor =  |       | [models]                  |  | [models]                  |
  |     "claude-opus-4"      |       |   implementor =           |  |   implementor =           |
  |   default_reviewer =     |       |     "claude-opus-4"       |  |     "claude-opus-4"       |
  |     "gpt-5.4"            |       |   reviewer = "gpt-5.4"    |  |   reviewer = "gpt-5.4"    |
  |   max_parallel_loops_cap |       |                           |  |                           |
  |     = 8                  |       | [limits]                  |  | [limits]                  |
  |   max_cluster_jobs = 20  |       |   max_rounds_harden = 10  |  |   max_parallel_loops = 5  |
  |                          |       |   max_rounds_implement=15  |  |                           |
  +---------------------------+       |   max_concurrent_test_    |  +---------------------------+
                                      |     jvm = 3               |
                                      |                           |
                                      | [services.api]            |
                                      |   path = "api/"           |
                                      |   test = "cargo test"     |
                                      |                           |
                                      | [services.web]            |
                                      |   path = "web/"           |
                                      |   test = "npm test"       |
                                      +---------------------------+

  MERGE ALGORITHM:

  +---------------------------+     +---------------------------+     +---------------------------+
  |     Cluster Config        | --> |     Repo Config           | --> |    Engineer Config        |
  |     (base defaults)       |     |  (override scalars,      |     |  (override scalars,      |
  +---------------------------+     |   deep merge services)    |     |   capped by cluster)     |
                                    +---------------------------+     +---------------------------+
                |                              |                                 |
                +------------------------------+---------------------------------+
                                               |
                                               v
                                    +---------------------------+
                                    |     MergedConfig          |
                                    |                           |
                                    |  implementor_model:       |
                                    |    engineer > repo >      |
                                    |    cluster > ERROR        |
                                    |                           |
                                    |  reviewer_model:          |
                                    |    engineer > repo >      |
                                    |    cluster > ERROR        |
                                    |                           |
                                    |  max_parallel_loops:      |
                                    |    min(engineer_value,    |
                                    |        cluster_cap)       |
                                    |                           |
                                    |  services:                |
                                    |    repo defines;          |
                                    |    engineer can ADD,      |
                                    |    NOT override existing  |
                                    |                           |
                                    |  max_rounds_harden:       |
                                    |    repo (not overridable) |
                                    |                           |
                                    |  max_rounds_implement:    |
                                    |    repo (not overridable) |
                                    +---------------------------+

  CLI FLAGS (highest priority of all, applied at request time):

      nemo submit --model-impl claude-opus-4 --model-review gpt-5.4

      These override the MergedConfig for that specific loop only.

  RESOLUTION ORDER (high to low):
      1. CLI flags
      2. ~/.nemo/config.toml (engineer)
      3. nemo.toml (repo/team)
      4. Cluster ConfigMap / env vars

  MISSING FIELD BEHAVIOR:
      If implementor_model or reviewer_model is None at all layers:
      --> ConfigError::MissingField { field: "implementor", role: "model" }
      --> nemo submit fails with: "No implementor model configured. Set in
          nemo.toml [models] or ~/.nemo/config.toml [models]"
```

---

## 11. Git Worktree Lifecycle

```
  Loop Engine                       Bare Repo (PVC)                     K8s Job Pod
      |                                 |                                   |
      |  1. fetch_and_resolve(branch)   |                                   |
      |                                 |                                   |
      |  +--ACQUIRE MUTEX-----------+   |                                   |
      |  |                          |   |                                   |
      |  |  git fetch --prune       |   |                                   |
      |  |------------------------->|   |                                   |
      |  |                          |   |                                   |
      |  |  resolve ref to SHA      |   |                                   |
      |  |  (git rev-parse)         |   |                                   |
      |  |------------------------->|   |                                   |
      |  |                          |   |                                   |
      |  |  sha = abc123def4...     |   |                                   |
      |  |<-------------------------|   |                                   |
      |  |                          |   |                                   |
      |  |  2. create_worktree(sha) |   |                                   |
      |  |                          |   |                                   |
      |  |  git worktree add        |   |                                   |
      |  |    /worktrees/{id}       |   |                                   |
      |  |    abc123def4            |   |                                   |
      |  |------------------------->|   |                                   |
      |  |                          |   |                                   |
      |  |  worktree_path =         |   |                                   |
      |  |  /worktrees/{id}         |   |                                   |
      |  |<-------------------------|   |                                   |
      |  |                          |   |                                   |
      |  +--RELEASE MUTEX-----------+   |                                   |
      |                                 |                                   |
      |  3. Dispatch K8s Job            |                                   |
      |     (mount worktree at /work)   |                                   |
      |------------------------------------------------------------------>|
      |                                 |                                   |
      |                                 |   4. Agent works in worktree      |
      |                                 |      - reads code                 |
      |                                 |      - makes changes              |
      |                                 |      - git commit                 |
      |                                 |      - git push (via sidecar)     |
      |                                 |<----------------------------------|
      |                                 |                                   |
      |                                 |   5. Agent exits                  |
      |  Job watcher: SUCCEEDED         |                                   |
      |<-------------------------------------------------------------------|
      |                                 |                                   |
      |  6. delete_worktree(path)       |                                   |
      |                                 |                                   |
      |  +--ACQUIRE MUTEX-----------+   |                                   |
      |  |                          |   |                                   |
      |  |  git worktree remove     |   |                                   |
      |  |    --force               |   |                                   |
      |  |    /worktrees/{id}       |   |                                   |
      |  |------------------------->|   |                                   |
      |  |                          |   |                                   |
      |  |  git worktree prune      |   |                                   |
      |  |------------------------->|   |                                   |
      |  |                          |   |                                   |
      |  +--RELEASE MUTEX-----------+   |                                   |
      |                                 |                                   |

  MUTEX SCOPE DETAIL:

  The tokio::sync::Mutex serializes ALL worktree operations:

  +=============================+
  | MUTEX HELD                  |  fetch_and_resolve + create_worktree
  |                             |  are ONE critical section.
  | git fetch --prune           |
  | git rev-parse (resolve SHA) |  WHY: prevents a concurrent fetch from
  | git worktree add            |  moving the ref between our resolve
  |                             |  and our worktree creation.
  +=============================+
             |
             | (mutex released)
             |
       Job runs (minutes)
             |
             | (mutex re-acquired)
             |
  +=============================+
  | MUTEX HELD                  |  delete_worktree is a separate
  |                             |  critical section.
  | git worktree remove --force |
  | git worktree prune          |  WHY: git worktree commands take a
  |                             |  file lock on .git/worktrees/.
  +=============================+  Explicit mutex avoids N processes
                                   blocking on the same file lock.

  CONCURRENT JOBS (worst case: 15 jobs):

  Job 1:  [===MUTEX===]--------(running)--------[===MUTEX===]
  Job 2:     [wait][===MUTEX===]----(running)----[===MUTEX===]
  Job 3:        [wait...][===MUTEX===]--(running)--[===MUTEX===]
  ...
  Job 15:                [wait............][===MUTEX===]--...

  Mutex hold time: <1s per operation
  Worst case queue: ~15s (acceptable for jobs that run 5-30 min)
```

---

## 12. Auth Flow

```
  Engineer's Machine                   k3s Cluster
  +---------------------------+        +-------------------------------------------+
  |                           |        |                                           |
  |  ~/.claude/               |        |  Namespace: nautiloop-jobs                     |
  |    (session tokens,       |        |                                           |
  |     Claude Max auth)      |        |  K8s Secrets (per engineer):              |
  |                           |        |  +-------------------------------------+  |
  |  OpenAI auth tokens       |        |  | Secret: nautiloop-creds-alice            |  |
  |    (Pro subscription)     |        |  |   (one secret per engineer, keys   |  |
  |                           |        |  |    named by provider)              |  |
  |  SSH private key          |        |  |   claude: <~/.claude/ session data>|  |
  |    (for git push)         |        |  |   openai: <opencode auth data>     |  |
  +------------+--------------+        |  |                                     |  |
               |                       |  | Secret: nautiloop-ssh-alice              |  |
               |                       |  |   ssh-key: -----BEGIN OPENSSH...   |  |
               |                       |  +-------------------------------------+  |
   nemo auth   |                       |                                           |
   (pushes     |                       +-------------------------------------------+
   creds to    |
   cluster)    |
               v
  +---------------------------+
  |  nemo auth                |
  |                           |
  |  1. Read ~/.claude/*      |
  |  2. Read OpenAI tokens    |
  |  3. Read SSH key          |
  |  4. Create/update K8s     |
  |     Secrets via API       |
  |     (scoped to engineer)  |
  +---------------------------+
               |
               | K8s API / nautiloop API
               v

  INSIDE A JOB POD (credential flow):

  +-------------------------------------------------------------------------+
  |  Job Pod                                                                |
  |                                                                         |
  |  +-------------------------------+    +------------------------------+  |
  |  |  Agent Container              |    |  Auth Sidecar               |  |
  |  |                               |    |                              |  |
  |  |  ~/.claude/ session dir        |    |  /secrets/model-credentials  |  |
  |  |  mounted at /work/home/.claude |    |    mounted from K8s Secret   |  |
  |  |  (for Claude session auth)     |    |  /secrets/ssh-key            |  |
  |  |                               |    |    mounted from K8s Secret   |  |
  |  |  OPENAI_BASE_URL=             |    |                              |  |
  |  |    http://localhost:9090/openai|    |                              |  |
  |  |                               |    |                              |  |
  |  |  Step 1a: claude -p sends     |    |                              |  |
  |  |  API request DIRECTLY to      |    |                              |  |
  |  |  api.anthropic.com using      |    |                              |  |
  |  |  session auth from ~/.claude/ |    |                              |  |
  |  |  ===============================>  |  (Claude bypasses sidecar)  |  |
  |  |                               |    |                              |  |
  |  |  Step 1b: opencode sends      |    |                              |  |
  |  |  API request to localhost:9090|    |                              |  |
  |  |  Request has NO auth header.  |    |                              |  |
  |  |  --------------------------->-+--->|  Step 2: Sidecar intercepts  |  |
  |  |                               |    |  Reads /secrets/model-creds  |  |
  |  |                               |    |  Injects Authorization:      |  |
  |  |                               |    |  Bearer header (OpenAI)     |  |
  |  |                               |    |                              |  |
  |  |                               |    |  Step 3: Sidecar forwards   |  |
  |  |                               |    |  to api.openai.com          |  |
  |  |                               |    |  with real credentials      |  |
  |  |                               |    |  ========================>  |  |
  |  |                               |    |           (HTTPS)           |  |
  |  |  Step 5: Agent receives       |    |                              |  |
  |  |  model response.              |    |  Step 4: Response streams   |  |
  |  |  Never saw the OpenAI key.<---+----|  back through sidecar.      |  |
  |  |                               |    |  (not buffered, streamed)   |  |
  |  |                               |    |                              |  |
  |  |  Step 6: git push             |    |                              |  |
  |  |  GIT_SSH_COMMAND points       |    |                              |  |
  |  |  to localhost:9091       ---->+--->|  Step 7: Sidecar SSH proxy  |  |
  |  |                               |    |  Uses /secrets/ssh-key      |  |
  |  |                               |    |  Connects to git remote     |  |
  |  |                               |    |  ========================>  |  |
  |  |                               |    |           (SSH)             |  |
  |  +-------------------------------+    +------------------------------+  |
  |                                                                         |
  +-------------------------------------------------------------------------+

  SECURITY INVARIANT:

  +-----------------------------------------------------------+
  |  The agent container NEVER has access to:                  |
  |    - API keys (anthropic, openai)                         |
  |    - SSH private keys                                     |
  |    - Session tokens                                       |
  |    - Any file under /secrets/                             |
  |                                                            |
  |  Even if the agent executes malicious code:               |
  |    - It can reach the internet (for deps, docs)           |
  |    - But it has NOTHING sensitive to exfiltrate            |
  |    - All secrets live only in the sidecar's filesystem    |
  |    - NetworkPolicy blocks direct egress (localhost only)  |
  |    - Egress logger (:9092) logs all outbound traffic      |
  +-----------------------------------------------------------+

  CREDENTIAL ROTATION:

  Engineer runs `nemo auth` again
       |
       v
  K8s Secret updated
       |
       v
  K8s volume mount propagates to running pods (~60s)
       |
       v
  Sidecar re-reads /secrets/* on EACH request (no restart needed)
```

---

## 13. Retry and Error Handling Flow

```
  +=============================================================+
  |  DECISION TREE: What happens when things go wrong           |
  +=============================================================+

  Job completes
       |
       v
  +----------+
  | Exit     |
  | code?    |
  +----+-----+
       |
  +----+----+----+----+----+
  |         |         |    |
  v         v         v    v
 exit 0   exit 137  exit 1  other
 (ok)     (OOM)     (error) non-zero
  |         |         |      |
  |         v         v      v
  |    +---------+  +-------------------+
  |    | OOM /   |  | Check error type  |
  |    | eviction|  +---+-------+-------+
  |    +---------+      |       |       |
  |         |      auth err  timeout  other
  |         v           |       |       |
  |    retry_count      v       v       v
  |    < 2 ?       +---------+ +---------+ +---------+
  |   /    \       | AWAIT_  | | retry   | | retry   |
  |  yes    no     | REAUTH  | | once    | | up to 2 |
  |  |       |     +---------+ +----+----+ +----+----+
  |  v       v          |          |            |
  |  retry   FAILED     |     fail again?  fail again?
  |  w/      (3rd       |     /    \       /      \
  |  backoff  failure)  |   yes     no   yes      no
  |  |                  |    |       |    |        |
  |  v                  |    v       v    v        v
  |  +----------+       |  FAILED  (ok) FAILED   (ok)
  |  | wait     |       |
  |  | 30s (1st)|       |
  |  | 120s(2nd)|       |
  |  +----------+       |
  |       |             |
  |       v             |
  |  re-dispatch        |
  |  same stage         |
  |  same inputs        |
  |  retry_count++      |
  |  round stays same   |
  |                     |
  v                     v
 (continue             (engineer runs
  loop)                 `nemo auth`,
                        loop resumes)


  +=============================================================+
  |  DETAILED FAILURE SCENARIOS                                 |
  +=============================================================+

  SCENARIO 1: Job OOM
  +---------------------------------------------------------------+
  |                                                               |
  |  Job exits 137 (OOMKilled)                                    |
  |       |                                                       |
  |       v                                                       |
  |  retry_count = 0    retry_count = 1    retry_count = 2        |
  |       |                   |                   |               |
  |       v                   v                   v               |
  |  wait 30s            wait 120s           FAILED               |
  |  re-dispatch         re-dispatch         reason: "OOM after   |
  |  retry_count=1       retry_count=2        3 attempts"         |
  |                                          notify engineer      |
  |                                                               |
  +---------------------------------------------------------------+

  SCENARIO 2: Verdict Parse Error
  +---------------------------------------------------------------+
  |                                                               |
  |  Review/audit job exits 0 but verdict JSON is malformed       |
  |       |                                                       |
  |       v                                                       |
  |  retry_count = 0         retry_count = 1         retry_count=2|
  |       |                       |                       |       |
  |       v                       v                       v       |
  |  re-dispatch             re-dispatch              FAILED      |
  |  same stage              same stage               reason:     |
  |  same inputs             same inputs              "Malformed  |
  |  retry_count=1           retry_count=2             verdict    |
  |                                                    after 2    |
  |                                                    retries"   |
  +---------------------------------------------------------------+

  SCENARIO 3: Model API Timeout
  +---------------------------------------------------------------+
  |                                                               |
  |  Job exits non-zero after 10 min API timeout                  |
  |       |                                                       |
  |       v                                                       |
  |  retry once                                                   |
  |       |                                                       |
  |  +----+----+                                                  |
  |  |         |                                                  |
  |  v         v                                                  |
  | succeeds  timeout again                                       |
  | (continue) |                                                  |
  |            v                                                  |
  |         FAILED                                                |
  |         reason: "Model API timeout after retry"               |
  +---------------------------------------------------------------+

  SCENARIO 4: Credentials Expired
  +---------------------------------------------------------------+
  |                                                               |
  |  Sidecar passes 401 through -> agent CLI exits non-zero       |
  |  Error message contains "auth" / "unauthorized" / "expired"   |
  |       |                                                       |
  |       v                                                       |
  |  Loop transitions to AWAITING_REAUTH                          |
  |  Engineer notified: "Credentials expired.                     |
  |    Run `nemo auth --claude` or `nemo auth --openai`"          |
  |       |                                                       |
  |       v (engineer runs nemo auth)                             |
  |                                                               |
  |  K8s Secret updated -> sidecar picks up on next request       |
  |  Loop transitions back to {prev stage}/DISPATCHED             |
  |  Same round, re-dispatches the failed job                     |
  +---------------------------------------------------------------+

  SCENARIO 5: Branch Diverged
  +---------------------------------------------------------------+
  |                                                               |
  |  Before dispatching next job, loop engine runs                |
  |  detect_divergence(branch)                                    |
  |       |                                                       |
  |  +----+----------+------------------+                         |
  |  |               |                  |                         |
  |  v               v                  v                         |
  | LocalAhead    RemoteAhead       ForceDeviated                 |
  | (normal)      (engineer pushed) (force push)                  |
  |  |               |                  |                         |
  |  v               v                  v                         |
  | continue      PAUSED             PAUSED                       |
  | (no action)   (auto-resume if    (always pause)               |
  |                configured,        |                           |
  |                else pause)        v                           |
  |                    |          "Branch diverged.                |
  |                    v           nemo resume or                  |
  |                 resume at      nemo cancel?"                   |
  |                 remote SHA                                     |
  +---------------------------------------------------------------+

  SCENARIO 6: Disk Full
  +---------------------------------------------------------------+
  |                                                               |
  |  git worktree add fails (no space left on device)             |
  |       |                                                       |
  |       v                                                       |
  |  Retry once after 60s (temp files may have been cleaned)      |
  |       |                                                       |
  |  +----+----+                                                  |
  |  |         |                                                  |
  |  v         v                                                  |
  | succeeds  fails again                                         |
  | (continue) |                                                  |
  |            v                                                  |
  |         FAILED                                                |
  |         reason: "Disk full: git worktree add failed"          |
  |         Engineer action: clean up old data or expand PVC      |
  +---------------------------------------------------------------+

  SCENARIO 7: Job Stuck (No Output)
  +---------------------------------------------------------------+
  |                                                               |
  |  Watchdog: no stdout/stderr for 15 minutes                    |
  |       |                                                       |
  |       v                                                       |
  |  Kill job (delete K8s Job)                                    |
  |  Retry once                                                   |
  |       |                                                       |
  |  +----+----+                                                  |
  |  |         |                                                  |
  |  v         v                                                  |
  | completes  stuck again                                        |
  | (continue) |                                                  |
  |            v                                                  |
  |         FAILED                                                |
  |         reason: "Job stuck: no output for 15 min"             |
  +---------------------------------------------------------------+

  SCENARIO 8: Max Rounds Exceeded
  +---------------------------------------------------------------+
  |                                                               |
  |  round >= max_rounds (default: 15 impl, 10 harden)           |
  |       |                                                       |
  |       v                                                       |
  |  Create PR with status NEEDS_HUMAN_REVIEW                    |
  |  Attach remaining issues from last verdict                    |
  |  Transition to FAILED                                         |
  |  reason: "Max rounds exceeded (N rounds).                     |
  |           PR created with outstanding issues."                |
  +---------------------------------------------------------------+

  SCENARIO 9: Control Plane Crash
  +---------------------------------------------------------------+
  |                                                               |
  |  Loop engine process dies mid-tick                            |
  |       |                                                       |
  |       v                                                       |
  |  Postgres transaction uncommitted -> rolled back              |
  |  K8s restarts Deployment (always)                             |
  |       |                                                       |
  |       v                                                       |
  |  On startup:                                                  |
  |  1. Run pending migrations                                    |
  |  2. Load all non-terminal loops from Postgres                 |
  |  3. Match running K8s Jobs back to DB rows                    |
  |     (via jobs.k8s_job_name UNIQUE)                            |
  |  4. Resume reconciliation from last committed state           |
  |                                                               |
  |  Jobs are idempotent: start from pinned SHA with same inputs  |
  |  State is durable in Postgres. No data loss.                  |
  +---------------------------------------------------------------+

  +=============================================================+
  |  BACKOFF SCHEDULE SUMMARY                                   |
  +=============================================================+
  |                                                             |
  |  Retry 1: wait 30s,  then re-dispatch                      |
  |  Retry 2: wait 120s, then re-dispatch                      |
  |  Retry 3: N/A -- mark FAILED                               |
  |                                                             |
  |  Retries do NOT increment the round counter.                |
  |  The loop remembers: "I was in round 3, IMPLEMENTING,      |
  |  retry 1 of 2" -- distinct from "round 4."                 |
  |                                                             |
  |  Postgres connection backoff: 1s, 2s, 4s, 8s... up to 60s  |
  |  K8s API backoff: 10s, max 3 attempts, then alert           |
  |  git fetch backoff: 30s, 120s, then FAILED                 |
  +=============================================================+
```

---

## Notation Reference

| Symbol | Meaning |
|--------|---------|
| `---->` | Network call or data flow |
| `===` | Boundary (cluster, mutex scope) |
| `[box]` | State or sub-state |
| `+---+` | Component boundary |
| `/ \` | Decision branch |
| `.....` | Repeated/elided steps |
