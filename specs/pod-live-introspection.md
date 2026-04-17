# Pod Live Introspection

## Overview

Expose what the agent container is ACTUALLY doing right now — running processes, CPU/mem, recent shell commands — directly in `nemo` CLI and `nemo helm`. Operators shouldn't need `kubectl exec` to diagnose a stalled loop. When an implement stage has been "RUNNING" for 20 minutes, the operator should be able to answer "is it making progress or is it stuck?" from one pane in the TUI.

Complements the three specs already merged:
- #126 log polish → cleans up what Claude SAID
- #127 local spec upload → fixes the input path
- #128 orchestrator judge → smarter transition decisions
- **This spec** → shows what Claude is DOING

## Baseline

Main at PR #128 merge.

Current diagnostic surface for a running agent:
- `nemo logs <id>` — Postgres-persisted log events (one line per NAUTILOOP_RESULT + progress-echo). Granularity: per-stage, not per-second.
- `nemo logs --tail` / `GET /pod-logs/:id` — live stdout from the agent container. Granularity: whatever Claude emits.
- `nemo helm` — same as above in a TUI frame.

Missing: the agent container's **runtime state**. To learn that a stage is burning 25 min on a cold `cargo check` vs. hanging on a flock vs. idle waiting for a model response, the operator has to:

```bash
kubectl exec <pod> -c agent -- ps auxf
kubectl top pod <pod>
kubectl exec <pod> -c agent -- ls /work/target/debug/deps | wc -l
```

That requires cluster access, kubectl permissions, and knowledge of which pod maps to which loop. It's not viable for engineers who just want a "what's it doing?" view.

## Problem Statement

### Problem 1: "Is it stuck or working?" is unanswerable from the CLI

Observed during dogfood session 2026-04-17: a cold-compile `cargo clippy` + `cargo test` trio burned 25 min on first run of a Rust spec. From `nemo helm`, the state was just `IMPLEMENTING/RUNNING round=1` with zero updates. Operators can't tell "Claude is compiling axum" from "Claude froze in an API retry loop."

### Problem 2: Common pathologies are invisible

Examples seen today that would be instantly diagnosable with pod introspection:
- Claude spawned 4 overlapping `cargo clippy`/`cargo test` processes fighting for target-dir locks (4x slowdown). Not visible anywhere except `ps auxf`.
- A previous loop got stuck in a reauth retry loop. `nemo status` said `AWAITING_REAUTH` but the real CPU burn was elsewhere.
- Claude ran `git init` on a broken worktree, silently creating a disjoint repo. No signal until the final "no commits between main and branch" error 40 minutes later.

### Problem 3: Tail-logs show output, not activity

`nemo logs --tail` shows what Claude writes to stdout. Claude often goes silent for minutes while running shell commands. During those silences, `--tail` shows nothing — but the container might be compiling, or hung, or the model might be thinking. Three very different situations, one observation.

## Functional Requirements

### FR-1: Control-plane snapshot endpoint

**FR-1a.** New endpoint `GET /pod-introspect/:loop_id` returns a JSON snapshot of the active pod's runtime state. Auth same as other endpoints (API-key bearer).

**FR-1b.** Response shape:

```json
{
  "loop_id": "uuid",
  "pod_name": "nautiloop-abc123-implement-r2-t1-xyz",
  "pod_phase": "Running" | "Pending" | "...",
  "collected_at": "2026-04-17T12:45:00Z",
  "container_stats": {
    "cpu_millicores": 508,
    "memory_bytes": 959963136
  },
  "processes": [
    {
      "pid": 12,
      "ppid": 1,
      "user": "agent",
      "cpu_percent": 3.2,
      "cmd": "claude",
      "age_seconds": 1320
    },
    {
      "pid": 126,
      "ppid": 124,
      "user": "agent",
      "cpu_percent": 0.0,
      "cmd": "cargo-clippy clippy --workspace -- -D warnings",
      "age_seconds": 900
    },
    {
      "pid": 4319,
      "ppid": 130,
      "user": "agent",
      "cpu_percent": 18.7,
      "cmd": "rustc --crate-name axum ...",
      "age_seconds": 12
    }
  ],
  "worktree": {
    "path": "/work",
    "target_dir_artifacts": 1069,
    "target_dir_bytes": 3221225472,
    "uncommitted_files": 2,
    "head_sha": "42bffd9..."
  }
}
```

**FR-1c.** The endpoint MUST NOT block on pod exec longer than 3s. If the exec times out, return HTTP 503 with `{"error": "pod introspection timeout"}` — the caller (helm) retries on next poll.

**FR-1d.** Terminal loops (CONVERGED, FAILED, HARDENED, CANCELLED, SHIPPED): return HTTP 410 with a message directing caller to `nemo inspect`. No pod to introspect.

**FR-1e.** The endpoint returns `null` for `container_stats` when the k8s metrics API is unavailable (e.g., metrics-server not installed in a given cluster). Fallback: callers render "stats unavailable" and still show processes.

### FR-2: Data collection

**FR-2a.** Collection is a single pod exec invocation on the `agent` container that runs a short shell script capturing:

- `ps -eo pid,ppid,user,pcpu,etime,args --sort=-pcpu --no-headers | head -30` — processes by CPU.
- `du -sb /work/target 2>/dev/null | awk '{print $1}'; ls /work/target/debug/deps 2>/dev/null | wc -l` — target-dir size + artifact count.
- `git -C /work rev-parse HEAD 2>/dev/null; git -C /work status --porcelain 2>/dev/null | wc -l` — head SHA + dirty-file count.

Output is a single newline-delimited JSON object the control plane parses. No bash-level JSON building — use `jq -Rs` after. (See FR-2d for the script location.)

**FR-2b.** CPU/memory stats are fetched from the Kubernetes metrics API (`metrics.k8s.io/v1beta1`) using `kube-rs`. If the API is unreachable or returns 404 (metrics-server absent), `container_stats` is null. The handler MUST NOT fail the entire request on stats absence.

**FR-2c.** The exec script has a 2s CPU wall-clock budget. On timeout, the partial output is still parsed and fields that failed are returned as nulls.

**FR-2d.** The script lives in the agent image at `/usr/local/bin/nautiloop-introspect`. It is pre-baked at image build time alongside the existing `nautiloop-agent-entry`.

### FR-3: RBAC

**FR-3a.** The `nautiloop-api-server` ServiceAccount Role in `nautiloop-jobs` gains `pods/exec` permission (get + create). Mirror of the pods/log permission already added for `nemo logs --tail`.

**FR-3b.** The `metrics.k8s.io` API group (`nodes` and `pods` verbs: `get`, `list`) is added to the api-server Role (cluster-scoped binding to the existing api-server SA). Skippable when metrics-server is absent; handler already degrades gracefully per FR-1e.

### FR-4: CLI surface

**FR-4a.** New one-shot command: `nemo ps <loop_id>` — prints a human-readable table of the introspection snapshot:

```
Pod: nautiloop-abc123-implement-r2-t1-xyz  Phase: Running
CPU: 508m  Mem: 915 MiB
Worktree: /work  HEAD: 42bffd9  dirty=2 files  target: 3.0 GiB (1069 artifacts)

PID   PPID  USER    CPU%   AGE      COMMAND
12    1     agent   3.2    22m     claude
126   124   agent   0.0    15m     cargo-clippy clippy --workspace -- -D warnings
4319  130   agent   18.7   12s     rustc --crate-name axum ...
...
```

Exit code 0 on success; 1 on error; 2 when the loop is terminal (with a hint to use `nemo inspect`).

**FR-4b.** `nemo ps --watch <loop_id>` — polls the endpoint every 2s and redraws the table. Press `q` to quit. Implemented via crossterm like `helm`.

### FR-5: Helm TUI integration

**FR-5a.** A new pane toggle in `nemo helm`: pressing `p` cycles a side-panel between closed → "inspect" (existing rounds view) → "introspect" (new FR-5b) → closed. The existing helm layout keeps its loops-list + log-pane; the new pane is an overlay on the right.

**FR-5b.** The introspect pane shows the same fields as FR-4a, polling every 2s while the loop is active. Header line `Pod: ... Phase: ...` plus three grouped sections: `Stats`, `Worktree`, `Processes` (top 10 by CPU).

**FR-5c.** When the loop transitions to terminal, the introspect pane shows "Pod gone. Loop: <state>. Run `nemo inspect <branch>` for round history." — no polling after terminal.

**FR-5d.** The introspect pane does NOT replace the log pane. Both are visible simultaneously in a horizontal split.

### FR-6: Recording (optional, gate behind config)

**FR-6a.** When `[observability] record_introspection = true` in `nemo.toml`, every `/pod-introspect` call on an active loop ALSO writes the snapshot to a new `pod_snapshots` table:

```sql
CREATE TABLE pod_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    loop_id UUID NOT NULL REFERENCES loops(id),
    pod_name TEXT NOT NULL,
    snapshot JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_pod_snapshots_loop ON pod_snapshots (loop_id, created_at DESC);
```

**FR-6b.** Default is `false` (no recording). When enabled, retention is 7 days (TTL enforced by a daily sweep in the reconciler).

**FR-6c.** Rationale: recorded snapshots let the orchestrator-judge (#128) detect "stage is stuck" patterns and escalate without waiting for max_rounds. Stage 2 fine-tune training data benefits from runtime signal, not just verdict signal.

## Non-Functional Requirements

### NFR-1: Cost

One pod exec per poll. At 2s polling from helm, that's ~30 execs/min per viewer. Each exec runs in ~100ms. Within the noise of current control-plane load. No rate-limit required at Stage 1; revisit if introspection traffic becomes non-trivial.

### NFR-2: Security

- The introspection script is read-only (ps, du, git-status). No writes.
- The script runs as the agent user (UID 1000). No privilege escalation.
- No agent secrets are exposed in the snapshot (process args can contain paths, not credentials — `/secrets/claude-creds/` appears as a path but its contents are not read).

### NFR-3: Backward compatibility

New endpoint, new CLI command, new helm keybind. Zero changes to existing endpoints/commands. Feature-flag-free for Stage 1.

### NFR-4: Degradation

Metrics-server absent → `container_stats: null`. Pod not yet started → HTTP 425 with `{"error": "pod not yet running"}`. Pod exec fails for any reason → partial snapshot with failed fields null. The view is ALWAYS renderable; never a full 500.

### NFR-5: Tests

- **Unit** (`control-plane/src/api/introspect.rs`): mock `K8sClient`; assert endpoint returns well-formed snapshot; assert 2s exec timeout; assert 3s handler timeout; assert metrics-absent degradation.
- **Integration** (`control-plane/tests/introspect_integration.rs`): full flow against a real k3d cluster (running pod), assert process list contains expected entries, assert stats populated.
- **CLI** (`cli/src/commands/ps.rs`): snapshot rendering deterministic given a fixed input JSON.

## Acceptance Criteria

A reviewer can verify by:

1. **Cold compile visibility:** start a loop on a Rust spec, during `IMPLEMENTING` run `nemo ps <id>`. Output shows `cargo-clippy`, `cargo check`, and `rustc --crate-name <X>` processes with CPU and age. Target-dir artifact count increases between calls.
2. **Stuck loop diagnosis:** artificially kill the network in a test pod (`kubectl exec ... -- iptables -A OUTPUT -j DROP`). `nemo ps` still returns — stats populated, processes listed. Caller sees claude PID alive but CPU == 0 — the "stuck, not working" signal.
3. **Helm integration:** in `nemo helm`, press `p` twice to cycle to the introspect pane. Updates every 2s without freezing the TUI. Press `p` twice more to close.
4. **Terminal guard:** after the loop converges, `nemo ps <id>` returns exit code 2 with a hint to `nemo inspect`.
5. **Stats absent:** stop `metrics-server` (dev k3d: `kubectl delete deploy -n kube-system metrics-server`). `nemo ps` still works; `CPU / Mem` line shows `unavailable`.
6. **Recording:** set `record_introspection = true`, poll the endpoint 5x during an active loop, confirm 5 rows in `pod_snapshots`.

## Out of Scope

- **Full container-runtime metrics** (disk IOPS, network throughput) — adds complexity for marginal diagnostic value. Revisit if real-world pathologies demand it.
- **Historical charts** (CPU over time, compile progress curves). The `pod_snapshots` table enables this later; no UI in Stage 1.
- **Cross-pod view** (sidecar + agent together). Stage 1 covers the agent container only. Sidecar introspection can ride in a follow-up.
- **Proactive stall detection in the orchestrator judge.** Noted as Stage 2 training-data benefit (FR-6c) but NOT a gating behavior in this spec.
- **Claude tool-call streaming.** Claude Code's internal tool history is separate from /proc; would require claude-side instrumentation. Out of scope.
- **Log scrubbing of process args.** FR-2 captures full `ps` args. A future spec could scrub known secret-bearing envs / flags. Today: no known secrets in args.

## Files Likely Touched

- `control-plane/migrations/<timestamp>_add_pod_snapshots.sql` — new table (FR-6).
- `control-plane/src/api/introspect.rs` — new handler module.
- `control-plane/src/api/mod.rs` — route wiring.
- `control-plane/src/k8s/client.rs` — pod exec helper, metrics API client wrapper.
- `control-plane/src/types/api.rs` — `PodIntrospectResponse`.
- `cli/src/commands/ps.rs` — new `nemo ps` command (+ `--watch`).
- `cli/src/commands/helm.rs` — FR-5 pane + keybind.
- `cli/src/client.rs` — new `get_pod_introspect` client method.
- `images/base/nautiloop-introspect` — shell script (FR-2d).
- `images/base/Dockerfile` — install the script.
- `dev/k8s/02-rbac.yaml` — pods/exec + metrics API group (FR-3).
- Tests per NFR-5.

## Baseline Branch

`main` at PR #128 merge.
