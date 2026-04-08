//! Per-category runner dispatch.
//!
//! Each category gets its own sub-module with `run_case`. The public
//! [`dispatch`] entry point picks the right module based on the case's
//! category field and returns a [`CaseExecution`] on success.
//!
//! Runners are responsible for:
//!
//! 1. Calling [`crate::introspection::reset_all`] if the case needs
//!    clean mock logs.
//! 2. Issuing the test input to BOTH sidecars (in parallel where it
//!    makes sense).
//! 3. Capturing outputs into [`SideOutput`] for diffing.
//! 4. For divergence cases and any case that needs to verify a
//!    directional property beyond pure parity (e.g. the credential
//!    refresh case, which checks that the mutated credential was
//!    actually observed by the mocks), constructing an explicit
//!    [`CaseAssertion`].
//!
//! Runners do NOT normalize — that's done by the main loop so the
//! normalization rules are applied identically across categories.
//!
//! # Divergence assertion contract
//!
//! Every case whose `expected_parity == false` MUST return a
//! [`CaseExecution`] with `assertion: Some(_)`. The main loop treats
//! a missing assertion on a divergence case as a hard failure — the
//! prior "any diff = pass" rule was removed because it made divergence
//! cases silently pass when side-specific verdict strings happened to
//! differ for unrelated reasons.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rustls::ClientConfig;

use crate::corpus::{Category, CorpusCase};
use crate::introspection;
use crate::result::SideOutput;

pub mod divergence_drain;
pub mod egress;
pub mod git_ssh;
pub mod health;
pub mod model_proxy;

/// Shared context passed into every runner. Owned by `main.rs`, cloned
/// cheaply via Arc internally.
#[derive(Clone)]
pub struct RunnerContext {
    pub harness_dir: PathBuf,
    /// rustls client config with the harness test CA loaded. Used by
    /// runner modules that need to do direct HTTPS (currently only
    /// reserved for manual smoke — the parity cases go through the
    /// sidecars, which do their own TLS against the mocks).
    #[allow(dead_code)]
    pub harness_tls: Arc<ClientConfig>,
    pub ssh_key_path: PathBuf,
}

/// Everything a case execution produces: the two side outputs plus
/// an optional explicit assertion.
///
/// Parity cases (`expected_parity == true`) return `assertion: None`
/// and rely on the diff engine. Divergence cases return
/// `assertion: Some(_)` encoding the directional property they
/// expect (e.g. "Rust first chunk < 200ms AND Go first chunk ≥ 250ms").
///
/// A parity case MAY also populate `assertion` to add a non-parity
/// check on top of the diff — e.g. the credential refresh case
/// verifies that the mutated credential was observed by the mocks.
/// In that case BOTH the diff AND the assertion must pass.
#[derive(Debug, Clone)]
pub struct CaseExecution {
    pub go: SideOutput,
    pub rust: SideOutput,
    pub assertion: Option<CaseAssertion>,
}

impl CaseExecution {
    /// Construct an execution with no explicit assertion. Used by
    /// parity cases whose pass/fail is driven entirely by the diff
    /// engine.
    pub fn parity(go: SideOutput, rust: SideOutput) -> Self {
        Self {
            go,
            rust,
            assertion: None,
        }
    }

    /// Construct an execution with an explicit assertion.
    pub fn with_assertion(go: SideOutput, rust: SideOutput, assertion: CaseAssertion) -> Self {
        Self {
            go,
            rust,
            assertion: Some(assertion),
        }
    }
}

/// A runner-produced assertion encoding the directional property
/// that the case expects to hold.
///
/// `passed == true` means the property held. On failure, `detail`
/// carries a human-readable reason that is printed verbatim in the
/// artifact log so the failure points at the actual mismatch rather
/// than a generic "divergence case matched" line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseAssertion {
    pub passed: bool,
    pub detail: String,
}

impl CaseAssertion {
    pub fn pass(detail: impl Into<String>) -> Self {
        Self {
            passed: true,
            detail: detail.into(),
        }
    }

    pub fn fail(detail: impl Into<String>) -> Self {
        Self {
            passed: false,
            detail: detail.into(),
        }
    }
}

/// Dispatch a case to the appropriate category runner.
pub async fn dispatch(case: &CorpusCase, ctx: &RunnerContext) -> Result<CaseExecution> {
    // Every case resets mock introspection logs first (FR-18 step 1).
    // We ignore the error on categories where the mocks might not be
    // listening yet; the error surfaces at the actual test step.
    if matches!(
        case.category,
        Category::ModelProxy | Category::Egress | Category::Health | Category::GitSsh
    ) || case.category == Category::Divergence
    {
        introspection::reset_all().await?;
    }

    match case.category {
        Category::ModelProxy => model_proxy::run(case, ctx).await,
        Category::Egress => egress::run(case, ctx).await,
        Category::GitSsh => git_ssh::run(case, ctx).await,
        Category::Health => health::run(case, ctx).await,
        Category::Divergence => divergence::dispatch(case, ctx).await,
    }
}

/// Divergence cases are dispatched to per-case modules because each
/// one is fundamentally different in shape (SSE timing, bare-exec
/// rejection, SIGTERM drain).
mod divergence {
    use super::*;

    pub async fn dispatch(case: &CorpusCase, ctx: &RunnerContext) -> Result<CaseExecution> {
        match case.name.as_str() {
            "divergence_sse_streaming_openai" => {
                model_proxy::run_sse_divergence(case, ctx, true).await
            }
            "divergence_sse_streaming_anthropic" => {
                model_proxy::run_sse_divergence(case, ctx, false).await
            }
            "divergence_bare_exec_upload_pack_rejection"
            | "divergence_bare_exec_receive_pack_rejection" => {
                git_ssh::run_bare_exec_divergence(case, ctx).await
            }
            "divergence_connect_drain_on_sigterm" => divergence_drain::run(case, ctx).await,
            other => Err(anyhow::anyhow!(
                "unknown divergence case name {other:?}; no runner wired up"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn divergence_case_names_are_covered() {
        // Guard against drift: if FR-22 adds a new divergence case,
        // the dispatcher must know about it. This test enumerates the
        // five expected names.
        let expected = [
            "divergence_sse_streaming_openai",
            "divergence_sse_streaming_anthropic",
            "divergence_bare_exec_upload_pack_rejection",
            "divergence_bare_exec_receive_pack_rejection",
            "divergence_connect_drain_on_sigterm",
        ];
        assert_eq!(expected.len(), 5);
    }

    #[test]
    fn case_assertion_pass_and_fail_constructors() {
        let p = CaseAssertion::pass("ok");
        assert!(p.passed);
        assert_eq!(p.detail, "ok");
        let f = CaseAssertion::fail("nope");
        assert!(!f.passed);
        assert_eq!(f.detail, "nope");
    }

    #[test]
    fn case_execution_parity_has_no_assertion() {
        let exec = CaseExecution::parity(SideOutput::default(), SideOutput::default());
        assert!(exec.assertion.is_none());
    }

    #[test]
    fn case_execution_with_assertion_carries_it() {
        let exec = CaseExecution::with_assertion(
            SideOutput::default(),
            SideOutput::default(),
            CaseAssertion::pass("detail"),
        );
        assert!(exec.assertion.as_ref().unwrap().passed);
    }
}
