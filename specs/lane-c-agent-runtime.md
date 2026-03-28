# Agent Runtime Layer

## Overview

Runtime infrastructure for Nemo agent jobs: the container image agents execute in, the auth sidecar that isolates credentials, the K8s Job template that composes them, the prompt templates that drive each stage, and the Terraform module that provisions the cluster. This spec covers Lane C of the implementation plan.

## Dependencies

- **Requires:** [Design doc](../docs/design.md) (architecture, resource model, loop logic, verdict schema). Note: Postgres (not SQLite) was decided during eng review; see design doc update.
- **Required by:** Control plane loop engine (dispatches jobs defined here), CLI (submits specs that trigger these jobs)

## Requirements

### Functional Requirements

#### Base Agent Image

- FR-1: The base image shall include git, curl, jq, build-essential, Node.js 22 LTS, and Python 3.12 runtime
- FR-2: The base image shall install claude-code via `npm install -g @anthropic-ai/claude-code`
- FR-3: The base image shall install opencode from `ghcr.io/anomalyco/opencode` (binary copy from their published image)
- FR-4: The image entrypoint (`/usr/local/bin/nemo-agent-entry`) shall read `$STAGE` and dispatch to the correct CLI tool with the correct flags. The entrypoint shall use `exec` to replace the shell with the CLI tool process (or use `tini` as PID 1) to ensure correct signal handling.
- FR-5: For IMPLEMENT and SPEC_REVISE stages, the entrypoint shall invoke `claude -p --output-format stream-json --dangerously-skip-permissions` with the prompt assembled from template + spec + feedback
- FR-6: For REVIEW and SPEC_AUDIT stages, the entrypoint shall invoke `opencode run --format json` with the prompt assembled from template + spec + diff context
- FR-7: For round > 1, the entrypoint shall pass `--resume $SESSION_ID` (claude) or `-s $SESSION_ID` (opencode) to continue the prior session
- FR-8: The entrypoint shall configure proxy environment variables so all outbound traffic routes through the sidecar egress logger: `HTTP_PROXY=http://localhost:9092`, `HTTPS_PROXY=http://localhost:9092`, `http_proxy=http://localhost:9092`, `https_proxy=http://localhost:9092`, `NO_PROXY=localhost,127.0.0.1`, `no_proxy=localhost,127.0.0.1`. Both upper- and lower-case variants are required because different tools respect different conventions. `NO_PROXY` prevents double-proxying when the agent calls localhost services (model API on :9090, git proxy on :9091, egress logger on :9092).
- FR-9: The entrypoint shall configure `ANTHROPIC_BASE_URL=http://localhost:9090/v1` and `OPENAI_BASE_URL=http://localhost:9090/v1` so model API calls route through the sidecar auth proxy
- FR-10: The entrypoint shall set `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL`, `GIT_COMMITTER_NAME`, `GIT_COMMITTER_EMAIL` from environment variables
- FR-11: The entrypoint shall set `GIT_SSH_COMMAND` to a script that connects to `localhost:9091` instead of the real remote. The sidecar runs a local SSH server on `:9091` that authenticates with the mounted SSH key and proxies the push to the actual git remote.
- FR-12: Per-monorepo images shall extend the base via `Dockerfile.nemo` in the repo root (e.g., `FROM ghcr.io/nemo/agent-base:latest`)
- FR-13: On exit, the entrypoint shall write structured output to both `/output/result.json` AND stdout in a common result envelope: `{ "stage": "implement|test|review|spec_audit|spec_revise", "data": { ...stage-specific fields... } }`. The control plane dispatches parsing based on the `stage` field. Stage-specific `data` fields: IMPLEMENT: `new_sha`, `token_usage`, `exit_code`, `session_id`; TEST: see FR-42d; REVIEW/SPEC_AUDIT: `verdict`, `token_usage`, `exit_code`, `session_id`; SPEC_REVISE: `revised_spec_path`, `token_usage`, `exit_code`, `session_id`. Pod logs are the durable channel for the control plane; `/output` is for the agent's own use during execution.

#### Auth Sidecar

- FR-14: The sidecar shall be a single static binary (Go, ~10 MB) listening on three localhost ports
- FR-15: Model API proxy (`:9090`): intercept requests to `api.anthropic.com` and `api.openai.com`, inject `Authorization` / `x-api-key` headers from K8s Secret mounted at `/secrets/model-credentials`
- FR-16: Model API proxy shall support both Anthropic header format (`x-api-key: $KEY`) and OpenAI header format (`Authorization: Bearer $KEY`)
- FR-17: Model API proxy shall pass through all other headers and body unmodified
- FR-18: Git SSH proxy (`:9091`): run a local SSH server that accepts connections from the agent container. On receiving any git SSH operation (fetch, push, clone, ls-remote), authenticate with the SSH private key mounted at `/secrets/ssh-key`, open a connection to the actual git remote, and proxy the operation through. The agent's `GIT_SSH_COMMAND` points to a wrapper script that connects to `localhost:9091`. The agent should not need to fetch (worktrees are pre-created by the control plane), but if it does, the proxy handles it transparently.
- FR-19: Egress logger (`:9092`): transparent HTTP/HTTPS CONNECT proxy that logs every outbound connection (timestamp, destination host:port, method, bytes sent, bytes received) to stdout in JSON-lines format
- FR-20: Egress logger shall NOT block or filter any traffic (agents need open internet)
- FR-21: The sidecar shall read credentials from files mounted into its container only; no credentials shall be mounted into the agent container. The sidecar shall re-read credential files from disk on each request (not cache at startup) so that K8s Secret volume updates propagate without restart.
- FR-22: On startup, the sidecar shall wait until all three ports are listening, then write a readiness file to `/tmp/shared/ready` (shared emptyDir volume) AND expose a K8s readiness probe on `:9093/healthz` (for kubelet). The readiness file is the mechanism the agent entrypoint polls; the HTTP probe is for K8s only.
- FR-23: On SIGTERM, the sidecar shall drain active connections (5s grace) then exit

#### K8s Job Template

- FR-24: Each agent job shall be a K8s Job with `restartPolicy: Never`, `imagePullSecrets` referencing the registry credential, and two containers: `agent` and `auth-sidecar`
- FR-25: The agent container shall mount: worktree volume (from bare repo PVC, path `/work`), session state PVC (path `/sessions`), spec files (ConfigMap or PVC, path `/specs`), output volume (emptyDir, path `/output`), shared readiness volume (emptyDir, path `/tmp/shared`), a writable tmpdir (emptyDir, path `/tmp`), and a writable home directory (emptyDir, path `/work/home`). The agent container shall set `securityContext: { runAsNonRoot: true, runAsUser: 1000, readOnlyRootFilesystem: true }` with writable volumes for `/work`, `/work/home`, `/output`, `/sessions`, `/tmp`, and `/tmp/shared`. `/work/home` is needed because claude-code writes to `$HOME/.claude/` and opencode writes to `$HOME/.config/opencode/`.
- FR-26: The sidecar container shall mount: model credentials Secret (path `/secrets/model-credentials`), SSH key Secret (path `/secrets/ssh-key`), shared readiness volume (emptyDir, path `/tmp/shared`). The Secret volumes shall NOT be mounted in the agent container.
- FR-27: The Job shall set these environment variables on the agent container: `STAGE`, `SPEC_PATH`, `FEEDBACK_PATH`, `SESSION_ID`, `BRANCH`, `SHA`, `MODEL`, `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL`, `ROUND`, `MAX_ROUNDS`, `LOOP_ID`, `HOME=/work/home`
- FR-28: Resource limits per job type:

| Container / Job type | CPU request | CPU limit | RAM request | RAM limit |
|----------------------|------------|-----------|-------------|-----------|
| IMPLEMENT | 250m | 500m | 1Gi | 2Gi |
| REVIEW | 250m | 500m | 1Gi | 2Gi |
| SPEC_AUDIT | 250m | 500m | 1Gi | 2Gi |
| SPEC_REVISE | 250m | 500m | 1Gi | 2Gi |
| TEST (default) | 500m | 1000m | 1Gi | 3Gi |
| TEST (jvm tag) | 1000m | 2000m | 2Gi | 6Gi |
| auth-sidecar (all) | 50m | 100m | 64Mi | 128Mi |

- FR-29: Jobs shall have `activeDeadlineSeconds: 900` (15 min) as a watchdog. The control plane may override this per stage.
- FR-30: The sidecar shall write a readiness file to `/tmp/shared/ready` (shared emptyDir volume). The agent entrypoint polls this file (100ms interval, 30s timeout). `shareProcessNamespace` shall NOT be used (it leaks `/proc` across containers).
- FR-31: Job names shall follow the pattern `nemo-{loop_id_short}-{stage}-r{round}` where `loop_id_short` is the first 8 characters of the loop ID, to stay under the K8s 63-character name limit (e.g., `nemo-a3f2b1c9-implement-r2`)
- FR-32: Jobs shall have labels: `nemo.dev/loop-id`, `nemo.dev/stage`, `nemo.dev/engineer`, `nemo.dev/round` for control plane queries

#### Prompt Templates

- FR-33: Default prompt templates shall ship as files embedded in the control plane binary and written to a ConfigMap on deploy
- FR-34: Repo-side overrides shall live in `.nemo/prompts/` and take precedence over defaults when present
- FR-35: `implement.md` template shall include: role definition (implementer), spec contents (injected), branch/SHA context, instruction to commit changes with meaningful messages, and (if round > 1) prior review feedback
- FR-36: `review.md` template shall include: role definition (adversarial reviewer), spec contents (injected), diff context (`git diff $BASE...$SHA`), the verdict JSON schema (inline), instruction to output valid JSON matching the schema, and instruction to check for: correctness vs spec, edge cases, error handling, test coverage gaps
- FR-37: `spec-audit.md` template shall include: role definition (spec auditor), spec contents (injected), instruction to check for: ambiguity, missing edge cases, untestable requirements, unresolved dependencies, feasibility concerns, contradiction with existing codebase patterns
- FR-38: `spec-revise.md` template shall include: role definition (spec author/reviser), spec contents (injected), audit findings (injected), instruction to revise the spec addressing each finding without removing existing valid requirements
- FR-39: Templates shall use `{{PLACEHOLDER}}` syntax for variable injection: `{{SPEC}}`, `{{DIFF}}`, `{{FEEDBACK}}`, `{{BRANCH}}`, `{{SHA}}`, `{{VERDICT_SCHEMA}}`, `{{AFFECTED_SERVICES}}`
- FR-40: The review verdict JSON schema (embedded in `review.md` and `spec-audit.md`) shall match the schema defined in the design doc: `{ clean: bool, confidence: float, issues: [{ severity, file, line, description, suggestion }], summary: string, token_usage: { input, output } }`

#### Network Egress Enforcement

- FR-41a: A K8s NetworkPolicy shall be applied to all agent pods in the `nemo-jobs` namespace that blocks all egress from the agent container EXCEPT to `localhost` (127.0.0.1/32). This forces all outbound traffic (model API, HTTP, git push) through the sidecar proxies. Without this policy, the sidecar is bypassable and credential isolation is not enforced.
- FR-41b: The NetworkPolicy shall allow DNS resolution to the cluster DNS service (kube-dns on port 53) for the whole pod. K8s NetworkPolicy operates at pod level and cannot distinguish between containers in the same pod. Both containers need DNS (sidecar for proxying, agent for resolving localhost service names). DNS resolution without the ability to connect (blocked by the egress-to-localhost-only rule in FR-41a) is harmless.

#### TEST Stage

- FR-42a: For the TEST stage, the control plane shall compute the affected services by running `git diff --name-only $BASE...$SHA` and mapping changed file paths against `[services.*.path]` in `nemo.toml`. The control plane passes the result as the `AFFECTED_SERVICES` environment variable (JSON array of service names) on the Job. The agent does NOT self-report affected services.
- FR-42b: The entrypoint shall look up the test command for each affected service from `nemo.toml` (mounted as a ConfigMap at `/specs/nemo.toml`), under the `[services.<name>.test]` section
- FR-42c: The entrypoint shall run each test command, capture exit code, stdout, and stderr per service
- FR-42d: The entrypoint shall write structured test results to `/output/result.json` and stdout using the common result envelope (FR-13) with stage `"test"` and data: `{ services: [{ name, test_command, exit_code, stdout, stderr }], all_passed: bool, token_usage }`

#### Terraform Module

- FR-43: The module shall provision a Hetzner Cloud server (default type: `ccx43`, configurable via `server_type` variable)
- FR-44: The module shall install k3s (latest stable) on the provisioned server with `--disable traefik` (use nginx-ingress instead for TLS support)
- FR-45: The module shall deploy Postgres 16 as a k3s pod with a 20Gi PVC (hostPath on single-node)
- FR-46: The module shall deploy the Nemo control plane (API server + loop engine) as a k3s Deployment with 2 replicas
- FR-47: The module shall create a 100Gi PVC for the shared bare repo and deploy a CronJob that runs `git fetch --all` every 60 seconds
- FR-48: The module shall configure nginx-ingress with Let's Encrypt TLS via cert-manager
- FR-49: The module shall create a K8s Namespace `nemo-system` for control plane components and `nemo-jobs` for agent jobs
- FR-50: The module shall create K8s Secrets for each team member (SSH key + model credentials), scoped to the `nemo-jobs` namespace
- FR-51: Required input variables: `hetzner_api_token`, `domain`, `git_repo_url`, `ssh_public_keys` (for server access)
- FR-52: Optional input variables: `server_type` (default `ccx43`), `server_location` (default `fsn1`), `node_count` (default `1`, for future multi-node support), `team_members` (list of `{ name, email }`), `postgres_password`, `control_plane_image`, `agent_base_image`
- FR-53: Outputs: `control_plane_url`, `kubeconfig` (sensitive), `server_ip`, `namespace_jobs`, `namespace_system`
- FR-54: The module shall configure k3s container log rotation: 50MB max per container, 5 files retained
- FR-55: The module shall deploy a CronJob that runs `pg_dump` daily to a separate directory on the host for Postgres backup
- FR-56: The module shall create a K8s Service named `nemo-postgres` on port 5432 exposing the Postgres pod. Control plane deployments shall receive `DATABASE_URL` env var set to `postgres://nemo:$PASSWORD@nemo-postgres:5432/nemo`. The Postgres password shall be stored as a K8s Secret and injected into both the Postgres pod and the control plane Deployment via `envFrom` / `secretKeyRef`.

### Non-Functional Requirements

- NFR-1: Base agent image size shall be under 2 GB (compressed). Minimize layers; use multi-stage build for tool installation.
- NFR-2: Auth sidecar binary shall be under 15 MB (static, no runtime dependencies)
- NFR-3: Auth sidecar startup to ready shall be under 2 seconds
- NFR-4: Agent job startup (image pull excluded, from pod scheduled to entrypoint running) shall be under 10 seconds
- NFR-5: Under a load of 10 concurrent connections with 1MB/sec throughput, the egress logger shall add less than 5ms p99 latency to proxied requests
- NFR-6: `terraform apply` on a clean state shall complete in under 10 minutes
- NFR-7: Model API proxy shall not buffer request/response bodies (stream through) to support streaming model responses
- NFR-8: All sidecar logs shall be structured JSON (parseable by k3s log collection)
- NFR-9: Terraform state shall be stored locally (no remote backend for V1). The `kubeconfig` output shall be marked sensitive.
- NFR-10: The git fetch CronJob shall not block worktree creation (fetch operates on the bare repo; worktree ops take the control plane mutex, not a filesystem lock on the fetch process)

## Behavior

### Worktree Lifecycle (Control Plane Responsibility)

The control plane owns the full lifecycle of git worktrees. Before creating a K8s Job, the control plane creates the worktree (via the git module, holding the worktree mutex) at a path under the bare repo PVC. The Job's pod mounts this pre-created worktree path as `/work`. After the Job completes (success or failure), the control plane deletes the worktree (again holding the mutex). The agent never creates or deletes worktrees.

### Normal Flow: Agent Job Lifecycle

1. Control plane creates the worktree (see above), then creates a K8s Job from the template, substituting environment variables and volume mounts for the specific loop/stage/round
2. K8s schedules the pod. Both containers start. Sidecar begins listening on :9090, :9091, :9092, writes `/tmp/shared/ready` to the shared emptyDir volume
3. Agent entrypoint polls for `/tmp/shared/ready` (100ms interval, 30s timeout)
4. Entrypoint reads `$STAGE`, loads the prompt template from `/specs/.nemo/prompts/{stage}.md` (repo override) or falls back to `/etc/nemo/prompts/{stage}.md` (default)
5. Entrypoint injects variables into template (spec content, feedback, branch, SHA, etc.)
6. Entrypoint invokes the CLI tool (claude or opencode) with the assembled prompt
7. CLI tool streams model API calls through :9090 (auth injection), makes outbound HTTP calls through :9092 (egress logging), performs git operations through :9091 (SSH proxy)
8. CLI tool completes. Entrypoint parses output, writes result JSON (in the common envelope per FR-13) to both `/output/result.json` AND stdout
9. Agent container exits 0. Sidecar receives SIGTERM, drains, exits.
10. Control plane watches for Job completion, reads result from pod logs (the durable channel) BEFORE deleting the Job. Pod logs are authoritative; `/output/result.json` is for the agent's own use during execution.
11. Control plane deletes the Job and associated resources, then deletes the worktree (see Worktree Lifecycle above)

### Session Continuation Flow (Round > 1)

1. Control plane sets `SESSION_ID` to the session ID from the previous round's `result.json`
2. Control plane sets `FEEDBACK_PATH` to the path of the review feedback file (written to the session PVC)
3. Entrypoint detects `$SESSION_ID` is set, passes `--resume $SESSION_ID` (claude) or `-s $SESSION_ID` (opencode)
4. The session PVC persists session state across Job instances for the same loop

### Dockerfile.nemo Extension Flow

1. Team creates `Dockerfile.nemo` in monorepo root: `FROM ghcr.io/nemo/agent-base:latest` + project-specific toolchain installs
2. Team builds and pushes to their registry: `docker build -f Dockerfile.nemo -t registry/nemo-agent-myrepo:latest .`
3. Team sets `agent_base_image` terraform variable (or `nemo.toml` `[image]` section) to the custom image tag
4. Control plane uses the custom image for all agent jobs in that cluster

## Edge Cases

| Scenario | Expected Behavior |
|----------|-------------------|
| Sidecar fails to start within 30s | Agent entrypoint exits 1 with error "sidecar readiness timeout". Job fails. Control plane retries per failure handling policy. |
| Model API returns 429 (rate limit) | Sidecar passes the 429 through. CLI tool handles retry internally (both claude-code and opencode have built-in retry). |
| Model API returns 401 (bad credentials) | Sidecar passes the 401 through. CLI tool exits non-zero. Job fails. Control plane marks loop FAILED, notifies engineer to run `nemo auth`. |
| SSH key rejected on git push | Git push proxy returns the SSH error. Entrypoint logs the error, exits non-zero. Control plane marks loop FAILED with "git auth failure". |
| Agent container OOM-killed | K8s marks container as OOMKilled. Job fails. Control plane retries with backoff (30s, 120s). On 3rd failure, loop FAILED. |
| Egress logger port conflict | Sidecar logs error and exits. Pod restart backoff applies. Should not happen in practice (ports are hardcoded localhost-only). |
| Session PVC full | CLI tool fails to write session state. Job exits non-zero. Control plane should alert engineer. Manual cleanup required for V1. |
| Worktree volume not mounted (bare repo PVC missing) | Agent entrypoint checks for `/work` mount, exits 1 with "worktree volume not found". Job fails immediately. |
| Job exceeds activeDeadlineSeconds | K8s terminates the pod. Control plane detects DeadlineExceeded condition, treats as timeout, retries once per design doc. |
| Template variable not set (e.g., missing SPEC_PATH) | Entrypoint validates all required env vars on startup, exits 1 with list of missing vars. Fail-fast before invoking any CLI tool. |
| Repo .nemo/prompts/ has partial overrides | Entrypoint loads per-template: if `.nemo/prompts/implement.md` exists, use it; otherwise fall back to default. Each template resolved independently. |
| Terraform apply with existing server | Hetzner provider detects existing server by name, updates in place or recreates if server_type changed. Standard Terraform behavior. |
| Concurrent git fetch and worktree creation | Not a conflict. `git fetch` updates the bare repo refs. `git worktree add` creates a new worktree from a ref. The control plane mutex serializes worktree create/delete, not fetch. |
| Multiple engineers with same model provider | Each engineer's credentials stored in separate K8s Secrets (`nemo-creds-{engineer-name}`). Job mounts only the submitting engineer's Secret. |
| ImagePullBackOff | K8s cannot pull agent or sidecar image (bad credentials, missing tag, registry down). Job stays pending. Control plane detects ImagePullBackOff condition after 60s, marks loop FAILED with "image pull failure", notifies engineer. |
| Credential rotation during running jobs | `nemo auth` warns if engineer has running jobs. Sidecar reads credentials from mounted file on each request (not cached at startup), so K8s Secret volume updates propagate automatically. |

## Error Handling

| Error | Detection | Response | Recovery |
|-------|-----------|----------|----------|
| Sidecar crash mid-job | Agent gets connection refused on proxy ports | Agent CLI fails, job exits non-zero | Control plane retries job (new pod, fresh sidecar) |
| Malformed result.json | Control plane JSON parse fails | Log raw output, mark job ERRORED | Control plane retries once. If still malformed, loop FAILED. |
| Terraform apply partial failure | Terraform exits non-zero with state file | Resources may be partially created | `terraform apply` is idempotent; re-run. `terraform destroy` to clean up. |
| k3s API unreachable from control plane | Job creation fails with connection error | Control plane retries with 10s backoff, max 3 attempts | If persistent, alert (k3s down or network issue) |
| Postgres PVC full | Postgres pod restarts with disk pressure | Control plane health check detects DB connection failure | Manual: expand PVC or clean old data |
| cert-manager fails TLS | Ingress serves self-signed cert | Control plane still reachable (CLI can skip TLS verify for V1) | Check DNS, cert-manager logs. Re-run terraform apply. |
| Agent writes to bare repo directly (bug) | Should not happen (push goes through sidecar proxy, which pushes to remote) | If detected, bare repo fetch cron self-heals by resetting to remote state | Fix the bug in entrypoint |

## Out of Scope

- CI/CD pipeline for building agent images (V1 is manual `docker build && docker push`)
- Multi-node k3s (V1 is single-node; `node_count` defaults to 1 with the variable present for future multi-node)
- GPU-backed jobs (all agent work is API-bound, not local inference)
- Custom sidecar configuration per job (V1 sidecar is identical for all jobs)
- Terraform remote state backend (V1 is local state)
- Helm chart packaging (V2)
- Automatic credential rotation
- Agent image vulnerability scanning
- Web dashboard (V1 is CLI-only per design doc)

## Acceptance Criteria

- [ ] `docker build` of base agent image succeeds and image size is under 2 GB compressed
- [ ] Running `claude -p "hello" --output-format stream-json` inside the base image produces valid JSON output (with sidecar providing auth)
- [ ] Running `opencode run --format json` inside the base image produces valid JSON output (with sidecar providing auth)
- [ ] Auth sidecar binary starts in under 2s and passes readiness probe on :9093/healthz
- [ ] Auth sidecar injects correct `x-api-key` header for Anthropic API requests proxied through :9090
- [ ] Auth sidecar injects correct `Authorization: Bearer` header for OpenAI API requests proxied through :9090
- [ ] Auth sidecar git push proxy successfully pushes a commit using mounted SSH key
- [ ] Egress logger logs all outbound connections with timestamp, host, method, bytes in JSON-lines format to stdout
- [ ] Agent container has no access to files under `/secrets/` (volume not mounted)
- [ ] K8s Job with both containers starts, agent waits for sidecar readiness, executes, writes result.json, and exits cleanly
- [ ] Session continuation works: round 2 job with SESSION_ID resumes prior session state from PVC
- [ ] Prompt template variable injection produces correct prompts for all four stages
- [ ] Repo-side `.nemo/prompts/implement.md` overrides the default template when present
- [ ] Review stage produces a verdict JSON file matching the schema (validated with JSON Schema)
- [ ] `terraform init && terraform apply` provisions a working Hetzner server with k3s, Postgres, and control plane in under 10 minutes
- [ ] `terraform output control_plane_url` returns the HTTPS URL of the running control plane
- [ ] `terraform destroy` cleanly removes all resources
- [ ] Job resource limits match the table in FR-28 for each job type
- [ ] Jobs exceeding activeDeadlineSeconds are terminated by K8s
- [ ] NetworkPolicy blocks agent container egress to all destinations except localhost; direct curl from agent container to external host fails
- [ ] Agent container runs as non-root (UID 1000) with read-only root filesystem
- [ ] TEST stage reads AFFECTED_SERVICES, runs test commands from nemo.toml, and writes structured results
- [ ] Sidecar re-reads credential files on each request (credential rotation without pod restart)
- [ ] pg_dump CronJob runs daily and writes backup to host directory
- [ ] k3s log rotation configured at 50MB/5 files per container

## Open Questions

- [ ] Claude Code Max subscription auth: does `claude -p` work headless with a session token, or do we need API keys for V1? If session tokens work, what file paths does the sidecar need to mount for `~/.claude/` state?
- [ ] OpenCode binary availability: is `opencode run --format json` stable in the current release, or should we pin a specific version? What is the exact structured output flag for enforcing the verdict JSON schema?
- [ ] Session PVC sizing: how large do claude-code and opencode session files get per round? Need to size the PVC appropriately (estimate: 100MB per session, 1Gi PVC per loop should suffice).
