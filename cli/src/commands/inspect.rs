use anyhow::Result;

use crate::client::NemoClient;

pub async fn run(client: &NemoClient, path: &str) -> Result<()> {
    // Prepend "agent/" if not already present so users can pass "alice/slug-hash"
    let branch = if path.starts_with("agent/") {
        path.to_string()
    } else {
        format!("agent/{path}")
    };

    // Pass branch as query param (not path segment) because branch names contain slashes
    let resp: serde_json::Value = client
        .get(&format!("/inspect?branch={}", urlencoding::encode(&branch)))
        .await?;

    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}
