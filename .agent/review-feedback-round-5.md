Not converged. I read all Rust source under `control-plane/src` and `cli/src` and found 3 high-confidence production bugs.

- `critical` — `control-plane/src/git/mod.rs:251` — `detect_divergence()` returns `LocalAhead` before checking `origin/<branch>`. If the remote branch advances while the local ref still equals `expected_sha`, the loop misses the engineer push and keeps running on stale history.
- `critical` — `control-plane/src/git/mod.rs:215`, `control-plane/src/git/mod.rs:454` — restart/recreate flow can overwrite existing remote branch commits. When PR lookup is absent/transient, `create_branch()` force-resets the local branch to base, then `create_pr()` force-pushes it, destroying any newer remote commits on that deterministic branch.
- `high` — `cli/src/commands/auth.rs:61`, `cli/src/client.rs:101`, `control-plane/src/api/handlers.rs:471` — `nemo auth` sends raw credential JSON as `credential_ref`, but the server requires a Kubernetes Secret name. That makes credential registration fail deterministically, so `AWAITING_REAUTH` loops cannot be recovered through the CLI.

If you want, I can do Round 6 with the same read-only bar after fixes land.
