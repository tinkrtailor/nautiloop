//! `divergence_connect_drain_on_sigterm` runner.
//!
//! This case has `order_hint: "last"` because it kills both sidecar
//! containers. The flow:
//!
//! 1. Open a CONNECT tunnel through the egress port to
//!    `egress-target:443` (mock-tcp-echo) on Go and Rust in parallel.
//! 2. Start a background task trickling 1 byte per 100ms into each
//!    tunnel. The tunnel echoes those bytes back.
//! 3. After 500ms of steady traffic, SIGTERM each sidecar via
//!    `docker compose kill --signal SIGTERM <service>`.
//! 4. Measure how long each tunnel continues to echo bytes after
//!    the SIGTERM. Expected:
//!    - Go: stops within ~200ms (no drain, listener closes immediately)
//!    - Rust: continues 2-5s (up to the 5s drain deadline in
//!      `sidecar/src/main.rs`).
//!
//! The runner encodes the drain duration on each side so the diff
//! engine sees different bodies (pass for divergence).

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::sleep;

use crate::compose::ports;
use crate::compose::{ComposeStack, SIDECAR_GO_SERVICE, SIDECAR_RUST_SERVICE};
use crate::corpus::CorpusCase;
use crate::result::SideOutput;
use crate::runner::{CaseAssertion, CaseExecution, RunnerContext};

/// How long to run the baseline steady-state traffic before firing
/// SIGTERM. Matches the spec's 500ms.
const BASELINE_MS: u64 = 500;

// ---- FR-22 / FR-27 drain thresholds with explicit CI tolerances ----
//
// The spec (FR-22) expects "Go stops within 200ms, Rust continues
// 2-5 seconds (up to the 5s drain deadline in FR-27)." All three
// bounds are enforced individually by `drain_assertion` below.
//
// Each constant names its spec-origin value and the CI-noise
// allowance added/subtracted from it. These are deliberately tight
// — the point of the divergence case is to prove Rust's drain
// behavior honors its deadline without ever degenerating into
// either (a) a Go-like immediate close or (b) an unbounded drain.

/// Maximum acceptable post-SIGTERM drain for the Go sidecar.
///
/// Spec: 200ms (Go closes listeners immediately, no drain loop).
/// CI tolerance: +50ms to absorb `docker compose kill` latency.
/// Enforcing `≤ 250ms` catches any future Go regression that
/// introduces an unintended drain path.
const GO_MAX_DRAIN_MS: u128 = 250;

/// Minimum acceptable post-SIGTERM drain for the Rust sidecar.
///
/// Spec: 2000ms (Rust must continue draining for at least 2s,
/// proving it is NOT behaving like Go). The baseline steady-state
/// trickle runs for `BASELINE_MS` (500ms) before SIGTERM, so 2s
/// past SIGTERM demonstrates the drain is actually happening
/// — a shorter drain would look indistinguishable from an
/// unlucky close near the Go bound.
/// CI tolerance: -100ms for measurement jitter.
/// Enforcing `≥ 1900ms` catches a Go-like immediate-close
/// regression on the Rust side.
const RUST_MIN_DRAIN_MS: u128 = 1900;

/// Maximum acceptable post-SIGTERM drain for the Rust sidecar.
///
/// Spec: 5000ms (FR-27 `SHUTDOWN_DRAIN_TIMEOUT` — Rust MUST close
/// all in-flight tunnels by this deadline, even at the cost of
/// dropping bytes).
/// CI tolerance: +500ms for scheduler / docker kill / pump read
/// latency. Enforcing `≤ 5500ms` catches any regression that
/// lets the drain run indefinitely (e.g. a future refactor that
/// accidentally awaits the inner task without the bounded timeout).
const RUST_MAX_DRAIN_MS: u128 = 5500;

/// Maximum wall clock we watch the tunnels for a post-SIGTERM drain.
///
/// Must be strictly greater than [`RUST_MAX_DRAIN_MS`] so a
/// violation of the upper bound is observable: the pump keeps
/// recording `last_byte_at` until either the tunnel closes OR this
/// watch window expires. We give ourselves 1000ms of headroom so
/// an over-bound drain of e.g. 5900ms is still measured accurately
/// rather than clipped at the watch edge.
const POST_SIGTERM_WATCH: Duration = Duration::from_millis(RUST_MAX_DRAIN_MS as u64 + 1000);

pub async fn run(_case: &CorpusCase, ctx: &RunnerContext) -> Result<CaseExecution> {
    // Establish both tunnels.
    let go_tunnel = open_connect_tunnel(ports::GO_EGRESS, "egress-target:443").await?;
    let rust_tunnel = open_connect_tunnel(ports::RUST_EGRESS, "egress-target:443").await?;

    let go_counter = Arc::new(Mutex::new(TunnelState::default()));
    let rust_counter = Arc::new(Mutex::new(TunnelState::default()));

    let go_pump = spawn_pump(go_tunnel, Arc::clone(&go_counter));
    let rust_pump = spawn_pump(rust_tunnel, Arc::clone(&rust_counter));

    // Baseline traffic.
    sleep(Duration::from_millis(BASELINE_MS)).await;

    // Fire SIGTERM via docker compose kill.
    let compose = ComposeStack::new(
        ctx.harness_dir.join("docker-compose.yml"),
        ctx.harness_dir.clone(),
    );
    compose.kill_signal(SIDECAR_GO_SERVICE, "SIGTERM").await?;
    let go_killed_at = Instant::now();
    compose.kill_signal(SIDECAR_RUST_SERVICE, "SIGTERM").await?;
    let rust_killed_at = Instant::now();

    // Watch until tunnels die or the watch window expires.
    let watch_end = Instant::now() + POST_SIGTERM_WATCH;
    while Instant::now() < watch_end {
        sleep(Duration::from_millis(50)).await;
        let go_closed = go_counter.lock().await.closed;
        let rust_closed = rust_counter.lock().await.closed;
        if go_closed && rust_closed {
            break;
        }
    }

    // Abort pumps so the tunnels drop.
    go_pump.abort();
    rust_pump.abort();

    let go_state = go_counter.lock().await.clone();
    let rust_state = rust_counter.lock().await.clone();

    let go_drain_ms = go_state
        .last_byte_at
        .map(|t| t.duration_since(go_killed_at).as_millis())
        .unwrap_or(0);
    let rust_drain_ms = rust_state
        .last_byte_at
        .map(|t| t.duration_since(rust_killed_at).as_millis())
        .unwrap_or(0);

    let assertion = drain_assertion(go_drain_ms, rust_drain_ms);

    let go_out = SideOutput {
        drain_stop_ms: Some(go_drain_ms),
        ..SideOutput::default()
    };
    let rust_out = SideOutput {
        drain_stop_ms: Some(rust_drain_ms),
        ..SideOutput::default()
    };

    Ok(CaseExecution::with_assertion(go_out, rust_out, assertion))
}

/// Evaluate the drain-on-SIGTERM divergence from measured
/// post-SIGTERM drain durations on each side. Extracted for unit
/// testing.
///
/// Three independent bounds are enforced (FR-22 + FR-27):
///
/// 1. Go stops within [`GO_MAX_DRAIN_MS`] (fast close, no drain loop).
/// 2. Rust continues emitting bytes for at least [`RUST_MIN_DRAIN_MS`]
///    (proves it is NOT behaving like Go and is actually draining).
/// 3. Rust stops by [`RUST_MAX_DRAIN_MS`] (FR-27 drain deadline —
///    the drain MUST be bounded).
///
/// All three must hold. Any single violation fails the case with a
/// specific message pointing at which bound was broken so the
/// failure artifact does not require inference.
fn drain_assertion(go_drain_ms: u128, rust_drain_ms: u128) -> CaseAssertion {
    let mut failures: Vec<String> = Vec::new();
    if go_drain_ms > GO_MAX_DRAIN_MS {
        failures.push(format!(
            "Go took {go_drain_ms}ms to stop, expected <={GO_MAX_DRAIN_MS}ms (spec: <=200ms + 50ms CI tolerance). A non-zero drain on the Go side indicates a regression from the fast-close baseline."
        ));
    }
    if rust_drain_ms < RUST_MIN_DRAIN_MS {
        failures.push(format!(
            "Rust drained only {rust_drain_ms}ms, expected >={RUST_MIN_DRAIN_MS}ms (spec: >=2000ms - 100ms CI tolerance); possible Go-like immediate close regression."
        ));
    }
    if rust_drain_ms > RUST_MAX_DRAIN_MS {
        failures.push(format!(
            "Rust drained {rust_drain_ms}ms, exceeded {RUST_MAX_DRAIN_MS}ms upper bound (spec: <=5000ms + 500ms CI tolerance); FR-27 drain deadline violated."
        ));
    }
    if failures.is_empty() {
        CaseAssertion::pass(format!(
            "drain divergence holds: go={go_drain_ms}ms (<= {GO_MAX_DRAIN_MS}ms) AND rust={rust_drain_ms}ms ({RUST_MIN_DRAIN_MS}ms <= _ <= {RUST_MAX_DRAIN_MS}ms)"
        ))
    } else {
        CaseAssertion::fail(failures.join(" | "))
    }
}

async fn open_connect_tunnel(proxy_port: u16, target: &str) -> Result<TcpStream> {
    let addr = format!("127.0.0.1:{proxy_port}");
    let mut stream = TcpStream::connect(&addr)
        .await
        .with_context(|| format!("connect {addr}"))?;
    let connect_line = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    stream
        .write_all(connect_line.as_bytes())
        .await
        .context("write CONNECT request")?;
    // Consume until CRLFCRLF.
    let mut buf = [0u8; 512];
    let mut head = Vec::with_capacity(512);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, stream.read(&mut buf)).await {
            Ok(Ok(0)) => return Err(anyhow!("sidecar closed before CONNECT response")),
            Ok(Ok(n)) => {
                head.extend_from_slice(&buf[..n]);
                if head.windows(4).any(|w| w == b"\r\n\r\n") {
                    return Ok(stream);
                }
            }
            Ok(Err(e)) => return Err(anyhow!("CONNECT read error: {e}")),
            Err(_) => return Err(anyhow!("CONNECT response timeout")),
        }
    }
    Err(anyhow!("CONNECT response did not complete"))
}

#[derive(Debug, Clone, Default)]
struct TunnelState {
    _bytes_written: u64,
    _bytes_read: u64,
    last_byte_at: Option<Instant>,
    closed: bool,
}

fn spawn_pump(
    mut stream: TcpStream,
    counter: Arc<Mutex<TunnelState>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = 0u64;
        let mut read_buf = [0u8; 256];
        loop {
            tick = tick.wrapping_add(1);
            let one = [(tick & 0xff) as u8];
            if stream.write_all(&one).await.is_err() {
                let mut s = counter.lock().await;
                s.closed = true;
                return;
            }
            // Short read attempt, but don't block forever.
            match tokio::time::timeout(Duration::from_millis(50), stream.read(&mut read_buf)).await
            {
                Ok(Ok(0)) => {
                    let mut s = counter.lock().await;
                    s.closed = true;
                    return;
                }
                Ok(Ok(n)) => {
                    let mut s = counter.lock().await;
                    s._bytes_read += n as u64;
                    s._bytes_written += 1;
                    s.last_byte_at = Some(Instant::now());
                }
                Ok(Err(_)) => {
                    let mut s = counter.lock().await;
                    s.closed = true;
                    return;
                }
                Err(_) => {
                    // read timed out — not closed, just no echo yet.
                    let mut s = counter.lock().await;
                    s._bytes_written += 1;
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Guard against drift: FR-22 says Go must stop "within 200ms"
    // and Rust must drain "2-5 seconds" bounded by FR-27's 5s
    // deadline. Encoding these as `const` assertions catches any
    // future edit that would loosen the bounds beyond the spec.
    const _: () = assert!(GO_MAX_DRAIN_MS < RUST_MIN_DRAIN_MS);
    const _: () = assert!(RUST_MIN_DRAIN_MS < RUST_MAX_DRAIN_MS);
    // Go: spec 200ms + 50ms CI tolerance.
    const _: () = assert!(GO_MAX_DRAIN_MS == 250);
    // Rust min: spec 2000ms - 100ms CI tolerance.
    const _: () = assert!(RUST_MIN_DRAIN_MS == 1900);
    // Rust max: spec 5000ms + 500ms CI tolerance.
    const _: () = assert!(RUST_MAX_DRAIN_MS == 5500);
    // Watch window must strictly exceed the upper bound so a
    // violation is observable rather than clipped at the edge.
    const _: () = assert!(POST_SIGTERM_WATCH.as_millis() > RUST_MAX_DRAIN_MS);

    #[test]
    fn default_tunnel_state_is_empty() {
        let s = TunnelState::default();
        assert!(!s.closed);
        assert!(s.last_byte_at.is_none());
    }

    #[test]
    fn drain_assertion_passes_on_expected_split() {
        // Mid-range drain: go closes in 120ms, rust drains 3200ms.
        let a = drain_assertion(120, 3200);
        assert!(a.passed, "detail: {}", a.detail);
        assert!(a.detail.contains("drain divergence holds"));
    }

    #[test]
    fn drain_assertion_edge_case_go_exactly_at_max() {
        let a = drain_assertion(GO_MAX_DRAIN_MS, RUST_MIN_DRAIN_MS);
        assert!(a.passed, "detail: {}", a.detail);
    }

    #[test]
    fn drain_assertion_edge_case_rust_exactly_at_max() {
        // Rust exactly at 5500ms must PASS (upper bound is inclusive).
        let a = drain_assertion(GO_MAX_DRAIN_MS, RUST_MAX_DRAIN_MS);
        assert!(a.passed, "detail: {}", a.detail);
    }

    #[test]
    fn drain_assertion_fails_when_go_drains_slowly() {
        let a = drain_assertion(GO_MAX_DRAIN_MS + 1, 3200);
        assert!(!a.passed);
        assert!(
            a.detail.contains("Go took 251ms to stop"),
            "detail: {}",
            a.detail
        );
        assert!(a.detail.contains("<=250ms"));
    }

    #[test]
    fn drain_assertion_fails_when_rust_closes_fast() {
        let a = drain_assertion(80, RUST_MIN_DRAIN_MS - 1);
        assert!(!a.passed);
        assert!(a.detail.contains("Rust drained only 1899ms"));
        assert!(a.detail.contains("Go-like immediate close regression"));
    }

    #[test]
    fn drain_assertion_fails_when_rust_exceeds_upper_bound() {
        // Regression guard for the codex r2 P1: a drain-forever bug
        // that would have silently passed the old (no upper bound)
        // assertion must now fail with a specific message.
        let a = drain_assertion(120, RUST_MAX_DRAIN_MS + 1);
        assert!(!a.passed);
        assert!(a.detail.contains("Rust drained 5501ms"));
        assert!(a.detail.contains("FR-27 drain deadline violated"));
    }

    #[test]
    fn drain_assertion_fails_when_rust_drain_is_way_over_deadline() {
        // A drain of 30s would be clipped to ~POST_SIGTERM_WATCH by
        // the runner loop, but even so the failure path must name
        // the upper bound explicitly.
        let a = drain_assertion(120, 30_000);
        assert!(!a.passed);
        assert!(a.detail.contains("exceeded 5500ms upper bound"));
    }

    #[test]
    fn drain_assertion_fails_when_both_sides_break_bounds() {
        // Go too slow AND rust too fast — both failures reported.
        let a = drain_assertion(GO_MAX_DRAIN_MS + 100, RUST_MIN_DRAIN_MS - 100);
        assert!(!a.passed);
        assert!(a.detail.contains("Go took"));
        assert!(a.detail.contains("Rust drained only"));
    }

    #[test]
    fn drain_assertion_reports_go_slow_and_rust_over_upper_bound_together() {
        // Pathological double-regression: Go gained a drain loop AND
        // Rust's deadline stopped bounding the drain. Both messages
        // must appear joined by " | ".
        let a = drain_assertion(GO_MAX_DRAIN_MS + 500, RUST_MAX_DRAIN_MS + 500);
        assert!(!a.passed);
        assert!(a.detail.contains("Go took"));
        assert!(a.detail.contains("exceeded 5500ms upper bound"));
        assert!(a.detail.contains(" | "));
    }
}
