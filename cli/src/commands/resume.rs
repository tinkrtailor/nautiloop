use anyhow::Result;

use crate::client::NemoClient;

#[derive(serde::Deserialize, serde::Serialize)]
struct ResumeResponse {
    loop_id: uuid::Uuid,
    state: String,
    resume_requested: bool,
}

pub async fn run(client: &NemoClient, loop_id: &str, json: bool) -> Result<()> {
    let resp: ResumeResponse = client
        .post(&format!("/resume/{loop_id}"), &serde_json::json!({}))
        .await?;

    if json {
        let output = serde_json::json!({
            "loop_id": resp.loop_id,
            "state": resp.state,
            "resume_requested": resp.resume_requested,
            "message": if resp.resume_requested { "Loop resumed." } else { "Resume not applicable for current state." },
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if resp.resume_requested {
        println!("Resumed loop {}", resp.loop_id);
        println!("  State: {}", resp.state);
        println!("  Loop will resume on next reconciliation tick.");
    } else {
        println!(
            "Loop {} is in state {} (resume not applicable)",
            resp.loop_id, resp.state
        );
    }
    Ok(())
}
