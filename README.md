# Nemo

A convergent, multi-model adversarial build system. Push a spec, get a clean PR.

```
nemo start spec.md       # implement, create PR
nemo ship spec.md        # implement + auto-merge
nemo harden spec.md      # harden spec, merge spec PR
```

## What it does

You write a spec. Nemo runs a convergent loop: **Claude implements**, **OpenAI reviews**. If the reviewer finds issues, the implementer fixes them. The loop runs until the reviewer finds nothing wrong. Then you get a PR.

Your agents keep working when you stop.

## How it works

```
Engineer's Mac                    Hetzner k3s Cluster
┌──────────┐                      ┌─────────────────────────────┐
│ nemo CLI │──── HTTPS ──────────>│ API Server (axum)           │
│          │                      │ Loop Engine (reconciler)    │
│ nemo start spec.md              │ Postgres                    │
│ nemo status                     │                             │
│ nemo ship spec.md               │ ┌─────────┐ ┌─────────┐    │
│                                 │ │Implement│ │ Review  │    │
│                                 │ │  Job    │ │  Job    │    │
│                                 │ │(Claude) │ │(OpenAI) │    │
│                                 │ └─────────┘ └─────────┘    │
└──────────┘                      └─────────────────────────────┘
```

The convergent loop:

```
Round 1: Claude implements → tests run → OpenAI reviews → issues found
Round 2: Claude fixes issues → tests run → OpenAI reviews → issues found
Round 3: Claude fixes issues → tests run → OpenAI reviews → CLEAN
→ PR created
```

Cross-model adversarial review: Claude never reviews its own work. A different model (OpenAI) does the review. Different models have different blind spots. This catches more bugs than single-model review.

## Architecture

- **Control plane** (Rust): API server + loop engine, deployed as two k3s Deployments
- **CLI** (Rust): `nemo` binary, runs on your machine
- **Agent jobs** (K8s Jobs): each stage runs as a separate pod with an auth sidecar
- **Auth sidecar** (Go): proxies model API calls (injects auth), proxies git push (SSH key), logs all egress
- **Terraform**: provisions Hetzner VPS, k3s, Postgres, control plane, bare repo

## Three verbs

| Command | What happens | Terminal state |
|---------|-------------|----------------|
| `nemo harden spec.md` | Adversarial spec review loop | HARDENED |
| `nemo start spec.md` | Implement → test → review loop, PR | CONVERGED |
| `nemo ship spec.md` | Same loop + auto-merge on convergence | SHIPPED |

Add `--harden` to `start` or `ship` to run spec hardening first.

## Prerequisites

- [1Password CLI](https://developer.1password.com/docs/cli) (`op`) — all secrets managed via 1Password
- [Terraform](https://developer.hashicorp.com/terraform/install) >= 1.5
- [Docker](https://docs.docker.com/get-docker/) with buildx
- A "Nemo" vault in 1Password with these items:

| 1Password Item | Fields | Purpose |
|----------------|--------|---------|
| `hetzner-cloud` | `credential` | Hetzner Cloud API token |
| `nemo-domain` | `domain`, `email` | Control plane domain + ACME email |
| `nemo-repo` | `ssh_url` | Git repo URL (SSH format) |
| `github-pat` | `credential` | GitHub PAT for PR creation/merge |
| `nemo-deploy-key` | `private_key` | SSH deploy key for repo access |
| `github-registry` | `username`, `pat` | GHCR credentials for image push/pull |
| `ssh-public-key` | `public_key` | SSH public key for server access |

## Deploy

### 1. Build and push images

```bash
./build-images.sh --tag 0.1.0
```

Builds 3 images (control-plane, agent-base, sidecar), pushes to GHCR. Authenticates via 1Password automatically.

Options: `--no-push` (local only), `--only control-plane` (single image), `--platform linux/arm64` (override arch).

### 2. Provision the cluster

```bash
cd terraform
op run --env-file=.env.1password -- terraform init
op run --env-file=.env.1password -- terraform apply
```

This provisions a Hetzner VPS, installs k3s with Traefik, deploys Postgres, the control plane (API + loop engine), and initializes the bare repo. Takes ~5 minutes on first run.

### 3. Set up each engineer

```bash
cd ~/your-monorepo
nemo init                    # generates nemo.toml
nemo auth                    # pushes credentials (Claude, OpenAI, SSH) to cluster
```

### 4. Use it

```bash
nemo start spec.md           # PR appears when done
nemo ship spec.md            # implement + auto-merge on convergence
nemo harden spec.md          # harden spec before implementing
nemo status                  # check progress
nemo logs <loop_id>          # stream job logs
```

### Teardown (save money)

```bash
cd terraform
op run --env-file=.env.1password -- terraform destroy
```

Destroys the server but keeps the Hetzner volume (Postgres data persists). Next `terraform apply` reattaches the same volume — no data loss.

### Update (new images)

```bash
./build-images.sh --tag 0.2.0
cd terraform
op run --env-file=.env.1password -- terraform apply \
  -var="control_plane_image=ghcr.io/tinkrtailor/nemo-control-plane:0.2.0" \
  -var="agent_base_image=ghcr.io/tinkrtailor/nemo-agent-base:0.2.0" \
  -var="sidecar_image=ghcr.io/tinkrtailor/nemo-sidecar:0.2.0"
```

All three images must be updated together to avoid version skew. The build script enforces this by only tagging `:latest` when building all images (not with `--only`).

## Configuration

Three layers, each overriding the previous:

```toml
# nemo.toml (repo root, checked in)
[repo]
name = "my-project"
default_branch = "main"

[models]
implementor = "claude-opus-4"
reviewer = "gpt-5.4"

[services.api]
path = "api/"
test = "cd api && cargo test"

[services.web]
path = "web/"
test = "cd web && npm test"
```

```toml
# ~/.nemo/config.toml (per engineer)
[identity]
name = "Alice"
email = "alice@example.com"

server_url = "https://nemo.internal"
api_key = "your-api-key"
```

## Security

- **Auth sidecar**: model credentials and SSH keys never touch the agent container
- **Egress logging**: all outbound traffic from agent pods is logged
- **Read-only reviewer**: review stage mounts the worktree read-only
- **Per-engineer credentials**: each engineer's subscriptions are scoped to their jobs

## Convergence data

Built through the exact process it automates:

| Lane | Rounds | Findings | Spec hardening |
|------|--------|----------|----------------|
| A (core loop) | 28 | 124 | 2 rounds |
| B (infrastructure) | 25 | 88 | 11 rounds |
| C (agent runtime) | 21 | 107 | 11 rounds |
| Integration | 7 | 12 | — |
| **Total** | **81** | **331** | — |

331 production bugs caught by cross-model adversarial review. The exit condition is quality, not iteration count.

## Project structure

```
control-plane/          Rust: API server + loop engine
  src/api/              REST endpoints (axum)
  src/loop_engine/      Convergent loop driver + reconciler
  src/state/            Postgres state store
  src/git/              Git operations (worktrees, branches, PRs)
  src/k8s/              K8s job builder + client
  src/config/           Three-layer config loading
  migrations/           SQL migrations

cli/                    Rust: nemo CLI binary
  src/commands/         start, ship, harden, status, auth, init, etc.

images/
  base/                 Agent base Docker image + entrypoint
  sidecar/              Auth sidecar (Go)

terraform/              Hetzner + k3s + Postgres + control plane

.nemo/prompts/          Default prompt templates (implement, review, etc.)

specs/                  Hardened specs (Lane A, B, C, V2 DAG)
docs/                   Design doc, architecture diagrams, convergence learnings
```

## License

Apache 2.0

## Status

V1 — built, adversarially reviewed (331 findings, 81 rounds), ready for deployment.
