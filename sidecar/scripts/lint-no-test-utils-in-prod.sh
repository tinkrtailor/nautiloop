#!/usr/bin/env bash
# sidecar/scripts/lint-no-test-utils-in-prod.sh
#
# Two hard-failing checks in CI:
#
# 1. No CI workflow step may reference the sidecar's internal
#    `__test_utils` feature (or its old `test-utils` spelling) on a
#    RELEASE / publish step. The feature re-enables the SSH SSRF
#    bypass path that integration tests rely on and MUST NEVER be
#    enabled in a release build.
#
#    The `rust-checks-with-test-utils` job in ci.yml legitimately
#    uses the feature — it runs `cargo test` / `cargo clippy`, not
#    `cargo build --release`. To distinguish, we match only the
#    feature combined with cargo build / install / release run.
#
# 2. (FR-28) The "extra CA bundle" escape-hatch env var used by the
#    Rust sidecar's TLS layer must only appear in the two files
#    named by SR-5:
#       - sidecar/tests/parity/docker-compose.yml
#       - .github/workflows/parity.yml
#    Any other reference inside a file type that could actually
#    propagate it into a running container (Dockerfile / YAML /
#    shell / .env / Terraform / HCL) is a lint failure.
#
#    The env var name is intentionally built from fragments below so
#    this script itself does not contain the literal token as a
#    single contiguous byte run — `git grep` will not match this
#    file even though it describes the pattern.
#
#    File-type scoping (runtime-propagation shapes only) keeps
#    documentation and reader code like `sidecar/src/tls.rs` out of
#    the check: a `.rs` file cannot set an env var inside a
#    container, so mere textual mention is not a security risk.

set -euo pipefail

WORKFLOWS=".github/workflows"

# ---- Check 1: __test_utils never referenced in release/build paths ----

if [ -d "$WORKFLOWS" ]; then
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

# ---- Check 2: FR-28 extra-CA-bundle env var allowlist ---------------
#
# The env var name is assembled from two fragments so the literal
# token never appears contiguously inside this script. `git grep` for
# the full name will not match this file.
CA_BUNDLE_ENV_PREFIX="NAUTILOOP_EXTRA"
CA_BUNDLE_ENV_SUFFIX="CA_BUNDLE"
CA_BUNDLE_ENV="${CA_BUNDLE_ENV_PREFIX}_${CA_BUNDLE_ENV_SUFFIX}"

# File-type scope: only files that could actually propagate an env
# var into a container at runtime. Rust source and markdown are
# intentionally NOT in scope because they cannot set env vars inside
# a running container (the Rust reader in sidecar/src/tls.rs only
# READS the var, which is fine).
SCOPED_PATHSPECS=(
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

# Exactly two file exclusions per SR-5 / FR-28.
ALLOWED_PATHS=(
  ':!sidecar/tests/parity/docker-compose.yml'
  ':!.github/workflows/parity.yml'
)

if git rev-parse --git-dir > /dev/null 2>&1; then
  # `-n` so the error output names line numbers. We match ANY
  # reference to the env var NAME within scoped files. A negative
  # lookahead at the start of the line excludes full-line comments
  # in YAML / Dockerfile / shell (lines whose first non-whitespace
  # character is `#`) so that pure documentation comments in
  # production files do not trip the lint. Any non-comment line
  # that mentions the var — assignment, export, interpolation,
  # heredoc, whatever — is flagged.
  CA_BUNDLE_PATTERN="^(?![[:space:]]*#).*${CA_BUNDLE_ENV}"
  MATCHES=$(
    git grep -nP "$CA_BUNDLE_PATTERN" \
      -- "${SCOPED_PATHSPECS[@]}" "${ALLOWED_PATHS[@]}" \
      2>/dev/null || true
  )
  if [ -n "$MATCHES" ]; then
    echo "ERROR: ${CA_BUNDLE_ENV} referenced outside the FR-28 allowlist:"
    echo "$MATCHES"
    echo ""
    echo "The only files allowed to reference this env var (per SR-5) are:"
    echo "  - sidecar/tests/parity/docker-compose.yml"
    echo "  - .github/workflows/parity.yml"
    echo ""
    echo "Full-line comments in YAML / Dockerfile / shell are allowed,"
    echo "but any non-comment reference inside a runtime-propagation"
    echo "file type (Dockerfile / YAML / shell / .env / terraform / HCL)"
    echo "is a FR-28 violation. Rust source and markdown docs are out"
    echo "of scope because they cannot set env vars in a container."
    exit 1
  fi
else
  echo "Not inside a git repo; skipping ${CA_BUNDLE_ENV} reference check"
fi

echo "OK: no __test_utils feature references in release CI workflows"
echo "OK: ${CA_BUNDLE_ENV} references are within the FR-28 allowlist"
