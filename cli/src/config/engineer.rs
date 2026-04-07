use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Engineer-level configuration from `~/.nemo/config.toml`.
///
/// In the per-repo-config model (see `specs/per-repo-config.md`), this file is
/// the lowest-priority source for all fields, with `server_url` and `api_key`
/// being the legacy fallback and `engineer`/`name`/`email` being the only
/// fields that are meaningfully global (per-user identity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineerConfig {
    #[serde(default = "default_server_url")]
    pub server_url: String,
    #[serde(default)]
    pub engineer: String,
    /// Display name for git attribution (GIT_AUTHOR_NAME).
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    pub api_key: Option<String>,
}

fn default_server_url() -> String {
    "https://localhost:8080".to_string()
}

impl Default for EngineerConfig {
    fn default() -> Self {
        Self {
            server_url: default_server_url(),
            engineer: String::new(),
            name: String::new(),
            email: String::new(),
            api_key: None,
        }
    }
}

/// Directory holding the engineer-level config (`~/.nemo/`).
pub fn dirs_path() -> PathBuf {
    dirs_path_from_home(home_dir())
}

/// Home directory used as the anchor for `~/.nemo/`.
fn home_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE")) // Windows fallback
        .unwrap_or_else(|_| "/tmp".to_string()); // Safe fallback, never cwd
    PathBuf::from(home)
}

/// Compute `.nemo/` under a specific home directory. Exposed for testability.
pub fn dirs_path_from_home(home: PathBuf) -> PathBuf {
    home.join(".nemo")
}

/// Get the config file path (`~/.nemo/config.toml`).
pub fn config_path() -> PathBuf {
    dirs_path().join("config.toml")
}

/// Load the engineer config, returning defaults if the file doesn't exist.
pub fn load_config() -> Result<EngineerConfig> {
    load_config_from(&config_path())
}

/// Load the engineer config from a specific path. Exposed for testability.
pub fn load_config_from(path: &std::path::Path) -> Result<EngineerConfig> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        let config: EngineerConfig = toml::from_str(&contents)?;
        Ok(config)
    } else {
        Ok(EngineerConfig::default())
    }
}

/// Save the engineer config to `~/.nemo/config.toml`.
/// Writes atomically via temp file to avoid a window where the file is world-readable.
pub fn save_config(config: &EngineerConfig) -> Result<()> {
    save_config_to(&config_path(), config)
}

/// Save the engineer config to a specific path. Exposed for testability.
pub fn save_config_to(path: &std::path::Path, config: &EngineerConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = toml::to_string_pretty(config)?;

    // Write to a temp file with restricted permissions first, then rename.
    // This avoids a window where the file exists with default umask permissions.
    let tmp_path = path.with_extension("toml.tmp");

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        file.write_all(contents.as_bytes())?;
    }

    #[cfg(not(unix))]
    {
        std::fs::write(&tmp_path, &contents)?;
    }

    std::fs::rename(&tmp_path, path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_load_missing_returns_default() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("missing.toml");
        let cfg = load_config_from(&path).unwrap();
        assert_eq!(cfg.server_url, "https://localhost:8080");
        assert!(cfg.api_key.is_none());
    }

    #[test]
    fn test_save_then_load_roundtrip() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let cfg = EngineerConfig {
            server_url: "http://example:8080".to_string(),
            engineer: "alice".to_string(),
            name: "Alice Example".to_string(),
            email: "alice@example.com".to_string(),
            api_key: Some("secret-key".to_string()),
        };
        save_config_to(&path, &cfg).unwrap();
        let loaded = load_config_from(&path).unwrap();
        assert_eq!(loaded.server_url, "http://example:8080");
        assert_eq!(loaded.engineer, "alice");
        assert_eq!(loaded.name, "Alice Example");
        assert_eq!(loaded.email, "alice@example.com");
        assert_eq!(loaded.api_key.as_deref(), Some("secret-key"));
    }

    #[cfg(unix)]
    #[test]
    fn test_save_uses_mode_0600() {
        use std::os::unix::fs::MetadataExt;
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let cfg = EngineerConfig::default();
        save_config_to(&path, &cfg).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "expected mode 0600 on engineer config, got {mode:o}"
        );
    }
}
