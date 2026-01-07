//! Worktree detection utilities for Diamond.
//!
//! Git worktrees share the refs database and Diamond state files,
//! which can cause conflicts and data loss. This module provides
//! utilities to detect worktree usage and warn users.
//!
//! Key functionality:
//! - Detect branches checked out in other worktrees (conflict prevention)
//! - Detect orphaned worktrees (branches deleted while worktree exists)
//! - Validate current worktree state at command startup

use anyhow::{Context, Result};
use crate::platform::DisplayPath;
use std::path::PathBuf;
use std::process::Command;

/// Information about a single worktree
#[derive(Debug, Clone)]
#[allow(dead_code)] // Tested, will be exposed for worktree-aware commands
pub struct WorktreeInfo {
    /// Absolute path to the worktree
    pub path: PathBuf,
    /// Branch name (without refs/heads/ prefix), or None if detached HEAD
    pub branch: Option<String>,
    /// Whether this is the current worktree we're running from
    pub is_current: bool,
    /// Whether this worktree is in a "bare" state (the main .git directory)
    pub is_bare: bool,
}

/// State of a worktree that could affect operations
#[derive(Debug, Clone, PartialEq)]
pub enum WorktreeState {
    /// Worktree is clean and not in any special state
    Clean,
    /// Worktree has uncommitted changes
    Dirty,
    /// Worktree is in the middle of a rebase
    MidRebase,
    /// Worktree is in the middle of a merge
    MidMerge,
    /// Worktree is in the middle of a cherry-pick
    MidCherryPick,
}

/// Information about worktree status (legacy, for compatibility)
#[derive(Debug)]
pub struct WorktreeStatus {
    /// Branches checked out in OTHER worktrees (not current)
    pub branches_in_other_worktrees: Vec<String>,
}

/// Information about an orphaned worktree
#[derive(Debug)]
#[allow(dead_code)] // Tested, will be exposed for worktree-aware commands
pub struct OrphanedWorktree {
    /// Path to the orphaned worktree
    pub path: PathBuf,
    /// The branch name that no longer exists
    pub missing_branch: String,
}

/// Get full worktree status.
pub fn get_worktree_status() -> Result<WorktreeStatus> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
        .context("Failed to run git worktree list --porcelain")?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut branches_in_other_worktrees = Vec::new();

    // Get current working directory to identify which worktree we're in
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.to_string_lossy().to_string());

    let mut current_wt_path;
    let mut is_current_worktree = false;

    for line in stdout.lines() {
        if line.starts_with("worktree ") {
            current_wt_path = line.strip_prefix("worktree ").unwrap_or("");
            // Check if this worktree path matches our current directory
            is_current_worktree = if let Some(ref c) = cwd {
                // Canonicalize both paths for comparison
                let wt_canonical = std::path::Path::new(&current_wt_path)
                    .canonicalize()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
                wt_canonical.as_ref() == Some(c)
            } else {
                false
            };
        } else if line.starts_with("branch refs/heads/") {
            let branch = line.strip_prefix("branch refs/heads/").unwrap_or("");
            if !is_current_worktree && !branch.is_empty() {
                branches_in_other_worktrees.push(branch.to_string());
            }
        }
    }

    Ok(WorktreeStatus {
        branches_in_other_worktrees,
    })
}

/// Get the path where a specific branch is checked out (in another worktree).
/// Returns None if the branch is not checked out in any other worktree.
/// Returns None if worktree listing fails (e.g., not in a git repo or single worktree).
pub fn get_worktree_path_for_branch(branch: &str) -> Result<Option<PathBuf>> {
    // If listing worktrees fails (e.g., not in a git repo, repo deleted, single worktree),
    // treat it as "no other worktrees exist" and return None instead of propagating error
    let worktrees = match list_worktrees() {
        Ok(wt) => wt,
        Err(_) => return Ok(None),
    };

    for wt in worktrees {
        // Skip current worktree and bare repos
        if wt.is_current || wt.is_bare {
            continue;
        }

        if let Some(ref wt_branch) = wt.branch {
            if wt_branch == branch {
                return Ok(Some(wt.path));
            }
        }
    }

    Ok(None)
}

/// Check if any branches in the list are checked out in other worktrees.
/// Returns an error listing the conflicting branches, or Ok if none conflict.
pub fn check_branches_for_worktree_conflicts(branches: &[String]) -> Result<()> {
    let status = get_worktree_status()?;

    let conflicts: Vec<&String> = branches
        .iter()
        .filter(|b| status.branches_in_other_worktrees.contains(*b))
        .collect();

    if !conflicts.is_empty() {
        let branch_list = conflicts.iter().map(|b| b.as_str()).collect::<Vec<_>>().join(", ");
        anyhow::bail!(
            "Cannot proceed: {} branch(es) checked out in other worktrees: {}\n\
             Run 'git worktree list' to see where.\n\
             Either close those worktrees or switch them to different branches.",
            conflicts.len(),
            branch_list
        );
    }

    Ok(())
}

/// Get the state of a worktree at the given path.
///
/// Checks for:
/// - Dirty working directory (uncommitted changes)
/// - Mid-rebase state
/// - Mid-merge state
/// - Mid-cherry-pick state
#[allow(dead_code)]
pub fn get_worktree_state(worktree_path: &PathBuf) -> Result<WorktreeState> {
    // Check for rebase in progress
    let git_dir = worktree_path.join(".git");
    let rebase_merge = if git_dir.is_dir() {
        git_dir.join("rebase-merge")
    } else {
        // Linked worktree - .git is a file pointing to the actual git dir
        worktree_path.join("rebase-merge")
    };
    let rebase_apply = if git_dir.is_dir() {
        git_dir.join("rebase-apply")
    } else {
        worktree_path.join("rebase-apply")
    };

    if rebase_merge.exists() || rebase_apply.exists() {
        return Ok(WorktreeState::MidRebase);
    }

    // Check for merge in progress
    let merge_head = if git_dir.is_dir() {
        git_dir.join("MERGE_HEAD")
    } else {
        worktree_path.join("MERGE_HEAD")
    };
    if merge_head.exists() {
        return Ok(WorktreeState::MidMerge);
    }

    // Check for cherry-pick in progress
    let cherry_pick_head = if git_dir.is_dir() {
        git_dir.join("CHERRY_PICK_HEAD")
    } else {
        worktree_path.join("CHERRY_PICK_HEAD")
    };
    if cherry_pick_head.exists() {
        return Ok(WorktreeState::MidCherryPick);
    }

    // Check for dirty working directory
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(worktree_path)
        .output()
        .context("Failed to check worktree status")?;

    if !output.stdout.is_empty() {
        return Ok(WorktreeState::Dirty);
    }

    Ok(WorktreeState::Clean)
}

/// Enhanced check for worktree conflicts that also considers worktree state.
///
/// In addition to checking if branches are checked out elsewhere, this also
/// checks if those worktrees are in a state that would be problematic
/// (dirty, mid-rebase, mid-merge).
#[allow(dead_code)]
pub fn check_branches_for_worktree_conflicts_enhanced(branches: &[String]) -> Result<()> {
    let worktrees = list_worktrees()?;

    let mut conflicts: Vec<String> = Vec::new();
    let mut state_conflicts: Vec<String> = Vec::new();

    for wt in &worktrees {
        if wt.is_current || wt.is_bare {
            continue;
        }

        if let Some(ref branch) = wt.branch {
            if branches.contains(branch) {
                // Check the state of this worktree
                match get_worktree_state(&wt.path) {
                    Ok(WorktreeState::Dirty) => {
                        state_conflicts.push(format!(
                            "'{}' checked out at {} (has uncommitted changes)",
                            branch,
                            DisplayPath(&wt.path)
                        ));
                    }
                    Ok(WorktreeState::MidRebase) => {
                        state_conflicts.push(format!(
                            "'{}' checked out at {} (rebase in progress)",
                            branch,
                            DisplayPath(&wt.path)
                        ));
                    }
                    Ok(WorktreeState::MidMerge) => {
                        state_conflicts.push(format!(
                            "'{}' checked out at {} (merge in progress)",
                            branch,
                            DisplayPath(&wt.path)
                        ));
                    }
                    Ok(WorktreeState::MidCherryPick) => {
                        state_conflicts.push(format!(
                            "'{}' checked out at {} (cherry-pick in progress)",
                            branch,
                            DisplayPath(&wt.path)
                        ));
                    }
                    Ok(WorktreeState::Clean) | Err(_) => {
                        conflicts.push(format!("'{}' checked out at {}", branch, DisplayPath(&wt.path)));
                    }
                }
            }
        }
    }

    // State conflicts are more severe - report them first
    if !state_conflicts.is_empty() {
        anyhow::bail!(
            "Cannot proceed: worktree(s) in problematic state:\n  {}\n\n\
             Resolve the state in those worktrees before proceeding.",
            state_conflicts.join("\n  ")
        );
    }

    if !conflicts.is_empty() {
        anyhow::bail!(
            "Cannot proceed: branch(es) checked out in other worktrees:\n  {}\n\n\
             Run 'git worktree list' to see all worktrees.\n\
             Either close those worktrees or switch them to different branches.",
            conflicts.join("\n  ")
        );
    }

    Ok(())
}

/// List all worktrees with their full information.
///
/// Returns structured information about each worktree including path,
/// branch (if any), and whether it's the current worktree.
#[allow(dead_code)] // Tested, used by validate_current_worktree
pub fn list_worktrees() -> Result<Vec<WorktreeInfo>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
        .context("Failed to run git worktree list --porcelain")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git worktree list failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Get current working directory to identify which worktree we're in
    let cwd = std::env::current_dir().ok().and_then(|p| p.canonicalize().ok());

    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;
    let mut is_bare = false;

    for line in stdout.lines() {
        if line.starts_with("worktree ") {
            // Save previous worktree if any
            if let Some(path) = current_path.take() {
                let is_current = cwd
                    .as_ref()
                    .and_then(|c| path.canonicalize().ok().map(|p| p == *c))
                    .unwrap_or(false);

                worktrees.push(WorktreeInfo {
                    path,
                    branch: current_branch.take(),
                    is_current,
                    is_bare,
                });
                is_bare = false;
            }

            let path_str = line.strip_prefix("worktree ").unwrap_or("");
            let path = PathBuf::from(path_str);
            // Canonicalize to normalize path separators for the platform
            // For worktrees, paths should always exist, but fall back to raw path if needed
            current_path = Some(path.canonicalize().unwrap_or(path));
        } else if line.starts_with("branch refs/heads/") {
            current_branch = line.strip_prefix("branch refs/heads/").map(|s| s.to_string());
        } else if line == "bare" {
            is_bare = true;
        } else if line == "detached" {
            // Detached HEAD - branch stays None
            current_branch = None;
        }
    }

    // Don't forget the last worktree
    if let Some(path) = current_path {
        let is_current = cwd
            .as_ref()
            .and_then(|c| path.canonicalize().ok().map(|p| p == *c))
            .unwrap_or(false);

        worktrees.push(WorktreeInfo {
            path,
            branch: current_branch,
            is_current,
            is_bare,
        });
    }

    Ok(worktrees)
}

/// Detect orphaned worktrees (worktrees whose branches have been deleted).
///
/// Returns a list of worktrees that reference branches that no longer exist.
/// This can happen when a branch is deleted from another worktree or the main tree.
#[allow(dead_code)] // Tested, for dm doctor command
pub fn detect_orphaned_worktrees() -> Result<Vec<OrphanedWorktree>> {
    let worktrees = list_worktrees()?;
    let mut orphaned = Vec::new();

    for wt in worktrees {
        // Skip bare repos and detached HEAD worktrees
        if wt.is_bare {
            continue;
        }

        if let Some(ref branch) = wt.branch {
            // Check if the branch actually exists
            let output = Command::new("git")
                .args(["rev-parse", "--verify", &format!("refs/heads/{}", branch)])
                .output()
                .context("Failed to verify branch existence")?;

            if !output.status.success() {
                orphaned.push(OrphanedWorktree {
                    path: wt.path,
                    missing_branch: branch.clone(),
                });
            }
        }
    }

    Ok(orphaned)
}

/// Validate the current worktree state.
///
/// This should be called at the start of commands to detect if the user
/// is running from an orphaned or problematic worktree.
///
/// Returns Ok(()) if the worktree is healthy, or an error with guidance.
#[allow(dead_code)] // Tested, for command startup validation
pub fn validate_current_worktree() -> Result<()> {
    let worktrees = list_worktrees()?;

    // Find the current worktree
    let current = worktrees.iter().find(|wt| wt.is_current);

    let Some(current) = current else {
        // Not in a worktree context (rare, but possible)
        return Ok(());
    };

    // Skip validation for bare repos
    if current.is_bare {
        return Ok(());
    }

    // Check if we're on a detached HEAD
    if current.branch.is_none() {
        // Detached HEAD - check if it's because the branch was deleted
        // We can't easily tell, so just warn
        return Ok(()); // Don't block, just let commands handle detached HEAD
    }

    // Verify the branch exists
    let branch = current.branch.as_ref().unwrap();
    let output = Command::new("git")
        .args(["rev-parse", "--verify", &format!("refs/heads/{}", branch)])
        .output()
        .context("Failed to verify branch existence")?;

    if !output.status.success() {
        anyhow::bail!(
            "This worktree's branch '{}' no longer exists!\n\
             The branch may have been deleted from another worktree.\n\n\
             To recover:\n\
             1. Switch to an existing branch: git checkout <branch>\n\
             2. Or remove this worktree: git worktree remove {}",
            branch,
            DisplayPath(&current.path)
        );
    }

    Ok(())
}

/// Check if the repository has multiple worktrees.
///
/// Useful for deciding whether to show worktree-related warnings.
#[allow(dead_code)] // Tested, for worktree-aware commands
pub fn has_multiple_worktrees() -> Result<bool> {
    let worktrees = list_worktrees()?;
    // Filter out bare repos - they don't count as "active" worktrees
    let active_worktrees = worktrees.iter().filter(|wt| !wt.is_bare).count();
    Ok(active_worktrees > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    use serial_test::serial;
    use std::path::Path;
    use std::process::Command;
    use tempfile::tempdir;

    /// RAII guard for safely changing and restoring the current directory
    struct DirGuard {
        original: std::path::PathBuf,
    }

    impl DirGuard {
        fn new(target: &Path) -> Self {
            // Use a known safe fallback if current dir is invalid
            let original = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
            std::env::set_current_dir(target).expect("Failed to change to target directory");
            Self { original }
        }
    }

    impl Drop for DirGuard {
        fn drop(&mut self) {
            // Restore original dir, ignore errors if original was invalid
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    fn init_test_repo(path: &Path) {
        crate::test_context::init_test_repo(path).expect("Failed to initialize test repo");
    }

    #[test]
    #[serial]
    fn test_get_worktree_status_single_worktree() {
        let dir = tempdir().unwrap();
        init_test_repo(dir.path());

        let _guard = DirGuard::new(dir.path());

        let status = get_worktree_status().unwrap();
        assert!(status.branches_in_other_worktrees.is_empty());
    }

    #[test]
    #[serial]
    fn test_get_worktree_status_identifies_branches_in_other_worktrees() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree with branch 'feature-branch'
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "feature-branch"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        let _guard = DirGuard::new(&main_path);

        let status = get_worktree_status().unwrap();
        assert!(status
            .branches_in_other_worktrees
            .contains(&"feature-branch".to_string()));
    }

    #[test]
    #[serial]
    fn test_get_worktree_path_for_branch() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree with branch 'locked-branch'
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "locked-branch"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        let _guard = DirGuard::new(&main_path);

        // Branch in other worktree should return the path
        let path = get_worktree_path_for_branch("locked-branch").unwrap();
        assert!(path.is_some());
        // Canonicalize both paths for comparison (handles /var vs /private/var on macOS)
        let returned_path = path.unwrap().canonicalize().unwrap();
        let expected_path = wt_path.canonicalize().unwrap();
        assert_eq!(returned_path, expected_path);

        // Nonexistent branch should return None
        assert!(get_worktree_path_for_branch("nonexistent-branch").unwrap().is_none());

        // master/main (current branch) should NOT be in other worktrees
        assert!(get_worktree_path_for_branch("master").unwrap().is_none());
    }

    #[test]
    #[serial]
    fn test_check_branches_no_conflicts() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree with branch 'other-branch'
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "other-branch"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        let _guard = DirGuard::new(&main_path);

        // Checking branches that are NOT in other worktrees should succeed
        let result = check_branches_for_worktree_conflicts(&["my-feature".to_string(), "another-feature".to_string()]);
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_check_branches_with_conflicts() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree with branch 'locked-branch'
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "locked-branch"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        let _guard = DirGuard::new(&main_path);

        // Checking a branch that IS in another worktree should fail
        let result = check_branches_for_worktree_conflicts(&["locked-branch".to_string()]);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("locked-branch"));
        assert!(err_msg.contains("checked out in other worktrees"));
    }

    #[test]
    #[serial]
    fn test_check_branches_empty_list() {
        let dir = tempdir().unwrap();
        init_test_repo(dir.path());

        let _guard = DirGuard::new(dir.path());

        // Empty branch list should always succeed
        let result = check_branches_for_worktree_conflicts(&[]);
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_check_branches_partial_conflicts() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree with branch 'locked-branch'
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "locked-branch"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        let _guard = DirGuard::new(&main_path);

        // Mix of conflicting and non-conflicting branches should still fail
        let result = check_branches_for_worktree_conflicts(&[
            "safe-branch".to_string(),
            "locked-branch".to_string(),
            "another-safe".to_string(),
        ]);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // Should mention the conflicting branch
        assert!(err_msg.contains("locked-branch"));
        // Should NOT mention non-conflicting branches
        assert!(!err_msg.contains("safe-branch"));
        assert!(!err_msg.contains("another-safe"));
    }

    #[test]
    #[serial]
    fn test_current_branch_not_in_other_worktrees() {
        // This test verifies that the current worktree's branch is NOT
        // reported as being "in another worktree"
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree with a different branch
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "other-branch"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        // From main worktree, "master" should NOT be in other worktrees
        let _guard = DirGuard::new(&main_path);
        let status = get_worktree_status().unwrap();

        // Current branch (master) should not be in the list
        assert!(!status.branches_in_other_worktrees.contains(&"master".to_string()));
        // Other worktree's branch SHOULD be in the list
        assert!(status.branches_in_other_worktrees.contains(&"other-branch".to_string()));
    }

    // =========================================================================
    // list_worktrees() tests
    // =========================================================================

    #[test]
    #[serial]
    fn test_list_worktrees_single() {
        let dir = tempdir().unwrap();
        init_test_repo(dir.path());

        let _guard = DirGuard::new(dir.path());

        let worktrees = list_worktrees().unwrap();
        assert_eq!(worktrees.len(), 1);
        assert!(worktrees[0].is_current);
        assert!(worktrees[0].branch.is_some());
    }

    #[test]
    #[serial]
    fn test_list_worktrees_multiple() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree with a branch
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "feature"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        let _guard = DirGuard::new(&main_path);

        let worktrees = list_worktrees().unwrap();
        assert_eq!(worktrees.len(), 2);

        // One should be current, one should not
        let current_count = worktrees.iter().filter(|wt| wt.is_current).count();
        assert_eq!(current_count, 1);

        // Find the feature branch worktree
        let feature_wt = worktrees.iter().find(|wt| wt.branch.as_deref() == Some("feature"));
        assert!(feature_wt.is_some());
        assert!(!feature_wt.unwrap().is_current);
    }

    #[test]
    #[serial]
    fn test_list_worktrees_returns_normalized_paths() {
        // This test verifies that paths returned from list_worktrees() are normalized
        // to use the platform's native path separators. This is critical for Windows
        // where git outputs forward slashes but PathBuf uses backslashes.
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "feature"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        let _guard = DirGuard::new(&main_path);

        let worktrees = list_worktrees().unwrap();
        assert_eq!(worktrees.len(), 2);

        // Find the feature branch worktree
        let feature_wt = worktrees.iter().find(|wt| wt.branch.as_deref() == Some("feature"));
        assert!(feature_wt.is_some());
        let feature_wt = feature_wt.unwrap();

        // CRITICAL: Verify that the path can be string-compared with a PathBuf-created path
        // This catches the Windows issue where git outputs "C:/foo/bar" but PathBuf uses "C:\foo\bar"
        let wt_path_str = wt_path.canonicalize().unwrap().to_string_lossy().to_string();
        let returned_path_str = feature_wt.path.to_string_lossy().to_string();

        assert_eq!(
            returned_path_str, wt_path_str,
            "Worktree path should be normalized to match platform PathBuf format.\n\
             Expected: {}\n\
             Got: {}",
            wt_path_str, returned_path_str
        );

        // ALSO verify that when displayed, the path doesn't have Windows UNC prefix
        let displayed = format!("{}", DisplayPath(&feature_wt.path));
        if cfg!(windows) {
            assert!(
                !displayed.starts_with(r"\\?\"),
                "Display path should not contain Windows UNC prefix: {}",
                displayed
            );
        }
    }

    // =========================================================================
    // detect_orphaned_worktrees() tests
    // =========================================================================

    #[test]
    #[serial]
    fn test_detect_orphaned_worktrees_none() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree - branch exists, not orphaned
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "feature"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        let _guard = DirGuard::new(&main_path);

        let orphaned = detect_orphaned_worktrees().unwrap();
        assert!(orphaned.is_empty(), "No worktrees should be orphaned");
    }

    #[test]
    #[serial]
    fn test_detect_orphaned_worktrees_finds_orphan() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree with a branch
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "feature"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        // Force delete the branch ref directly (bypassing worktree protection)
        // This simulates what happens if branch is deleted via database corruption
        // or force operations
        Command::new("git")
            .args(["update-ref", "-d", "refs/heads/feature"])
            .current_dir(&main_path)
            .output()
            .expect("git update-ref -d failed");

        let _guard = DirGuard::new(&main_path);

        let orphaned = detect_orphaned_worktrees().unwrap();
        assert_eq!(orphaned.len(), 1);
        assert_eq!(orphaned[0].missing_branch, "feature");
    }

    // =========================================================================
    // validate_current_worktree() tests
    // =========================================================================

    #[test]
    #[serial]
    fn test_validate_current_worktree_healthy() {
        let dir = tempdir().unwrap();
        init_test_repo(dir.path());

        let _guard = DirGuard::new(dir.path());

        // Normal repo should validate fine
        let result = validate_current_worktree();
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_validate_current_worktree_orphaned() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree with a branch
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "feature"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        // Force delete the branch ref directly (bypassing worktree protection)
        Command::new("git")
            .args(["update-ref", "-d", "refs/heads/feature"])
            .current_dir(&main_path)
            .output()
            .expect("git update-ref -d failed");

        // Now validate from the orphaned worktree
        let _guard = DirGuard::new(&wt_path);

        let result = validate_current_worktree();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("no longer exists"));
        assert!(err_msg.contains("feature"));
    }

    // =========================================================================
    // Detached HEAD handling tests
    // =========================================================================

    #[test]
    #[serial]
    fn test_list_worktrees_detached_head() {
        let dir = tempdir().unwrap();
        init_test_repo(dir.path());

        // Get the current commit hash
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("git rev-parse failed");
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Checkout the commit directly (detached HEAD)
        Command::new("git")
            .args(["checkout", &commit])
            .current_dir(dir.path())
            .output()
            .expect("git checkout failed");

        let _guard = DirGuard::new(dir.path());

        let worktrees = list_worktrees().unwrap();
        assert_eq!(worktrees.len(), 1);
        // Detached HEAD should have branch = None
        assert!(worktrees[0].branch.is_none());
        assert!(worktrees[0].is_current);
    }

    #[test]
    #[serial]
    fn test_detect_orphaned_worktrees_ignores_detached_head() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Get the current commit hash
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&main_path)
            .output()
            .expect("git rev-parse failed");
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Create a worktree in detached HEAD state
        Command::new("git")
            .args(["worktree", "add", "--detach", wt_path.to_str().unwrap(), &commit])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add --detach failed");

        let _guard = DirGuard::new(&main_path);

        // Detached HEAD worktrees should NOT be considered orphaned
        let orphaned = detect_orphaned_worktrees().unwrap();
        assert!(orphaned.is_empty(), "Detached HEAD worktree should not be orphaned");
    }

    #[test]
    #[serial]
    fn test_validate_current_worktree_detached_head_passes() {
        let dir = tempdir().unwrap();
        init_test_repo(dir.path());

        // Get the current commit hash
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("git rev-parse failed");
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Checkout the commit directly (detached HEAD)
        Command::new("git")
            .args(["checkout", &commit])
            .current_dir(dir.path())
            .output()
            .expect("git checkout failed");

        let _guard = DirGuard::new(dir.path());

        // Detached HEAD should pass validation (don't block the user)
        let result = validate_current_worktree();
        assert!(result.is_ok(), "Detached HEAD should pass validation");
    }

    // =========================================================================
    // has_multiple_worktrees() tests
    // =========================================================================

    #[test]
    #[serial]
    fn test_has_multiple_worktrees_single() {
        let dir = tempdir().unwrap();
        init_test_repo(dir.path());

        let _guard = DirGuard::new(dir.path());

        assert!(!has_multiple_worktrees().unwrap());
    }

    #[test]
    #[serial]
    fn test_has_multiple_worktrees_multiple() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "feature"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        let _guard = DirGuard::new(&main_path);

        assert!(has_multiple_worktrees().unwrap());
    }

    #[test]
    #[serial]
    fn test_checkout_error_message_is_user_friendly() {
        // This test verifies that the actual error message from trying to checkout
        // a branch in another worktree doesn't contain Windows UNC prefix
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path).unwrap();
        init_test_repo(&main_path);

        // Create a worktree with a branch
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "locked-branch"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        let _guard = DirGuard::new(&main_path);
        let _ctx = crate::test_context::TestRepoContext::new(&main_path);

        // Try to checkout the locked branch from the main worktree using GitGateway
        let gateway = crate::git_gateway::GitGateway::new().unwrap();
        let result = gateway.checkout_branch_worktree_safe("locked-branch");

        // Should fail
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();

        // Verify error message is informative
        assert!(err_msg.contains("already checked out"));

        // CRITICAL: Verify no Windows UNC prefix in the error message
        if cfg!(windows) {
            assert!(
                !err_msg.contains(r"\\?\"),
                "Error message should not contain Windows UNC prefix: {}",
                err_msg
            );
        }

        // Verify the actual path is shown (not a placeholder)
        // The error should contain some recognizable part of the path
        let path_str = wt_path.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            err_msg.contains("worktree") || err_msg.contains(&path_str),
            "Error message should show the actual path: {}",
            err_msg
        );
    }
}
