use async_trait::async_trait;

use crate::error::Result;

/// Trait abstracting git operations on the bare repo.
#[async_trait]
pub trait GitOperations: Send + Sync + 'static {
    /// Check if a file exists in the repo at the given ref (default branch).
    async fn spec_exists(&self, spec_path: &str) -> Result<bool>;

    /// Get the current SHA of a branch.
    async fn get_branch_sha(&self, branch: &str) -> Result<Option<String>>;

    /// Create a new branch from the default branch HEAD.
    async fn create_branch(&self, branch: &str) -> Result<String>;

    /// Read a file's content from the repo at the given ref.
    async fn read_file(&self, path: &str, git_ref: &str) -> Result<String>;

    /// Fetch from the remote.
    async fn fetch(&self) -> Result<()>;

    /// Detect if a branch has diverged from the expected SHA.
    async fn has_diverged(&self, branch: &str, expected_sha: &str) -> Result<bool>;
}

/// In-memory mock for testing.
pub mod mock {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[derive(Debug, Clone)]
    pub struct MockGitOperations {
        files: Arc<RwLock<HashMap<String, String>>>,
        branches: Arc<RwLock<HashMap<String, String>>>,
        default_sha: String,
    }

    impl MockGitOperations {
        pub fn new() -> Self {
            Self {
                files: Arc::new(RwLock::new(HashMap::new())),
                branches: Arc::new(RwLock::new(HashMap::new())),
                default_sha: "0000000000000000000000000000000000000000".to_string(),
            }
        }

        /// Add a file to the mock repo.
        pub async fn add_file(&self, path: &str, content: &str) {
            let mut files = self.files.write().await;
            files.insert(path.to_string(), content.to_string());
        }

        /// Set a branch SHA.
        pub async fn set_branch_sha(&self, branch: &str, sha: &str) {
            let mut branches = self.branches.write().await;
            branches.insert(branch.to_string(), sha.to_string());
        }
    }

    impl Default for MockGitOperations {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl GitOperations for MockGitOperations {
        async fn spec_exists(&self, spec_path: &str) -> Result<bool> {
            let files = self.files.read().await;
            Ok(files.contains_key(spec_path))
        }

        async fn get_branch_sha(&self, branch: &str) -> Result<Option<String>> {
            let branches = self.branches.read().await;
            Ok(branches.get(branch).cloned())
        }

        async fn create_branch(&self, branch: &str) -> Result<String> {
            let sha = self.default_sha.clone();
            let mut branches = self.branches.write().await;
            branches.insert(branch.to_string(), sha.clone());
            Ok(sha)
        }

        async fn read_file(&self, path: &str, _git_ref: &str) -> Result<String> {
            let files = self.files.read().await;
            files.get(path).cloned().ok_or_else(|| {
                crate::error::NemoError::Git(format!("File not found: {path}"))
            })
        }

        async fn fetch(&self) -> Result<()> {
            Ok(())
        }

        async fn has_diverged(&self, branch: &str, expected_sha: &str) -> Result<bool> {
            let branches = self.branches.read().await;
            match branches.get(branch) {
                Some(sha) => Ok(sha != expected_sha),
                None => Ok(false),
            }
        }
    }
}
