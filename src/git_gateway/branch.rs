//! Branch operations for GitGateway.

use anyhow::{bail, Context, Result};

use super::verbose_cmd;
use super::GitGateway;

impl GitGateway {
    /// Get the name of the currently checked out branch
    pub fn get_current_branch_name(&self) -> Result<String> {
        self.backend.get_current_branch()
    }

    /// Get short commit info for a branch (hash + message summary)
    /// Used by log long mode
    pub fn get_branch_commit_info(&self, branch: &str) -> Option<String> {
        let short_id = self.backend.get_short_sha(branch).ok()?;
        let message = self.backend.get_commit_subject(branch).ok()?;
        Some(format!("({} {})", short_id, message))
    }

    /// Create a new branch and switch to it
    pub fn create_branch(&self, name: &str) -> Result<()> {
        verbose_cmd("checkout", &["-b", name]);
        self.backend.create_branch(name)
    }

    /// Check if a branch exists
    pub fn branch_exists(&self, name: &str) -> Result<bool> {
        self.backend.branch_exists(name)
    }

    /// Checkout a branch (force mode)
    /// Uses force checkout to ensure working tree and index are properly reset
    /// This is used by rebase/restack operations that need to force checkout
    pub fn checkout_branch(&self, name: &str) -> Result<()> {
        verbose_cmd("checkout", &["-f", name]);
        self.backend.checkout_branch_force(name)
    }

    /// Checkout a branch with full safety checks
    /// Fails if there are uncommitted changes OR if the branch is checked out in another worktree
    /// This is the safest checkout mode for user-initiated commands
    pub fn checkout_branch_worktree_safe(&self, name: &str) -> Result<()> {
        // Check for uncommitted changes FIRST (most common case)
        if self.has_staged_or_modified_changes()? {
            bail!(
                "Cannot checkout '{}' - you have uncommitted changes.\n\
                Commit or stash your changes first:\n\
                • git add -A && git commit -m \"WIP\"\n\
                • git stash",
                name
            );
        }

        // Check if branch is in another worktree
        if let Some(worktree_path) = crate::worktree::get_worktree_path_for_branch(name)? {
            bail!(
                "Branch '{}' is already checked out at:\n  \
                 {}",
                name,
                worktree_path.display()
            );
        }

        // Safe to proceed with checkout
        verbose_cmd("checkout", &[name]);
        self.backend.checkout_branch(name)
    }

    /// List all local branch names
    pub fn list_branches(&self) -> Result<Vec<String>> {
        self.backend.list_branches()
    }

    /// Delete a local branch
    pub fn delete_branch(&self, name: &str) -> Result<()> {
        verbose_cmd("branch", &["-D", name]);
        self.backend.delete_branch(name)
    }

    /// Rename a branch
    pub fn rename_branch(&self, old_name: &str, new_name: &str) -> Result<()> {
        verbose_cmd("branch", &["-m", old_name, new_name]);
        self.backend.rename_branch(old_name, new_name)
    }

    /// Fast-forward merge a branch into the current branch
    pub fn merge_branch_ff(&self, branch_name: &str) -> Result<()> {
        // Verify fast-forward is possible using is_ancestor check
        // Current HEAD must be an ancestor of the target branch
        let current_branch = self.get_current_branch_name()?;
        if !self.backend.is_ancestor(&current_branch, branch_name)? {
            bail!(
                "Cannot fast-forward: branch '{}' has diverged from '{}'",
                current_branch,
                branch_name
            );
        }

        // Use git merge --ff-only for the actual merge
        verbose_cmd("merge", &["--ff-only", branch_name]);
        let output = std::process::Command::new("git")
            .args(["merge", "--ff-only", branch_name])
            .current_dir(&self.workdir)
            .output()
            .context(format!("Failed to fast-forward merge '{}'", branch_name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to fast-forward merge '{}': {}", branch_name, stderr.trim());
        }

        Ok(())
    }

    /// Create a branch at the current HEAD without switching to it
    pub fn create_branch_at_head(&self, name: &str) -> Result<()> {
        verbose_cmd("branch", &[name]);
        self.backend.create_branch_at(name, "HEAD")
    }

    /// Create a branch at a specific ref (commit or branch)
    pub fn create_branch_at_ref(&self, name: &str, at_ref: &str) -> Result<()> {
        verbose_cmd("branch", &[name, at_ref]);
        self.backend.create_branch_at(name, at_ref)
    }
}
