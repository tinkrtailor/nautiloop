//! Branch naming utilities for agent branches.
//!
//! Branch names follow the pattern: `agent/{engineer}/{spec-slug}-{short-hash}`
//! where `short-hash` is the first 8 hex chars of SHA-256 of the ORIGINAL
//! spec file content at submission time.
//!
//! This module re-exports the core `generate_branch_name` function from `types`
//! and provides additional branch-related utilities.

pub use crate::types::generate_branch_name;

/// Extract the engineer name from a branch name.
///
/// Branch format: `agent/{engineer}/{slug}-{hash}`
pub fn extract_engineer(branch: &str) -> Option<&str> {
    let parts: Vec<&str> = branch.splitn(3, '/').collect();
    if parts.len() >= 3 && parts[0] == "agent" {
        Some(parts[1])
    } else {
        None
    }
}

/// Extract the spec slug (without hash) from a branch name.
///
/// Branch format: `agent/{engineer}/{slug}-{hash}`
pub fn extract_slug(branch: &str) -> Option<&str> {
    let parts: Vec<&str> = branch.splitn(3, '/').collect();
    if parts.len() >= 3 && parts[0] == "agent" {
        let slug_hash = parts[2];
        // The hash is the last 8 chars after the last hyphen
        if let Some(last_hyphen) = slug_hash.rfind('-') {
            Some(&slug_hash[..last_hyphen])
        } else {
            Some(slug_hash)
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_engineer() {
        assert_eq!(
            extract_engineer("agent/alice/invoice-cancel-a1b2c3d4"),
            Some("alice")
        );
        assert_eq!(extract_engineer("agent/bob/my-spec-12345678"), Some("bob"));
        assert_eq!(extract_engineer("main"), None);
        assert_eq!(extract_engineer("feature/foo"), None);
    }

    #[test]
    fn test_extract_slug() {
        assert_eq!(
            extract_slug("agent/alice/invoice-cancel-a1b2c3d4"),
            Some("invoice-cancel")
        );
        assert_eq!(extract_slug("agent/bob/my-spec-12345678"), Some("my-spec"));
        assert_eq!(extract_slug("main"), None);
    }

    #[test]
    fn test_generate_branch_name_format() {
        let branch =
            generate_branch_name("alice", "specs/billing/invoice-cancel.md", "spec content");
        assert!(branch.starts_with("agent/alice/invoice-cancel-"));
        // 8 hex chars after the last hyphen
        let parts: Vec<&str> = branch.rsplitn(2, '-').collect();
        assert_eq!(parts[0].len(), 8);
        assert!(parts[0].chars().all(|c| c.is_ascii_hexdigit()));
    }
}
