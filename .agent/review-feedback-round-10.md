Not clean. I found 6 real Rust bugs.

- `control-plane/src/api/handlers.rs:177` + `control-plane/src/loop_engine/driver.rs:85` - `/start` returns success even when `set_sha()` fails, but `handle_pending()` refuses to advance loops with empty `sha`; a transient DB write failure can leave a loop stuck in `PENDING` forever.
- `control-plane/src/loop_engine/driver.rs:1191` + `control-plane/src/state/postgres.rs:515` - harden redispatch infers the last sub-stage from `get_jobs()`, but jobs are only ordered by `(round, attempt)`; audit and revise commonly share `attempt = 1`, so retries can nondeterministically redispatch the wrong harden stage.
- `control-plane/src/api/handlers.rs:491` + `control-plane/src/k8s/job_builder.rs:123` - `/credentials` accepts Secret names up to 253 chars, then `job_builder` prefixes that into a volume name (`cred-{secret}`), which must fit the much shorter K8s volume-name limit; accepted inputs can later fail job admission.
- `control-plane/src/loop_engine/driver.rs:1314` + `control-plane/src/k8s/job_builder.rs:37` - loop context sets `engineer` to the engineer UUID, so `NEMO_ENGINEER` and the `nemo.dev/engineer` label contain UUIDs instead of engineer names, breaking engineer metadata/label semantics.
- `cli/src/commands/auth.rs:23` + `cli/src/main.rs:196` + `cli/src/commands/start.rs:22` + `cli/src/commands/status.rs:24` + `control-plane/src/api/handlers.rs:78` - `nemo auth` normalizes engineer names, but `start`/`status` use the raw configured value; `auth` can succeed for `John Doe` as `john-doe` while `start` is rejected and `status` queries the wrong identity.
- `cli/src/commands/auth.rs:105` - even `nemo auth --claude` or `--openai` prints a `kubectl create secret` command that requires both credential files, so following the CLI’s own success instructions can fail for single-provider setups.

If you want, I can do round 11 with fix suggestions only, still read-only.
