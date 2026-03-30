# Adversarial Multi-Model Review

> ADR: Different models have different reasoning biases. Same-model review shares blind spots with the author. Parallel multi-model review catches more issues at the cost of additional compute.

## Overview

Add configurable parallel multi-model review to both hardening stages:

1. **Spec hardening** — multiple models audit the spec in parallel before implementation
2. **Implementation hardening** — multiple models review the code in parallel after implementation

Same pattern for both. When `mode = "adversarial"`, dispatch N review jobs in parallel (one per configured reviewer with valid credentials), collect all verdicts, apply `any-fail` policy. If any reviewer finds issues, merge all findings (tagged by source model) and feed to the revise/implement stage.

When `mode = "single"`, behavior is unchanged — backward compatible.

## Dependencies

- **Requires:** Lane A core loop (stages, verdicts, feedback files), Lane C agent runtime (job dispatch, credential injection)
- **Required by:** Nothing — additive feature

## Requirements

### Functional Requirements

**FR-1: Review config section in nemo.toml**

```toml
[review]
mode = "adversarial"                                    # "single" | "adversarial"
reviewers = ["claude-opus-4", "gpt-5.4", "gemini-3.1-pro"]
verdict_policy = "any-fail"                             # only supported policy for now
```

When `mode = "single"`: use `config.models.reviewer` as today. `reviewers` list is ignored.
When `mode = "adversarial"`: dispatch to all entries in `reviewers` that have valid credentials for the submitting engineer.

**FR-2: Credential gating**

Before dispatching reviewers, filter the `reviewers` list to only those with valid credentials for the engineer:

```
active_reviewers = reviewers.filter(|r| engineer has valid credential for provider_of(r))
```

Provider derivation uses convention-based prefix matching:

| Prefix     | Provider   | CLI tool      |
|------------|------------|---------------|
| `claude-*` | `claude`   | `claude`      |
| `gpt-*`    | `openai`   | `opencode`    |
| `gemini-*` | `google`   | `gemini-cli`  |

If `active_reviewers.len() == 0`: fail the loop with reason "no valid reviewer credentials".
If `active_reviewers.len() == 1`: effectively single-model, but still uses adversarial verdict aggregation path.

**FR-3: Parallel dispatch for spec hardening (audit stage)**

When the loop engine reaches the audit stage and `mode = "adversarial"`:

1. Create one K8s Job per active reviewer, all using the same:
   - Spec content (same branch SHA)
   - Prompt template (`.nemo/prompts/spec-audit.md`)
   - Stage name `audit`
2. Each job gets `NEMO_MODEL=<reviewer-model>` and the corresponding `NEMO_CRED_*`
3. Job names: `{loop-id}-audit-r{round}-{provider}` (e.g., `abc123-audit-r1-claude`, `abc123-audit-r1-openai`)
4. All jobs dispatch simultaneously — no sequencing

**FR-4: Parallel dispatch for implementation hardening (review stage)**

Identical to FR-3 but for the `review` stage:

1. Create one K8s Job per active reviewer
2. Same diff/branch, same prompt template (`.nemo/prompts/review.md`)
3. Job names: `{loop-id}-review-r{round}-{provider}`
4. All jobs dispatch simultaneously

**FR-5: Verdict collection and aggregation**

The loop engine waits for ALL parallel review jobs to complete (or timeout/fail), then aggregates:

```rust
struct AggregatedVerdict {
    clean: bool,                          // false if ANY reviewer verdict is not clean
    reviewer_verdicts: Vec<TaggedVerdict>,
    merged_issues: Vec<TaggedIssue>,
    summary: String,                      // combined summary
}

struct TaggedVerdict {
    reviewer_model: String,               // "gpt-5.4"
    verdict: ReviewVerdict,               // existing type, unchanged
}

struct TaggedIssue {
    reviewer_model: String,               // which model found this
    issue: Issue,                          // existing Issue type, unchanged
}
```

Aggregation rules:
- `clean = reviewer_verdicts.iter().all(|v| v.verdict.clean)`
- `merged_issues`: flat list of all issues from all reviewers, each tagged with source model
- No deduplication — reviewers finding the same issue is signal, not noise. The revise stage sees all findings.

**FR-6: Feedback file format for multi-reviewer findings**

The feedback file passed to the revise/implement stage extends the existing `FeedbackFile`:

```rust
struct FeedbackFile {
    round: u32,
    source: FeedbackSource,               // Review | Audit
    issues: Option<Vec<TaggedIssue>>,      // was Vec<Issue>, now tagged
    failures: Option<Vec<TestFailure>>,    // unchanged
    reviewer_models: Vec<String>,          // which models reviewed
}
```

The revise stage prompt must render issues grouped by reviewer model so the agent can see which model found what.

**FR-7: Timeout and failure handling for parallel jobs**

| Scenario | Behavior |
|---|---|
| All jobs complete | Aggregate normally |
| Some jobs complete, some timeout | Aggregate completed verdicts only. Log warning for timed-out reviewers |
| Some jobs complete, some fail (crash/OOM) | Aggregate completed verdicts only. Log warning for failed reviewers |
| All jobs timeout or fail | Treat as single failed review with reason "all reviewers failed" |
| One job completes clean, another is still running | Wait for all. Do not short-circuit |

Never short-circuit on first failure — always wait for all jobs. A reviewer that finds 0 issues is still valuable signal when another reviewer found 5.

**FR-8: Re-review dispatches all reviewers**

After revise, the next audit/review round dispatches ALL active reviewers again (not just the ones that previously failed). A fix for one reviewer's finding may introduce issues another reviewer would catch.

**FR-9: Model-to-CLI-tool convention**

The agent container entrypoint resolves the CLI tool from the model identifier prefix:

```bash
case "$NEMO_MODEL" in
  claude-*) TOOL="claude" ;;
  gpt-*)    TOOL="opencode" ;;
  gemini-*) TOOL="gemini-cli" ;;
  *)        echo "Unknown model prefix: $NEMO_MODEL" >&2; exit 1 ;;
esac
```

This is the only dispatch logic. No registry table needed for v1.

**FR-10: CLI display of per-reviewer results**

`nemo status` shows per-reviewer verdicts when in adversarial mode:

```
Loop abc123  Round 3  HARDENING (audit)
  Reviewers:
    claude-opus-4   ✓ clean    (0 issues)
    gpt-5.4         ✗ failed   (3 issues: 2 critical, 1 high)
    gemini-3.1-pro  ✗ failed   (1 issue: 1 critical)
  Verdict: FAIL (any-fail policy, 2/3 reviewers found issues)
```

`nemo logs --reviewer gpt-5.4` shows the full output from a specific reviewer job.

**FR-11: Local spec hardening (Claude Code)**

For pre-implementation spec review running locally (not on Nemo), provide a `nemo review-spec <spec-path>` CLI command that:

1. Reads `[review]` config from `nemo.toml`
2. For each reviewer with valid credentials (checked via `~/.nemo/config.toml`):
   - Calls the model's API directly (not K8s Jobs)
   - Sends the spec content + the spec-audit prompt template
   - Collects the verdict
3. All API calls run in parallel (tokio tasks)
4. Aggregates and displays results in the same format as `nemo status`
5. Exits with code 0 if clean, 1 if any-fail

This allows spec authors to run adversarial review locally before submitting to Nemo for implementation.

### Non-Functional Requirements

**NFR-1: No additional latency in single mode**

When `mode = "single"`, the code path must be identical to today. No overhead from adversarial infrastructure.

**NFR-2: Parallel job overhead**

Dispatching N parallel jobs must not take more than N × 100ms wall-clock (jobs are independent K8s API calls, can be concurrent).

**NFR-3: Backward compatibility**

If `[review]` section is absent from `nemo.toml`, default to `mode = "single"` with existing behavior. No config migration required.

**NFR-4: Credential failure is not a loop failure**

If 2 of 3 reviewers have valid credentials, dispatch 2. Only fail the loop if 0 reviewers are available.

## Behavior

### Normal Flow (Adversarial Spec Hardening)

```
1. Engineer submits: nemo harden spec.md
2. Loop engine reads [review] config: mode=adversarial, 3 reviewers
3. Credential check: engineer has claude + openai keys (2/3 active)
4. Dispatch audit jobs in parallel:
   - abc123-audit-r1-claude  (NEMO_MODEL=claude-opus-4)
   - abc123-audit-r1-openai  (NEMO_MODEL=gpt-5.4)
5. Both complete:
   - claude: clean=true, 0 issues
   - gpt-5.4: clean=false, 2 issues (1 critical, 1 high)
6. Aggregate: clean=false (any-fail)
7. Build feedback file with tagged issues
8. Dispatch revise stage with merged findings
9. Revise completes
10. Dispatch audit round 2 (both reviewers again)
11. Both clean → aggregate: clean=true → HARDENED
```

### Normal Flow (Adversarial Implementation Hardening)

```
1. Implementation converges (tests pass, implement stage clean)
2. Loop enters review stage, reads [review] config: mode=adversarial
3. Same parallel dispatch, collection, aggregation as spec hardening
4. If any-fail: findings fed back to implement stage for fixes
5. Re-review with all reviewers
6. All clean → CONVERGED
```

### Alternative: All Reviewers Agree (Fast Path)

All reviewers return `clean=true` on first round. Aggregate is clean. Proceed immediately. No revise round needed. This is the best case — adversarial review adds one parallel fan-out step but no extra rounds.

### Alternative: Partial Credential Availability

Engineer has only Claude credentials. `active_reviewers = ["claude-opus-4"]`. Adversarial mode runs with 1 reviewer. Functionally identical to single mode but uses the aggregation code path. No error, no warning.

### Alternative: Reviewer Disagrees Across Rounds

Round 1: GPT finds issue A, Gemini finds issue B. Revise fixes both. Round 2: GPT clean, Gemini finds issue C (introduced by fix for B). Revise fixes C. Round 3: both clean. This is the expected adversarial pattern — different models catch different regressions.

## Edge Cases

| Edge Case | Behavior |
|---|---|
| `reviewers` list is empty | Error at config parse time: "adversarial mode requires at least one reviewer" |
| `reviewers` contains unknown prefix | Error at dispatch time: "unknown model prefix: {prefix}" |
| Same model listed twice | Dispatch two independent jobs. No dedup. (Useful for testing non-determinism) |
| Reviewer job produces unparseable verdict | Treat as failed review with 1 critical issue: "reviewer produced invalid output" |
| Round limit exceeded in adversarial mode | Same behavior as today — FAILED with reason. Uses `max_rounds_harden` / `max_rounds_implement` |
| Config changes between rounds | Config is read once at loop start. Mid-loop config changes don't affect running loops |
| `mode = "adversarial"` but no `reviewers` key | Error at config parse time |

## Data Model Changes

### New Types

```rust
/// Config for review behavior
pub struct ReviewConfig {
    pub mode: ReviewMode,                    // Single | Adversarial
    pub reviewers: Vec<String>,              // model identifiers
    pub verdict_policy: VerdictPolicy,       // AnyFail (only option for now)
}

pub enum ReviewMode {
    Single,
    Adversarial,
}

pub enum VerdictPolicy {
    AnyFail,
}

/// Aggregated result from parallel reviewers
pub struct AggregatedVerdict {
    pub clean: bool,
    pub reviewer_verdicts: Vec<TaggedVerdict>,
    pub merged_issues: Vec<TaggedIssue>,
    pub summary: String,
}

pub struct TaggedVerdict {
    pub reviewer_model: String,
    pub verdict: ReviewVerdict,
}

pub struct TaggedIssue {
    pub reviewer_model: String,
    pub issue: Issue,
}
```

### Modified Types

```rust
// NemoConfig: add review field
pub struct NemoConfig {
    // ... existing fields ...
    pub review: ReviewConfig,               // NEW
}

// FeedbackFile: issues become tagged
pub struct FeedbackFile {
    pub round: u32,
    pub source: FeedbackSource,
    pub issues: Option<Vec<TaggedIssue>>,   // CHANGED: was Vec<Issue>
    pub failures: Option<Vec<TestFailure>>,
    pub reviewer_models: Vec<String>,       // NEW
}

// LoopRecord: track active reviewers for observability
pub struct LoopRecord {
    // ... existing fields ...
    pub active_reviewers: Option<Vec<String>>,  // NEW: resolved at loop start
}
```

### Database Migration

Add column to `loops` table:

```sql
ALTER TABLE loops ADD COLUMN active_reviewers JSONB DEFAULT NULL;
```

Add table for per-reviewer verdict storage:

```sql
CREATE TABLE reviewer_verdicts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    loop_id UUID NOT NULL REFERENCES loops(id),
    round INT NOT NULL,
    stage VARCHAR(20) NOT NULL,           -- "audit" or "review"
    reviewer_model VARCHAR(100) NOT NULL,
    clean BOOLEAN NOT NULL,
    issues JSONB NOT NULL DEFAULT '[]',
    summary TEXT,
    token_usage JSONB,
    job_name VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_reviewer_verdicts_loop ON reviewer_verdicts(loop_id, round, stage);
```

## Config Schema

### nemo.toml

```toml
[review]
mode = "adversarial"                                    # default: "single"
reviewers = ["claude-opus-4", "gpt-5.4", "gemini-3.1-pro"]
verdict_policy = "any-fail"                             # default: "any-fail"
```

### Defaults (when [review] section absent)

```rust
impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            mode: ReviewMode::Single,
            reviewers: vec![],
            verdict_policy: VerdictPolicy::AnyFail,
        }
    }
}
```

## Out of Scope

- **Weighted voting or majority-vote policies** — only `any-fail` for v1
- **Deduplication of findings across reviewers** — intentionally not deduped; duplicates are signal
- **Model registry table** — convention-based prefix matching is sufficient for v1
- **Adversarial mode for the implement stage** — only review/audit stages
- **Cross-reviewer session continuity** — each reviewer sees the spec/code fresh each round
- **Cost tracking per reviewer** — token usage is captured per-verdict but not aggregated into cost estimates

## Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| AC-1 | `mode = "single"` behaves identically to current behavior | Existing tests pass unchanged |
| AC-2 | `mode = "adversarial"` dispatches parallel K8s Jobs for audit stage | Integration test: submit harden, verify N jobs created |
| AC-3 | `mode = "adversarial"` dispatches parallel K8s Jobs for review stage | Integration test: submit impl loop, verify N jobs at review |
| AC-4 | Verdicts aggregated with any-fail policy | Unit test: 1 clean + 1 dirty = dirty |
| AC-5 | Feedback file contains tagged issues from all reviewers | Unit test: verify tagged issue format |
| AC-6 | Missing credentials reduce active reviewers, don't fail | Integration test: 1/3 credentials valid → 1 reviewer dispatched |
| AC-7 | 0 valid credentials fails the loop | Unit test: verify error |
| AC-8 | `nemo status` shows per-reviewer results | Manual verification |
| AC-9 | `nemo logs --reviewer <model>` shows reviewer-specific output | Manual verification |
| AC-10 | Re-review dispatches all active reviewers, not just failed ones | Integration test: verify job count on round 2 |
| AC-11 | `nemo review-spec` runs local parallel review via API calls | Integration test: mock API, verify parallel calls |
| AC-12 | Timed-out reviewer doesn't block aggregation of completed reviewers | Unit test: 1 complete + 1 timeout = aggregate 1 |
| AC-13 | `[review]` section absent defaults to single mode | Unit test: parse config without section |
| AC-14 | Per-reviewer verdicts stored in `reviewer_verdicts` table | Integration test: query table after review round |

## Open Questions

None — all design decisions resolved in conversation.
