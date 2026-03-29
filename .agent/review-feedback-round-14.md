Read all Rust source in `control-plane/src` and `cli/src`. Not clean yet.

- High: remotely deleted branches can be misclassified as local-only, so loops may keep running instead of handling `RemoteGone` correctly in `control-plane/src/git/mod.rs:266`
- High: `nemo init` stops recursion as soon as a parent dir has a marker file, so nested services in monorepos get skipped in `cli/src/commands/init.rs:123`
- Medium: engineer names are format-checked but not length-limited before being copied into the K8s label `nemo.dev/engineer`, which can exceed the 63-char label limit between `control-plane/src/api/handlers.rs:81` and `control-plane/src/k8s/job_builder.rs:40`
- Medium: feedback redispatch sets `ctx.feedback_path` but never persists `record.feedback_path` before job record creation, so DB job history can lose which feedback file triggered the retry in `control-plane/src/loop_engine/driver.rs:1197` and `control-plane/src/loop_engine/driver.rs:1482`
- Medium: auto-detected repo services are keyed only by leaf directory name, so same-basename services in different paths collide and one disappears in `control-plane/src/config/repo.rs:188`

If you want, I can do Round 15 with fixes only.
