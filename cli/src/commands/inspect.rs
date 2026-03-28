use anyhow::Result;

use crate::client::NemoClient;

pub async fn run(client: &NemoClient, path: &str) -> Result<()> {
    let resp: serde_json::Value = client.get(&format!("/inspect/{path}")).await?;

    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}
