use anyhow::Result;

use crate::client::NemoClient;

#[derive(serde::Deserialize, serde::Serialize)]
struct ApproveResponse {
    loop_id: uuid::Uuid,
    state: String,
    approve_requested: bool,
}

pub async fn run(client: &NemoClient, loop_id: &str, json: bool) -> Result<()> {
    let resp: ApproveResponse = client
        .post(&format!("/approve/{loop_id}"), &serde_json::json!({}))
        .await?;

    if json {
        let output = serde_json::json!({
            "loop_id": resp.loop_id,
            "state": resp.state,
            "approve_requested": resp.approve_requested,
            "message": if resp.approve_requested {
                "Approved loop \u{2014} implementation will start on next reconciliation tick."
            } else {
                "Approve not applicable for current state."
            },
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if resp.approve_requested {
        println!("Approved loop {}", resp.loop_id);
        println!("  State: {}", resp.state);
        println!("  Implementation will start on next reconciliation tick.");
    } else {
        println!(
            "Loop {} is in state {} (approve not applicable)",
            resp.loop_id, resp.state
        );
    }
    Ok(())
}
