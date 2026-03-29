Starting with a full inventory of Rust source, then I’ll read crate-by-crate and look for correctness, regression, and test gaps.
API/types pass looks mostly tight so far. Moving into state, loop engine, git, k8s, config, and CLI for the remaining review surface.
The remaining surface is mostly config/bootstrap and CLI command glue. I’m reading that now, plus the rest of the git module to check PR/merge edge cases.
I’ve finished the first-pass read. I’m doing one independent sweep now for edge-case bugs and regressions before I call it clean.
1. High: cancellation can lose to ship-side effects  
`control-plane/src/loop_engine/driver.rs:62-65`, `control-plane/src/loop_engine/driver.rs:760-817`  
A tick only checks `cancel_requested` once at entry. If the user cancels while the same tick is already evaluating a clean review result, that tick can still create/merge the PR and transition the loop to `SHIPPED` before the next tick sees the cancel.

2. Medium: `force_resume` can leak across reauth resumes  
`control-plane/src/api/handlers.rs:422-432`, `control-plane/src/loop_engine/driver.rs:962-978`  
`POST /resume?...force=true` sets `force_resume` for any resumable state, including `AWAITING_REAUTH`. The reauth resume path clears `resume_requested` but never clears `force_resume`, so a later `PausedForceDeviated` pause can be resumed without a fresh `?force=true` acknowledgement.

3. Medium: `nemo init` can emit invalid or mis-shaped TOML for service names  
`cli/src/commands/init.rs:47-52`, `cli/src/commands/init.rs:136-143`  
Service names are derived from raw directory names and written directly into `[services.{name}]`. Names containing `.` create nested tables instead of a single service key, and names with spaces or other punctuation can generate invalid `nemo.toml`.

4. Low: CLI config file is not permission-hardened on non-Unix platforms  
`cli/src/config.rs:83-86`  
The Unix path writes `~/.nemo/config.toml` with `0600`, but the non-Unix path uses plain `std::fs::write`, which can leave `api_key` readable under default filesystem ACLs.

Read all Rust source under `control-plane/src` and `cli/src`. Not clean; not converged. No other concrete Rust findings stood out beyond the items above.
