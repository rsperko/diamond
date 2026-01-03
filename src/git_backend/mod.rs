//! Git backend abstraction for reftable compatibility.
//!
//! # Architecture Overview
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        GitGateway                               │
//! │  (High-level operations: rebase, sync, backup, etc.)           │
//! │                                                                 │
//! │  ┌─────────────────────────────────────────────────────────┐   │
//! │  │              Box<dyn GitBackend>                         │   │
//! │  │  (Low-level git operations: refs, blobs, branches)      │   │
//! │  └─────────────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────────────┘
//!                              │
//!              ┌───────────────┴───────────────┐
//!              ▼                               ▼
//!     ┌────────────────┐             ┌────────────────────┐
//!     │  Git2Backend   │             │ SubprocessBackend  │
//!     │  (libgit2)     │             │ (git CLI)          │
//!     │                │             │                    │
//!     │ files-format   │             │ any format         │
//!     │ repos only     │             │ including reftable │
//!     └────────────────┘             └────────────────────┘
//! ```
//!
//! # Why This Design?
//!
//! libgit2 does not yet support the reftable ref format (Git 2.45+). Rather than
//! scattering `if reftable { subprocess } else { git2 }` throughout the codebase,
//! we use a trait-based backend approach that:
//!
//! 1. **Hides format differences** - Code using `GitBackend` doesn't know or care
//!    about the underlying format
//! 2. **Enables easy migration** - When libgit2 adds reftable support, we update
//!    `create_backend()` to always use `Git2Backend` and remove `SubprocessBackend`
//! 3. **Centralizes git operations** - All low-level git ops go through one interface
//!
//! # Usage
//!
//! Most code should use `GitGateway`, not `GitBackend` directly. The gateway
//! provides higher-level operations and handles the backend selection automatically.
//!
//! ```ignore
//! let gateway = GitGateway::new()?;
//! gateway.create_branch("feature")?;
//! ```
//!
//! # Canonical Types
//!
//! This module defines the canonical `Oid` and `RefFormat` types used throughout
//! Diamond. Other modules re-export these rather than defining their own.

mod git2_backend;
mod subprocess_backend;

pub use git2_backend::Git2Backend;
pub use subprocess_backend::SubprocessBackend;

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Git ref storage format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefFormat {
    /// Traditional loose refs + packed-refs
    Files,
    /// New binary reftable format (Git 2.45+)
    Reftable,
}

/// Git object ID (40-character hex string).
///
/// This is the canonical OID type used throughout Diamond. It validates
/// that the string is a proper 40-character hex SHA-1 hash.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Oid(String);

impl Oid {
    /// Create an Oid from a hex string (validates format)
    #[allow(dead_code)] // Useful utility for validated OID creation
    pub fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.len() != 40 {
            anyhow::bail!("Invalid OID length: expected 40, got {}", s.len());
        }
        if !s.chars().all(|c| c.is_ascii_hexdigit()) {
            anyhow::bail!("Invalid OID: contains non-hex characters");
        }
        Ok(Self(s.to_lowercase()))
    }

    /// Create an Oid without validation (internal use only)
    ///
    /// Use this when you're certain the string is valid (e.g., from git output)
    pub(crate) fn from_str_unchecked(s: &str) -> Self {
        Self(s.trim().to_string())
    }

    /// Get the hex string representation
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get short form (first 7 chars)
    #[allow(dead_code)] // Useful utility
    pub fn short(&self) -> &str {
        &self.0[..7.min(self.0.len())]
    }

    /// Convert to git2::Oid
    #[allow(dead_code)] // Used by git2_backend and tests
    pub fn to_git2(&self) -> Result<git2::Oid> {
        git2::Oid::from_str(&self.0).context("Failed to parse OID")
    }
}

impl std::fmt::Display for Oid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<git2::Oid> for Oid {
    fn from(oid: git2::Oid) -> Self {
        Self(oid.to_string())
    }
}

/// Unified backend for all git operations.
///
/// This trait abstracts over git2 and subprocess implementations,
/// allowing transparent reftable support.
#[allow(dead_code)] // Comprehensive interface - methods used incrementally during migration
pub trait GitBackend: Send {
    // =========================================================================
    // Path accessors
    // =========================================================================

    /// Path to .git directory
    fn git_dir(&self) -> &Path;

    /// Path to working directory
    fn workdir(&self) -> &Path;

    /// The ref format this repo uses
    fn ref_format(&self) -> RefFormat;

    // =========================================================================
    // Branch operations
    // =========================================================================

    /// Get the current branch name (fails if detached HEAD)
    fn get_current_branch(&self) -> Result<String>;

    /// Check if currently on a branch (not detached HEAD)
    fn is_on_branch(&self) -> Result<bool>;

    /// Create a new branch at current HEAD
    fn create_branch(&self, name: &str) -> Result<()>;

    /// Create a new branch at a specific ref
    fn create_branch_at(&self, name: &str, at_ref: &str) -> Result<()>;

    /// Check if a branch exists
    fn branch_exists(&self, name: &str) -> Result<bool>;

    /// Checkout a branch
    fn checkout_branch(&self, name: &str) -> Result<()>;

    /// Checkout a branch with force (discard local changes)
    fn checkout_branch_force(&self, name: &str) -> Result<()>;

    /// List all local branches
    fn list_branches(&self) -> Result<Vec<String>>;

    /// Delete a branch
    fn delete_branch(&self, name: &str) -> Result<()>;

    /// Rename a branch
    fn rename_branch(&self, old_name: &str, new_name: &str) -> Result<()>;

    // =========================================================================
    // Commit operations
    // =========================================================================

    /// Stage all changes (git add -A)
    fn stage_all(&self) -> Result<()>;

    /// Stage tracked file updates only (git add -u)
    fn stage_updates(&self) -> Result<()>;

    /// Stage a specific file
    fn stage_file(&self, path: &str) -> Result<()>;

    /// Create a commit with a message
    fn commit(&self, message: &str) -> Result<()>;

    /// Amend the last commit
    fn amend_commit(&self, message: Option<&str>) -> Result<()>;

    // =========================================================================
    // Ref operations (for Diamond's metadata refs)
    // =========================================================================

    /// Create or update a reference pointing to an OID
    fn create_reference(&self, name: &str, target: &Oid, force: bool, msg: &str) -> Result<()>;

    /// Delete a reference
    fn delete_reference(&self, name: &str) -> Result<()>;

    /// Find a reference, returning its name and target OID
    fn find_reference(&self, name: &str) -> Result<Option<(String, Oid)>>;

    /// List references matching a glob pattern
    fn list_references(&self, pattern: &str) -> Result<Vec<(String, Oid)>>;

    // =========================================================================
    // Blob operations (Diamond stores parent names in blobs)
    // =========================================================================

    /// Create a blob from content, returns OID
    fn create_blob(&self, content: &[u8]) -> Result<Oid>;

    /// Read a blob's content by OID
    fn read_blob(&self, oid: &Oid) -> Result<Vec<u8>>;

    // =========================================================================
    // Validation / status operations
    // =========================================================================

    /// Check for any uncommitted changes (staged or unstaged)
    fn has_uncommitted_changes(&self) -> Result<bool>;

    /// Check for staged changes only
    fn has_staged_changes(&self) -> Result<bool>;

    /// Check for staged or modified (but not untracked) files
    fn has_staged_or_modified_changes(&self) -> Result<bool>;

    /// Get the merge base of two refs
    fn get_merge_base(&self, a: &str, b: &str) -> Result<Oid>;

    /// Check if a ref is an ancestor of another
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool>;

    /// Check if a branch has been merged into another
    fn is_branch_merged(&self, branch: &str, into: &str) -> Result<bool>;

    // =========================================================================
    // Commit info operations
    // =========================================================================

    /// Get the SHA of a branch or ref
    fn get_ref_sha(&self, reference: &str) -> Result<Oid>;

    /// Get the short SHA (7 chars) of a ref
    fn get_short_sha(&self, reference: &str) -> Result<String>;

    /// Get the commit subject line
    fn get_commit_subject(&self, reference: &str) -> Result<String>;

    /// Get relative time of commit (e.g., "2 hours ago")
    fn get_commit_time_relative(&self, reference: &str) -> Result<String>;

    /// Count commits between base and HEAD
    fn get_commit_count_since(&self, base: &str) -> Result<usize>;
}

/// Detect the ref format of a repository
pub fn detect_ref_format(path: &Path) -> Result<RefFormat> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-ref-format"])
        .current_dir(path)
        .output()
        .context("Failed to detect ref format")?;

    if !output.status.success() {
        // Older git or not a repo - assume files format
        return Ok(RefFormat::Files);
    }

    let format = String::from_utf8_lossy(&output.stdout);
    match format.trim() {
        "reftable" => Ok(RefFormat::Reftable),
        _ => Ok(RefFormat::Files),
    }
}

/// Create the appropriate backend for a repository
pub fn create_backend(path: &Path) -> Result<Box<dyn GitBackend>> {
    let format = detect_ref_format(path)?;

    match format {
        RefFormat::Reftable => {
            // Reftable repos must use subprocess - libgit2 doesn't support it
            Ok(Box::new(SubprocessBackend::open(path)?))
        }
        RefFormat::Files => {
            // Try git2 first, fall back to subprocess if it fails
            match Git2Backend::open(path) {
                Ok(backend) => Ok(Box::new(backend)),
                Err(_) => Ok(Box::new(SubprocessBackend::open(path)?)),
            }
        }
    }
}
