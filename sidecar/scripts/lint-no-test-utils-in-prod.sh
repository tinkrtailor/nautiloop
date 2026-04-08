#!/usr/bin/env bash
# sidecar/scripts/lint-no-test-utils-in-prod.sh
#
# Two checks run here, both hard failures in CI:
#
# 1. No CI workflow step may reference the sidecar's internal
#    `__test_utils` feature (or its old `test-utils` spelling) on a
#    RELEASE / publish step. The feature re-enables the SSH SSRF
#    bypass path that integration tests rely on and MUST NEVER be
#    enabled in a release build.
#
#    The `rust-checks-with-test-utils` job in ci.yml legitimately
#    uses the feature — it runs `cargo test`, not `cargo build
#    --release`. To distinguish, we match only `--features
#    ...__test_utils` combined with the `cargo build` / `cargo
#    install` / `cargo run` (no `--release` filter logic — we
#    simply require the feature never appears on a cargo
#    build/install invocation).
#
#    This is enforced by a more specific pattern than the v1
#    script, scoped to release-ish cargo subcommands.
#
# 2. (FR-28) `NAUTILOOP_EXTRA_CA_BUNDLE` MUST appear ONLY in:
#       - sidecar/tests/parity/docker-compose.yml
#       - .github/workflows/parity.yml
#    Any other reference is a security lint failure because the env
#    var is the test-only escape hatch that lets the Rust sidecar
#    trust the harness test CA. Production images MUST NOT set it.
set -euo pipefail

WORKFLOWS=".github/workflows"

# ---- Check 1: __test_utils never referenced in release/build paths ----

if [ -d "$WORKFLOWS" ]; then
  # Match `cargo build ... --features ... (__)?test[-_]utils` AND
  # `cargo install ...`. `cargo test` and `cargo clippy --all-targets`
  # with the feature are allowed because they don't produce a
  # release binary.
  BAD_PATTERN='cargo[[:space:]]+(build|install|run[[:space:]]+--release)([^#\n]*?)--features[[:space:]]*[^[:space:]]*(__)?test[-_]utils'

  if grep -rPzln --include='*.yml' --include='*.yaml' "$BAD_PATTERN" "$WORKFLOWS" 2>/dev/null; then
    echo "ERROR: CI workflows build/install/release-run with the internal __test_utils feature."
    echo "This feature is test-only and must NOT be enabled in release builds."
    echo "See sidecar/Cargo.toml [features] for the rationale."
    exit 1
  fi
else
  echo "No $WORKFLOWS directory; skipping __test_utils check"
fi

# ---- Check 2: NAUTILOOP_EXTRA_CA_BUNDLE assignment allowlist (FR-28) ----
#
# This env var is the test-only escape hatch that lets the Rust
# sidecar trust an extra CA bundle at runtime. Production images
# MUST NOT set it. The FR-28 allowlist narrows the files allowed
# to ACTUALLY SET the variable (ENV, environment:, export=, etc.)
# to exactly two:
#
#   - sidecar/tests/parity/docker-compose.yml
#   - .github/workflows/parity.yml
#
# Mere textual mentions in comments, docs, and the reader code
# (sidecar/src/tls.rs) are fine. We match only the assignment
# shapes that actually cause the env var to propagate into a
# container at runtime:
#
#   - `NAUTILOOP_EXTRA_CA_BUNDLE:` (YAML key inside an
#     `environment:` or `env:` block)
#   - `NAUTILOOP_EXTRA_CA_BUNDLE=` (shell assignment or Dockerfile
#     `ENV NAUTILOOP_EXTRA_CA_BUNDLE=...`)
#   - `ENV[[:space:]]+NAUTILOOP_EXTRA_CA_BUNDLE` (Dockerfile
#     space-separated form)
#
# Anything matching those patterns outside the allowlist is a
# hard failure. Plain occurrences of the name in comments,
# documentation, and the Rust reader are allowed.

ASSIGN_PATTERN='(NAUTILOOP_EXTRA_CA_BUNDLE[:=]|ENV[[:space:]]+NAUTILOOP_EXTRA_CA_BUNDLE)'

# Only check files where an assignment actually propagates the
# variable to a running process or image. Markdown/docstrings/error
# message strings in .rs files are NOT assignments and are allowed
# to mention the name textually. The real danger is a Dockerfile,
# compose file, shell script, CI workflow, or k8s manifest that
# puts the variable into a production container.
#
# File-type filter passed to git-grep. Each entry is a pathspec
# that INCLUDES files of that shape anywhere in the tree.
ASSIGN_CHECK_FILES=(
  'Dockerfile'
  '*.Dockerfile'
  'Dockerfile.*'
  '*.yml'
  '*.yaml'
  '*.sh'
  '.env'
  '.env.*'
  '*.tf'
  '*.hcl'
)

ALLOWED_ASSIGN=(
  ':!sidecar/tests/parity/docker-compose.yml'
  ':!.github/workflows/parity.yml'
  # The lint script itself shows the pattern in comments. Exclude
  # it from the assignment check too.
  ':!sidecar/scripts/lint-no-test-utils-in-prod.sh'
)

if git rev-parse --git-dir > /dev/null 2>&1; then
  MATCHES=$(
    git grep -En "$ASSIGN_PATTERN" \
      -- "${ASSIGN_CHECK_FILES[@]}" "${ALLOWED_ASSIGN[@]}" \
      2>/dev/null || true
  )
  if [ -n "$MATCHES" ]; then
    echo "ERROR: NAUTILOOP_EXTRA_CA_BUNDLE assigned outside the FR-28 allowlist:"
    echo "$MATCHES"
    echo ""
    echo "The only files allowed to ASSIGN this env var are:"
    echo "  - sidecar/tests/parity/docker-compose.yml"
    echo "  - .github/workflows/parity.yml"
    echo ""
    echo "Textual mentions in comments / docs / reader code are fine;"
    echo "the lint only catches actual assignments in Dockerfiles,"
    echo "compose/yaml, shell scripts, terraform, and env files."
    exit 1
  fi
else
  echo "Not inside a git repo; skipping NAUTILOOP_EXTRA_CA_BUNDLE assignment check"
fi

echo "OK: no __test_utils feature references in release CI workflows"
echo "OK: NAUTILOOP_EXTRA_CA_BUNDLE references are within the FR-28 allowlist"
