Starting with repo state and any Lane B round-tracking docs, then I’ll review the current diff in read-only mode and report only concrete findings.
Repo is effectively clean aside from one untracked sandbox file. I’m checking what Lane B contains relative to `main` so I can do the round review against actual branch changes, not the empty worktree.
I’ve got the branch context and prior-round history. Now I’m doing a fresh pass on the live Rust code, with extra attention to the Round 17 areas plus any regressions around loop resume, output validation, and CLI/API edge cases.
Not clean. Not converged.

1. High — `control-plane/src/loop_engine/driver.rs:574-582`, `control-plane/src/loop_engine/driver.rs:833-845`, `control-plane/src/git/mod.rs:522-556`
`merge_pr()` is not idempotent, but both the harden auto-merge path and ship auto-merge path call it unconditionally. If `gh pr merge` succeeds and the process dies before `record.merge_sha` / terminal state is persisted, the next tick retries the merge against an already-merged PR and gets a hard error instead of converging.

2. Medium — `control-plane/src/loop_engine/driver.rs:995-1008` compared with `control-plane/src/loop_engine/driver.rs:962-973`
`AWAITING_REAUTH` resume refreshes `record.sha` from the local branch only. Unlike paused resume, it does not `fetch` and prefer `origin/<branch>`. If the engineer pushed while the loop was waiting for reauth, resume can redispatch stale code and then false-pause again on the next divergence check.

3. Medium — `control-plane/src/loop_engine/driver.rs:354-372`, `control-plane/src/loop_engine/driver.rs:464-497`, `control-plane/src/types/verdict.rs:54-58`
Implement-stage success only checks that `output_json` exists, not that it parses as `ImplOutput { sha, affected_services }`. Any syntactically valid JSON can advance the loop into testing, so malformed implement output is treated as success and the stage contract is effectively unenforced.

4. Low — `cli/src/commands/init.rs:57-81`
`nemo init` sanitizes service names into TOML table keys but never deduplicates post-sanitization collisions. Distinct services like `foo.bar` and `foo-bar` both become `[services.foo-bar]`, producing duplicate/ambiguous config entries.

Read-only review only; no files changed.
