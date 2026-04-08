//! Parsing for `docker compose logs --timestamps --no-log-prefix
//! <service>` output.
//!
//! The `--timestamps` flag prefixes every line with an RFC3339Nano
//! timestamp followed by a single space and then the container's
//! stdout/stderr payload. `--no-log-prefix` strips the
//! `<service>  | ` column so the harness can compare payloads
//! across services.
//!
//! Example raw line:
//!
//! ```text
//! 2026-04-08T12:34:56.789012345Z level=info msg="listening on :9090"
//! ```
//!
//! This module parses each line into a [`crate::result::LogLine`],
//! splitting on the first whitespace character. Normalization
//! (FR-19) later clears the timestamp field so the diff engine
//! compares message content only.

use crate::result::LogLine;

/// Parse the stdout of `docker compose logs --timestamps
/// --no-log-prefix <service>` into a list of [`LogLine`] entries.
///
/// - Empty lines are skipped.
/// - Lines that do not start with an RFC3339-ish timestamp (e.g. a
///   panic line printed by the runtime that cut through the
///   timestamp prefix) are preserved with an empty timestamp field
///   and the full raw content as the message.
/// - Trailing carriage returns / whitespace on each message are
///   trimmed.
pub fn parse_docker_logs(raw: &str) -> Vec<LogLine> {
    let mut out = Vec::new();
    for line in raw.split('\n') {
        let line = line.trim_end_matches(['\r']);
        if line.trim().is_empty() {
            continue;
        }
        match split_timestamp(line) {
            Some((ts, msg)) => {
                let msg = msg.trim_end().to_string();
                if msg.is_empty() {
                    continue;
                }
                out.push(LogLine {
                    timestamp: ts.to_string(),
                    message: msg,
                });
            }
            None => out.push(LogLine {
                timestamp: String::new(),
                message: line.trim_end().to_string(),
            }),
        }
    }
    out
}

/// Split a docker-logs line on the first whitespace character and
/// return `(timestamp, rest)` if the first token looks like an
/// RFC3339 timestamp (contains `T` and ends with `Z` or a timezone
/// offset). Otherwise returns `None`.
fn split_timestamp(line: &str) -> Option<(&str, &str)> {
    let (first, rest) = line.split_once(' ')?;
    if looks_like_rfc3339(first) {
        Some((first, rest))
    } else {
        None
    }
}

fn looks_like_rfc3339(token: &str) -> bool {
    if token.len() < 10 || !token.contains('T') {
        return false;
    }
    let last = token.chars().last().unwrap_or('_');
    if last == 'Z' {
        return true;
    }
    // Timezone offsets end in e.g. `+00:00` or `-05:30`. The cheap
    // check: a colon within the last four characters.
    token
        .chars()
        .rev()
        .take(6)
        .any(|c| c == '+' || c == '-' || c == ':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_line_with_timestamp() {
        let raw = "2026-04-08T12:34:56.789012345Z level=info msg=\"listening\"\n";
        let parsed = parse_docker_logs(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].timestamp, "2026-04-08T12:34:56.789012345Z");
        assert_eq!(parsed[0].message, "level=info msg=\"listening\"");
    }

    #[test]
    fn parses_multiple_lines() {
        let raw = "\
2026-04-08T12:00:00Z first
2026-04-08T12:00:01Z second
2026-04-08T12:00:02Z third
";
        let parsed = parse_docker_logs(raw);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].message, "first");
        assert_eq!(parsed[1].message, "second");
        assert_eq!(parsed[2].message, "third");
    }

    #[test]
    fn skips_empty_lines() {
        let raw = "\
2026-04-08T12:00:00Z a

2026-04-08T12:00:01Z b
";
        let parsed = parse_docker_logs(raw);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].message, "a");
        assert_eq!(parsed[1].message, "b");
    }

    #[test]
    fn handles_lines_without_timestamp() {
        // A panic line inside a Rust runtime can bypass the normal
        // logging path and get written without the `--timestamps`
        // prefix. Preserve such lines with empty timestamp.
        let raw = "panic at the disco\n2026-04-08T12:00:00Z regular\n";
        let parsed = parse_docker_logs(raw);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].timestamp, "");
        assert_eq!(parsed[0].message, "panic at the disco");
        assert_eq!(parsed[1].timestamp, "2026-04-08T12:00:00Z");
    }

    #[test]
    fn trims_trailing_whitespace_on_messages() {
        let raw = "2026-04-08T12:00:00Z trailing spaces   \n";
        let parsed = parse_docker_logs(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].message, "trailing spaces");
    }

    #[test]
    fn handles_timezone_offset_format() {
        let raw = "2026-04-08T12:00:00.000000000+00:00 msg here\n";
        let parsed = parse_docker_logs(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].timestamp, "2026-04-08T12:00:00.000000000+00:00");
        assert_eq!(parsed[0].message, "msg here");
    }

    #[test]
    fn empty_input_returns_empty_vec() {
        assert!(parse_docker_logs("").is_empty());
    }

    #[test]
    fn only_whitespace_input_returns_empty_vec() {
        assert!(parse_docker_logs("\n\n   \n").is_empty());
        assert!(parse_docker_logs("\n\n").is_empty());
    }
}
