use anyhow::Result;

use crate::client::NemoClient;

#[derive(serde::Deserialize)]
struct ApproveResponse {
    loop_id: uuid::Uuid,
    state: String,
    approve_requested: bool,
}

pub async fn run(client: &NemoClient, loop_id: &str) -> Result<()> {
    let resp: ApproveResponse = client
        .post(&format!("/approve/{loop_id}"), &serde_json::json!({}))
        .await?;

    println!("Approved loop {}", resp.loop_id);
    println!("  State: {}", resp.state);
    if resp.approve_requested {
        println!("  Implementation will start on next reconciliation tick.");
    }
    Ok(())
}
