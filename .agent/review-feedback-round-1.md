I reviewed all Rust sources under `control-plane/src` and `cli/src`.

1. `control-plane/src/git/mod.rs:233`, `control-plane/src/loop_engine/driver.rs:157`, `control-plane/src/loop_engine/driver.rs:231` — Critical  
   `has_diverged()` is implemented as plain `sha != expected_sha`, so a normal agent commit looks like divergence. Running jobs can be paused instead of progressing, and completed jobs can be rejected after successful output writes.  
   Fix: replace the boolean check with ancestry-aware divergence detection (`merge-base --is-ancestor` semantics). Only pause when the branch tip is not a descendant of the dispatched SHA, and distinguish remote-ahead vs force-deviated.

2. `control-plane/src/loop_engine/driver.rs:900` — Critical  
   `start_implementing()` changes `phase` to `implement` but never updates `stage`. If a harden loop transitions into implement, the row becomes `phase=implement` with `stage=spec_audit/spec_revise`, which violates the DB check constraint in `control-plane/migrations/20260328000001_initial_schema.sql:92`.  
   Fix: set `record.stage` on every stage transition (`SpecAudit`, `SpecRevise`, `Implementing`, `Testing`, `Reviewing`) before `update_loop()`.

3. `control-plane/src/loop_engine/driver.rs:1211`, `control-plane/src/loop_engine/driver.rs:1243`, `control-plane/migrations/20260328000001_initial_schema.sql:132` — High  
   The driver inserts a `jobs` row before creating the K8s job and always uses `attempt = 1`. If K8s creation fails once, the retry hits the unique index on `(loop_id, stage, round, attempt)` and the loop wedges permanently.  
   Fix: either create the DB row only after successful K8s creation, or allocate/increment attempts atomically and persist failed create attempts distinctly.

4. `control-plane/src/api/handlers.rs:35`, `control-plane/src/git/mod.rs:171`, `control-plane/src/git/mod.rs:467` — High  
   Runtime git flow hardcodes `origin/main` for spec reads, branch base, and merge SHA lookup, even though repo config exposes `default_branch`. Repos using `master`, `trunk`, etc. will fail incorrectly.  
   Fix: resolve the configured default branch once and thread it through API + git operations; remove all `origin/main` literals.

5. `control-plane/src/types/api.rs:20`, `cli/src/commands/start.rs:33`, `control-plane/src/api/handlers.rs:23` — Medium  
   The CLI sends `model_overrides`, the API accepts it, but the server never stores or applies it. Users can request model overrides and silently get defaults instead.  
   Fix: persist per-loop overrides and use them when building stage configs, or reject the field until implemented.
