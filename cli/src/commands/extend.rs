use anyhow::Result;

use crate::client::NemoClient;

#[derive(serde::Deserialize, serde::Serialize)]
struct ExtendResponse {
    loop_id: uuid::Uuid,
    prior_max_rounds: u32,
    new_max_rounds: u32,
    resumed_to_state: String,
}

pub async fn run(client: &NemoClient, loop_id: &str, add_rounds: u32, json: bool) -> Result<()> {
    let resp: ExtendResponse = client
        .post(
            &format!("/extend/{loop_id}"),
            &serde_json::json!({ "add_rounds": add_rounds }),
        )
        .await?;

    if json {
        let output = serde_json::json!({
            "loop_id": resp.loop_id,
            "prior_max_rounds": resp.prior_max_rounds,
            "new_max_rounds": resp.new_max_rounds,
            "resumed_to_state": resp.resumed_to_state,
            "message": format!("Extended by {} rounds.", add_rounds),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("Extended loop {}", resp.loop_id);
    println!(
        "  max_rounds: {} -> {} (+{})",
        resp.prior_max_rounds, resp.new_max_rounds, add_rounds
    );
    println!("  Resuming at: {}", resp.resumed_to_state);
    println!("  Loop will continue on next reconciliation tick.");
    Ok(())
}
