//! Binary entry point for the Nautiloop auth-sidecar parity harness.
//!
//! Implements the flow described in `specs/sidecar-parity-harness.md`
//! "Driver program structure" section:
//!
//! 1. Parse CLI args (FR-20).
//! 2. Resolve + validate the CGNAT subnet against the FR-29 whitelist.
//! 3. Load the corpus and apply `--category` / `--case` filters.
//! 4. Bring up the docker compose stack (FR-16 / FR-17).
//! 5. Wait for mock + sidecar readiness (FR-17 points 3-4).
//! 6. Run each case, print progress (FR-18).
//! 7. Run `order_hint: "last"` cases after everything else.
//! 8. Summarize, dump artifact log (NFR-9), return non-zero on failure.
//! 9. Drop guard always tears down on panic (NFR-7).
//!
//! This binary never modifies sidecar source. It just talks to running
//! containers through the published host ports and the mock
//! introspection API.

mod args;
mod compose;
mod container_logs;
mod corpus;
mod diff;
mod health_probe;
mod introspection;
mod normalize;
mod report;
mod result;
mod runner;
mod subnet;
mod tls_client;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;

use crate::args::Args;
use crate::compose::{ComposeGuard, ComposeStack, SIDECAR_GO_SERVICE, SIDECAR_RUST_SERVICE};
use crate::corpus::{Category, CorpusCase, load_corpus, partition_by_order_hint};
use crate::normalize::normalize;
use crate::report::{dump_run_log, print_case_result, print_summary};
use crate::result::{CaseOutcome, RunSummary, SideOutput};
use crate::runner::{CaseAssertion, CaseExecution, RunnerContext};

/// Grace period between a runner returning and the per-case sidecar
/// container log snapshot via `docker compose logs --since ...`.
///
/// Best-effort observability bound: long enough to capture log
/// lines emitted by typical response-path finalizers in both
/// Rust's tracing/JSON emit path and Go's unbuffered
/// `fmt.Println`, but NOT a race-free guarantee. Any log line
/// emitted after the snapshot is silently absent.
///
/// Container logs do not participate in the parity diff — this
/// grace only affects post-hoc debuggability, never pass/fail.
const CONTAINER_LOG_GRACE: Duration = Duration::from_millis(500);

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    // FR-29: resolve + validate subnet, then export to env for compose.
    let subnet = subnet::resolve_and_validate(args.subnet.as_deref())
        .context("subnet whitelist validation failed (FR-29)")?;
    // SAFETY: `set_var` is only called here, before any child process
    // spawn, and no other thread in this binary writes to environment.
    unsafe {
        std::env::set_var(subnet::SUBNET_ENV_VAR, &subnet);
    }
    tracing::info!(%subnet, "resolved PARITY_NET_SUBNET");

    // Resolve paths relative to the harness crate directory. This
    // binary is expected to be invoked from the repo root OR from the
    // parity harness dir; in either case the corpus + compose paths
    // should resolve to real files.
    let harness_dir = resolve_harness_dir();
    let corpus_path = harness_dir.join(&args.corpus_dir);
    let compose_path = harness_dir.join(&args.compose_file);

    let cases = load_corpus(&corpus_path)
        .with_context(|| format!("loading corpus from {}", corpus_path.display()))?;
    let filtered: Vec<CorpusCase> = cases
        .into_iter()
        .filter(|c| {
            if let Some(cat) = &args.category
                && c.category.as_str() != cat
            {
                return false;
            }
            if let Some(name) = &args.case
                && &c.name != name
            {
                return false;
            }
            true
        })
        .collect();
    if filtered.is_empty() {
        anyhow::bail!(
            "no corpus cases matched --category={:?} --case={:?}",
            args.category,
            args.case
        );
    }
    tracing::info!(count = filtered.len(), "loaded corpus cases");

    // FR-17 step 1-4: compose build/up + health gate.
    let compose = ComposeStack::new(&compose_path, &harness_dir);
    if !args.no_rebuild {
        tracing::info!("running docker compose build");
        compose
            .build()
            .await
            .context("docker compose build failed")?;
    } else {
        tracing::info!("--no-rebuild set; skipping docker compose build");
    }
    tracing::info!("running docker compose up -d");
    compose.up().await.context("docker compose up failed")?;
    let guard = ComposeGuard::new(compose.clone());

    health_probe::wait_mock_health(Duration::from_secs(60))
        .await
        .context("mock services did not become healthy in 60s (FR-17 step 3)")?;
    health_probe::wait_sidecar_ready(Duration::from_secs(30))
        .await
        .context("sidecars did not become ready in 30s (FR-17 step 4)")?;

    // Build the shared context for each runner.
    let test_ca_path = harness_dir.join("fixtures/test-ca/ca.pem");
    let harness_tls = tls_client::build_harness_client_config(&test_ca_path)
        .with_context(|| format!("loading harness test CA from {}", test_ca_path.display()))?;
    let ssh_key_path = harness_dir.join("fixtures/go-secrets/ssh-key/id_ed25519");
    let ctx = RunnerContext {
        harness_dir: harness_dir.clone(),
        harness_tls,
        ssh_key_path,
    };

    // Partition into (normal, last) per FR-22 order_hint.
    let (rest, last) = partition_by_order_hint(&filtered);

    let mut outcomes: Vec<CaseOutcome> = Vec::with_capacity(filtered.len());
    for case in rest {
        let outcome = run_case(case, &ctx, &compose).await;
        print_case_result(&outcome);
        outcomes.push(outcome);
    }
    for case in last {
        let outcome = run_case(case, &ctx, &compose).await;
        print_case_result(&outcome);
        outcomes.push(outcome);
    }

    let summary = RunSummary::from_outcomes(&outcomes);
    print_summary(&summary);

    // NFR-9: always dump the run log.
    let log_path = harness_dir.join("harness-run.log");
    let docker_logs = compose.logs().await.unwrap_or_default();
    dump_run_log(&log_path, &summary, &outcomes, &docker_logs)
        .context("writing harness-run.log")?;

    // FR-17 step 6: if --stop OR all tests passed, tear down.
    if args.stop || summary.all_passed() {
        tracing::info!("tearing down docker compose stack");
        drop(guard);
    } else {
        tracing::warn!("leaving docker compose stack up for inspection (use --stop to override)");
        guard.disarm();
    }

    if !summary.all_passed() {
        std::process::exit(1);
    }
    Ok(())
}

/// Run a single case, catching any runner error and turning it into a
/// failed outcome so the run never aborts mid-corpus.
///
/// Per-case flow:
///
/// 1. Record the wall-clock start time so docker logs can be fetched
///    with `--since <rfc3339>`.
/// 2. Dispatch to the category runner, which returns a
///    [`CaseExecution`] containing the two side outputs plus an
///    optional explicit assertion.
/// 3. Attach the per-case sidecar container logs (finding #5) to
///    each side BEFORE normalization.
/// 4. Normalize both sides (strips dynamic fields per FR-19,
///    including the new container-log timestamp).
/// 5. Diff the two sides.
/// 6. Evaluate pass/fail:
///    - Parity cases: diff must be empty AND (no assertion OR the
///      assertion passed).
///    - Divergence cases: the runner MUST have produced an
///      assertion, and that assertion MUST pass. The prior "any
///      diff = pass" rule was removed because it masked real
///      regressions.
async fn run_case(case: &CorpusCase, ctx: &RunnerContext, compose: &ComposeStack) -> CaseOutcome {
    let start = Instant::now();
    let since = chrono::Utc::now().to_rfc3339();

    match runner::dispatch(case, ctx).await {
        Ok(execution) => {
            let CaseExecution {
                mut go,
                mut rust,
                assertion,
            } = execution;

            // Finding #5: attach per-case sidecar container logs.
            //
            // NOTE — best-effort observability, NOT a gating signal.
            // `docker compose logs --since <case-start>` is a
            // snapshot: any log line the sidecar emits AFTER this
            // snapshot call is silently absent from the captured
            // `container_logs`. We wait [`CONTAINER_LOG_GRACE`]
            // before snapshotting so the typical "finalizer writes
            // one more line after the response flushes" pattern is
            // captured. This grace is long enough for both Rust's
            // tracing/JSON emit path and Go's `fmt.Println` (both
            // unbuffered) but it is NOT race-free for pathological
            // cases that defer log emission by hundreds of ms.
            //
            // Container logs DO NOT participate in the parity diff
            // (see `diff.rs` doc comment) — they are dumped per
            // case in `harness-run.log` for post-hoc debugging only.
            // A dropped late log line therefore cannot cause a
            // false-positive pass/fail, only reduce the debuggability
            // of an already-failing case.
            tokio::time::sleep(CONTAINER_LOG_GRACE).await;
            tracing::debug!(
                grace_ms = CONTAINER_LOG_GRACE.as_millis() as u64,
                case = %case.name,
                "snapshotting per-case sidecar container logs (best-effort)"
            );
            let go_raw_logs = compose
                .logs_for_service_since(SIDECAR_GO_SERVICE, &since)
                .await;
            let rust_raw_logs = compose
                .logs_for_service_since(SIDECAR_RUST_SERVICE, &since)
                .await;
            go.container_logs = container_logs::parse_docker_logs(&go_raw_logs);
            rust.container_logs = container_logs::parse_docker_logs(&rust_raw_logs);

            normalize(&mut go, &case.normalize);
            normalize(&mut rust, &case.normalize);
            let diff = diff::diff_sides(&go, &rust);
            let expected_parity = case.expected_parity;
            let (passed, failure_detail) =
                evaluate_outcome(expected_parity, &diff, assertion.as_ref());
            if passed {
                CaseOutcome::pass(
                    &case.name,
                    case.path.to_string_lossy(),
                    expected_parity,
                    go,
                    rust,
                    start.elapsed(),
                    assertion_or_divergence_note(case, assertion.as_ref()),
                )
            } else {
                CaseOutcome::fail(
                    &case.name,
                    case.path.to_string_lossy(),
                    expected_parity,
                    go,
                    rust,
                    start.elapsed(),
                    failure_detail,
                )
            }
        }
        Err(e) => CaseOutcome::fail(
            &case.name,
            case.path.to_string_lossy(),
            case.expected_parity,
            SideOutput::default(),
            SideOutput::default(),
            start.elapsed(),
            format!("runner error: {e:?}"),
        ),
    }
}

/// Pure pass/fail evaluator. Extracted so the combined matrix of
/// (expected_parity × assertion presence × diff emptiness) can be
/// unit tested without spinning up docker or running a runner.
///
/// Returns `(passed, failure_detail)`. `failure_detail` is only
/// meaningful when `passed == false`.
fn evaluate_outcome(
    expected_parity: bool,
    diff: &str,
    assertion: Option<&CaseAssertion>,
) -> (bool, String) {
    if expected_parity {
        // Parity case: diff must be empty AND any assertion must pass.
        let diff_ok = diff.is_empty();
        let assertion_ok = assertion.map(|a| a.passed).unwrap_or(true);
        if diff_ok && assertion_ok {
            return (true, String::new());
        }
        let mut detail = String::new();
        if !diff_ok {
            detail.push_str(diff);
        }
        if let Some(a) = assertion
            && !a.passed
        {
            if !detail.is_empty() {
                detail.push('\n');
            }
            detail.push_str("assertion failed: ");
            detail.push_str(&a.detail);
        }
        (false, detail)
    } else {
        // Divergence case: assertion is REQUIRED.
        match assertion {
            Some(a) if a.passed => (true, String::new()),
            Some(a) => (
                false,
                format!("divergence assertion failed: {}", a.detail),
            ),
            None => (
                false,
                "divergence case produced no assertion; runner must encode the expected directional property explicitly"
                    .to_string(),
            ),
        }
    }
}

/// Format the pass-path note for a case. Divergence cases get the
/// descriptor text from the corpus plus the runner's assertion
/// detail; parity cases with a passing assertion get the detail;
/// simple parity cases get empty.
fn assertion_or_divergence_note(case: &CorpusCase, assertion: Option<&CaseAssertion>) -> String {
    match (&case.category, &case.divergence, assertion) {
        (Category::Divergence, Some(d), Some(a)) => format!(
            "divergence: {} | go={} | rust={} | verified: {}",
            d.description, d.go_expected, d.rust_expected, a.detail
        ),
        (Category::Divergence, Some(d), None) => format!(
            "divergence: {} | go={} | rust={}",
            d.description, d.go_expected, d.rust_expected
        ),
        (_, _, Some(a)) if a.passed => format!("assertion: {}", a.detail),
        _ => String::new(),
    }
}

/// Resolve the harness directory — the folder containing
/// `Cargo.toml` for this crate. This is `env!("CARGO_MANIFEST_DIR")`
/// at compile time so the binary can be invoked from any working
/// directory and still find its fixtures.
fn resolve_harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parity_case_passes_with_empty_diff_and_no_assertion() {
        let (passed, detail) = evaluate_outcome(true, "", None);
        assert!(passed);
        assert!(detail.is_empty());
    }

    #[test]
    fn parity_case_passes_with_empty_diff_and_passing_assertion() {
        let a = CaseAssertion::pass("credential refreshed");
        let (passed, _) = evaluate_outcome(true, "", Some(&a));
        assert!(passed);
    }

    #[test]
    fn parity_case_fails_when_diff_not_empty() {
        let (passed, detail) = evaluate_outcome(true, "http_status: go=200, rust=403", None);
        assert!(!passed);
        assert!(detail.contains("http_status"));
    }

    #[test]
    fn parity_case_fails_when_assertion_fails_even_with_empty_diff() {
        // The credential refresh case: both sides agree on response
        // body (parity diff is empty) but neither reloaded the
        // credential file (assertion fails). Must fail.
        let a = CaseAssertion::fail("neither side upstreamed new bearer");
        let (passed, detail) = evaluate_outcome(true, "", Some(&a));
        assert!(!passed);
        assert!(detail.contains("assertion failed"));
        assert!(detail.contains("neither side"));
    }

    #[test]
    fn parity_case_fails_with_combined_detail_when_both_diff_and_assertion_fail() {
        let a = CaseAssertion::fail("assertion detail");
        let (passed, detail) = evaluate_outcome(true, "http_status: go=200, rust=500", Some(&a));
        assert!(!passed);
        assert!(detail.contains("http_status"));
        assert!(detail.contains("assertion detail"));
    }

    #[test]
    fn divergence_case_passes_when_assertion_passes() {
        let a = CaseAssertion::pass("rust streamed, go buffered");
        let (passed, _) = evaluate_outcome(false, "", Some(&a));
        assert!(passed);
    }

    #[test]
    fn divergence_case_passes_regardless_of_diff_when_assertion_passes() {
        // A divergence case does not gate on the diff engine — the
        // assertion is the only gate. Even with a large diff string
        // the case passes if the assertion holds.
        let a = CaseAssertion::pass("rust streamed, go buffered");
        let (passed, _) = evaluate_outcome(false, "http_status: go=200, rust=500", Some(&a));
        assert!(passed);
    }

    #[test]
    fn divergence_case_fails_when_assertion_fails() {
        let a = CaseAssertion::fail("rust is buffered, expected streaming");
        let (passed, detail) = evaluate_outcome(false, "http_body differs", Some(&a));
        assert!(!passed);
        assert!(detail.contains("divergence assertion failed"));
        assert!(detail.contains("rust is buffered"));
    }

    #[test]
    fn divergence_case_fails_when_no_assertion_is_produced() {
        // Regression guard for the P1 finding: the prior "any diff
        // = pass" rule would have marked this case as passing. With
        // the assertion contract, it must fail.
        let (passed, detail) =
            evaluate_outcome(false, "http_body differs\nhttp_header only on go", None);
        assert!(!passed);
        assert!(detail.contains("no assertion"));
    }
}
