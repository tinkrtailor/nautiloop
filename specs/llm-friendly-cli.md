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

> **Note on state names**: The authoritative state enum is `LoopState` in `control-plane/src/types/mod.rs`. At time of writing it contains: Pending, Hardening, AwaitingApproval, Implementing, Testing, Reviewing, Converged, Failed, Cancelled, Paused, AwaitingReauth, Hardened, Shipped. The `help_ai.md` template must reflect whatever states exist in that enum at implementation time.

- **What nautiloop is**: one paragraph. Convergent loop orchestrator, cross-model adversarial review, self-hosted.
- **State machine diagram** (ASCII): the full loop lifecycle with every transition.
- **Terminal states**: CONVERGED, HARDENED, FAILED, CANCELLED, SHIPPED — what each means.
- **Typical workflow 1 (implement)**: `nemo start spec.md` → `nemo approve <id>` → `nemo logs <id>` → wait → PR.
- **Typical workflow 2 (harden-first)**: `nemo start spec.md` → review hardened spec PR → `nemo approve <id>` → watch → PR. (Harden is the default; use `--no-harden` to skip it. The `--harden` flag is deprecated and emits a warning.)
- **Typical workflow 3 (ship)**: `nemo ship spec.md` → (no approval, no human) → auto-merged PR.
- **Recovery playbooks**: AWAITING_REAUTH → `nemo auth --claude && nemo resume <id>`. PAUSED → `nemo resume`. FAILED (max rounds) → `nemo extend --add 10 <id>` OR investigate. Stale kubectl context? Don't switch, use `--context=<name>` per command.
- **Config hierarchy**: engineer (`~/.nemo/config.toml`) > repo (`nemo.toml` on main) > cluster (control plane ConfigMap). Explain which lives where.
- **Command catalog**: full list with one-line descriptions, same as `nemo --help`, but grouped into categories: loop lifecycle, observability, identity, config.
- **Example spec structure**: the minimum skeleton a spec needs — overview, FRs, acceptance criteria. Points at `docs/spec-authoring.md` (future) if it exists.
- **Known failure modes**: reviewer nitpick-loops, max_rounds exhaustion, network drops mid-pod. For each, how to detect and recover.

**FR-1b.** Output is Markdown, ~200-400 lines, readable end-to-end. Generated from a template file embedded in the binary via `include_str!`. Maintained in `cli/src/commands/help_ai.md` — not duplicated per-platform.

**FR-1c.** `nemo help ai --format=json` emits the same information as structured JSON. For agents that prefer to parse rather than read prose.

The JSON schema for `nemo help ai --format=json`:

```json
{
  "overview": "string — what nautiloop is",
  "state_machine": {
    "states": [
      { "name": "PENDING", "terminal": false, "description": "string" }
    ],
    "transitions": [
      { "from": "PENDING", "to": "AWAITING_APPROVAL", "trigger": "string" }
    ]
  },
  "workflows": [
    {
      "name": "implement",
      "description": "string",
      "steps": [
        { "command": "nemo start spec.md", "description": "string" }
      ]
    }
  ],
  "recovery_playbooks": [
    {
      "state": "AWAITING_REAUTH",
      "description": "string",
      "commands": ["nemo auth --claude", "nemo resume <id>"]
    }
  ],
  "config_hierarchy": {
    "levels": [
      { "name": "engineer", "path": "~/.nemo/config.toml", "description": "string" }
    ]
  },
  "command_catalog": {
    "loop_lifecycle": [
      { "command": "start", "short": "string" }
    ],
    "observability": [],
    "identity": [],
    "config": []
  },
  "spec_structure": "string — minimum skeleton description",
  "known_failure_modes": [
    { "name": "string", "detection": "string", "recovery": "string" }
  ]
}
```

All top-level keys are required. The ASCII state machine diagram from the Markdown version is represented as structured `states` and `transitions` arrays rather than a rendered diagram.

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

**FR-2b.** Both `nemo <cmd> --help` and `nemo help <cmd>` show the full `long_about` with examples. This is clap's natural behavior when `long_about` is set.

> **Implementation note**: Clap displays `long_about` for both `--help` and `help <cmd>` by default. No custom help template is needed for this requirement. The short description (from `about`) is shown only in the parent command's subcommand listing (e.g., `nemo --help`).

**FR-2c.** Applied to ALL subcommands: harden, start, ship, status, helm, logs, ps, cancel, approve, inspect, resume, extend, init, auth, models, config, cache (once shipped).

### FR-3: `--json` output mode on every stateful command

**FR-3a.** Commands whose output an agent might parse get a `--json` flag:
- `nemo status --json` — **already implemented**; preserve existing output schema (loops array with `loop_id`, `state`, etc.)
- `nemo inspect <branch> --json` — already emits JSON by default; add `--json` as a no-op flag for consistency so agents can pass `--json` uniformly without error
- `nemo approve <id> --json` — structured response object
- `nemo cancel <id> --json` — structured response
- `nemo resume <id> --json` — structured response
- `nemo extend <id> --json` — structured response
- `nemo models --json` — providers + available models as JSON
- `nemo auth --json` — push results as JSON
- `nemo cache show --json` — **already implemented**; preserve existing output schema

> **Note**: `status` and `cache show` already support `--json` with established output schemas. Do not change their existing field names or structure. New `--json` implementations on other commands should follow the same conventions (snake_case keys, `serde_json::to_string_pretty`).

**FR-3b.** JSON output schema documented in `nemo help ai` (FR-1). Stable field names, no presentation-level keys.

**FR-3c.** `--json` always emits JSON regardless of TTY status. Without `--json`, output is always plain text regardless of TTY status (no auto-detection). There is no implicit format switching based on whether stdout is a terminal.

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

Matching rules:
- Patterns are **case-insensitive substring** matches against the error message body.
- Patterns are checked **in definition order**; **first match wins** (no accumulation).
- Where possible, combine substring matching with HTTP status code (e.g., 409 + "approve" → state conflict hint) to reduce fragility.
- If the server changes its error message format in a future version, hints gracefully degrade: unmatched errors pass through with no hint rather than showing a wrong hint.

> **Fragility note**: String-based pattern matching is inherently coupled to server error message wording. This is acceptable for v1 since the server and CLI are co-versioned. If the server adds structured error codes in the future, hints should migrate to code-based matching.

**FR-4c.** `--no-hints` flag suppresses the hints for scripting contexts where stable error output matters.

### FR-5: `nemo help --all`

**FR-5a.** New flag: `nemo help --all` dumps every subcommand's long_about in one shot. One-call total CLI documentation.

**FR-5b.** Output format: Markdown, headings per command. Same prose as individual `nemo help <cmd>` but concatenated.

**FR-5c.** `nemo help --all --format=json` returns a single JSON object with the following schema:

```json
{
  "commands": {
    "approve": {
      "short": "Approve a loop awaiting approval",
      "long": "Full long_about text including examples...",
      "options": [
        {
          "name": "--server",
          "short": "-s",
          "type": "string",
          "required": false,
          "description": "Control plane server URL"
        }
      ],
      "positional_args": [
        {
          "name": "LOOP_ID",
          "required": true,
          "description": "Loop ID"
        }
      ]
    }
  }
}
```

- `commands` is a map from command name to command descriptor.
- `short`: the one-line `about` string.
- `long`: the full `long_about` text (including examples, see-also).
- `options`: array of flag/option descriptors. `short` is null if no short flag. `type` is one of `"string"`, `"bool"`, `"integer"`.
- `positional_args`: array of positional argument descriptors. Omitted (or empty array) if the command takes no positional args.

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

> **Implementation note**: Feature flags in `cli/src/capabilities.rs` are hardcoded boolean constants, updated manually when features ship. They are NOT Cargo feature gates — they represent server-side/product-level capability presence, not compile-time conditional compilation. The `commands` array is derived from the clap `Command` definition at build time (iterate subcommands).

### Implementation Note: Custom Help Subcommand

The built-in clap `help` subcommand must be replaced with a custom `Help` subcommand (using `#[command(name = "help")]` and `#[command(disable_help_subcommand = true)]` on the parent) that handles:

- `help ai` / `help llm` — FR-1 mega-primer
- `help --all` — FR-5 full dump
- `help --format=json` — JSON output for FR-1c and FR-5c
- `help <cmd>` — falls back to rendering the matching subcommand's `long_about` (or `about` if no `long_about` is set)
- `help` (no args) — renders the same output as `nemo --help` (subcommand listing)

The `--help` flag on individual subcommands continues to use clap's built-in `--help` handler, which naturally displays `long_about` when set.

## Non-Functional Requirements

### NFR-1: No behavior change for existing commands

All existing invocations produce identical stdout/stderr. The new flags (`--json`, `--no-hints`, `--all`, `--format=json`) are additive. The existing one-line-per-command help output is preserved.

### NFR-2: Tests

- **Unit** (`cli/src/commands/help_ai.rs`): `nemo help ai` renders with no errors; contains section headings for state machine, workflows, recovery.
- **Unit** (`cli/src/commands/*.rs`): each command's `long_about` contains "Example:" substring.
- **Integration**: `nemo help --all --format=json` parses as valid JSON with expected keys.
- **Unit** (`cli/src/commands/error_hints.rs`): for each `(pattern, hint)` pair, assert that a synthetic error message containing the pattern produces the expected hint. Also assert that an unrecognized error message produces no hint (passthrough).

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
