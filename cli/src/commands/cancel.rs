use anyhow::Result;

use crate::client::NemoClient;

#[derive(serde::Deserialize, serde::Serialize)]
struct CancelResponse {
    loop_id: uuid::Uuid,
    state: String,
    cancel_requested: bool,
}

pub async fn run(client: &NemoClient, loop_id: &str, json: bool) -> Result<()> {
    let resp: CancelResponse = client.delete(&format!("/cancel/{loop_id}")).await?;

    if json {
        let output = serde_json::json!({
            "loop_id": resp.loop_id,
            "state": resp.state,
            "cancel_requested": resp.cancel_requested,
            "message": if resp.cancel_requested { "Loop cancelled." } else { "Cancel not applicable for current state." },
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if resp.cancel_requested {
        println!("Cancel requested for loop {}", resp.loop_id);
        println!("  Current state: {}", resp.state);
        println!("  The loop engine will cancel the loop on the next tick.");
    } else {
        println!("Loop {} state: {}", resp.loop_id, resp.state);
    }
    Ok(())
}
