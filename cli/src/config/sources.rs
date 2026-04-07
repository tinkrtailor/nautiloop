//! Multi-source configuration resolver with provenance tracking.
//!
//! See `specs/per-repo-config.md`.
//!
//! Resolution precedence (first match wins):
//! * `server_url`:
//!   1. `--server` CLI flag
//!   2. `NEMO_SERVER_URL` env var
//!   3. `<repo>/nemo.toml [server].url`
//!   4. `~/.nemo/config.toml server_url`
//!   5. built-in default (`https://localhost:8080`)
//! * `api_key`:
//!   1. `NEMO_API_KEY` env var
//!   2. `<repo>/.nemo/credentials`
//!   3. `~/.nemo/config.toml api_key`
//!   4. None
//! * `engineer`, `name`, `email`: from `~/.nemo/config.toml` only.
//!
//! The resolver also:
//! * walks up from the current working directory to find the repo root
//!   (first ancestor containing `nemo.toml` or `.git`),
//! * prints a one-line stderr warning the first time a per-repo source shadows
//!   a non-empty global source for the same key (FR-15, per-process only —
//!   no persistent state).

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::credentials;
use super::engineer::{EngineerConfig, load_config};
use super::repo_toml;

/// Where a resolved config value came from. Used for `nemo config`
/// provenance display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// `--server` CLI flag (only applies to `server_url`).
    CliFlag,
    /// `NEMO_SERVER_URL` or `NEMO_API_KEY` environment variable.
    EnvVar,
    /// `<repo>/nemo.toml` `[server].url`.
    RepoToml,
    /// `<repo>/.nemo/credentials` file.
    RepoCredentials,
    /// `~/.nemo/config.toml` (legacy fallback for url/api_key; sole source
    /// for identity fields).
    GlobalFile,
    /// Built-in hard-coded default (e.g. `https://localhost:8080`).
    Default,
}

impl ConfigSource {
    /// Human-readable label for display in `nemo config`.
    pub fn label(self) -> &'static str {
        match self {
            Self::CliFlag => "--server flag",
            Self::EnvVar => "env var",
            Self::RepoToml => "nemo.toml",
            Self::RepoCredentials => ".nemo/credentials",
            Self::GlobalFile => "~/.nemo/config.toml",
            Self::Default => "default",
        }
    }
}

/// A resolved configuration value paired with its source.
#[derive(Debug, Clone)]
pub struct Resolved<T> {
    pub value: T,
    pub source: ConfigSource,
}

impl<T> Resolved<T> {
    pub fn new(value: T, source: ConfigSource) -> Self {
        Self { value, source }
    }
}

/// Fully resolved CLI configuration with provenance.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub server_url: Resolved<String>,
    pub api_key: Option<Resolved<String>>,
    pub engineer: Resolved<String>,
    pub name: Resolved<String>,
    pub email: Resolved<String>,
    /// The detected repo root (first ancestor of cwd containing `nemo.toml`
    /// or `.git`). `None` if the CLI is being run outside any repo.
    pub repo_root: Option<PathBuf>,
}

/// Environment-variable name for the server URL override.
pub const ENV_SERVER_URL: &str = "NEMO_SERVER_URL";
/// Environment-variable name for the API key override.
pub const ENV_API_KEY: &str = "NEMO_API_KEY";

/// Built-in default server URL, used only if every other source is empty.
pub const DEFAULT_SERVER_URL: &str = "https://localhost:8080";

/// Walks up from `start` looking for the first ancestor containing a repo
/// marker (`nemo.toml` or `.git`, the latter as a file or a directory for
/// worktree support).
///
/// Returns the directory containing the marker, or `None` if none is found
/// before reaching the filesystem root.
///
/// Nested repo behavior: the nearest marker wins, which means submodules and
/// nested worktrees resolve to the innermost repo — consistent with how git
/// itself resolves `GIT_DIR`.
pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let mut current: Option<&Path> = Some(start);
    while let Some(dir) = current {
        if has_repo_marker(dir) {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

/// Returns true if `dir` contains a `nemo.toml` file OR a `.git` entry
/// (file or directory — worktree-aware).
fn has_repo_marker(dir: &Path) -> bool {
    if dir.join("nemo.toml").is_file() {
        return true;
    }
    // `.git` may be a directory (normal clone) or a file (worktree, submodule).
    let git = dir.join(".git");
    git.exists()
}

/// Resolve all CLI configuration values. This is the main entry point used
/// by `main.rs` and `nemo config`.
///
/// `cli_server` is the value of the `--server` global flag, if the user
/// passed it.
pub fn resolve(cli_server: Option<&str>) -> Result<ResolvedConfig> {
    let cwd = std::env::current_dir()?;
    let repo_root = find_repo_root(&cwd);

    let global = load_config().unwrap_or_default();

    let env_server = std::env::var(ENV_SERVER_URL).ok().and_then(non_empty);
    let env_api_key = std::env::var(ENV_API_KEY).ok().and_then(non_empty);

    Ok(resolve_from(ResolveInputs {
        cli_server: cli_server.and_then(|s| non_empty(s.to_string())),
        env_server,
        env_api_key,
        repo_root: repo_root.as_deref(),
        global: &global,
    }))
}

/// All inputs required to resolve a config. Extracted so `resolve_from`
/// can be pure and easy to unit-test without touching the real environment.
pub struct ResolveInputs<'a> {
    pub cli_server: Option<String>,
    pub env_server: Option<String>,
    pub env_api_key: Option<String>,
    pub repo_root: Option<&'a Path>,
    pub global: &'a EngineerConfig,
}

/// Pure, deterministic resolver. No env or filesystem access except for
/// reading `<repo_root>/nemo.toml` and `<repo_root>/.nemo/credentials` —
/// both of which take the repo_root as an explicit argument and are thus
/// easy to point at a tempdir in tests.
pub fn resolve_from(inputs: ResolveInputs<'_>) -> ResolvedConfig {
    let ResolveInputs {
        cli_server,
        env_server,
        env_api_key,
        repo_root,
        global,
    } = inputs;

    // --- server_url ---
    let repo_toml_url = repo_root.and_then(repo_toml::server_url_from_repo_toml);
    let global_server_url = non_empty(global.server_url.clone());
    let global_server_url_is_default = global.server_url == DEFAULT_SERVER_URL;

    let server_url = if let Some(v) = cli_server.clone() {
        Resolved::new(v, ConfigSource::CliFlag)
    } else if let Some(v) = env_server.clone() {
        Resolved::new(v, ConfigSource::EnvVar)
    } else if let Some(v) = repo_toml_url.clone() {
        Resolved::new(v, ConfigSource::RepoToml)
    } else if let Some(v) = global_server_url.clone() {
        // If the only "global" value is the default (the EngineerConfig default
        // kicks in when ~/.nemo/config.toml is absent), attribute it to Default
        // so provenance display is accurate.
        if global_server_url_is_default && !global_file_exists() {
            Resolved::new(v, ConfigSource::Default)
        } else {
            Resolved::new(v, ConfigSource::GlobalFile)
        }
    } else {
        Resolved::new(DEFAULT_SERVER_URL.to_string(), ConfigSource::Default)
    };

    // FR-15: warn once per process if a higher-priority source shadowed a
    // different non-empty global server_url.
    if let Some(global_val) = global_server_url.as_ref()
        && !global_server_url_is_default
        && matches!(
            server_url.source,
            ConfigSource::CliFlag | ConfigSource::EnvVar | ConfigSource::RepoToml
        )
        && server_url.value != *global_val
    {
        maybe_warn_shadowing(
            "server_url",
            server_url.source.label(),
            ConfigSource::GlobalFile.label(),
        );
    }

    // --- api_key ---
    let repo_cred = repo_root.and_then(|root| credentials::read_credentials(root).ok().flatten());
    let global_api_key = global.api_key.clone().and_then(non_empty);

    let api_key = if let Some(v) = env_api_key.clone() {
        Some(Resolved::new(v, ConfigSource::EnvVar))
    } else if let Some(v) = repo_cred.clone() {
        Some(Resolved::new(v, ConfigSource::RepoCredentials))
    } else {
        global_api_key
            .clone()
            .map(|v| Resolved::new(v, ConfigSource::GlobalFile))
    };

    if let (Some(global_val), Some(resolved)) = (global_api_key.as_ref(), api_key.as_ref())
        && matches!(
            resolved.source,
            ConfigSource::EnvVar | ConfigSource::RepoCredentials
        )
        && resolved.value != *global_val
    {
        maybe_warn_shadowing(
            "api_key",
            resolved.source.label(),
            ConfigSource::GlobalFile.label(),
        );
    }

    // --- identity fields (global only) ---
    let engineer = Resolved::new(global.engineer.clone(), ConfigSource::GlobalFile);
    let name = Resolved::new(global.name.clone(), ConfigSource::GlobalFile);
    let email = Resolved::new(global.email.clone(), ConfigSource::GlobalFile);

    ResolvedConfig {
        server_url,
        api_key,
        engineer,
        name,
        email,
        repo_root: repo_root.map(Path::to_path_buf),
    }
}

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
}

/// Returns true if the engineer config file `~/.nemo/config.toml` exists on disk.
/// Used to distinguish "user explicitly set default URL in file" from "no file
/// at all, EngineerConfig::default() kicked in".
fn global_file_exists() -> bool {
    super::engineer::config_path().exists()
}

/// Per-process gate so that FR-15 only prints the shadowing warning once,
/// even across many calls to `resolve_from`. Tracked globally because the
/// CLI is a short-lived process — per-process is the spec semantic.
static SHADOW_WARNED: AtomicBool = AtomicBool::new(false);

fn maybe_warn_shadowing(key: &str, winning_source: &str, shadowed_source: &str) {
    if SHADOW_WARNED
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        eprintln!("note: {key} from {winning_source} overrides value from {shadowed_source}");
    }
}

/// Test-only helper to reset the shadow warning gate.
#[cfg(test)]
pub(crate) fn reset_shadow_warning_gate() {
    SHADOW_WARNED.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn empty_global() -> EngineerConfig {
        EngineerConfig {
            server_url: DEFAULT_SERVER_URL.to_string(),
            engineer: String::new(),
            name: String::new(),
            email: String::new(),
            api_key: None,
        }
    }

    fn global_with(server_url: &str, engineer: &str, api_key: Option<&str>) -> EngineerConfig {
        EngineerConfig {
            server_url: server_url.to_string(),
            engineer: engineer.to_string(),
            name: "Global Name".to_string(),
            email: "global@example.com".to_string(),
            api_key: api_key.map(String::from),
        }
    }

    // ----- server_url precedence -----

    #[test]
    fn test_resolve_server_url_cli_flag_beats_env() {
        reset_shadow_warning_gate();
        let global = empty_global();
        let out = resolve_from(ResolveInputs {
            cli_server: Some("http://cli:1".to_string()),
            env_server: Some("http://env:2".to_string()),
            env_api_key: None,
            repo_root: None,
            global: &global,
        });
        assert_eq!(out.server_url.value, "http://cli:1");
        assert_eq!(out.server_url.source, ConfigSource::CliFlag);
    }

    #[test]
    fn test_resolve_server_url_env_beats_repo_toml() {
        reset_shadow_warning_gate();
        let tmp = tempdir().unwrap();
        std::fs::write(
            tmp.path().join("nemo.toml"),
            r#"[server]
url = "http://repo:3"
"#,
        )
        .unwrap();
        let global = empty_global();
        let out = resolve_from(ResolveInputs {
            cli_server: None,
            env_server: Some("http://env:2".to_string()),
            env_api_key: None,
            repo_root: Some(tmp.path()),
            global: &global,
        });
        assert_eq!(out.server_url.value, "http://env:2");
        assert_eq!(out.server_url.source, ConfigSource::EnvVar);
    }

    #[test]
    fn test_resolve_server_url_repo_toml_beats_global() {
        reset_shadow_warning_gate();
        let tmp = tempdir().unwrap();
        std::fs::write(
            tmp.path().join("nemo.toml"),
            r#"[server]
url = "http://repo:3"
"#,
        )
        .unwrap();
        let global = global_with("http://global:4", "alice", None);
        let out = resolve_from(ResolveInputs {
            cli_server: None,
            env_server: None,
            env_api_key: None,
            repo_root: Some(tmp.path()),
            global: &global,
        });
        assert_eq!(out.server_url.value, "http://repo:3");
        assert_eq!(out.server_url.source, ConfigSource::RepoToml);
    }

    #[test]
    fn test_resolve_server_url_global_is_fallback() {
        reset_shadow_warning_gate();
        let tmp = tempdir().unwrap();
        // No nemo.toml
        let global = global_with("http://global:4", "alice", None);
        let out = resolve_from(ResolveInputs {
            cli_server: None,
            env_server: None,
            env_api_key: None,
            repo_root: Some(tmp.path()),
            global: &global,
        });
        assert_eq!(out.server_url.value, "http://global:4");
        // Source is either GlobalFile (if ~/.nemo/config.toml exists on the
        // test box) or Default (if it doesn't). Both are acceptable.
        assert!(matches!(
            out.server_url.source,
            ConfigSource::GlobalFile | ConfigSource::Default
        ));
    }

    // ----- api_key precedence -----

    #[test]
    fn test_resolve_api_key_env_beats_repo_credentials() {
        reset_shadow_warning_gate();
        let tmp = tempdir().unwrap();
        credentials::write_credentials(tmp.path(), "repo-key").unwrap();
        let global = empty_global();
        let out = resolve_from(ResolveInputs {
            cli_server: None,
            env_server: None,
            env_api_key: Some("env-key".to_string()),
            repo_root: Some(tmp.path()),
            global: &global,
        });
        let key = out.api_key.unwrap();
        assert_eq!(key.value, "env-key");
        assert_eq!(key.source, ConfigSource::EnvVar);
    }

    #[test]
    fn test_resolve_api_key_repo_credentials_beats_global() {
        reset_shadow_warning_gate();
        let tmp = tempdir().unwrap();
        credentials::write_credentials(tmp.path(), "repo-key").unwrap();
        let global = global_with(DEFAULT_SERVER_URL, "alice", Some("global-key"));
        let out = resolve_from(ResolveInputs {
            cli_server: None,
            env_server: None,
            env_api_key: None,
            repo_root: Some(tmp.path()),
            global: &global,
        });
        let key = out.api_key.unwrap();
        assert_eq!(key.value, "repo-key");
        assert_eq!(key.source, ConfigSource::RepoCredentials);
    }

    #[test]
    fn test_resolve_api_key_global_fallback() {
        reset_shadow_warning_gate();
        let tmp = tempdir().unwrap();
        let global = global_with(DEFAULT_SERVER_URL, "alice", Some("global-key"));
        let out = resolve_from(ResolveInputs {
            cli_server: None,
            env_server: None,
            env_api_key: None,
            repo_root: Some(tmp.path()),
            global: &global,
        });
        let key = out.api_key.unwrap();
        assert_eq!(key.value, "global-key");
        assert_eq!(key.source, ConfigSource::GlobalFile);
    }

    #[test]
    fn test_resolve_api_key_none_when_nothing_set() {
        reset_shadow_warning_gate();
        let tmp = tempdir().unwrap();
        let global = empty_global();
        let out = resolve_from(ResolveInputs {
            cli_server: None,
            env_server: None,
            env_api_key: None,
            repo_root: Some(tmp.path()),
            global: &global,
        });
        assert!(out.api_key.is_none());
    }

    // ----- identity fields -----

    #[test]
    fn test_resolve_identity_only_from_global() {
        reset_shadow_warning_gate();
        let tmp = tempdir().unwrap();
        // Even with a repo root, identity is sourced from the global file.
        // We do not even read `<repo>/nemo.toml` for identity fields.
        std::fs::write(
            tmp.path().join("nemo.toml"),
            r#"[server]
url = "http://repo:3"
"#,
        )
        .unwrap();
        let global = global_with("http://global:4", "alice", None);
        let out = resolve_from(ResolveInputs {
            cli_server: None,
            env_server: None,
            env_api_key: None,
            repo_root: Some(tmp.path()),
            global: &global,
        });
        assert_eq!(out.engineer.value, "alice");
        assert_eq!(out.engineer.source, ConfigSource::GlobalFile);
        assert_eq!(out.name.value, "Global Name");
        assert_eq!(out.email.value, "global@example.com");
    }

    // ----- repo detection -----

    #[test]
    fn test_repo_detection_walks_up_from_subdir() {
        let tmp = tempdir().unwrap();
        std::fs::write(
            tmp.path().join("nemo.toml"),
            "[repo]\nname=\"x\"\ndefault_branch=\"main\"\n",
        )
        .unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        let found = find_repo_root(&nested).unwrap();
        // On macOS, tempdir may be under /private/var/folders and canonicalize
        // inconsistently; compare canonicalized paths.
        assert_eq!(
            found.canonicalize().unwrap(),
            tmp.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_repo_detection_handles_worktree_git_file() {
        // In worktrees, .git is a file, not a directory.
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join(".git"), "gitdir: /some/path\n").unwrap();
        let found = find_repo_root(tmp.path()).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            tmp.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_repo_detection_handles_git_directory() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let found = find_repo_root(tmp.path()).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            tmp.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_repo_detection_nemo_toml_wins_over_parent_git() {
        // Outer dir has .git, inner dir has nemo.toml. Inner should win.
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let inner = tmp.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(
            inner.join("nemo.toml"),
            "[repo]\nname=\"x\"\ndefault_branch=\"main\"\n",
        )
        .unwrap();
        let found = find_repo_root(&inner).unwrap();
        assert_eq!(found.canonicalize().unwrap(), inner.canonicalize().unwrap());
    }

    #[test]
    fn test_repo_detection_returns_none_outside_repo() {
        // Use a tempdir with no markers and make sure we do NOT walk up to an
        // ancestor repo (the working copy of this crate itself is a git repo).
        // To do that, canonicalize the tempdir and check only if the walk
        // finds something *above* the temp root's parent chain. A simpler
        // approach: use the root path as start to simulate "no repo".
        let found = find_repo_root(Path::new("/"));
        assert!(found.is_none());
    }

    // ----- no-repo fallback -----

    #[test]
    fn test_resolve_no_repo_falls_back_to_env_plus_global() {
        reset_shadow_warning_gate();
        let global = global_with("http://global:4", "alice", Some("global-key"));
        let out = resolve_from(ResolveInputs {
            cli_server: None,
            env_server: None,
            env_api_key: None,
            repo_root: None,
            global: &global,
        });
        assert_eq!(out.server_url.value, "http://global:4");
        assert_eq!(out.api_key.as_ref().unwrap().value, "global-key");
    }

    #[test]
    fn test_resolve_no_repo_env_wins() {
        reset_shadow_warning_gate();
        let global = empty_global();
        let out = resolve_from(ResolveInputs {
            cli_server: None,
            env_server: Some("http://env:2".to_string()),
            env_api_key: Some("env-key".to_string()),
            repo_root: None,
            global: &global,
        });
        assert_eq!(out.server_url.value, "http://env:2");
        assert_eq!(out.server_url.source, ConfigSource::EnvVar);
        assert_eq!(out.api_key.as_ref().unwrap().value, "env-key");
        assert_eq!(out.api_key.as_ref().unwrap().source, ConfigSource::EnvVar);
    }
}
