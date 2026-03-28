use anyhow::Result;

use crate::client::NemoClient;

#[derive(serde::Deserialize)]
struct CancelResponse {
    loop_id: uuid::Uuid,
    state: String,
    reason: String,
}

pub async fn run(client: &NemoClient, loop_id: &str) -> Result<()> {
    let resp: CancelResponse = client.delete(&format!("/cancel/{loop_id}")).await?;
    println!("Cancelled loop {}", resp.loop_id);
    println!("  State:  {}", resp.state);
    println!("  Reason: {}", resp.reason);
    Ok(())
}
