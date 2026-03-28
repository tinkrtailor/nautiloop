use anyhow::Result;

use crate::client::NemoClient;

#[derive(serde::Deserialize)]
struct ResumeResponse {
    loop_id: uuid::Uuid,
    state: String,
    resume_requested: bool,
}

pub async fn run(client: &NemoClient, loop_id: &str) -> Result<()> {
    let resp: ResumeResponse = client
        .post(&format!("/resume/{loop_id}"), &serde_json::json!({}))
        .await?;

    println!("Resumed loop {}", resp.loop_id);
    println!("  State: {}", resp.state);
    if resp.resume_requested {
        println!("  Loop will resume on next reconciliation tick.");
    }
    Ok(())
}
