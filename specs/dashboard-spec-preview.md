# Dashboard Spec Preview

## Overview

Render the current (possibly hardened) spec content as Markdown on the dashboard's loop detail page, specifically for loops in `AWAITING_APPROVAL` where the engineer is about to approve a hardened spec and kick off implementation. The user sees exactly what was hardened — every audit/revise edit — before saying yes.

Scope is narrow: spec rendering + a prominent Approve CTA that the engineer can reach from the phone. No editing, no side-by-side diff versus main (that's a separate feature), no rendering of non-spec markdown.

## Baseline

Main at PR #172 merge.

Current dashboard loop detail page (from #166, extended by #170):
- Hero header with state badge + elapsed + actions row (Approve, Cancel, Resume, Extend, Open PR)
- Rounds table with per-round verdicts
- Live log pane
- Token/cost breakdown

What the engineer CANNOT see today without leaving the dashboard:
- The actual spec text the loop is working from
- What changed during the harden phase vs. the spec they originally submitted

Result: operators approve hardened specs blindly or Ctrl-Tab to GitHub to read the branch's `specs/<file>.md` contents. Defeats the "CTO-on-the-go" story for the most consequential tap on the page.

## Problem Statement

### Problem 1: Approve is a high-stakes one-tap action

Tapping Approve on an `AWAITING_APPROVAL` loop commits minutes-to-hours of paid compute and opens a real PR against a real repo. The operator should see WHAT they're approving with their thumb before they tap. Today that means switching apps to GitHub, navigating to the branch, opening the hardened spec file, and scrolling.

### Problem 2: Harden-phase edits are invisible in the dashboard

A hardened spec can be materially different from what the engineer submitted: new FRs, tightened acceptance criteria, clarified scope. The rounds table shows `audit r1 → not clean → 8 issues`, `revise r1 → patched`, `audit r2 → clean`, but NOT the final text. The operator learns "it was hardened" without learning "into what."

### Problem 3: The CTO-on-the-go story breaks at the approve gate

The dashboard pitch is "approve from your phone." Reality: "approve from your phone after pulling GitHub on desktop to read the spec." That's a worse user experience than just doing it on desktop.

## Functional Requirements

### FR-1: Rendered spec pane on loop detail

**FR-1a.** Loop detail page (`/dashboard/loops/:id`) gains a new section above the rounds table, titled `Spec`. Visibility rules:

| Loop state | Spec pane visible? | Content shown |
|---|---|---|
| PENDING | No | The loop hasn't been created yet; nothing to show. |
| AWAITING_APPROVAL | YES, expanded by default | Current spec from the agent branch (post-harden if harden ran) |
| HARDENING | YES, collapsed by default | Current branch spec (live-updates as revise commits land) |
| IMPLEMENTING / TESTING / REVIEWING | YES, collapsed by default | Snapshot at approve-time (spec doesn't change during implement) |
| Terminal (CONVERGED / FAILED / etc.) | YES, collapsed by default | Final branch spec content |

"Collapsed by default" = visible header + expand-to-read chevron, consistent with the existing Inspect pod disclosure from #137.

**FR-1b.** Source of truth: always the spec file on the agent branch's HEAD (not the PR description, not the spec on main). When the loop hasn't hardened, this is identical to what the engineer submitted. When hardened, this is the hardened version.

**FR-1c.** The spec path is read from the loop record's `spec_path` field (e.g., `specs/foo.md`). The raw file content is fetched from the bare repo at the branch's tip SHA.

### FR-2: Markdown rendering

**FR-2a.** Rendered with `pulldown-cmark` (already in the Rust ecosystem, minimal deps). Security-conscious options:
- No raw HTML passthrough (`pulldown_cmark::Options::empty()` then enable only what's safe: tables, strikethrough, task lists, footnotes).
- No inline scripts. Ever.
- External links open in new tab via `rel="noopener noreferrer" target="_blank"`.
- Images: allowed but `loading="lazy"` and `referrerpolicy="no-referrer"`. If the image URL points at a private host the user's browser won't follow; that's acceptable.

**FR-2b.** Rendered HTML is placed inside a container with class `spec-content` that scopes the styling so the spec's markdown doesn't bleed into the rest of the dashboard's UI.

**FR-2c.** Minimal CSS for the rendered markdown: headings keep the dashboard's font and color hierarchy, code blocks get a monospace + surface-colored background, tables get subtle borders matching the dashboard palette. The pane should feel like part of the dashboard, not a raw GitHub-style markdown render.

**FR-2d.** Code blocks get NO syntax highlighting in v1 (lower priority; adds 50KB+ to JS/CSS budget). Follow-up spec can add `highlight.js` or similar.

### FR-3: Prominent approve CTA

**FR-3a.** When the loop is in `AWAITING_APPROVAL`:
- The Spec pane is expanded by default (FR-1a).
- A sticky "Approve" button pins to the bottom of the spec pane as the user scrolls, on mobile only (viewport < 640px).
- On desktop, the existing actions row at the top stays visible (page scroll doesn't hide it).

**FR-3b.** The sticky mobile button is a normal `<button>` with full-width, high-contrast, `var(--green)` background, `var(--text)` foreground. Tapping triggers the same `/approve/:id` flow the desktop button uses.

**FR-3c.** Cancel is NOT stickied — cancel-by-accident is a harder-to-recover-from error. Keep it in the top actions row only.

### FR-4: Server endpoint

**FR-4a.** New endpoint `GET /dashboard/loops/:id/spec` returns the raw spec text as `text/markdown; charset=utf-8`:

```
GET /dashboard/loops/:id/spec
200 OK
Content-Type: text/markdown; charset=utf-8

# Spec Title
...
```

**FR-4b.** Auth: same cookie-or-bearer as other `/dashboard/*` routes.

**FR-4c.** 404 if the loop doesn't exist; 404 if the spec file doesn't exist at the branch tip (e.g., spec was deleted during a bug in revise). 503 if the bare repo is unavailable.

**FR-4d.** Response is the raw file content. Markdown → HTML rendering happens server-side in the handler that renders the loop detail page, using the raw fetch above as input. The raw endpoint is also exposed so clients / scripts can fetch it without HTML noise.

### FR-5: Live updates for HARDENING

**FR-5a.** When a loop is in `HARDENING` state AND the spec pane is expanded, the JS polls the raw endpoint every 10s and re-renders if the content changed. Debounced: if the spec changes more than 3 times in 30s the polling backs off to every 30s.

**FR-5b.** A small indicator (pulsing dot or "updated just now" text) shows when the spec was last refreshed. The indicator disappears when the loop exits HARDENING.

**FR-5c.** For other states (not HARDENING), the spec pane does NOT poll — the spec is stable once implement starts.

### FR-6: Diff against submitted (optional, phase 2)

**FR-6a.** NOT in v1. Intentionally deferred: showing a unified diff between the original-submitted spec (commit 1 on the branch, pre-harden) and the current hardened spec would be useful but doubles the surface area. Phase 2 can add a `Show diff from submission` toggle that overlays a red/green unified diff on top of the rendered markdown.

## Non-Functional Requirements

### NFR-1: No new storage

Spec content is read on-demand from the bare repo. No new columns, no migrations, no caching layer.

### NFR-2: Rendered content cost

`pulldown-cmark` is MIT-licensed, ~250KB compiled. Server-side render time for a 25KB spec: <10ms. Acceptable overhead for the loop-detail-page response.

### NFR-3: Security

Markdown rendering is the only new attack surface. Mitigated by disabling raw HTML in pulldown-cmark options (FR-2a). No URL rewriting, no fetch of remote assets server-side, no DOM sanitization beyond what pulldown-cmark already provides.

### NFR-4: Tests

- **Unit** (`control-plane/src/api/dashboard/spec.rs`): raw endpoint returns file content for existing loop; 404 for missing; 404 for missing file on branch.
- **Unit**: markdown rendering with raw HTML input produces escaped output (`<script>` becomes `&lt;script&gt;`).
- **Integration**: end-to-end page-render test for an AWAITING_APPROVAL loop — spec pane is expanded, contains rendered markdown, sticky button is present in the response.
- **Manual**: open dashboard, hit `/dashboard/loops/<id>` for a loop in each state listed in FR-1a, verify collapse/expand defaults match.

## Acceptance Criteria

A reviewer can verify by:

1. **AWAITING_APPROVAL spec visible**: submit a spec with `nemo start --harden`. When loop hits AWAITING_APPROVAL, open the dashboard loop detail page. Spec pane is expanded by default. Rendered markdown headings, lists, code blocks, tables all look right (not plaintext mess).
2. **HARDENING live updates**: start a loop, open the detail page during HARDENING. Expand the spec pane. Trigger a revise commit (manually or by waiting). The spec content updates within 10s.
3. **Non-AWAITING collapsed**: open the detail page for a CONVERGED loop. Spec pane exists but is collapsed. Click chevron to expand. Content is correct.
4. **Mobile sticky approve**: open AWAITING_APPROVAL detail page on a phone viewport. Scroll down through the spec. The Approve button stays pinned to the bottom as the user reads.
5. **Security**: craft a spec with `<script>alert("xss")</script>` and a link `[click](javascript:alert(1))`. Submit and let it hit AWAITING_APPROVAL. Rendered page escapes both: the literal text appears as text, no alerts fire.
6. **Raw endpoint**: `curl -H "Authorization: Bearer <key>" http://localhost:18080/dashboard/loops/<id>/spec` returns the spec file contents as plain markdown.

## Out of Scope

- **Side-by-side diff** between submitted and hardened spec (FR-6, phase 2).
- **Syntax highlighting in code blocks** (FR-2d, phase 2).
- **Editing the spec from the dashboard**. Read-only view. To change a spec, engineers edit locally and resubmit.
- **Rendering non-spec markdown** (e.g., READMEs, docs). Spec files only, via the known loop path.
- **Download-as-PDF** or print-friendly rendering.
- **Collaborative comments / annotations on the spec**. Different surface.
- **Rendering the spec PR body**. The spec file itself is the source of truth.
- **Embedding the rendered spec in the PR description**. GitHub already renders markdown; no need to double-render.

## Files Likely Touched

- `control-plane/Cargo.toml` — add `pulldown-cmark` dep (~250KB compiled).
- `control-plane/src/api/dashboard/spec.rs` — new module: raw endpoint + server-side markdown-to-HTML renderer.
- `control-plane/src/api/dashboard/mod.rs` — route wiring for `/dashboard/loops/:id/spec`.
- `control-plane/src/api/dashboard/templates.rs` (or equivalent) — loop detail page now renders the spec pane.
- `control-plane/assets/dashboard.css` — `.spec-content` scoped styling + sticky-approve button styles.
- `control-plane/assets/dashboard.js` — polling for HARDENING state, collapse/expand toggle, sticky button positioning.
- Tests per NFR-4.

## Baseline Branch

`main` at PR #172 merge.
