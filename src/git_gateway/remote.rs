//! Remote operations for GitGateway.

use anyhow::{bail, Context, Result};

use super::verbose_cmd;
use super::GitGateway;

/// Represents the sync state between a local branch and its remote tracking branch.
///
/// This is used to detect divergence before operations like submit that could
/// overwrite remote changes made by coworkers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchSyncState {
    /// Local and remote are at the same commit
    InSync,
    /// Local has commits not yet pushed to remote
    Ahead(usize),
    /// Remote has commits not yet pulled to local
    Behind(usize),
    /// Both local and remote have diverged from their common ancestor
    Diverged { local_ahead: usize, remote_ahead: usize },
    /// No remote tracking branch exists (new branch or untracked)
    NoRemote,
}

/// Result of syncing a branch from remote
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncBranchResult {
    /// Branch was updated with N commits from remote
    Updated(usize),
    /// Branch was already in sync
    AlreadySynced,
    /// Local has commits not on remote (normal before submit)
    LocalAhead(usize),
    /// Branch was diverged but force-synced to remote
    ForceSynced,
    /// Branch is diverged - requires --force to sync
    Diverged { local_ahead: usize, remote_ahead: usize },
    /// No remote tracking branch exists
    NoRemote,
}

impl GitGateway {
    /// Fetch from remote using the git command (for reliable credential handling)
    pub fn fetch_remote(&self, remote: &str) -> Result<()> {
        verbose_cmd("fetch", &[remote]);

        // Verify remote exists first
        if !self.has_remote(remote)? {
            bail!(
                "No remote '{}' configured. Add one with: git remote add {} <url>",
                remote,
                remote
            );
        }

        // Shell out to git for fetch - this properly uses credential helpers (osxkeychain, etc.)
        let output = std::process::Command::new("git")
            .args(["fetch", remote])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git fetch")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "Authentication failed. Check:\n\
                • SSH keys are set up and added to agent (ssh-add -l)\n\
                • Remote URL is correct (git remote -v)\n\
                \nGit error: {}",
                stderr.trim()
            );
        }

        Ok(())
    }

    /// Fetch from the configured remote
    pub fn fetch_origin(&self) -> Result<()> {
        self.fetch_remote(&self.remote)
    }

    /// Fast-forward a branch to its upstream
    pub fn fast_forward_branch(&self, branch: &str) -> Result<()> {
        // Checkout the branch
        self.checkout_branch(branch)?;

        // Use git merge --ff-only
        let upstream_name = format!("{}/{}", self.remote, branch);

        verbose_cmd("merge", &["--ff-only", &upstream_name]);

        let output = std::process::Command::new("git")
            .args(["merge", "--ff-only", &upstream_name])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git merge --ff-only")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to fast-forward branch: {}", stderr.trim());
        }

        Ok(())
    }

    /// Get the URL for a remote by name
    ///
    /// # Arguments
    /// * `remote_name` - Name of the remote (e.g., "origin")
    ///
    /// # Returns
    /// The URL configured for the remote
    ///
    /// # Errors
    /// Returns error if remote doesn't exist or URL is not valid UTF-8
    pub fn get_remote_url(&self, remote_name: &str) -> Result<String> {
        let output = std::process::Command::new("git")
            .args(["remote", "get-url", remote_name])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git remote get-url")?;

        if !output.status.success() {
            bail!("No '{}' remote configured", remote_name);
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Check the sync state between a local branch and its remote tracking branch.
    ///
    /// This detects whether the local branch has diverged from the remote, which is
    /// important for collaborative workflows where multiple developers may push to
    /// the same branch.
    ///
    /// # Arguments
    /// * `branch` - The local branch name to check
    /// * `remote` - The remote name (e.g., "origin")
    ///
    /// # Returns
    /// * `InSync` - Local and remote are at the same commit
    /// * `Ahead(n)` - Local has n commits not yet pushed
    /// * `Behind(n)` - Remote has n commits not yet pulled
    /// * `Diverged { local_ahead, remote_ahead }` - Both have diverged
    /// * `NoRemote` - No remote tracking branch exists
    pub fn check_remote_sync_with_remote(&self, branch: &str, remote: &str) -> Result<BranchSyncState> {
        let remote_branch = format!("{}/{}", remote, branch);

        // Check if remote tracking branch exists
        let check_output = std::process::Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/{}", remote_branch),
            ])
            .current_dir(&self.workdir)
            .status()
            .context("Failed to run git show-ref")?;

        if !check_output.success() {
            return Ok(BranchSyncState::NoRemote);
        }

        // Use git rev-list --left-right --count to get ahead/behind
        let output = std::process::Command::new("git")
            .args([
                "rev-list",
                "--left-right",
                "--count",
                &format!("{}...{}", branch, remote_branch),
            ])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git rev-list")?;

        if !output.status.success() {
            // No common ancestor - treat as diverged
            return Ok(BranchSyncState::Diverged {
                local_ahead: 1,
                remote_ahead: 1,
            });
        }

        let counts = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = counts.split_whitespace().collect();
        if parts.len() != 2 {
            return Ok(BranchSyncState::InSync);
        }

        let local_ahead: usize = parts[0].parse().unwrap_or(0);
        let remote_ahead: usize = parts[1].parse().unwrap_or(0);

        match (local_ahead, remote_ahead) {
            (0, 0) => Ok(BranchSyncState::InSync),
            (n, 0) => Ok(BranchSyncState::Ahead(n)),
            (0, n) => Ok(BranchSyncState::Behind(n)),
            (l, r) => Ok(BranchSyncState::Diverged {
                local_ahead: l,
                remote_ahead: r,
            }),
        }
    }

    /// Check the sync state between a local branch and the configured remote
    pub fn check_remote_sync(&self, branch: &str) -> Result<BranchSyncState> {
        self.check_remote_sync_with_remote(branch, &self.remote)
    }

    /// Sync a local branch from its remote tracking branch.
    ///
    /// This is the "pull" operation for collaborative workflows. It handles:
    /// - Fast-forward when behind
    /// - Force-reset when diverged (if force=true)
    /// - Warning when diverged (if force=false)
    ///
    /// # Arguments
    /// * `branch` - The local branch name to sync
    /// * `remote` - The remote name (e.g., "origin")
    /// * `force` - If true, reset to remote even when diverged
    ///
    /// # Returns
    /// * `Updated(n)` - Fast-forwarded with n commits
    /// * `AlreadySynced` - Already up to date
    /// * `LocalAhead(n)` - Local has n commits not on remote
    /// * `ForceSynced` - Was diverged, force-reset to remote
    /// * `Diverged` - Diverged and requires --force
    /// * `NoRemote` - No remote tracking branch
    pub fn sync_branch_from_remote_with_name(
        &self,
        branch: &str,
        remote: &str,
        force: bool,
    ) -> Result<SyncBranchResult> {
        // First check the sync state
        let sync_state = self.check_remote_sync_with_remote(branch, remote)?;

        match sync_state {
            BranchSyncState::NoRemote => Ok(SyncBranchResult::NoRemote),

            BranchSyncState::InSync => Ok(SyncBranchResult::AlreadySynced),

            BranchSyncState::Ahead(n) => Ok(SyncBranchResult::LocalAhead(n)),

            BranchSyncState::Behind(n) => {
                // Fast-forward to remote
                self.fast_forward_to_remote_with_name(branch, remote)?;
                Ok(SyncBranchResult::Updated(n))
            }

            BranchSyncState::Diverged {
                local_ahead,
                remote_ahead,
            } => {
                if force {
                    // Reset to remote
                    self.reset_branch_to_remote_with_name(branch, remote)?;
                    Ok(SyncBranchResult::ForceSynced)
                } else {
                    Ok(SyncBranchResult::Diverged {
                        local_ahead,
                        remote_ahead,
                    })
                }
            }
        }
    }

    /// Sync a local branch from the configured remote (convenience method)
    pub fn sync_branch_from_remote(&self, branch: &str, force: bool) -> Result<SyncBranchResult> {
        self.sync_branch_from_remote_with_name(branch, &self.remote, force)
    }

    /// Fast-forward a branch to its remote tracking branch
    fn fast_forward_to_remote_with_name(&self, branch: &str, remote: &str) -> Result<()> {
        let remote_branch = format!("{}/{}", remote, branch);
        let local_ref = format!("refs/heads/{}", branch);
        let remote_ref = format!("refs/remotes/{}", remote_branch);

        // Get the remote commit SHA
        let output = std::process::Command::new("git")
            .args(["rev-parse", &remote_ref])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git rev-parse")?;

        if !output.status.success() {
            bail!("No remote tracking branch for '{}'", branch);
        }

        let remote_sha = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Update local branch to point to remote commit
        let output = std::process::Command::new("git")
            .args(["update-ref", &local_ref, &remote_sha])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git update-ref")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to fast-forward branch: {}", stderr.trim());
        }

        Ok(())
    }

    /// Reset a branch to its remote tracking branch (discards local commits)
    fn reset_branch_to_remote_with_name(&self, branch: &str, remote: &str) -> Result<()> {
        // Implementation is the same as fast-forward - just update the ref
        self.fast_forward_to_remote_with_name(branch, remote)
    }

    /// Stash uncommitted changes
    ///
    /// Returns true if changes were stashed, false if nothing to stash
    /// Uses --include-untracked to stash untracked files as well
    pub fn stash_push(&self, message: &str) -> Result<bool> {
        // Check if there's anything to stash
        if !self.has_uncommitted_changes()? {
            return Ok(false);
        }

        verbose_cmd("stash", &["push", "--include-untracked", "-m", message]);

        let output = std::process::Command::new("git")
            .args(["stash", "push", "--include-untracked", "-m", message])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git stash push")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to stash changes: {}", stderr.trim());
        }

        Ok(true)
    }

    /// Pop the most recent stash
    pub fn stash_pop(&self) -> Result<()> {
        verbose_cmd("stash", &["pop"]);

        let output = std::process::Command::new("git")
            .args(["stash", "pop"])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git stash pop")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to pop stash: {}", stderr.trim());
        }

        Ok(())
    }

    /// Delete a branch from a remote
    ///
    /// Equivalent to `git push <remote> --delete <branch>`
    pub fn delete_remote_branch_with_name(&self, branch: &str, remote: &str) -> Result<()> {
        verbose_cmd("push", &[remote, "--delete", branch]);

        let output = std::process::Command::new("git")
            .args(["push", remote, "--delete", branch])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git push --delete")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to delete remote branch: {}", stderr.trim());
        }

        Ok(())
    }

    /// Delete a branch from the configured remote (convenience method)
    pub fn delete_remote_branch(&self, branch: &str) -> Result<()> {
        self.delete_remote_branch_with_name(branch, &self.remote)
    }
}
