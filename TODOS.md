# TODOS

## Agent Integration

### Evaluate opencode serve as in-Job sidecar for review stage

**What:** Run `opencode serve` as a sidecar within review Jobs. Control plane sends prompts via REST (`POST /session/:id/prompt_async`) instead of spawning `opencode run` per prompt.

**Why:** Within a single review round, the reviewer may need multiple prompts (initial review, follow-up on specific files, verdict generation). `opencode serve` avoids cold-starting opencode for each sub-prompt. This is a micro-optimization within the Job model, not the persistent-pod approach (which was evaluated and rejected for V1 due to 3.3% overhead vs. significant complexity increase).

**Context:** Both an independent Codex review and an eng review rejected persistent pods: K8s Jobs provide crash recovery, blast radius isolation, and debuggability for free. But `opencode serve` within a Job is a different, smaller optimization. Measure first: if review-stage cold start is <2s, this isn't worth it. The `Stage` trait boundary makes this swappable without touching the loop engine.

**Effort:** S
**Priority:** P3
**Depends on:** Lane A core loop (V1)


## Observability

### Cost/usage tracking per loop and per engineer

**What:** Log token usage (or at minimum, session duration and round count) to Postgres per loop. Surface via `nemo costs` CLI command.

**Why:** With Max/Pro subscriptions, billing is at the subscription level. But tracking usage per loop helps debug runaway loops, plan capacity, and understand which specs are expensive.

**Context:** The review verdict schema already includes `token_usage`. Store this in Postgres alongside loop metadata. For subscription-based auth, round count and wall-clock time per job are the useful metrics. Aggregate by engineer, by day, by spec. CLI command: `nemo costs [--engineer alice] [--last 7d]`.

**Effort:** M
**Priority:** P2
**Depends on:** Postgres schema (V1 core)

### Webhook/Slack notifications for loop state changes

**What:** Notify engineers when loops reach terminal or action-required states: CONVERGED, FAILED, AWAITING_APPROVAL, AWAITING_REAUTH, DIVERGED.

**Why:** Currently engineers must poll `nemo status`. With 5 parallel loops, you want push notifications when something needs attention or is ready.

**Context:** Two approaches: (a) generic webhook endpoint in nautiloop.toml (engineer configures their own Slack/Discord/email integration), or (b) built-in Slack integration. Webhook is more flexible and lower effort. Add `[notifications]` section to nautiloop.toml with `webhook_url` and `events` filter. Loop engine fires HTTP POST on state transitions.

**Effort:** M
**Priority:** P2
**Depends on:** Loop engine core (V1)

## Completed

### Verify reviewer CLI headless mode

**What:** Evaluated OpenCode (opencode.ai), Codex CLI, and Crush for headless reviewer capabilities.

**Why:** Agent image design depended on this.

**Context:** Decision: OpenCode (anomalyco/opencode) as default reviewer. Supports headless (`opencode run --format json`), persistent server (`opencode serve` + REST API), ChatGPT Plus/Pro subscription auth, auto-approve permissions, official Docker image. Claude Code stays as implementer (Max subscription). Cross-model adversarial: Claude implements, OpenAI reviews.

**Completed:** pre-v1 (2026-03-28)
