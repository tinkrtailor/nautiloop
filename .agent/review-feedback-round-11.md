Found 6 real production bugs after reading all Rust source in `control-plane/src` and `cli/src`.

- High — orphaned running jobs on divergence: `control-plane/src/loop_engine/driver.rs:189`
  - When a branch becomes `RemoteAhead`, `ForceDeviated`, or `RemoteGone` during `JobStatus::Running`, the loop is paused/cancelled in DB only.
  - The active K8s job is not deleted, so the agent can keep running and even keep pushing after Nemo decided to stop.

- High — arbitrary engineer credential takeover: `control-plane/src/api/handlers.rs:533`
  - `POST /credentials` accepts any `engineer` name and auto-creates or overwrites that identity under global bearer auth.
  - Any authenticated caller can bind credentials to another engineer name, so later jobs for that identity use attacker-chosen secrets.

- Medium — false reauth on normal job failures: `control-plane/src/loop_engine/driver.rs:972`, `control-plane/src/loop_engine/driver.rs:1434`
  - Auth-expiry detection is substring-based: `auth`, `credential`, `unauthorized`, `api key`, `401`, etc.
  - Ordinary failures mentioning those strings get misclassified as expired credentials, sending loops to `AwaitingReauth` instead of normal retry/failure handling.

- Medium — stale SHA if PR creation fails after review cleanup: `control-plane/src/loop_engine/driver.rs:649`
  - Review-clean path removes `.agent`, updates `record.sha` in memory, then calls `create_pr()`.
  - If PR creation fails, the new SHA is never persisted, so the next tick can treat Nemo’s own cleanup commit as external divergence and pause incorrectly.

- Medium — same stale-SHA bug in harden-only flow: `control-plane/src/loop_engine/driver.rs:452`
  - Harden-only audit-clean path does cleanup before `create_pr()` but does not persist the post-cleanup SHA first.
  - A transient PR failure can make the next tick falsely detect remote advancement caused by Nemo’s own cleanup commit.

- High — `nemo init` generates unusable repo config: `cli/src/commands/init.rs:12`
  - The command claims to generate `nemo.toml` by scanning the repo, but it writes a hardcoded file and does not scan anything.
  - The output also does not match the repo config schema expected by `control-plane/src/config/repo.rs:82` because it omits required `[repo]` data, so the advertised init flow produces a broken config.

- Medium — silent truncated live logs on SSE disconnect: `cli/src/commands/logs.rs:85`
  - If SSE ends without an explicit `"type":"end"` event, `nemo logs` exits `Ok(())` with no warning.
  - Network drops, proxy timeouts, or server restarts therefore look like successful completion while the loop may still be running.
