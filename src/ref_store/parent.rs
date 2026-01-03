//! Parent relationship operations for RefStore.

use anyhow::{Context, Result};
use std::collections::HashSet;

use super::{RefStore, PARENT_REF_PREFIX};

/// Validate parent branch name from blob content
///
/// Detects corruption from:
/// - Empty blobs (filesystem corruption, incomplete writes)
/// - Invalid characters (path traversal, control characters)
/// - Excessive length (>255 chars, git branch name limit)
///
/// # Errors
/// Returns an error if the parent name is corrupted
///
/// **Note:** This is public to allow `dm doctor` to validate refs manually.
/// Normal code should use `get_parent()` which validates automatically.
pub fn validate_parent_name(parent: &str, branch: &str) -> Result<()> {
    // Check empty or whitespace-only
    if parent.trim().is_empty() {
        anyhow::bail!(
            "Corrupted metadata: parent ref for branch '{}' contains empty value.\n\
             Run 'dm doctor --fix' to repair.",
            branch
        );
    }

    // Check for path traversal
    if parent.contains("..") || parent.contains('/') {
        anyhow::bail!(
            "Corrupted metadata: parent ref for branch '{}' contains invalid characters: '{}'.\n\
             Run 'dm doctor --fix' to repair.",
            branch,
            parent
        );
    }

    // Check for control characters (null bytes, newlines, etc.)
    if parent.chars().any(|c| c.is_control()) {
        anyhow::bail!(
            "Corrupted metadata: parent ref for branch '{}' contains control characters.\n\
             Run 'dm doctor --fix' to repair.",
            branch
        );
    }

    // Check max length (git branch names max 255 chars)
    if parent.len() > 255 {
        anyhow::bail!(
            "Corrupted metadata: parent ref for branch '{}' exceeds max length.\n\
             Run 'dm doctor --fix' to repair.",
            branch
        );
    }

    Ok(())
}

#[allow(dead_code)]
impl RefStore {
    /// Set a branch's parent (creates blob with parent name, ref points to blob)
    ///
    /// Creates: refs/diamond/parent/<branch> -> blob("<parent>")
    ///
    /// # Safety
    /// This operation validates that the parent branch exists before writing,
    /// preventing orphaned relationships pointing to non-existent branches.
    /// Uses git's atomic reference update mechanism. If the process crashes
    /// between blob creation and ref update, only an orphan blob remains (harmless).
    ///
    /// # Errors
    /// Returns an error if:
    /// - Branch and parent are the same (self-referential)
    /// - The parent branch doesn't exist
    pub fn set_parent(&self, branch: &str, parent: &str) -> Result<()> {
        // Reject self-referential parents
        if branch == parent {
            anyhow::bail!(
                "Branch '{}' cannot be its own parent. This would create a circular reference.",
                branch
            );
        }

        // Validate parent branch exists (critical safety check)
        // This prevents orphaned parent refs pointing to non-existent branches
        if !self.gateway.branch_exists(parent)? {
            anyhow::bail!(
                "Parent branch '{}' does not exist. Cannot set parent relationship.",
                parent
            );
        }

        let ref_name = format!("{}{}", PARENT_REF_PREFIX, branch);

        // Create blob containing parent branch name
        let blob_oid = self
            .gateway
            .create_blob(parent.as_bytes())
            .context("Failed to create parent blob")?;

        // Create/update ref pointing to the blob
        // Note: git2's reference() is atomic at the filesystem level
        self.gateway
            .create_reference(
                &ref_name,
                &blob_oid,
                true, // force: overwrite if exists
                &format!("dm: set parent of {} to {}", branch, parent),
            )
            .context(format!("Failed to create parent ref for {}", branch))?;

        Ok(())
    }

    /// Get a branch's parent without validation (for diagnostic tools only)
    ///
    /// **Warning:** This bypasses CRITICAL-8 corruption detection.
    /// Only use in `dm doctor` or other diagnostic tools that need to inspect
    /// potentially corrupted data.
    ///
    /// Returns None if branch has no parent (is trunk or untracked)
    ///
    /// # Normal Usage
    /// Use `get_parent()` instead, which validates the parent name automatically.
    pub fn get_parent_unchecked(&self, branch: &str) -> Result<Option<String>> {
        let ref_name = format!("{}{}", PARENT_REF_PREFIX, branch);
        self.read_ref_as_string(&ref_name)
    }

    /// Get a branch's parent (reads blob content from ref with validation)
    ///
    /// Returns None if branch has no parent (is trunk or untracked)
    ///
    /// This method validates the parent name to detect corruption (CRITICAL-8).
    /// If you need to inspect potentially corrupted refs (e.g., in `dm doctor`),
    /// use `get_parent_unchecked()` instead.
    pub fn get_parent(&self, branch: &str) -> Result<Option<String>> {
        match self.get_parent_unchecked(branch)? {
            Some(parent) => {
                // Validate blob content before returning
                validate_parent_name(&parent, branch)?;
                Ok(Some(parent))
            }
            None => Ok(None),
        }
    }

    /// Remove a branch's parent ref (for untracking or deletion)
    pub fn remove_parent(&self, branch: &str) -> Result<()> {
        let ref_name = format!("{}{}", PARENT_REF_PREFIX, branch);
        // delete_reference is idempotent - returns Ok even if ref doesn't exist
        self.gateway
            .delete_reference(&ref_name)
            .context(format!("Failed to delete parent ref for {}", branch))
    }

    /// Get all children of a branch (derived by scanning all parent refs)
    ///
    /// This is O(n) where n = number of tracked branches
    pub fn get_children(&self, parent: &str) -> Result<HashSet<String>> {
        let mut children = HashSet::new();

        // Scan all parent refs
        let pattern = format!("{}*", PARENT_REF_PREFIX);
        for (ref_name, oid) in self.gateway.list_references(&pattern)? {
            // Read the blob content to get the parent name
            if let Ok(blob_content) = self.gateway.read_blob(&oid) {
                let stored_parent = String::from_utf8_lossy(&blob_content).to_string();
                if stored_parent == parent {
                    // Extract child name from refs/diamond/parent/<name>
                    if let Some(child) = ref_name.strip_prefix(PARENT_REF_PREFIX) {
                        children.insert(child.to_string());
                    }
                }
            }
        }

        Ok(children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_context::TestRepoContext;
    use tempfile::tempdir;

    fn init_test_repo(path: &std::path::Path) -> anyhow::Result<git2::Repository> {
        let repo = git2::Repository::init(path)?;
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
        drop(tree);
        Ok(repo)
    }

    // CRITICAL-8: Tests for blob content validation

    #[test]
    fn test_detect_empty_blob() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create a branch
        repo.branch("feature", &repo.head()?.peel_to_commit()?, false)?;

        // Corrupt with empty blob
        let empty_blob = repo.blob(b"")?;
        repo.reference("refs/diamond/parent/feature", empty_blob, true, "corrupt")?;

        // Try to read parent (should fail with corruption error)
        let result = ref_store.get_parent("feature");

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("corrupt") || err.contains("empty"), "Error was: {}", err);

        Ok(())
    }

    #[test]
    fn test_detect_whitespace_only_blob() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create a branch
        repo.branch("feature", &repo.head()?.peel_to_commit()?, false)?;

        // Corrupt with whitespace-only blob
        let ws_blob = repo.blob(b"   \n\t  ")?;
        repo.reference("refs/diamond/parent/feature", ws_blob, true, "corrupt")?;

        // Try to read parent (should fail)
        let result = ref_store.get_parent("feature");

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("corrupt") || err.contains("empty"), "Error was: {}", err);

        Ok(())
    }

    #[test]
    fn test_detect_path_traversal() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create a branch
        repo.branch("feature", &repo.head()?.peel_to_commit()?, false)?;

        // Corrupt with path traversal
        let evil_blob = repo.blob(b"../../../etc/passwd")?;
        repo.reference("refs/diamond/parent/feature", evil_blob, true, "corrupt")?;

        // Try to read parent (should fail)
        let result = ref_store.get_parent("feature");

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("corrupt") || err.contains("invalid"), "Error was: {}", err);

        Ok(())
    }

    #[test]
    fn test_detect_control_characters() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create a branch
        repo.branch("feature", &repo.head()?.peel_to_commit()?, false)?;

        // Corrupt with control characters
        let evil_blob = repo.blob(b"branch\0name")?;
        repo.reference("refs/diamond/parent/feature", evil_blob, true, "corrupt")?;

        // Try to read parent (should fail)
        let result = ref_store.get_parent("feature");

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("corrupt") || err.contains("control"), "Error was: {}", err);

        Ok(())
    }

    #[test]
    fn test_detect_excessive_length() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create a branch
        repo.branch("feature", &repo.head()?.peel_to_commit()?, false)?;

        // Corrupt with very long name (>255 chars)
        let long_name = "a".repeat(300);
        let evil_blob = repo.blob(long_name.as_bytes())?;
        repo.reference("refs/diamond/parent/feature", evil_blob, true, "corrupt")?;

        // Try to read parent (should fail)
        let result = ref_store.get_parent("feature");

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("corrupt") || err.contains("length"), "Error was: {}", err);

        Ok(())
    }

    #[test]
    fn test_valid_parent_name_accepted() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create feature branch (main already exists from init_test_repo)
        repo.branch("feature", &repo.head()?.peel_to_commit()?, false)?;

        // Set valid parent
        let parent_blob = repo.blob(b"main")?;
        repo.reference("refs/diamond/parent/feature", parent_blob, true, "set parent")?;

        // Should read successfully
        let result = ref_store.get_parent("feature")?;
        assert_eq!(result, Some("main".to_string()));

        Ok(())
    }

    #[test]
    fn test_detect_git_ref_style_names() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create a branch
        repo.branch("feature", &repo.head()?.peel_to_commit()?, false)?;

        // Corrupt with full git ref path (should be just branch name)
        let evil_blob = repo.blob(b"refs/heads/main")?;
        repo.reference("refs/diamond/parent/feature", evil_blob, true, "corrupt")?;

        // Try to read parent (should fail - contains '/')
        let result = ref_store.get_parent("feature");

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("corrupt") || err.contains("invalid"), "Error was: {}", err);

        Ok(())
    }

    #[test]
    fn test_get_children_handles_corrupted_refs() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create parent branch
        repo.branch("parent", &repo.head()?.peel_to_commit()?, false)?;

        // Create two children: one valid, one corrupted
        repo.branch("child-valid", &repo.head()?.peel_to_commit()?, false)?;
        repo.branch("child-corrupt", &repo.head()?.peel_to_commit()?, false)?;

        // Set valid parent ref for child-valid
        let valid_blob = repo.blob(b"parent")?;
        repo.reference("refs/diamond/parent/child-valid", valid_blob, true, "set parent")?;

        // Set corrupted parent ref for child-corrupt (empty blob)
        let corrupt_blob = repo.blob(b"")?;
        repo.reference("refs/diamond/parent/child-corrupt", corrupt_blob, true, "corrupt")?;

        // get_children() should handle this gracefully
        // It reads blob content but doesn't validate (performance optimization)
        let children = ref_store.get_children("parent")?;

        // Should find at least the valid child
        // The corrupted one will have empty string as parent, so won't match
        assert!(children.contains("child-valid"));
        assert!(
            !children.contains("child-corrupt"),
            "Corrupted ref should not match parent"
        );

        Ok(())
    }
}
