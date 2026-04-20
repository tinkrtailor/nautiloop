# Dashboard State Filter Fix

## Overview

The dashboard's state filter chips (`Active`, `Converged`, `Failed`, `All`) are broken: when ANY chip other than the default is selected, the card grid goes empty. Only the implicit "no filter" URL (`/dashboard` with no `state_filter` query param) shows loops. Selecting `Active`, `Converged`, or `Failed` from the header chips returns zero cards even when loops in those states clearly exist.

Observed on 2026-04-20 immediately after the mobile dashboard (#166) shipped: operator clicks `Active (0)` — grid empty. Clicks `All` — grid empty. Refreshes without a query string — grid populated.

Hypothesis: the filter applies client-side, but the `/dashboard/state` response only returns a subset of loops (likely scoped to the default `Mine` engineer filter + a default state-is-active filter on the server), and when the user picks `Converged` client-side, the data simply isn't in the payload to filter.

## Baseline

Main at PR #173 merge. `/dashboard/state` JSON response from `control-plane/src/api/dashboard/aggregate.rs` builds the payload the card grid polls every 5s. The FR-3e filter chips in `specs/mobile-dashboard.md` specified state chips (`Active/Converged/Failed/All`) plus engineer chips (`Mine/Team/<engineer-names>`) as independent, chip-based client-side filters.

Two candidate causes:
1. `/dashboard/state` uses `get_loops_for_engineer(engineer, team=false, all=false)` — the `all=false` argument excludes terminal loops. When the user clicks `Converged`, those loops were never fetched.
2. The client-side filter JS checks `loop.state === "ACTIVE"` (uppercase literal) against a server-emitted `"IMPLEMENTING"` / `"REVIEWING"` / etc., and never matches.

Whichever the root cause, user-visible symptom is the same: chips filter nothing.

## Problem Statement

### Problem 1: The primary filter UI is broken

The state chips are the most-visible filter control on the page. Operators see the chip counts (`Converged (2)`) and expect clicking to show those two loops. Clicking shows zero. The counts are computed correctly but the filter that uses them is broken.

### Problem 2: All/Team toggle confusion compounds it

With `Active + Mine` selected (default), the user sees their own active loops. Clicking `All` resets the state filter — but the result is an empty grid because `/dashboard/state` only returned the active loops. The user has no way to see terminal loops from the dashboard without knowing the URL hack (remove the query string, pick the right combo, etc.).

### Problem 3: Breaks CTO-on-the-go story

"Tap Converged on your phone to see what shipped this week" is the core morning-coffee workflow. Currently: tap Converged → see nothing → assume nothing shipped → miss the news.

## Functional Requirements

### FR-1: `/dashboard/state` includes terminal loops in the 7-day window

**FR-1a.** The aggregator call switches from `get_loops_for_engineer(eng, team, all=false)` to `get_loops_for_engineer(eng, team, all=true)` AND caps the set to loops whose `created_at >= now() - 7 days`. Terminal loops from earlier than 7 days ago are still reachable via `/dashboard/feed` (#147 FR-12).

**FR-1b.** The response payload gains a `loops` array containing ALL loops in the window (active + terminal), not just active ones. Client filters from the complete set.

**FR-1c.** Count chips (`Active (N)`, `Converged (N)`, `Failed (N)`, `All (N)`) are computed server-side from this complete set so the number always matches what clicking would show.

### FR-2: Client-side filter logic uses the right state buckets

**FR-2a.** The client-side filter maps dashboard chip labels to loop states:

| Chip | Matches states |
|---|---|
| `Active` | `PENDING`, `AWAITING_APPROVAL`, `IMPLEMENTING`, `TESTING`, `REVIEWING`, `HARDENING`, `AWAITING_REAUTH`, `PAUSED` |
| `Converged` | `CONVERGED`, `HARDENED`, `SHIPPED` |
| `Failed` | `FAILED`, `CANCELLED` |
| `All` | any state |

**FR-2b.** Comparison uses string equality on the server-sent state value. Canonical spelling is the `LoopState::to_string()` output (uppercase snake-style: `IMPLEMENTING`, `AWAITING_REAUTH`, etc.). Centralized in a single `STATE_BUCKETS` JS constant so adding a new state later is one-line.

**FR-2c.** `Cancelled` counts as failure not active; engineer signaling "I stopped this" is a failure disposition from the dashboard's perspective even though it's not an error.

### FR-3: URL state syncing

**FR-3a.** The current chip selection is reflected in the URL: `/dashboard?state=active&scope=mine` (etc.). Reloading the page restores the same filter. Sharing the URL shares the view.

**FR-3b.** Valid `state` values: `active` (default), `converged`, `failed`, `all`. Invalid → fall through to default.

**FR-3c.** Valid `scope` values: `mine` (default), `team`, or a specific engineer handle. Invalid → fall through to default.

**FR-3d.** The chips update `history.replaceState()` on click — no page reload, URL stays in sync.

### FR-4: Empty-state messaging

**FR-4a.** When the filter legitimately matches zero loops (e.g., fresh install, no loops have converged yet), the empty-state text depends on the active filter:

| Active filter | Empty state message |
|---|---|
| `Active` + any scope | "No active loops right now. Start one with `nemo start <spec>`." |
| `Converged` + any scope | "No converged loops in the last 7 days. Check the [Feed](/dashboard/feed) for older events." |
| `Failed` + any scope | "No failures in the last 7 days. Good week." |
| `All` | "No loops in the last 7 days. Get started: `nemo start <spec>`." |

**FR-4b.** Instead of the current generic "No loops match the current filters." Each version is short (one sentence), points at a next action, and matches the dashboard's direct voice.

## Non-Functional Requirements

### NFR-1: Performance

Returning all loops in a 7-day window (not just active) is a larger payload, but bounded. In steady-state usage, 50-200 loops per week. At ~400 bytes per summary, payload is 20-80KB uncompressed, <15KB gzipped. Acceptable over the 5s polling interval.

### NFR-2: Backward compatibility

Old `/dashboard/state` payload shape is a strict superset (adds terminal loops to the existing list). Existing clients see more data, not less.

### NFR-3: Tests

- **Unit** (`control-plane/src/api/dashboard/aggregate.rs`): assert the query fetches terminal loops in the 7-day window; assert loops older than 7 days are excluded.
- **Unit** (JS, if tooling exists): `STATE_BUCKETS` maps `IMPLEMENTING` to Active, `CONVERGED` to Converged, etc.
- **Integration**: full dashboard request → server renders the page → grid HTML contains both active and terminal cards when `?state=all` → clicking a chip updates URL and filters.
- **Manual**: open dashboard, click each state chip, verify the visible cards match the chip's category.

## Acceptance Criteria

A reviewer can verify by:

1. **Active shows active**: with at least one IMPLEMENTING loop, click `Active` chip. Grid shows that loop. Click `Converged`. Grid shows any converged loops in the last 7 days. Click `Failed`. Grid shows failed loops.
2. **URL sync**: click `Converged` chip. URL updates to include `?state=converged`. Reload. Still on converged view.
3. **Empty-state copy**: with zero converged loops, click `Converged`. Empty-state message matches FR-4a specifically for that chip.
4. **Counts match**: the chip count `Converged (N)` equals the number of cards shown when that chip is active.
5. **All filter**: `All` shows every loop in the 7-day window regardless of state.
6. **Regression**: default landing (no query string) shows `Active + Mine` exactly as before — no change in no-filter behavior.

## Out of Scope

- **Custom date range picker** (e.g., "last 30 days"). 7-day window is fixed; older loops go through `/dashboard/feed`.
- **Persistent per-user default filter preference** (e.g., "always land on Team view"). Engineers default to `Mine`, cookie doesn't change it.
- **Saved views / favorites**. Not enough signal to justify.
- **Multi-state select** (`Active` + `Converged` simultaneously). Chips are single-select on the state row.

## Files Likely Touched

- `control-plane/src/api/dashboard/aggregate.rs` — fetch all loops in 7-day window, add per-state counts to the response.
- `control-plane/src/api/dashboard/handlers.rs` — parse `state` / `scope` query params on initial page render for server-side active-chip marking.
- `control-plane/src/state/mod.rs` / `postgres.rs` — if a new `get_loops_since(engineer, team, cutoff)` method is cleaner than reusing `get_loops_for_engineer(all=true)`, add it.
- `control-plane/assets/dashboard.js` — `STATE_BUCKETS` constant, chip click handler updates URL + filters client-side, empty-state copy selector.
- `control-plane/assets/dashboard.css` — if empty-state text needs any styling tweaks (probably not, matches existing text styles).
- Tests per NFR-3.

## Baseline Branch

`main` at PR #173 merge.
