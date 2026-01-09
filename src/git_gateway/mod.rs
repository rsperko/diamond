//! Git operations gateway for Diamond.
//!
//! This module provides a unified interface to git operations, automatically
//! detecting the repository's ref format and using either git2 (for "files"
//! repos) or git CLI subprocess calls (for "reftable" repos).
//!
//! # Operations
//!
//! - **Branch management**: create, delete, rename, checkout branches
//! - **Commit operations**: stage, commit, amend
//! - **Rebase operations**: rebase branches, detect and handle conflicts
//! - **Backup/restore**: create backup refs for undo functionality
//! - **Ref/blob operations**: read/write refs and blobs (reftable compatible)
//!
//! # Example
//!
//! ```ignore
//! use crate::git_gateway::GitGateway;
//!
//! let gateway = GitGateway::new()?;
//! gateway.create_branch("feature")?;
//! gateway.commit("Initial feature work")?;
//! ```

pub mod backup;
mod branch;
mod commit;
mod diamond_refs;
mod rebase;
pub mod refs;
mod remote;
mod status;
mod validation;

#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::{Path, PathBuf};

#[cfg(not(test))]
use crate::config::Config;
use crate::context::ExecutionContext;
use crate::git_backend::{self, GitBackend};

// Re-export ref types for convenience
pub use refs::RefFormat;

// Re-export public types
pub use self::backup::BackupRef;
pub use self::rebase::RebaseOutcome;
pub use self::remote::{BranchSyncState, SyncBranchResult};
#[allow(unused_imports)] // Used in ui::conflict module
pub use self::status::{ConflictType, ConflictedFile};

/// Default remote name when config cannot be loaded
const DEFAULT_REMOTE: &str = "origin";

/// Log a git command if verbose mode is enabled
pub(crate) fn verbose_cmd(cmd: &str, args: &[&str]) {
    if ExecutionContext::is_verbose() {
        eprintln!("  {} git {} {}", "[cmd]".dimmed(), cmd, args.join(" "));
    }
}

/// Format a time difference in seconds as a human-readable relative time string
#[allow(dead_code)] // Useful utility for displaying relative times
pub(crate) fn format_relative_time(diff_secs: i64) -> String {
    if diff_secs < 0 {
        return "in the future".to_string();
    }

    let minutes = diff_secs / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    let weeks = days / 7;
    let months = days / 30;
    let years = days / 365;

    if years > 0 {
        if years == 1 {
            "1 year ago".to_string()
        } else {
            format!("{} years ago", years)
        }
    } else if months > 0 {
        if months == 1 {
            "1 month ago".to_string()
        } else {
            format!("{} months ago", months)
        }
    } else if weeks > 0 {
        if weeks == 1 {
            "1 week ago".to_string()
        } else {
            format!("{} weeks ago", weeks)
        }
    } else if days > 0 {
        if days == 1 {
            "1 day ago".to_string()
        } else {
            format!("{} days ago", days)
        }
    } else if hours > 0 {
        if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", hours)
        }
    } else if minutes > 0 {
        if minutes == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{} minutes ago", minutes)
        }
    } else if diff_secs == 1 {
        "1 second ago".to_string()
    } else {
        format!("{} seconds ago", diff_secs)
    }
}

/// Unified interface to git operations.
///
/// All git operations in Diamond go through this gateway, which automatically
/// detects the repository's ref format and uses either git2 (for "files" repos)
/// or git CLI subprocess calls (for "reftable" repos).
///
/// This abstraction enables:
/// - Consistent error handling across all git operations
/// - Automatic reftable support via subprocess fallbacks
/// - Easy testing with isolated repositories
/// - Centralized logging and debugging
///
/// The gateway stores the configured remote name (from `.diamond/config.toml`)
/// and uses it for all remote operations.
pub struct GitGateway {
    /// Git backend that handles all basic operations
    backend: Box<dyn GitBackend>,
    /// Path to .git directory
    pub(crate) git_dir: PathBuf,
    /// Path to working directory
    pub(crate) workdir: PathBuf,
    /// The configured remote name (e.g., "origin", "upstream")
    remote: String,
    /// Reference format (files or reftable)
    #[allow(dead_code)] // Used for reftable support infrastructure
    format: RefFormat,
}

impl GitGateway {
    /// Detect the ref format of a repository
    pub(crate) fn detect_format(path: &Path) -> Result<RefFormat> {
        git_backend::detect_ref_format(path)
    }

    /// Get paths from git CLI (works with any ref format)
    fn get_paths(path: &Path) -> Result<(PathBuf, PathBuf)> {
        // Get git directory
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(path)
            .output()
            .context("Failed to run git rev-parse --git-dir")?;

        if !output.status.success() {
            anyhow::bail!("Not a git repository");
        }

        let git_dir_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let git_dir = if Path::new(&git_dir_str).is_absolute() {
            PathBuf::from(git_dir_str)
        } else {
            path.join(&git_dir_str)
        };

        // Get working directory
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(path)
            .output()
            .context("Failed to run git rev-parse --show-toplevel")?;

        if !output.status.success() {
            anyhow::bail!("Not a git repository");
        }

        let workdir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());

        Ok((git_dir, workdir))
    }

    /// Create a new GitGateway from the current directory.
    ///
    /// Automatically detects the ref format and uses git2 for "files" repos
    /// or subprocess calls for "reftable" repos.
    ///
    /// Loads the remote name from config (`.diamond/config.toml`), falling back
    /// to "origin" if config cannot be loaded.
    ///
    /// In test mode, uses the thread-local test repository path if set via `TestRepoContext`.
    pub fn new() -> Result<Self> {
        #[cfg(test)]
        {
            if let Some(path) = crate::test_context::test_repo_path() {
                return Self::from_path(path);
            }

            // SAFETY CHECK: In tests, we should ALWAYS have a test repo path set
            // If we get here, a test is about to modify the diamond repository itself
            panic!(
                "SAFETY VIOLATION: GitGateway::new() called in test without TestRepoContext!\n\
                 This would modify the diamond repository. Use TestRepoContext in your test:\n\
                 \n\
                 let dir = tempdir()?;\n\
                 let _repo = init_test_repo(dir.path())?;\n\
                 let _ctx = TestRepoContext::new(dir.path());  // <- ADD THIS\n\
                 let gateway = GitGateway::new()?;"
            );
        }

        #[cfg(not(test))]
        {
            let cwd = std::env::current_dir().context("Failed to get current directory")?;
            Self::from_path(&cwd)
        }
    }

    /// Create a GitGateway from a specific path
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        // Create the backend (auto-detects format)
        let backend = git_backend::create_backend(path)?;

        // Detect format
        let format = Self::detect_format(path)?;

        // Get paths via subprocess (always works)
        let (git_dir, workdir) = Self::get_paths(path)?;

        // Load remote from config, with fallback to "origin"
        #[cfg(not(test))]
        let remote = Config::load()
            .map(|c| c.remote)
            .unwrap_or_else(|_| DEFAULT_REMOTE.to_string());

        #[cfg(test)]
        let remote = DEFAULT_REMOTE.to_string();

        Ok(Self {
            backend,
            git_dir,
            workdir,
            remote,
            format,
        })
    }

    /// Get the repository's ref format
    #[allow(dead_code)] // Used for reftable support infrastructure
    pub fn ref_format(&self) -> RefFormat {
        self.format
    }

    /// Get the git directory path
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// Get the working directory path
    #[allow(dead_code)] // Used for reftable support infrastructure
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// Get a reference to the backend
    #[allow(dead_code)] // Available for potential future use
    pub fn backend(&self) -> &dyn GitBackend {
        self.backend.as_ref()
    }

    /// Get the configured remote name
    pub fn remote(&self) -> &str {
        &self.remote
    }

    /// Check if a remote exists in the repository
    pub fn has_remote(&self, remote_name: &str) -> Result<bool> {
        // Use subprocess - this is a rare operation and works on all formats
        let output = std::process::Command::new("git")
            .args(["remote", "get-url", remote_name])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git remote get-url")?;
        Ok(output.status.success())
    }

    /// Resolve a reference (branch name, commit hash, etc.) to a commit OID
    ///
    /// Returns error if the reference doesn't exist or isn't a valid commit.
    /// Note: Returns git2::Oid for backward compatibility with existing callers.
    pub fn resolve_ref(&self, reference: &str) -> Result<git2::Oid> {
        let oid = self.backend.get_ref_sha(reference)?;
        oid.to_git2()
    }

    /// Resolve a reference to our wrapper Oid type (works with reftable)
    #[allow(dead_code)] // Used in tests; useful for reftable-compatible code
    pub fn resolve_to_oid(&self, reference: &str) -> Result<refs::Oid> {
        self.backend.get_ref_sha(reference)
    }

    /// Silently clean up orphaned Diamond refs.
    ///
    /// This is called by high-frequency commands (log, info, navigation) to prevent
    /// orphaned metadata accumulation. Errors are silently ignored since cleanup
    /// is best-effort and should never block normal operations.
    pub fn cleanup_orphaned_refs_silently(&self) {
        let _ = crate::validation::silent_cleanup_orphaned_refs(self);
    }
}
