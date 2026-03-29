Read all Rust source under `control-plane/src` and `cli/src`. Not clean.

- High - `POST /credentials` accepts an unvalidated `provider`, then `build_job()` uses it as Secret key/path, `sub_path`, mount path, and env suffix, so arbitrary strings can generate invalid or unsafe pod specs and break dispatch. See `control-plane/src/api/handlers.rs:553` and `control-plane/src/k8s/job_builder.rs:121`
- High - `/credentials` claims it cannot create engineers, but it auto-creates them; with shared API-key auth, any authenticated caller can register credentials for arbitrary engineer names. See `control-plane/src/api/handlers.rs:479` and `control-plane/src/api/handlers.rs:533`
- Medium - several client-input validation failures are surfaced as `Internal`, which maps to HTTP 500 instead of a 4xx: invalid engineer names, invalid `credential_ref`, and missing `?force=true` on forced resume. See `control-plane/src/api/handlers.rs:389`, `control-plane/src/api/handlers.rs:485`, `control-plane/src/api/handlers.rs:509`, `control-plane/src/error.rs:89`
- Medium - CLI auth UX is misleading: help says it “Push[es] local model credentials to cluster,” but the command only checks local files and registers a Secret name with the API; it never creates or verifies the Kubernetes Secret. See `cli/src/main.rs:132` and `cli/src/commands/auth.rs:59`
- Low - config architecture appears drifted: runtime uses `NemoConfig`, while the separate cluster/engineer/merged stack looks unused, and `max_cluster_jobs` appears defined without any live enforcement in the loop engine. See `control-plane/src/config/mod.rs:12`, `control-plane/src/config/cluster.rs:34`, `control-plane/src/config/merged.rs:77`
- Low - merge-event persistence is stubbed in the Postgres store, so recorded merge events are not actually saved. See `control-plane/src/state/postgres.rs:713`

If you want, I can do round 13 as a fix plan only, still without editing.
