use anyhow::Result;

use crate::client::NemoClient;

/// Push local model credentials to the cluster.
///
/// In V1, this is a placeholder for the credential transport mechanism.
/// The exact mechanism (direct K8s API, API server relay, or SSH tunnel)
/// is TBD per the spec's open questions.
pub async fn run(_client: &NemoClient, claude: bool, openai: bool) -> Result<()> {
    let providers: Vec<&str> = match (claude, openai) {
        (true, false) => vec!["claude"],
        (false, true) => vec!["openai"],
        _ => vec!["claude", "openai"],
    };

    for provider in &providers {
        // Check for local credentials
        let cred_path = match *provider {
            "claude" => {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                format!("{home}/.claude/credentials.json")
            }
            "openai" => {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                format!("{home}/.config/openai/auth.json")
            }
            _ => continue,
        };

        if !std::path::Path::new(&cred_path).exists() {
            println!("No {provider} credentials found at {cred_path}");
            println!("  Run the {provider} CLI to authenticate first.");
            continue;
        }

        println!("Found {provider} credentials at {cred_path}");
        println!("  Credential transport mechanism is not yet implemented (see spec open questions).");
        println!("  For now, credentials are expected to be available in the cluster.");
    }

    Ok(())
}
