use anyhow::Result;

use crate::api_types::StatusResponse;
use crate::client::NemoClient;

pub async fn fetch(client: &NemoClient, engineer: &str, team: bool) -> Result<StatusResponse> {
    client.get(&status_path(engineer, team)).await
}

fn status_path(engineer: &str, team: bool) -> String {
    if team {
        "/status?team=true".to_string()
    } else {
        // Percent-encode engineer name to handle special characters
        let encoded: String = engineer
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' {
                    format!("{}", b as char)
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        format!("/status?engineer={encoded}")
    }
}

pub async fn run(client: &NemoClient, engineer: &str, team: bool, json: bool) -> Result<()> {
    let resp = fetch(client, engineer, team).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp.loops)?);
        return Ok(());
    }

    if resp.loops.is_empty() {
        println!("No active loops.");
        return Ok(());
    }

    // Table output
    println!(
        "{:<38} {:<12} {:<10} {:<20} {:<40} {:<8}",
        "LOOP ID", "STATE", "STAGE", "ENGINEER", "SPEC", "ROUND"
    );
    println!("{}", "-".repeat(138));

    for l in &resp.loops {
        let state_display = match &l.sub_state {
            Some(sub) => format!("{}/{}", l.state, sub),
            None => l.state.clone(),
        };
        let stage_display = l.current_stage.as_deref().unwrap_or("-");
        println!(
            "{:<38} {:<12} {:<10} {:<20} {:<40} {:<8}",
            l.loop_id, state_display, stage_display, l.engineer, l.spec_path, l.round
        );
    }

    Ok(())
}
