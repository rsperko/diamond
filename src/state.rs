//! Stack metadata persistence for Diamond.
//!
//! This module manages Diamond's operation state, which is stored in `.git/diamond/`:
//! - `operation_state.json` - State of in-progress operations (sync, restack, move)
//! - `operation.lock` - Exclusive lock to prevent concurrent operations
//!
//! For branch hierarchy metadata, see `ref_store.rs` which stores parent relationships
//! as git refs (`refs/diamond/parent/<branch>`).

use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::program_name::program_name;

/// Maximum age (in seconds) for a lock file to be considered stale.
/// If the lock holder PID is dead AND the lock is older than this, we clean it up.
const STALE_LOCK_AGE_SECS: u64 = 300; // 5 minutes

/// An exclusive lock on Diamond operations.
///
/// This prevents multiple Diamond processes from running concurrent operations
/// that could corrupt repository state. The lock is held for the duration of
/// multi-step operations like sync, restack, and move.
///
/// The lock is automatically released when dropped.
#[derive(Debug)]
pub struct OperationLock {
    #[allow(dead_code)]
    file: File,
    path: PathBuf,
}

impl OperationLock {
    /// Acquire an exclusive operation lock.
    ///
    /// This will fail immediately if another process holds the lock.
    /// Use `acquire_blocking` to wait for the lock.
    ///
    /// # Errors
    /// Returns an error if another Diamond operation is in progress.
    pub fn acquire() -> Result<Self> {
        let repo_root = find_git_root()?;
        Self::acquire_from(&repo_root)
    }

    /// Acquire an exclusive operation lock from a specific repository root.
    ///
    /// This method handles stale locks automatically. If the lock is held by a
    /// process that no longer exists (crashed), the stale lock is cleaned up
    /// and acquisition proceeds.
    pub fn acquire_from(repo_root: &Path) -> Result<Self> {
        let diamond_dir = repo_root.join(".git").join("diamond");
        if !diamond_dir.exists() {
            fs::create_dir_all(&diamond_dir)?;
        }

        let lock_path = diamond_dir.join("operation.lock");

        // First attempt to acquire the lock
        match Self::try_acquire_lock(&lock_path) {
            Ok(lock) => Ok(lock),
            Err(first_error) => {
                // Lock acquisition failed - check if it's stale
                if Self::is_lock_stale(&lock_path)? {
                    // Clean up stale lock and retry
                    eprintln!("Cleaning up stale lock from crashed process...");
                    if let Err(e) = fs::remove_file(&lock_path) {
                        // If we can't remove it, another process might have
                        eprintln!("Warning: Could not remove stale lock: {}", e);
                    }
                    // Retry acquisition
                    Self::try_acquire_lock(&lock_path)
                } else {
                    // Not stale, return original error
                    Err(first_error)
                }
            }
        }
    }

    /// Try to acquire the lock file (internal helper)
    fn try_acquire_lock(lock_path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(lock_path)
            .with_context(|| format!("Failed to create lock file at {:?}", lock_path))?;

        // Try to acquire exclusive lock (non-blocking)
        match file.try_lock_exclusive() {
            Ok(()) => {
                // Write PID and timestamp to lock file
                let mut file_clone = file.try_clone()?;
                let timestamp = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                writeln!(file_clone, "{}:{}", std::process::id(), timestamp)?;

                Ok(Self {
                    file,
                    path: lock_path.to_path_buf(),
                })
            }
            Err(_) => {
                // Read the lock file to see who holds it
                let holder_info = fs::read_to_string(lock_path).unwrap_or_default();
                let holder_pid = holder_info.split(':').next().unwrap_or("").trim();

                bail!(
                    "Another Diamond operation is in progress{}.\n\n\
                     If this is incorrect (e.g., after a crash), delete the lock file:\n\
                     rm {:?}",
                    if !holder_pid.is_empty() {
                        format!(" (PID: {})", holder_pid)
                    } else {
                        String::new()
                    },
                    lock_path
                );
            }
        }
    }

    /// Check if a lock file is stale (holder process is dead)
    fn is_lock_stale(lock_path: &Path) -> Result<bool> {
        if !lock_path.exists() {
            return Ok(false);
        }

        // Read lock file content: "PID:timestamp"
        let content = fs::read_to_string(lock_path).unwrap_or_default();
        let parts: Vec<&str> = content.trim().split(':').collect();

        let holder_pid: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let lock_timestamp: u64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);

        if holder_pid == 0 {
            // Can't determine PID, assume not stale (be conservative)
            return Ok(false);
        }

        // Check if the process is still running
        if is_process_running(holder_pid) {
            return Ok(false);
        }

        // Process is dead - check if lock is old enough to be considered stale
        // This prevents a race where a process just started and we incorrectly
        // think it's dead
        if lock_timestamp > 0 {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let age = now.saturating_sub(lock_timestamp);
            if age < STALE_LOCK_AGE_SECS {
                // Lock is recent - could be a race, don't clean up
                return Ok(false);
            }
        }

        // Process is dead and lock is old enough - it's stale
        Ok(true)
    }

    /// Read the content of the lock file (for testing/diagnostics)
    ///
    /// This reads from the existing file handle, which works on both Unix and Windows.
    /// On Windows, opening a new file handle to read an exclusively-locked file fails,
    /// so we must read from the file handle that holds the lock.
    #[cfg(test)]
    pub(crate) fn read_content(&self) -> Result<String> {
        use std::io::{Read, Seek, SeekFrom};

        // Clone the file handle so we don't affect the original
        let mut file = self.file.try_clone()?;

        // Seek to beginning of file
        file.seek(SeekFrom::Start(0))?;

        // Read content
        let mut content = String::new();
        file.read_to_string(&mut content)?;

        Ok(content)
    }
}

/// Check if a process with the given PID is still running.
#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
    // On Unix, we can use kill(pid, 0) to check if process exists
    // Without sending an actual signal
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn is_process_running(_pid: u32) -> bool {
    // On non-Unix systems, assume process is running (be conservative)
    // This is a safe default that won't accidentally clean up valid locks
    //
    // TODO(windows): Add Windows-specific implementation for proper stale lock detection.
    // Windows users must manually delete .git/diamond/operation.lock if Diamond crashes.
    // Fix: Add the `sysinfo` crate and use:
    //   use sysinfo::{System, SystemExt};
    //   let system = System::new_all();
    //   system.process(sysinfo::Pid::from(pid as usize)).is_some()
    // See: agent_notes/code_review_20260102/reliability_issues.md Issue #1
    true
}

impl Drop for OperationLock {
    fn drop(&mut self) {
        // Unlock and remove the lock file
        let _ = self.file.unlock();
        let _ = fs::remove_file(&self.path);
    }
}

/// Type of operation that can be interrupted by conflicts.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum OperationType {
    Sync,
    Restack,
    Move,
    /// Insert operation (dm create --insert) - inserting a new branch between parent and child
    Insert,
}

impl std::fmt::Display for OperationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sync => write!(f, "sync"),
            Self::Restack => write!(f, "restack"),
            Self::Move => write!(f, "move"),
            Self::Insert => write!(f, "insert"),
        }
    }
}

/// Generalized state for any in-progress operation (sync, restack, move, etc.)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OperationState {
    /// What type of operation is in progress
    pub operation_type: OperationType,
    /// Whether the operation is currently in progress
    pub in_progress: bool,
    /// The branch currently being rebased
    pub current_branch: Option<String>,
    /// Branches remaining to be rebased
    pub remaining_branches: Vec<String>,
    /// All branches that were part of this operation (for backup restoration on abort)
    #[serde(default)]
    pub all_branches: Vec<String>,
    /// Branches that have been successfully completed (for progress tracking)
    #[serde(default)]
    pub completed_branches: Vec<String>,
    /// The branch we started on (to return to after operation)
    pub original_branch: String,
    /// For move: the target parent branch
    pub move_target_parent: Option<String>,
    /// For move: the old parent branch (for rollback on abort)
    pub old_parent: Option<String>,
}

impl OperationState {
    /// Load operation state from .git/diamond/operation_state.json
    pub fn load() -> Result<Option<Self>> {
        let repo_root = find_git_root()?;
        Self::load_from(&repo_root)
    }

    pub fn load_from(repo_root: &Path) -> Result<Option<Self>> {
        let state_path = repo_root.join(".git").join("diamond").join("operation_state.json");

        if !state_path.exists() {
            return Ok(None);
        }

        let file = std::fs::File::open(&state_path)
            .with_context(|| format!("Failed to open operation state at {:?}", state_path))?;
        let reader = std::io::BufReader::new(file);
        let state: OperationState = serde_json::from_reader(reader).with_context(|| {
            format!(
                "Operation state file is corrupted. To recover, delete the file:\n  rm {:?}",
                state_path
            )
        })?;

        if state.in_progress {
            Ok(Some(state))
        } else {
            Ok(None)
        }
    }

    /// Save operation state to .git/diamond/operation_state.json
    pub fn save(&self) -> Result<()> {
        let repo_root = find_git_root()?;
        self.save_to(&repo_root)
    }

    pub fn save_to(&self, repo_root: &Path) -> Result<()> {
        let diamond_dir = repo_root.join(".git").join("diamond");

        if !diamond_dir.exists() {
            fs::create_dir_all(&diamond_dir)?;
        }

        let state_path = diamond_dir.join("operation_state.json");

        // Serialize to string first
        let content = serde_json::to_string_pretty(self)?;

        // Create file with restrictive permissions (0600 on Unix)
        // This protects sensitive operation state on shared systems
        let mut file = File::create(&state_path)
            .with_context(|| format!("Failed to create operation state file at {:?}", state_path))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&state_path, perms)
                .with_context(|| "Failed to set operation state file permissions")?;
        }

        file.write_all(content.as_bytes())?;

        Ok(())
    }

    /// Delete operation state file
    pub fn clear() -> Result<()> {
        let repo_root = find_git_root()?;
        Self::clear_from(&repo_root)
    }

    pub fn clear_from(repo_root: &Path) -> Result<()> {
        let state_path = repo_root.join(".git").join("diamond").join("operation_state.json");

        if state_path.exists() {
            fs::remove_file(&state_path)?;
        }

        Ok(())
    }

    /// Create a new batch operation state (Sync or Restack).
    ///
    /// Both Sync and Restack have identical state structure, differing only in operation type.
    fn new_batch(op_type: OperationType, original_branch: String, branches: Vec<String>) -> Self {
        Self {
            operation_type: op_type,
            in_progress: true,
            current_branch: None,
            remaining_branches: branches.clone(),
            all_branches: branches,
            completed_branches: Vec::new(),
            original_branch,
            move_target_parent: None,
            old_parent: None,
        }
    }

    /// Create a new Sync operation state
    pub fn new_sync(original_branch: String, branches: Vec<String>) -> Self {
        Self::new_batch(OperationType::Sync, original_branch, branches)
    }

    /// Create a new Restack operation state
    pub fn new_restack(original_branch: String, branches: Vec<String>) -> Self {
        Self::new_batch(OperationType::Restack, original_branch, branches)
    }

    /// Create a new Move operation state
    /// The old_parent is stored for rollback on abort
    pub fn new_move(
        original_branch: String,
        branches: Vec<String>,
        target_parent: String,
        old_parent: Option<String>,
    ) -> Self {
        Self {
            operation_type: OperationType::Move,
            in_progress: true,
            current_branch: None,
            remaining_branches: branches.clone(),
            all_branches: branches,
            completed_branches: Vec::new(),
            original_branch,
            move_target_parent: Some(target_parent),
            old_parent,
        }
    }

    /// Create a new Insert operation state (for dm create --insert)
    ///
    /// # Arguments
    /// * `new_branch` - The newly created branch being inserted
    /// * `child_branch` - The branch being reparented to the new branch
    /// * `original_parent` - The child's original parent (for rollback on abort)
    pub fn new_insert(new_branch: String, child_branch: String, original_parent: String) -> Self {
        Self {
            operation_type: OperationType::Insert,
            in_progress: true,
            current_branch: Some(child_branch.clone()),
            remaining_branches: vec![child_branch],
            all_branches: vec![],
            completed_branches: Vec::new(),
            original_branch: new_branch,
            move_target_parent: None,
            old_parent: Some(original_parent),
        }
    }
}

/// Acquire an operation lock and verify no operation is in progress.
///
/// This is the primary entry point for commands that need exclusive access.
/// It combines:
/// 1. File-based locking (prevents concurrent processes)
/// 2. State file checking (detects interrupted operations)
///
/// Returns an `OperationLock` that must be held for the duration of the operation.
///
/// # Errors
/// - Returns error if another Diamond process is running
/// - Returns error if a previous operation needs continue/abort
pub fn acquire_operation_lock() -> Result<OperationLock> {
    // First, try to acquire the file lock
    let lock = OperationLock::acquire()?;

    // Then check for interrupted operations (we hold the lock, so no race)
    check_for_interrupted_operation()?;

    Ok(lock)
}

/// Internal: Check for interrupted operations that need continue/abort.
fn check_for_interrupted_operation() -> Result<()> {
    if let Some(state) = OperationState::load()? {
        if state.in_progress {
            // Check if git actually has a rebase in progress
            let git_rebase_active = is_git_rebase_in_progress()?;

            if !git_rebase_active {
                // State is stale - user likely ran `git rebase --abort` directly
                // Clean it up automatically but warn about potential inconsistency
                eprintln!(
                    "⚠ Cleaning up stale {} operation state (git rebase was aborted externally)",
                    state.operation_type
                );
                // For sync/restack, branches may have been partially modified
                if !state.all_branches.is_empty() {
                    let completed_count = state.all_branches.len() - state.remaining_branches.len();
                    if completed_count > 0 {
                        eprintln!(
                            "  Note: {} of {} branches may have been modified before abort.",
                            completed_count,
                            state.all_branches.len()
                        );
                        eprintln!("  Run '{} doctor' to verify repository consistency.", program_name());
                    }
                }
                OperationState::clear()?;
                return Ok(());
            }

            // Build a detailed error message with operation state
            let mut msg = format!("A {} is already in progress.\n", state.operation_type);
            if let Some(ref current) = state.current_branch {
                msg.push_str(&format!("  Current branch: {}\n", current));
            }
            if !state.remaining_branches.is_empty() {
                msg.push_str(&format!("  Remaining: {} branches\n", state.remaining_branches.len()));
            }
            msg.push_str(&format!(
                "\nUse '{} continue' after resolving conflicts, or '{} abort' to cancel.",
                program_name(),
                program_name()
            ));
            bail!("{}", msg);
        }
    }
    Ok(())
}

/// Check if git has a rebase in progress by looking for rebase state directories
fn is_git_rebase_in_progress() -> Result<bool> {
    let repo_root = find_git_root()?;
    let git_dir = repo_root.join(".git");

    // Git stores rebase state in one of these directories
    let rebase_merge = git_dir.join("rebase-merge");
    let rebase_apply = git_dir.join("rebase-apply");

    Ok(rebase_merge.exists() || rebase_apply.exists())
}

// ============================================================================
// Helper functions
// ============================================================================

/// Find the root of the git repository
///
/// In test mode, uses the thread-local test repository path if set via `TestRepoContext`.
pub fn find_git_root() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = crate::test_context::test_repo_path() {
        // Test path should already be the git root
        if path.join(".git").exists() {
            return Ok(path);
        }
        // If not, search upward like normal
        let mut dir = path.as_path();
        loop {
            if dir.join(".git").exists() {
                return Ok(dir.to_path_buf());
            }
            if let Some(parent) = dir.parent() {
                dir = parent;
            } else {
                anyhow::bail!("Not inside a git repository");
            }
        }
    }

    let current_dir = std::env::current_dir()?;
    let mut dir = current_dir.as_path();

    loop {
        if dir.join(".git").exists() {
            return Ok(dir.to_path_buf());
        }
        if let Some(parent) = dir.parent() {
            dir = parent;
        } else {
            anyhow::bail!("Not inside a git repository");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // OperationState tests

    #[test]
    fn test_operation_state_new_sync() {
        let state = OperationState::new_sync(
            "main".to_string(),
            vec!["feature-1".to_string(), "feature-2".to_string()],
        );

        assert_eq!(state.operation_type, OperationType::Sync);
        assert!(state.in_progress);
        assert_eq!(state.original_branch, "main");
        assert_eq!(state.remaining_branches, vec!["feature-1", "feature-2"]);
        assert!(state.move_target_parent.is_none());
    }

    #[test]
    fn test_operation_state_new_restack() {
        let state = OperationState::new_restack("feature".to_string(), vec!["child-1".to_string()]);

        assert_eq!(state.operation_type, OperationType::Restack);
        assert!(state.in_progress);
        assert_eq!(state.original_branch, "feature");
        assert!(state.move_target_parent.is_none());
    }

    #[test]
    fn test_operation_state_new_move() {
        let state = OperationState::new_move(
            "feature".to_string(),
            vec!["feature".to_string(), "child".to_string()],
            "develop".to_string(),
            Some("main".to_string()),
        );

        assert_eq!(state.operation_type, OperationType::Move);
        assert!(state.in_progress);
        assert_eq!(state.original_branch, "feature");
        assert_eq!(state.move_target_parent, Some("develop".to_string()));
        assert_eq!(state.old_parent, Some("main".to_string()));
    }

    #[test]
    fn test_operation_state_new_insert() {
        let state = OperationState::new_insert(
            "new-branch".to_string(),
            "child-branch".to_string(),
            "original-parent".to_string(),
        );

        assert_eq!(state.operation_type, OperationType::Insert);
        assert!(state.in_progress);
        // original_branch is the new branch being inserted
        assert_eq!(state.original_branch, "new-branch");
        // current_branch is the child being rebased
        assert_eq!(state.current_branch, Some("child-branch".to_string()));
        // remaining_branches contains the child
        assert_eq!(state.remaining_branches, vec!["child-branch"]);
        // old_parent stores original parent for rollback
        assert_eq!(state.old_parent, Some("original-parent".to_string()));
        // move_target_parent not used for insert
        assert!(state.move_target_parent.is_none());
    }

    #[test]
    fn test_operation_state_save_load() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir_all(root.join(".git").join("diamond"))?;

        let state = OperationState::new_sync(
            "main".to_string(),
            vec!["feature-1".to_string(), "feature-2".to_string()],
        );

        state.save_to(root)?;

        let loaded = OperationState::load_from(root)?;
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert_eq!(loaded.operation_type, OperationType::Sync);
        assert!(loaded.in_progress);
        assert_eq!(loaded.original_branch, "main");
        assert_eq!(loaded.remaining_branches, vec!["feature-1", "feature-2"]);

        Ok(())
    }

    #[test]
    fn test_operation_state_clear() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir_all(root.join(".git").join("diamond"))?;

        let state = OperationState::new_sync("main".to_string(), vec![]);
        state.save_to(root)?;

        assert!(OperationState::load_from(root)?.is_some());

        OperationState::clear_from(root)?;

        assert!(OperationState::load_from(root)?.is_none());

        Ok(())
    }

    #[test]
    fn test_operation_state_load_nonexistent() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir_all(root.join(".git").join("diamond"))?;

        let loaded = OperationState::load_from(root)?;
        assert!(loaded.is_none());

        Ok(())
    }

    #[test]
    fn test_operation_state_not_in_progress_returns_none() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir_all(root.join(".git").join("diamond"))?;

        let mut state = OperationState::new_sync("main".to_string(), vec![]);
        state.in_progress = false;
        state.save_to(root)?;

        let loaded = OperationState::load_from(root)?;
        assert!(loaded.is_none());

        Ok(())
    }

    #[test]
    fn test_operation_state_corrupt_json_gives_helpful_error() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        let diamond_dir = root.join(".git").join("diamond");
        fs::create_dir_all(&diamond_dir)?;

        // Write invalid JSON to the state file
        let state_path = diamond_dir.join("operation_state.json");
        fs::write(&state_path, "{ this is not valid JSON }")?;

        // Try to load - should fail with helpful error
        let result = OperationState::load_from(root);
        assert!(result.is_err());

        let err = result.unwrap_err().to_string();
        // Error should mention corruption and provide recovery instructions
        assert!(
            err.contains("corrupted"),
            "Error should mention corruption. Got: {}",
            err
        );
        assert!(
            err.contains("delete") || err.contains("rm"),
            "Error should provide recovery instructions. Got: {}",
            err
        );

        Ok(())
    }

    #[test]
    fn test_operation_state_with_move_fields() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir_all(root.join(".git").join("diamond"))?;

        let state = OperationState::new_move(
            "feature".to_string(),
            vec!["feature".to_string(), "child-1".to_string(), "child-2".to_string()],
            "develop".to_string(),
            Some("main".to_string()),
        );

        state.save_to(root)?;

        let loaded = OperationState::load_from(root)?.unwrap();
        assert_eq!(loaded.operation_type, OperationType::Move);
        assert_eq!(loaded.move_target_parent, Some("develop".to_string()));
        assert_eq!(loaded.old_parent, Some("main".to_string()));
        assert_eq!(loaded.remaining_branches.len(), 3);

        Ok(())
    }

    #[test]
    fn test_operation_state_load_corrupted_json() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        let state_path = root.join(".git").join("diamond").join("operation_state.json");
        fs::create_dir_all(state_path.parent().unwrap())?;

        fs::write(&state_path, "{ truncated...")?;

        let result = OperationState::load_from(root);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("parse") || err.contains("JSON") || err.contains("json"),
            "Error should mention JSON parsing: {}",
            err
        );

        Ok(())
    }

    #[test]
    fn test_operation_state_load_empty_file() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        let state_path = root.join(".git").join("diamond").join("operation_state.json");
        fs::create_dir_all(state_path.parent().unwrap())?;

        fs::write(&state_path, "")?;

        let result = OperationState::load_from(root);
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn test_operation_state_file_permissions() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir_all(root.join(".git").join("diamond"))?;

        let state = OperationState::new_sync("main".to_string(), vec!["feature".to_string()]);
        state.save_to(root)?;

        let state_path = root.join(".git").join("diamond").join("operation_state.json");
        let metadata = fs::metadata(&state_path)?;
        let perms = metadata.permissions();

        // Check that permissions are 0600 (owner read/write only)
        let mode = perms.mode() & 0o777;
        assert_eq!(mode, 0o600, "Expected permissions 0600, got {:o}", mode);

        Ok(())
    }

    // ============================================================================
    // OperationLock tests
    // ============================================================================

    #[test]
    fn test_operation_lock_acquire_and_release() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir_all(root.join(".git").join("diamond"))?;

        // Acquire lock
        let lock = OperationLock::acquire_from(root)?;

        // Lock file should exist
        let lock_path = root.join(".git").join("diamond").join("operation.lock");
        assert!(lock_path.exists(), "Lock file should exist");

        // Drop lock
        drop(lock);

        // Lock file should be removed
        assert!(!lock_path.exists(), "Lock file should be removed after drop");

        Ok(())
    }

    #[test]
    fn test_operation_lock_prevents_concurrent_acquisition() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir_all(root.join(".git").join("diamond"))?;

        // Acquire first lock
        let _lock1 = OperationLock::acquire_from(root)?;

        // Second acquisition should fail
        let result = OperationLock::acquire_from(root);
        assert!(result.is_err(), "Second lock acquisition should fail");

        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Another Diamond operation is in progress"),
            "Error should mention concurrent operation: {}",
            err
        );

        Ok(())
    }

    #[test]
    fn test_operation_lock_released_allows_new_acquisition() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir_all(root.join(".git").join("diamond"))?;

        // Acquire and release first lock
        {
            let _lock1 = OperationLock::acquire_from(root)?;
            // Lock is released here when _lock1 goes out of scope
        }

        // Second acquisition should succeed
        let lock2 = OperationLock::acquire_from(root);
        assert!(lock2.is_ok(), "Second lock should succeed after first is released");

        Ok(())
    }

    #[test]
    fn test_operation_lock_contains_pid() -> Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        fs::create_dir_all(root.join(".git").join("diamond"))?;

        let lock = OperationLock::acquire_from(root)?;

        // Read lock file content using the file handle we have
        // (avoids Windows issue where opening a new handle to an exclusively-locked file fails)
        let content = lock.read_content()?;
        let expected_pid = std::process::id().to_string();

        assert!(
            content.contains(&expected_pid),
            "Lock file should contain current PID {}. Got: {}",
            expected_pid,
            content
        );

        Ok(())
    }
}
