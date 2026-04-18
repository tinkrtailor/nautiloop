use std::collections::HashMap;

use crate::api_types::{InspectResponse, LoopSummary};
use super::cost::{self, PricingConfig, format_cost, format_tokens, round_total_tokens, round_duration_secs};

/// Build the compact one-line header summary (FR-1a).
///
/// Format: `nautiloop · N active · X impl · Y review · Z harden · W awaiting · T tokens · $C · Dh Dm`
pub fn build_header(
    loops: &[LoopSummary],
    all_inspect: &HashMap<uuid::Uuid, InspectResponse>,
    pricing: &PricingConfig,
    team: bool,
) -> String {
    let non_terminal: Vec<&LoopSummary> = loops
        .iter()
        .filter(|l| !is_terminal_state(&l.state))
        .collect();

    if non_terminal.is_empty() {
        return if team {
            "nautiloop · team view · no active loops · press s to start a new spec".to_string()
        } else {
            "nautiloop · no active loops · press s to start a new spec".to_string()
        };
    }

    let active_count = non_terminal.len();

    // Stage breakdown
    let mut impl_count = 0;
    let mut review_count = 0;
    let mut harden_count = 0;
    let mut awaiting_count = 0;
    let mut test_count = 0;
    for l in &non_terminal {
        match l.state.as_str() {
            "IMPLEMENTING" => impl_count += 1,
            "REVIEWING" => review_count += 1,
            "HARDENING" => harden_count += 1,
            "AWAITING_APPROVAL" => awaiting_count += 1,
            "TESTING" => test_count += 1,
            _ => {}
        }
    }

    // Cumulative tokens, cost, and duration from inspect data
    let mut total_input_tokens = 0u64;
    let mut total_output_tokens = 0u64;
    let mut total_duration = 0i64;
    let mut has_unknown_cost = false;
    let mut total_cost = 0.0f64;

    for l in &non_terminal {
        if let Some(inspect) = all_inspect.get(&l.loop_id) {
            for round in &inspect.rounds {
                let (inp, out) = round_total_tokens(round);
                total_input_tokens += inp;
                total_output_tokens += out;
                total_duration += round_duration_secs(round);

                // Cost calculation: we don't know which model per-stage,
                // so we try the loop's known model or use a default heuristic.
                // For simplicity, use the first matching model from defaults.
                let cost = pricing.calculate_cost(Some("claude-sonnet-4-6"), inp, out);
                match cost {
                    Some(c) => total_cost += c,
                    None => has_unknown_cost = true,
                }
            }
        }
    }

    let total_tokens = total_input_tokens + total_output_tokens;
    let tokens_str = format_tokens(total_tokens);
    let cost_str = if has_unknown_cost {
        format!("{}†", format_cost(Some(total_cost)))
    } else {
        format_cost(Some(total_cost))
    };
    let duration_str = cost::format_duration_secs(total_duration);

    // Build stage parts
    let mut stage_parts = Vec::new();
    if impl_count > 0 { stage_parts.push(format!("{impl_count} impl")); }
    if test_count > 0 { stage_parts.push(format!("{test_count} test")); }
    if review_count > 0 { stage_parts.push(format!("{review_count} review")); }
    if harden_count > 0 { stage_parts.push(format!("{harden_count} harden")); }
    if awaiting_count > 0 { stage_parts.push(format!("{awaiting_count} awaiting")); }

    let prefix = if team { "nautiloop · team view" } else { "nautiloop" };
    let stages = stage_parts.join(" · ");

    format!(
        "{prefix} · {active_count} active · {stages} · {tokens_str} tokens · {cost_str} · {duration_str}"
    )
}

fn is_terminal_state(state: &str) -> bool {
    matches!(state, "CONVERGED" | "FAILED" | "CANCELLED" | "HARDENED" | "SHIPPED")
}

/// Build approval context hints for the footer (FR-10a).
pub fn approval_hints(loop_item: &LoopSummary) -> Vec<(&'static str, &'static str)> {
    let mut hints = Vec::new();
    match loop_item.state.as_str() {
        "AWAITING_APPROVAL" => {
            hints.push(("a", "approve"));
            hints.push(("x", "cancel"));
            hints.push(("R", "see rounds"));
        }
        "CONVERGED" | "HARDENED" | "SHIPPED" => {
            if loop_item.spec_pr_url.is_some() {
                hints.push(("o", "open PR"));
            }
            hints.push(("R", "see rounds"));
        }
        _ => {}
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_loop(state: &str) -> LoopSummary {
        LoopSummary {
            loop_id: uuid::Uuid::new_v4(),
            engineer: "alice".to_string(),
            spec_path: "specs/test.md".to_string(),
            branch: "agent/alice/test".to_string(),
            state: state.to_string(),
            sub_state: None,
            round: 1,
            current_stage: None,
            active_job_name: None,
            spec_pr_url: None,
            failed_from_state: None,
            kind: "implement".to_string(),
            max_rounds: 15,
            created_at: "2026-04-10T10:00:00Z".to_string(),
            updated_at: "2026-04-10T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn header_no_active_loops() {
        let header = build_header(&[], &HashMap::new(), &PricingConfig::default(), false);
        assert!(header.contains("no active loops"));
    }

    #[test]
    fn header_team_view() {
        let header = build_header(&[], &HashMap::new(), &PricingConfig::default(), true);
        assert!(header.contains("team view"));
    }

    #[test]
    fn header_with_active_loops() {
        let loops = vec![
            make_loop("IMPLEMENTING"),
            make_loop("IMPLEMENTING"),
            make_loop("REVIEWING"),
            make_loop("AWAITING_APPROVAL"),
        ];
        let header = build_header(&loops, &HashMap::new(), &PricingConfig::default(), false);
        assert!(header.contains("4 active"));
        assert!(header.contains("2 impl"));
        assert!(header.contains("1 review"));
        assert!(header.contains("1 awaiting"));
    }

    #[test]
    fn header_excludes_terminal() {
        let loops = vec![
            make_loop("IMPLEMENTING"),
            make_loop("CONVERGED"),
            make_loop("FAILED"),
        ];
        let header = build_header(&loops, &HashMap::new(), &PricingConfig::default(), false);
        assert!(header.contains("1 active"));
    }
}
