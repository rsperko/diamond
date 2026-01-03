//! Commit operations for GitGateway.

use anyhow::{bail, Context, Result};

use crate::program_name::program_name;

use super::GitGateway;

impl GitGateway {
    /// Stage all changes (git add -A)
    pub fn stage_all(&self) -> Result<()> {
        self.backend.stage_all()
    }

    /// Stage only updates to already-tracked files (git add -u)
    ///
    /// This updates index entries to match the working directory for files
    /// that are already tracked, without adding new untracked files.
    pub fn stage_updates(&self) -> Result<()> {
        self.backend.stage_updates()
    }

    /// Stage a specific file
    pub fn stage_file(&self, path: &str) -> Result<()> {
        self.backend.stage_file(path)
    }

    /// Create a commit on HEAD
    pub fn commit(&self, message: &str) -> Result<()> {
        self.backend.commit(message)
    }

    /// Amend the HEAD commit with staged changes
    pub fn amend_commit(&self, message: Option<&str>) -> Result<()> {
        self.backend.amend_commit(message)
    }

    /// Create a commit using the default editor for the message
    pub fn commit_with_editor(&self) -> Result<()> {
        let workdir = &self.workdir;

        let status = std::process::Command::new("git")
            .args(["commit"])
            .current_dir(workdir)
            .status()
            .context("Failed to run git commit")?;

        if !status.success() {
            bail!("git commit failed (editor may have been cancelled)");
        }
        Ok(())
    }

    /// Amend the HEAD commit, opening an editor for the message
    pub fn amend_with_editor(&self) -> Result<()> {
        let workdir = &self.workdir;

        let status = std::process::Command::new("git")
            .args(["commit", "--amend"])
            .current_dir(workdir)
            .status()
            .context("Failed to run git commit --amend")?;

        if !status.success() {
            bail!("git commit --amend failed (editor may have been cancelled)");
        }
        Ok(())
    }

    /// Amend the HEAD commit with reset author (uses current user as author)
    pub fn amend_reset_author(&self, message: Option<&str>) -> Result<()> {
        let workdir = &self.workdir;

        let mut args = vec!["commit", "--amend", "--reset-author"];
        if let Some(msg) = message {
            args.push("-m");
            args.push(msg);
        } else {
            args.push("--no-edit");
        }

        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(workdir)
            .status()
            .context("Failed to run git commit --amend --reset-author")?;

        if !status.success() {
            bail!("git commit --amend --reset-author failed");
        }
        Ok(())
    }

    /// Run interactive rebase from a base commit
    /// This opens the editor for interactive rebase
    pub fn interactive_rebase(&self, base: &str) -> Result<()> {
        if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            bail!("Interactive rebase requires a terminal. Cannot run in non-interactive mode.");
        }

        let workdir = &self.workdir;

        let status = std::process::Command::new("git")
            .args(["rebase", "-i", base])
            .current_dir(workdir)
            .status()
            .context("Failed to run git rebase -i")?;

        if !status.success() {
            if self.rebase_in_progress()? {
                bail!(
                    "Rebase paused due to conflicts. Resolve and run '{} continue' or '{} abort'.",
                    program_name(),
                    program_name()
                );
            }
            bail!("git rebase -i failed");
        }
        Ok(())
    }

    /// Get the number of commits between the current branch and a base branch
    pub fn get_commit_count_since(&self, base: &str) -> Result<usize> {
        self.backend.get_commit_count_since(base)
    }

    /// Get the commit messages between the current branch and a base branch
    /// Returns messages from newest to oldest
    pub fn get_commit_messages_since(&self, base: &str) -> Result<Vec<String>> {
        let output = std::process::Command::new("git")
            .args(["log", "--format=%s", &format!("{}..HEAD", base)])
            .current_dir(&self.workdir)
            .output()
            .context(format!("Failed to get commit messages since '{}'", base))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to get commit messages: {}", stderr.trim());
        }

        let messages: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(messages)
    }

    /// Soft reset to a base (for squashing)
    pub fn soft_reset_to(&self, base: &str) -> Result<()> {
        super::verbose_cmd("reset", &["--soft", base]);
        let output = std::process::Command::new("git")
            .args(["reset", "--soft", base])
            .current_dir(&self.workdir)
            .output()
            .context(format!("Failed to soft reset to '{}'", base))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to soft reset to '{}': {}", base, stderr.trim());
        }

        Ok(())
    }

    /// Hard reset the current branch to a commit reference
    pub fn hard_reset_to(&self, commit_ref: &str) -> Result<()> {
        super::verbose_cmd("reset", &["--hard", commit_ref]);
        let output = std::process::Command::new("git")
            .args(["reset", "--hard", commit_ref])
            .current_dir(&self.workdir)
            .output()
            .context(format!("Failed to hard reset to '{}'", commit_ref))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to hard reset to '{}': {}", commit_ref, stderr.trim());
        }

        Ok(())
    }

    /// Restore all files from a commit to the working directory without changing HEAD
    ///
    /// This is used by `pop` to preserve committed changes as uncommitted after
    /// switching to the parent branch. Files from the commit are checked out
    /// into the working tree but not staged.
    pub fn restore_files_from_commit(&self, commit_ref: &str) -> Result<()> {
        super::verbose_cmd("checkout", &[commit_ref, "--", "."]);
        let output = std::process::Command::new("git")
            .args(["checkout", commit_ref, "--", "."])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git checkout")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to restore files from {}: {}", commit_ref, stderr.trim());
        }

        Ok(())
    }

    /// Get list of commits between two refs (from older to newer)
    ///
    /// Returns a list of (OID string, message) tuples for commits in the range.
    /// The commits are returned in reverse chronological order (newest first).
    ///
    /// # Arguments
    /// * `from_ref` - The older/ancestor commit (exclusive)
    /// * `to_ref` - The newer/descendant commit (inclusive)
    pub fn get_commits_between(&self, from_ref: &str, to_ref: &str) -> Result<Vec<(String, String)>> {
        let output = std::process::Command::new("git")
            .args(["log", "--format=%H %s", &format!("{}..{}", from_ref, to_ref)])
            .current_dir(&self.workdir)
            .output()
            .context(format!("Failed to get commits between '{}' and '{}'", from_ref, to_ref))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to get commits: {}", stderr.trim());
        }

        let commits: Vec<(String, String)> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    Some((parts[0].to_string(), parts[1].to_string()))
                } else if parts.len() == 1 && !parts[0].is_empty() {
                    Some((parts[0].to_string(), String::new()))
                } else {
                    None
                }
            })
            .collect();

        Ok(commits)
    }

    /// Get list of files changed between two refs
    ///
    /// Returns paths of all files that were added, modified, or deleted.
    pub fn get_changed_files(&self, from_ref: &str, to_ref: &str) -> Result<Vec<String>> {
        let output = std::process::Command::new("git")
            .args(["diff", "--name-only", from_ref, to_ref])
            .current_dir(&self.workdir)
            .output()
            .context(format!(
                "Failed to get changed files between '{}' and '{}'",
                from_ref, to_ref
            ))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to get changed files: {}", stderr.trim());
        }

        let files: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        Ok(files)
    }

    /// Get file content at a specific ref
    ///
    /// Returns the file content as bytes.
    pub fn get_file_at_ref(&self, git_ref: &str, path: &str) -> Result<Vec<u8>> {
        let spec = format!("{}:{}", git_ref, path);
        let output = std::process::Command::new("git")
            .args(["show", &spec])
            .current_dir(&self.workdir)
            .output()
            .context(format!("Failed to get file '{}' at ref '{}'", path, git_ref))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("File '{}' not found at ref '{}': {}", path, git_ref, stderr.trim());
        }

        Ok(output.stdout)
    }
}
