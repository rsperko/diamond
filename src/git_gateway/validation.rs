//! Validation operations for GitGateway.

use anyhow::{bail, Result};

use super::GitGateway;
use crate::program_name::program_name;

impl GitGateway {
    /// Check if working directory has uncommitted changes
    /// This includes untracked files but ignores files in .gitignore
    pub fn has_uncommitted_changes(&self) -> Result<bool> {
        self.backend.has_uncommitted_changes()
    }

    /// Check if there are any staged changes in the index
    pub fn has_staged_changes(&self) -> Result<bool> {
        self.backend.has_staged_changes()
    }

    /// Check if there are staged or modified changes (for rebase operations)
    /// Unlike has_uncommitted_changes(), this ALLOWS untracked files
    /// Rebase is fine with untracked files
    #[allow(dead_code)] // Will be used in sync/restack/move commands
    pub fn has_staged_or_modified_changes(&self) -> Result<bool> {
        self.backend.has_staged_or_modified_changes()
    }

    /// Require clean working tree for rebase operations (allows untracked files)
    pub fn require_clean_for_rebase(&self) -> Result<()> {
        if self.has_staged_or_modified_changes()? {
            bail!(
                "Cannot proceed with staged or modified changes.\n\
                Commit or stash your changes first:\n\
                • git add -A && git commit -m \"WIP\"\n\
                • git stash\n\
                \n\
                Note: Untracked files are OK and will be preserved."
            );
        }
        Ok(())
    }

    /// Require clean working tree before destructive operations
    #[allow(dead_code)] // Will be used when migrating commands
    pub fn require_clean_working_tree(&self, operation: &str) -> Result<()> {
        if self.has_uncommitted_changes()? {
            bail!(
                "Cannot {} with uncommitted changes.\n\
                Commit or stash your changes first:\n\
                • git add -A && git commit -m \"WIP\"\n\
                • git stash",
                operation
            );
        }
        Ok(())
    }

    /// Get the merge-base (common ancestor) of two branches
    /// This is useful for rebasing only the unique commits of a branch
    #[allow(dead_code)] // May be useful for future features
    pub fn get_merge_base(&self, branch1: &str, branch2: &str) -> Result<String> {
        let oid = self.backend.get_merge_base(branch1, branch2)?;
        Ok(oid.to_string())
    }

    /// Check if a branch is fully merged into target
    /// A branch is merged if its tip commit is an ancestor of the target
    pub fn is_branch_merged(&self, branch: &str, target: &str) -> Result<bool> {
        self.backend.is_branch_merged(branch, target)
    }

    /// Get short commit hash for a branch (7 characters)
    pub fn get_short_hash(&self, branch: &str) -> Result<String> {
        self.backend.get_short_sha(branch)
    }

    /// Get full commit SHA for a branch (40 characters)
    pub fn get_branch_sha(&self, branch: &str) -> Result<String> {
        let oid = self.backend.get_ref_sha(branch)?;
        Ok(oid.to_string())
    }

    /// Get commit message subject line (first line) for a branch
    pub fn get_commit_subject(&self, branch: &str) -> Result<String> {
        let subject = self.backend.get_commit_subject(branch)?;
        // Truncate to 50 chars if needed
        if subject.len() > 50 {
            Ok(format!("{}...", &subject[..47]))
        } else {
            Ok(subject)
        }
    }

    /// Get relative commit time (e.g., "2 days ago") for a branch
    pub fn get_commit_time_relative(&self, branch: &str) -> Result<String> {
        self.backend.get_commit_time_relative(branch)
    }

    /// Check if ancestor_ref is an ancestor of descendant_ref
    pub fn is_ancestor(&self, ancestor_ref: &str, descendant_ref: &str) -> Result<bool> {
        self.backend.is_ancestor(ancestor_ref, descendant_ref)
    }

    /// Validates that a parent branch exists in git
    ///
    /// Returns an actionable error if the parent doesn't exist, suggesting
    /// the user run `dm sync` or `dm move` to repair orphaned branches.
    ///
    /// # Errors
    /// Returns error if parent branch doesn't exist in git
    pub fn validate_parent_exists(&self, parent: &str) -> Result<()> {
        if !self.branch_exists(parent)? {
            bail!(
                "Parent branch '{}' does not exist in git.\n\
                This can happen if the parent was merged/deleted remotely.\n\
                \n\
                To fix:\n\
                • Run '{} sync' to automatically repair orphaned branches\n\
                • Or use '{} move --onto <existing-branch>' to manually reparent",
                parent,
                program_name(),
                program_name()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_context::{init_test_repo, TestRepoContext};
    use tempfile::tempdir;

    #[test]
    fn test_validate_parent_exists_when_parent_exists() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create a branch that will serve as the parent
        gateway.create_branch("parent-branch")?;

        // Validate should succeed since parent-branch exists
        let result = gateway.validate_parent_exists("parent-branch");
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_validate_parent_exists_when_parent_missing() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Try to validate a branch that doesn't exist
        let result = gateway.validate_parent_exists("nonexistent-branch");

        // Should fail
        assert!(result.is_err());

        // Check error message contains expected text
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("does not exist in git"));
        assert!(err_msg.contains("nonexistent-branch"));
        assert!(err_msg.contains("sync") || err_msg.contains("move"));

        Ok(())
    }

    #[test]
    fn test_validate_parent_exists_trunk_always_valid() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // main branch exists by default (trunk)
        let result = gateway.validate_parent_exists("main");
        assert!(result.is_ok());

        Ok(())
    }
}
