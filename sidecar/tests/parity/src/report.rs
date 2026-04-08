//! NFR-9 run log dumper and stdout progress printer.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::result::{CaseOutcome, RunSummary};

/// Print a per-case line to stdout during the run.
pub fn print_case_result(outcome: &CaseOutcome) {
    let marker = if outcome.passed { "PASS" } else { "FAIL" };
    println!(
        "[{}] {:<46}  {:>6}ms  ({})",
        marker, outcome.name, outcome.duration_ms, outcome.source_path
    );
    if !outcome.notes.is_empty() {
        println!("        note: {}", outcome.notes);
    }
    if !outcome.passed {
        for line in outcome.diff.lines() {
            println!("        {line}");
        }
    }
}

/// Print the run summary to stdout.
pub fn print_summary(summary: &RunSummary) {
    println!();
    println!("==== parity harness summary ====");
    println!("  total:  {}", summary.total);
    println!("  passed: {}", summary.passed);
    println!("  failed: {}", summary.failed);
    for (cat, row) in &summary.by_category {
        println!("  {cat:<14} pass={} fail={}", row.passed, row.failed);
    }
    if !summary.failures.is_empty() {
        println!();
        println!("  failed cases:");
        for name in &summary.failures {
            println!("    - {name}");
        }
    }
    println!();
}

/// Write the FR-26 / NFR-9 artifact log dump. The format is:
///
/// ```text
/// ==== summary ====
/// ...
/// ==== case results ====
/// [PASS|FAIL] name (source): Nms
///   notes: ...                      (pass path, if any)
///   <diff lines>                    (fail path)
///   [sidecar-go container_logs: N lines]
///     <log message>
///     ...
///   [sidecar-rust container_logs: N lines]
///     <log message>
///     ...
/// ...
/// ==== docker compose logs (if available) ====
/// ...
/// ```
///
/// Per-case container logs are captured by [`crate::main::run_case`]
/// via `docker compose logs --timestamps --since <case-start>` and
/// attached to [`crate::result::SideOutput::container_logs`] before
/// normalization strips the timestamp field (finding #5 of the
/// codex review). They are printed here unconditionally because Go
/// and Rust emit different log content and cannot participate in
/// the strict diff.
///
/// `docker_logs` is the caller-captured stdout of `docker compose logs`,
/// or an empty string if the caller couldn't get it.
pub fn dump_run_log(
    path: impl AsRef<Path>,
    summary: &RunSummary,
    outcomes: &[CaseOutcome],
    docker_logs: &str,
) -> Result<()> {
    let path_ref = path.as_ref();
    if let Some(parent) = path_ref.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create parent for {}", path_ref.display()))?;
    }
    let mut f = File::create(path_ref).with_context(|| format!("create {}", path_ref.display()))?;
    writeln!(f, "==== summary ====")?;
    writeln!(f, "total={}", summary.total)?;
    writeln!(f, "passed={}", summary.passed)?;
    writeln!(f, "failed={}", summary.failed)?;
    for (cat, row) in &summary.by_category {
        writeln!(f, "category {cat}: pass={} fail={}", row.passed, row.failed)?;
    }
    writeln!(f)?;
    writeln!(f, "==== case results ====")?;
    for o in outcomes {
        let marker = if o.passed { "PASS" } else { "FAIL" };
        writeln!(
            f,
            "[{marker}] {} ({}): {}ms",
            o.name, o.source_path, o.duration_ms
        )?;
        if !o.notes.is_empty() {
            writeln!(f, "  notes: {}", o.notes)?;
        }
        if !o.passed {
            for line in o.diff.lines() {
                writeln!(f, "  {line}")?;
            }
        }
        // Per-case container log dumps (always, pass or fail) for
        // debuggability. Empty containers produce the header only.
        writeln!(
            f,
            "  [sidecar-go container_logs: {} lines]",
            o.go_side.container_logs.len()
        )?;
        for line in &o.go_side.container_logs {
            writeln!(f, "    {}", line.message)?;
        }
        writeln!(
            f,
            "  [sidecar-rust container_logs: {} lines]",
            o.rust_side.container_logs.len()
        )?;
        for line in &o.rust_side.container_logs {
            writeln!(f, "    {}", line.message)?;
        }
    }
    writeln!(f)?;
    writeln!(f, "==== docker compose logs ====")?;
    writeln!(f, "{docker_logs}")?;
    f.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::tempdir;

    use super::*;
    use crate::result::{CaseOutcome, LogLine, SideOutput};

    #[test]
    fn dump_run_log_writes_expected_sections() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("subdir/harness-run.log");
        let outcomes = vec![
            CaseOutcome::pass(
                "ok_case",
                "corpus/ok.json",
                true,
                SideOutput::default(),
                SideOutput::default(),
                Duration::from_millis(12),
                "all fine",
            ),
            CaseOutcome::fail(
                "bad_case",
                "corpus/bad.json",
                true,
                SideOutput::default(),
                SideOutput::default(),
                Duration::from_millis(30),
                "mismatch\non two lines",
            ),
        ];
        let summary = RunSummary::from_outcomes(&outcomes);
        dump_run_log(&path, &summary, &outcomes, "docker log blob").unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("==== summary ===="));
        assert!(written.contains("passed=1"));
        assert!(written.contains("failed=1"));
        assert!(written.contains("[PASS] ok_case"));
        assert!(written.contains("[FAIL] bad_case"));
        assert!(written.contains("  mismatch"));
        assert!(written.contains("  on two lines"));
        assert!(written.contains("docker log blob"));
    }

    #[test]
    fn dump_run_log_creates_parent_directory() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("deep/nested/path/harness-run.log");
        dump_run_log(&path, &RunSummary::default(), &[], "").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn dump_run_log_includes_per_case_container_logs() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("harness-run.log");

        let go_side = SideOutput {
            container_logs: vec![
                LogLine {
                    timestamp: String::new(),
                    message: "go_started".into(),
                },
                LogLine {
                    timestamp: String::new(),
                    message: "go_handled_request".into(),
                },
            ],
            ..SideOutput::default()
        };
        let rust_side = SideOutput {
            container_logs: vec![LogLine {
                timestamp: String::new(),
                message: "rust INFO listening".into(),
            }],
            ..SideOutput::default()
        };

        let outcomes = vec![CaseOutcome::pass(
            "c",
            "corpus/c.json",
            true,
            go_side,
            rust_side,
            Duration::from_millis(10),
            "",
        )];
        let summary = RunSummary::from_outcomes(&outcomes);
        dump_run_log(&path, &summary, &outcomes, "").unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("[sidecar-go container_logs: 2 lines]"));
        assert!(written.contains("go_started"));
        assert!(written.contains("go_handled_request"));
        assert!(written.contains("[sidecar-rust container_logs: 1 lines]"));
        assert!(written.contains("rust INFO listening"));
    }
}
