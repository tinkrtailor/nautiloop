# LLM-Friendly nemo CLI

## Overview

Make `nemo` self-describing enough that any LLM (Claude Code, Cursor, codex, opencode, an arbitrary agent built on a raw API) can learn to operate nautiloop end-to-end by reading the CLI's own output. Standard `--help` is a starting point; this spec adds examples, workflows, mental-model primers, a mega-help command, and machine-readable JSON output.

Goal: `nemo help ai` is the single command an agent runs to understand what nautiloop is, what state a loop can be in, and how to drive a complete workflow without reading source or docs.

## Baseline

Main at PR #159 merge.

Current state:
- `nemo --help` / `nemo -h` / `nemo help` — flat list of 17 commands with one-line descriptions.
- `nemo <command> --help` — shows flags + positional args for that command. No examples. No context.
- `nemo help <command>` — same as `nemo <command> --help` (clap's default).
- No machine-readable output. No workflow-level documentation. No mental-model primer.
- No way to dump everything in one call.

What works today: an LLM with repo access can `cat cli/src/main.rs` and figure it out. What doesn't: an LLM without repo access, or one limited to shell-only tool access, has to probe 17 commands to build a mental model.

## Problem Statement

### Problem 1: Help text is accurate but context-free

`nemo approve --help` says:

```
Approve a loop awaiting approval
Usage: nemo approve [OPTIONS] <LOOP_ID>
Arguments:  <LOOP_ID>  Loop ID
Options:    --server <SERVER>
```

An LLM reads this and learns: "there's an approve command that takes a loop ID." It does NOT learn: **when** approve is needed (only on loops in `AWAITING_APPROVAL`), **what happens** on successful approve (state transitions to implement dispatch), **how to get a loop ID** (`nemo status`), or **what error you get** if the state is wrong.

### Problem 2: No overall mental model anywhere in the CLI

Nautiloop has a non-trivial state machine (PENDING → AWAITING_APPROVAL → IMPLEMENTING → TESTING → REVIEWING → {CONVERGED, Implementing-again, FAILED}). There's a separate harden flow. There are terminal states vs resumable states. There's an engineer/cluster/repo config hierarchy. None of this is discoverable from `--help`.

An LLM without this context guesses or fails. An LLM with this context reasons correctly.

### Problem 3: No examples

A single worked example per command — "here's the output, here's the typical next step" — teaches an LLM more than 10 lines of flag descriptions. We have zero examples anywhere.

### Problem 4: No JSON / parseable output

Every CLI output is free-form text. Agents parsing `nemo status` to find a loop ID have to regex the table. `nemo status --json` would be a one-line win.

### Problem 5: No "what went wrong" context on errors

When `nemo approve` fails because the loop is in `IMPLEMENTING`, the error is:

```
Error: API error (409 Conflict): {"error":"Cannot approve: loop is in IMPLEMENTING, not AWAITING_APPROVAL"}
```

An LLM can parse that. But a friendlier error would name the recovery path: "Loops in IMPLEMENTING are already running; no approve needed. Use `nemo logs <id>` to watch." That converts an error into a workflow cue.

## Functional Requirements

### FR-1: `nemo help ai` — the mega-primer

**FR-1a.** New subcommand `nemo help ai` (also `nemo help llm`, alias) prints a single comprehensive Markdown document covering:

- **What nautiloop is**: one paragraph. Convergent loop orchestrator, cross-model adversarial review, self-hosted.
- **State machine diagram** (ASCII): the full loop lifecycle with every transition.
- **Terminal states**: CONVERGED, HARDENED, FAILED, CANCELLED, SHIPPED — what each means.
- **Typical workflow 1 (implement)**: `nemo start spec.md` → `nemo approve <id>` → `nemo logs <id>` → wait → PR.
- **Typical workflow 2 (harden-first)**: `nemo start spec.md --harden` → review hardened spec PR → `nemo approve <id>` → watch → PR.
- **Typical workflow 3 (ship)**: `nemo ship spec.md` → (no approval, no human) → auto-merged PR.
- **Recovery playbooks**: AWAITING_REAUTH → `nemo auth --claude && nemo resume <id>`. PAUSED → `nemo resume`. FAILED (max rounds) → `nemo extend --add 10 <id>` OR investigate. Stale kubectl context? Don't switch, use `--context=<name>` per command.
- **Config hierarchy**: engineer (`~/.nemo/config.toml`) > repo (`nemo.toml` on main) > cluster (control plane ConfigMap). Explain which lives where.
- **Command catalog**: full list with one-line descriptions, same as `nemo --help`, but grouped into categories: loop lifecycle, observability, identity, config.
- **Example spec structure**: the minimum skeleton a spec needs — overview, FRs, acceptance criteria. Points at `docs/spec-authoring.md` (future) if it exists.
- **Known failure modes**: reviewer nitpick-loops, max_rounds exhaustion, network drops mid-pod. For each, how to detect and recover.

**FR-1b.** Output is Markdown, ~200-400 lines, readable end-to-end. Generated from a template file embedded in the binary via `include_str!`. Maintained in `cli/src/commands/help_ai.md` — not duplicated per-platform.

**FR-1c.** `nemo help ai --format=json` emits the same information as structured JSON (sections keyed, workflow steps as arrays). For agents that prefer to parse rather than read prose.

### FR-2: Per-command `long_about` with examples

**FR-2a.** Every command in `cli/src/main.rs` gets a clap `#[command(long_about = ...)]` attribute containing:

- The existing short description (unchanged)
- A blank line
- **Example:** section with 1-3 realistic invocations and their expected outputs
- **See also:** list of related commands

Example for `nemo approve`:

```
Approve a loop awaiting approval.

Moves a loop from AWAITING_APPROVAL to the next active stage. Required for:
- Loops started with `nemo start` (PENDING → AWAITING_APPROVAL → approve → IMPLEMENTING)
- Loops that hardened first and are waiting for engineer review of the hardened spec

Does nothing useful on any other state; errors with 409 Conflict.

Example:
  $ nemo approve 8cb88352-5cf4-4dda-9cd0-6a0d6851ba92
  Approved loop 8cb88352-5cf4-4dda-9cd0-6a0d6851ba92
    State: AWAITING_APPROVAL
    Implementation will start on next reconciliation tick.

See also: nemo status (find loop IDs), nemo logs (watch after approve).
```

**FR-2b.** `nemo <cmd> --help` shows the short description (clap default). `nemo help <cmd>` shows the long_about with examples.

**FR-2c.** Applied to ALL subcommands: harden, start, ship, status, helm, logs, ps, cancel, approve, inspect, resume, extend, init, auth, models, config, cache (once shipped).

### FR-3: `--json` output mode on every stateful command

**FR-3a.** Commands whose output an agent might parse get a `--json` flag:
- `nemo status --json` — list of loops as JSON array
- `nemo inspect <branch> --json` — already emits JSON (confirm); no change
- `nemo approve <id> --json` — structured response object
- `nemo cancel <id> --json` — structured response
- `nemo resume <id> --json` — structured response
- `nemo extend <id> --json` — structured response
- `nemo models --json` — providers + available models as JSON
- `nemo auth --json` — push results as JSON

**FR-3b.** JSON output schema documented in `nemo help ai` (FR-1). Stable field names, no presentation-level keys.

**FR-3c.** When stdout is not a TTY and `--json` is not passed, commands emit the plain-text table (unchanged behavior for humans). Adding `--json` always emits JSON regardless of TTY.

### FR-4: Error messages include recovery hints

**FR-4a.** The API returns specific error codes for state-transition violations. The CLI catches the common ones and adds a recovery hint line:

| Server error | CLI-added recovery hint |
|---|---|
| "Cannot approve: loop is in IMPLEMENTING, not AWAITING_APPROVAL" | "Loops in IMPLEMENTING are already running. Run `nemo logs <id>` to watch." |
| "Cannot cancel: loop is in CONVERGED, not non-terminal state" | "This loop has already completed. Check the PR with `nemo inspect <branch>`." |
| "Cannot approve: loop is in PENDING, not AWAITING_APPROVAL" | "Wait ~5s for the reconciler to advance PENDING → AWAITING_APPROVAL, then retry." |
| "Claude token expired" | "Run `nemo auth --claude` to refresh cluster credentials. If the local token is also stale, open Claude Code to refresh then retry." |
| Spec not found | "Spec must exist on the repo's default branch, or be uploaded via `--local-spec` (see `nemo start --help`)." |

**FR-4b.** Recovery hints are CLI-side; server doesn't change. Each hint lives in `cli/src/commands/error_hints.rs` as `(pattern, hint)` pairs. Unknown errors pass through unchanged.

**FR-4c.** `--no-hints` flag suppresses the hints for scripting contexts where stable error output matters.

### FR-5: `nemo help --all`

**FR-5a.** New flag: `nemo help --all` dumps every subcommand's long_about in one shot. One-call total CLI documentation.

**FR-5b.** Output format: Markdown, headings per command. Same prose as individual `nemo help <cmd>` but concatenated.

**FR-5c.** `nemo help --all --format=json` returns a single JSON object: `{ "commands": { "approve": { "short": "...", "long": "...", "options": [...] }, ... } }`.

### FR-6: Version + capability report

**FR-6a.** `nemo --version` (existing) unchanged.

**FR-6b.** `nemo capabilities` (new) prints JSON describing which features this CLI version supports:

```json
{
  "version": "0.6.0",
  "commands": ["harden", "start", "ship", "status", "helm", "logs", "ps", "cancel",
               "approve", "inspect", "resume", "extend", "init", "auth", "models",
               "config", "cache", "help"],
  "features": {
    "qa_stage": false,
    "orchestrator_judge": true,
    "pluggable_cache": true,
    "harden_by_default": true,
    "nemo_extend": true,
    "pod_introspect": true,
    "dashboard": false
  }
}
```

**FR-6c.** Lets an agent check `nemo capabilities` once at startup and know what it can and cannot rely on in this CLI version. Avoids version-sniffing via `nemo --version` + external lookup.

## Non-Functional Requirements

### NFR-1: No behavior change for existing commands

All existing invocations produce identical stdout/stderr. The new flags (`--json`, `--no-hints`, `--all`, `--format=json`) are additive. The existing one-line-per-command help output is preserved.

### NFR-2: Tests

- **Unit** (`cli/src/commands/help_ai.rs`): `nemo help ai` renders with no errors; contains section headings for state machine, workflows, recovery.
- **Unit** (`cli/src/commands/*.rs`): each command's `long_about` contains "Example:" substring.
- **Integration**: `nemo help --all --format=json` parses as valid JSON with expected keys.
- **Manual**: run each of the FR-4 error-path scenarios, verify the hint line appears.

## Acceptance Criteria

1. **LLM can operate nautiloop from `nemo help ai` alone**: an operator gives a fresh LLM (no prior nautiloop knowledge, no repo access) only the output of `nemo help ai`. The LLM can correctly describe how to submit a spec, approve it, watch it, and recover from AWAITING_REAUTH.
2. **Per-command examples**: `nemo help approve` shows an example invocation with expected output.
3. **JSON everywhere stateful**: `nemo status --json | jq '.loops[0].loop_id'` returns a UUID string.
4. **Error hints present**: invoke `nemo approve` on an IMPLEMENTING loop; stderr includes a recovery hint directing the user to `nemo logs`.
5. **Mega-help reachable**: `nemo help --all` prints all command docs. `nemo help --all --format=json` parses as valid JSON.
6. **Capabilities reflect build**: `nemo capabilities` returns a JSON object; `features.qa_stage` is false until #159 ships.
7. **No regressions**: all pre-existing `nemo` invocations produce the same output as before.

## Out of Scope

- **Man pages.** Could auto-generate from clap, but `nemo help` is the better discovery surface for LLMs.
- **Shell completions.** Separate concern; easy follow-up with `clap_complete`.
- **Interactive TUI help browser.** `nemo helm` is already a TUI; a `?` keybind to open help docs inside helm is a phase-3 helm spec, not CLI.
- **Translating help to languages other than English.** English-only in v1.
- **Auto-generating docs site from help text.** Nice future; out of scope for this spec.
- **Examples that actually execute** (doctest-style). Too brittle when cluster state varies. Examples are illustrative text.

## Files Likely Touched

- `cli/src/main.rs` — add `long_about` to every subcommand; add `--json` flag to stateful ones; new `help ai`, `help --all`, `capabilities` subcommands.
- `cli/src/commands/help_ai.md` — new: embedded primer document.
- `cli/src/commands/help_ai.rs` — new: renders the primer (text / JSON).
- `cli/src/commands/error_hints.rs` — new: pattern → hint table + wrapping logic.
- `cli/src/client.rs` — surface error patterns to the hint system.
- `cli/src/commands/status.rs`, `inspect.rs`, `approve.rs`, `cancel.rs`, `resume.rs`, `extend.rs`, `models.rs`, `auth.rs` — add `--json` output paths.
- `cli/src/capabilities.rs` — new: compile-time feature flags → JSON.
- Tests per NFR-2.

## Baseline Branch

`main` at PR #159 merge.
