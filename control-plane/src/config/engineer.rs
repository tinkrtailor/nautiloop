//! Engineer-level configuration loaded from `~/.nemo/config.toml`.
//!
//! This file is optional. Missing file means no overrides.
//! The `[identity]` section provides engineer name and email used by `nemo auth`.

use serde::Deserialize;

use super::repo::{LimitsConfig, ModelConfig};

/// Engineer identity from `[identity]` section.
#[derive(Debug, Clone, Deserialize)]
pub struct IdentityConfig {
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub ssh_key_path: Option<String>,
}

/// Engineer-level configuration from `~/.nemo/config.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EngineerConfig {
    #[serde(default)]
    pub identity: Option<IdentityConfig>,
    #[serde(default)]
    pub models: Option<ModelConfig>,
    #[serde(default)]
    pub limits: Option<LimitsConfig>,
}

impl EngineerConfig {
    /// Load from `~/.nemo/config.toml`. Returns None if file doesn't exist.
    pub fn load() -> Result<Option<Self>, String> {
        let home = match std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            Ok(h) => h,
            Err(_) => return Ok(None),
        };

        let path = std::path::PathBuf::from(home)
            .join(".nemo")
            .join("config.toml");

        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

        let config: Self = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;

        Ok(Some(config))
    }

    /// Parse from a TOML string.
    pub fn parse(content: &str) -> Result<Self, String> {
        toml::from_str(content).map_err(|e| format!("Failed to parse engineer config: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engineer_config_parse() {
        let toml = r#"
            [identity]
            name = "alice"
            email = "alice@example.com"
            ssh_key_path = "~/.ssh/id_ed25519"

            [models]
            implementor = "claude-sonnet-4"

            [limits]
            max_rounds_harden = 3
        "#;

        let config = EngineerConfig::parse(toml).unwrap();
        let identity = config.identity.unwrap();
        assert_eq!(identity.name, "alice");
        assert_eq!(identity.email, "alice@example.com");
        assert_eq!(identity.ssh_key_path, Some("~/.ssh/id_ed25519".to_string()));
        assert_eq!(
            config.models.unwrap().implementor,
            Some("claude-sonnet-4".to_string())
        );
        assert_eq!(config.limits.unwrap().max_rounds_harden, Some(3));
    }

    #[test]
    fn test_engineer_config_empty() {
        let toml = "";
        let config = EngineerConfig::parse(toml).unwrap();
        assert!(config.identity.is_none());
        assert!(config.models.is_none());
        assert!(config.limits.is_none());
    }

    #[test]
    fn test_engineer_config_models_only() {
        let toml = r#"
            [models]
            reviewer = "gpt-5.4"
        "#;
        let config = EngineerConfig::parse(toml).unwrap();
        assert!(config.identity.is_none());
        assert_eq!(config.models.unwrap().reviewer, Some("gpt-5.4".to_string()));
    }
}
