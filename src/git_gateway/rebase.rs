//! Rebase operations for GitGateway.

use anyhow::{bail, Context, Result};
use colored::Colorize;

use crate::program_name::program_name;

use super::verbose_cmd;
use super::GitGateway;

/// Outcome of a rebase operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseOutcome {
    /// Rebase completed successfully
    Success,
    /// Rebase paused due to conflicts requiring user resolution
    Conflicts,
}

impl RebaseOutcome {
    /// Returns true if the rebase has conflicts requiring resolution
    pub fn has_conflicts(&self) -> bool {
        matches!(self, RebaseOutcome::Conflicts)
    }
}

impl GitGateway {
    /// Rebase a branch onto a new base
    /// Returns Ok(Success) if successful, Ok(Conflicts) if there are conflicts, Err for other failures
    ///
    /// NOTE: git2 doesn't have direct rebase support, so we use Command for now
    /// This is a known limitation that we'll accept for V1.0
    #[allow(dead_code)] // Will be used when migrating commands
    pub fn rebase_onto(&self, branch: &str, onto: &str) -> Result<RebaseOutcome> {
        // Check for uncommitted changes before proceeding
        if self.has_staged_or_modified_changes()? {
            bail!(
                "Cannot rebase - you have uncommitted changes.\n\
                Commit or stash your changes first:\n\
                • git add -A && git commit -m \"WIP\"\n\
                • git stash"
            );
        }

        // Checkout the branch first (with worktree safety check)
        self.checkout_branch_worktree_safe(branch)?;

        // Rebase onto the new base using git command
        verbose_cmd("rebase", &[onto]);
        let output = std::process::Command::new("git")
            .args(["rebase", onto])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git rebase")?;

        if output.status.success() {
            return Ok(RebaseOutcome::Success);
        }

        // Check if rebase is in progress (real conflicts)
        if self.rebase_in_progress()? {
            return Ok(RebaseOutcome::Conflicts);
        }

        // Not conflicts - it's an error
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git rebase failed: {}", stderr.trim());
    }

    /// Rebase a branch onto its parent using --fork-point
    /// This uses the reflog to determine the correct fork point, which handles
    /// the case where the parent branch has been amended
    /// Command: git rebase --fork-point <onto>
    pub fn rebase_fork_point(&self, branch: &str, onto: &str) -> Result<RebaseOutcome> {
        // Checkout the branch first
        self.checkout_branch(branch)?;

        // Rebase using --fork-point which uses reflog to find correct base
        verbose_cmd("rebase", &["--fork-point", onto]);
        let output = std::process::Command::new("git")
            .args(["rebase", "--fork-point", onto])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git rebase --fork-point")?;

        if output.status.success() {
            return Ok(RebaseOutcome::Success);
        }

        // Check if rebase is in progress (real conflicts)
        if self.rebase_in_progress()? {
            return Ok(RebaseOutcome::Conflicts);
        }

        // If --fork-point fails (no reflog), fall back to regular rebase
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("fatal:") {
            // Warn user about fallback - this may include extra commits
            eprintln!(
                "{} fork-point detection failed for '{}' (no reflog?), using standard rebase",
                "⚠".yellow(),
                branch
            );
            eprintln!("  This may include extra commits. If you see unexpected conflicts,");
            eprintln!(
                "  run '{} abort' and manually rebase with the correct base.",
                program_name()
            );
            // Try without --fork-point as fallback
            return self.rebase_onto(branch, onto);
        }

        bail!("git rebase --fork-point failed: {}", stderr.trim());
    }

    /// Rebase a branch onto a new base, excluding commits from old_base
    /// This is used by delete --reparent to move only the child's unique commits
    /// Command: git rebase --onto <new_base> <old_base> <branch>
    pub fn rebase_onto_from(&self, branch: &str, new_base: &str, old_base: &str) -> Result<RebaseOutcome> {
        // Checkout the branch first (with worktree safety check)
        self.checkout_branch_worktree_safe(branch)?;

        // Rebase using --onto to specify both new base and old base
        verbose_cmd("rebase", &["--onto", new_base, old_base]);
        let output = std::process::Command::new("git")
            .args(["rebase", "--onto", new_base, old_base])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git rebase --onto")?;

        if output.status.success() {
            return Ok(RebaseOutcome::Success);
        }

        // Check if rebase is in progress (real conflicts)
        if self.rebase_in_progress()? {
            return Ok(RebaseOutcome::Conflicts);
        }

        // Not conflicts - it's an error
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git rebase --onto failed: {}", stderr.trim());
    }

    /// Abort an in-progress rebase
    #[allow(dead_code)] // Will be used when migrating commands
    pub fn rebase_abort(&self) -> Result<()> {
        let status = std::process::Command::new("git")
            .args(["rebase", "--abort"])
            .current_dir(&self.workdir)
            .status()
            .context("Failed to run git rebase --abort")?;

        if !status.success() {
            bail!("git rebase --abort failed");
        }
        Ok(())
    }

    /// Continue a rebase after resolving conflicts
    #[allow(dead_code)] // Will be used when migrating commands
    pub fn rebase_continue(&self) -> Result<RebaseOutcome> {
        // Use GIT_EDITOR=true to suppress the editor (takes precedence over config)
        // Also redirect stdin from /dev/null to prevent any interactive prompts from blocking
        use std::process::Stdio;

        let output = std::process::Command::new("git")
            .args(["rebase", "--continue"])
            .env("GIT_EDITOR", "true")
            .stdin(Stdio::null())
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git rebase --continue")?;

        if output.status.success() {
            Ok(RebaseOutcome::Success)
        } else {
            Ok(RebaseOutcome::Conflicts)
        }
    }

    /// Check if there's a rebase in progress
    #[allow(dead_code)] // Will be used when migrating commands
    pub fn rebase_in_progress(&self) -> Result<bool> {
        // Check for rebase-merge or rebase-apply directories
        let rebase_merge = self.git_dir.join("rebase-merge");
        let rebase_apply = self.git_dir.join("rebase-apply");

        Ok(rebase_merge.exists() || rebase_apply.exists())
    }

    /// Check if a branch is already based on another branch
    /// (i.e., the merge-base equals the base tip, meaning base is an ancestor of branch)
    /// This is used for crash recovery to skip already-rebased branches
    pub fn is_branch_based_on(&self, branch: &str, base: &str) -> Result<bool> {
        // base is an ancestor of branch means branch is based on base
        self.backend.is_ancestor(base, branch)
    }
}
