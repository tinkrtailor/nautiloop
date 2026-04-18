use anyhow::Result;

use crate::api_types::CacheResponse;
use crate::client::NemoClient;

/// Run `nemo cache show`. Prints resolved cache config and disk usage.
pub async fn run(client: &NemoClient, json: bool) -> Result<()> {
    let resp: CacheResponse = client.get("/cache").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    // Plain text output (FR-6a format).
    if resp.disabled {
        println!("Cache: disabled");
        println!("\nNo /cache mount or cache env vars are set on agent pods.");
        return Ok(());
    }

    // Volume info line (FR-6a: "Cache volume: nautiloop-cache (50 GiB)")
    if let Some(cap) = resp.volume_capacity_gi {
        println!("Cache volume: {} ({cap} GiB)", resp.volume_name);
    } else {
        println!("Cache volume: {}", resp.volume_name);
    }

    // Disk usage line (FR-6a: "Disk usage: 2.1 GiB / 50 GiB (4%)")
    if let Some(ref usage) = resp.disk_usage {
        if let Some(cap) = resp.volume_capacity_gi {
            println!("Disk usage:   {} / {cap} GiB", usage.total);
        } else {
            println!("Disk usage:   {}", usage.total);
        }

        if !usage.subdirectories.is_empty() {
            println!();
            println!("Subdirectory sizes:");
            let mut dirs: Vec<_> = usage.subdirectories.iter().collect();
            dirs.sort_by_key(|(path, _)| path.as_str());
            for (path, size) in dirs {
                println!("  {path:<30} {size}");
            }
        } else {
            println!("Disk usage:   empty (no cache subdirectories)");
        }
        println!();
    } else {
        println!("Disk usage:   unavailable (no running pod)");
        println!();
    }

    // Active env vars
    if resp.env.is_empty() {
        println!("Active env vars: (none)");
    } else {
        println!("Active env vars (from control-plane config):");
        let mut keys: Vec<_> = resp.env.keys().collect();
        keys.sort();
        // Find max key length for alignment
        let max_len = keys.iter().map(|k| k.len()).max().unwrap_or(0);
        for key in keys {
            let val = &resp.env[key];
            println!("  {key:<max_len$} = {val}");
        }
    }

    Ok(())
}
