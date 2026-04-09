use anyhow::Result;

use crate::client::NemoClient;

/// Push local model credentials to the cluster.
///
/// Reads local credential files, validates they exist, and registers them
/// with the control plane so AWAITING_REAUTH loops can recover via `nemo resume`.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: &NemoClient,
    engineer: &str,
    name: &str,
    email: &str,
    claude: bool,
    openai: bool,
    opencode_auth: bool,
    ssh: bool,
) -> Result<()> {
    if engineer.is_empty() {
        anyhow::bail!("Engineer name not configured. Run: nemo config --set engineer=<your-name>");
    }

    let mut providers: Vec<&str> = Vec::new();
    if claude {
        providers.push("claude");
    }
    if openai {
        providers.push("openai");
    }
    if opencode_auth {
        providers.push("opencode-auth");
    }
    if ssh {
        providers.push("ssh");
    }
    // Default: everything we can find if none explicitly specified.
    // opencode-auth is included so ChatGPT-plan users get subscription auth
    // wired up automatically alongside any Platform key they have (#67).
    if providers.is_empty() {
        providers = vec!["claude", "openai", "opencode-auth", "ssh"];
    }
    let any_explicit = claude || openai || opencode_auth || ssh;

    let mut any_registered = false;
    let mut any_error = false;

    for provider in &providers {
        let cred_path = match *provider {
            "claude" => {
                // Claude Code credential paths (checked in priority order)
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                let config_dir =
                    std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
                let candidates = [
                    format!("{home}/.claude/.credentials.json"), // claude-worktree convention
                    format!("{config_dir}/claude-code/credentials.json"), // XDG standard
                    format!("{home}/.claude/credentials.json"),  // legacy
                ];
                candidates
                    .iter()
                    .find(|p| std::path::Path::new(p).exists())
                    .cloned()
                    .unwrap_or_else(|| candidates[0].clone())
            }
            "openai" => {
                // OpenCode / OpenAI credential paths (checked in priority order)
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                let config_dir =
                    std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
                let candidates = [
                    format!("{config_dir}/opencode/credentials.json"), // opencode reviewer auth
                    format!("{config_dir}/openai/credentials.json"),   // direct OpenAI
                ];
                candidates
                    .iter()
                    .find(|p| std::path::Path::new(p).exists())
                    .cloned()
                    .unwrap_or_else(|| candidates[0].clone())
            }
            "opencode-auth" => {
                // opencode's subscription auth bundle (OAuth tokens for
                // ChatGPT Plus/Team/Enterprise plans). Mirrors what the
                // opencode CLI writes when the user runs `opencode auth login`.
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                let data_dir = std::env::var("XDG_DATA_HOME")
                    .unwrap_or_else(|_| format!("{home}/.local/share"));
                format!("{data_dir}/opencode/auth.json")
            }
            "ssh" => {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                format!("{home}/.ssh/id_ed25519")
            }
            _ => continue,
        };

        if !std::path::Path::new(&cred_path).exists() {
            // Silently skip opencode-auth in the default (all-providers) run.
            // Most engineers have either a Platform key OR a subscription, not both,
            // and we don't want the default `nemo auth` to error on "no auth.json".
            if *provider == "opencode-auth" && !any_explicit {
                continue;
            }
            eprintln!("No {provider} credentials found at {cred_path}");
            match *provider {
                "claude" => eprintln!("  Run: claude login"),
                "openai" => {
                    eprintln!("  Create {cred_path} with your OpenAI API key as content")
                }
                "opencode-auth" => {
                    eprintln!("  Run: opencode auth login (then re-run nemo auth)")
                }
                "ssh" => eprintln!("  Run: ssh-keygen -t ed25519"),
                _ => {}
            }
            // If the provider was explicitly requested (not default "all"), treat as error
            if any_explicit {
                any_error = true;
            }
            continue;
        }

        // Read the credential file
        let content = match std::fs::read_to_string(&cred_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Warning: could not read {provider} credentials at {cred_path}: {e}");
                any_error = true;
                continue;
            }
        };

        if content.trim().is_empty() {
            eprintln!("Error: {provider} credentials at {cred_path} are empty");
            any_error = true;
            continue;
        }

        // For claude/openai/opencode-auth, validate content is either valid JSON or a raw
        // API key string. Reject obviously malformed content (e.g. truncated JSON, binary data).
        // opencode-auth is always a JSON bundle, so validate it strictly.
        if *provider != "ssh" {
            let trimmed = content.trim();
            if *provider == "opencode-auth" {
                if serde_json::from_str::<serde_json::Value>(trimmed).is_err() {
                    eprintln!("Error: opencode-auth credentials at {cred_path} are not valid JSON");
                    any_error = true;
                    continue;
                }
            } else if trimmed.starts_with('{')
                && serde_json::from_str::<serde_json::Value>(trimmed).is_err()
            {
                eprintln!("Error: {provider} credentials at {cred_path} contain malformed JSON");
                any_error = true;
                continue;
            }
        }

        // Register credentials with the control plane
        match client
            .register_credentials(
                engineer,
                provider,
                &content,
                if name.is_empty() { None } else { Some(name) },
                if email.is_empty() { None } else { Some(email) },
            )
            .await
        {
            Ok(()) => {
                println!("Registered {provider} credentials with control plane");
                any_registered = true;
            }
            Err(e) => {
                eprintln!("Failed to register {provider} credentials: {e}");
                eprintln!("  Credentials found locally at {cred_path} but could not be pushed.");
                eprintln!("  Ensure the control plane is reachable and your API key is valid.");
                any_error = true;
            }
        }
    }

    if any_registered {
        println!();
        println!("Credentials registered. If you have loops in AWAITING_REAUTH state,");
        println!("resume them with: nemo resume <loop-id>");
    }

    if any_error {
        if any_registered {
            anyhow::bail!("Some credential uploads failed (see errors above)");
        } else {
            anyhow::bail!("All credential uploads failed");
        }
    }

    if !any_registered {
        anyhow::bail!("No credential files found. Run the provider CLI to authenticate first.");
    }

    Ok(())
}
