//! Per-repo credentials file: `<repo>/.nemo/credentials`.
//!
//! Format: raw API key, trimmed, single line. No JSON, no TOML — mirrors how
//! SSH private keys work: one file, one secret, mode-enforced.
//!
//! See `specs/per-repo-config.md` FR-2, FR-9, NFR-3.
//!
//! The public items below are consumed by `config::sources::resolve` and the
//! `nemo config --local --set api_key=...` path. `dead_code` is suppressed at
//! the module level during early steps because the consumers land in later
//! steps of the same spec; the allow is kept narrow to this file.
#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Relative path of the credentials file within a repo.
pub const CREDENTIALS_RELATIVE_PATH: &str = ".nemo/credentials";

/// Compute the credentials file path for a given repo root.
pub fn credentials_path(repo_root: &Path) -> PathBuf {
    repo_root.join(CREDENTIALS_RELATIVE_PATH)
}

/// Read the API key from `<repo_root>/.nemo/credentials`.
///
/// Returns:
/// * `Ok(None)` if the file does not exist.
/// * `Ok(Some(key))` with the trimmed, single-line content if the file exists
///   and is non-empty.
/// * `Ok(None)` if the file exists but is empty after trimming (treated as
///   "not set" so the caller falls through to the next source).
/// * `Err(...)` only on IO errors (read failure).
///
/// On unix, if the file is readable by group or other (mode broader than
/// 0600), a warning is printed to stderr. The key is still returned — we
/// do not auto-fix (principle of least surprise).
pub fn read_credentials(repo_root: &Path) -> Result<Option<String>> {
    let path = credentials_path(repo_root);
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    check_mode_warn(&path);

    let trimmed = contents.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

#[cfg(unix)]
fn check_mode_warn(path: &Path) {
    use std::os::unix::fs::MetadataExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.mode() & 0o777;
        if mode & 0o077 != 0 {
            eprintln!(
                "warning: {} is mode {:o}, should be 0600. \
                 Fix with: chmod 600 {}",
                path.display(),
                mode,
                path.display()
            );
        }
    }
}

#[cfg(not(unix))]
fn check_mode_warn(_path: &Path) {
    // Windows/non-unix: no POSIX mode, skip check.
}

/// Write the API key to `<repo_root>/.nemo/credentials` atomically with mode 0600.
///
/// Behavior:
/// * Creates `<repo_root>/.nemo/` if missing.
/// * Writes to `<repo_root>/.nemo/credentials.tmp` with mode 0600 (unix),
///   flushes, then renames to the final path. This guarantees the file is
///   never world-readable during the write.
/// * Appends a single trailing newline (POSIX convention).
/// * Rejects an empty key with an error — empty keys would silently break
///   authentication with no recourse.
pub fn write_credentials(repo_root: &Path, api_key: &str) -> Result<()> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        bail!("refusing to write empty api_key to credentials file");
    }

    let dir = repo_root.join(".nemo");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create directory {}", dir.display()))?;

    let final_path = credentials_path(repo_root);
    let tmp_path = dir.join("credentials.tmp");

    let mut body = String::with_capacity(trimmed.len() + 1);
    body.push_str(trimmed);
    body.push('\n');

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)
            .with_context(|| format!("failed to open {}", tmp_path.display()))?;
        file.write_all(body.as_bytes())
            .with_context(|| format!("failed to write {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to fsync {}", tmp_path.display()))?;
    }

    #[cfg(not(unix))]
    {
        std::fs::write(&tmp_path, body.as_bytes())
            .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    }

    std::fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "failed to rename {} -> {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_read_missing_returns_none() {
        let tmp = tempdir().unwrap();
        let result = read_credentials(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_write_then_read_roundtrip() {
        let tmp = tempdir().unwrap();
        write_credentials(tmp.path(), "abc123").unwrap();
        let result = read_credentials(tmp.path()).unwrap();
        assert_eq!(result.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_write_rejects_empty_key() {
        let tmp = tempdir().unwrap();
        assert!(write_credentials(tmp.path(), "").is_err());
        assert!(write_credentials(tmp.path(), "   ").is_err());
        assert!(write_credentials(tmp.path(), "\n").is_err());
    }

    #[test]
    fn test_read_trims_trailing_newline_and_whitespace() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join(".nemo");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("credentials"), "  secret-key  \n\n").unwrap();
        let result = read_credentials(tmp.path()).unwrap();
        assert_eq!(result.as_deref(), Some("secret-key"));
    }

    #[test]
    fn test_read_empty_file_returns_none() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join(".nemo");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("credentials"), "   \n").unwrap();
        let result = read_credentials(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn test_write_creates_file_mode_0600() {
        use std::os::unix::fs::MetadataExt;
        let tmp = tempdir().unwrap();
        write_credentials(tmp.path(), "secret").unwrap();
        let path = credentials_path(tmp.path());
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "expected mode 0600 on credentials file, got {mode:o}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_write_overwrites_existing() {
        let tmp = tempdir().unwrap();
        write_credentials(tmp.path(), "first").unwrap();
        write_credentials(tmp.path(), "second").unwrap();
        let result = read_credentials(tmp.path()).unwrap();
        assert_eq!(result.as_deref(), Some("second"));
    }

    #[cfg(unix)]
    #[test]
    fn test_read_still_returns_key_when_mode_is_loose() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir().unwrap();
        write_credentials(tmp.path(), "secret").unwrap();
        // Manually relax the mode to 0644.
        let path = credentials_path(tmp.path());
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&path, perms).unwrap();

        // Should still return the key (a warning is printed to stderr but the
        // read succeeds — principle of least surprise).
        let result = read_credentials(tmp.path()).unwrap();
        assert_eq!(result.as_deref(), Some("secret"));
    }
}
