//! Shared branch tree building for TUI displays.
//!
//! This module provides common functionality for building and displaying
//! branch trees across different TUI commands (log, checkout, etc.).
//!
//! The key concept is "stack order": branches are displayed with trunk at
//! the bottom and children above their parents, matching the mental model
//! of building a stack upward from a base.

use anyhow::Result;

use crate::git_gateway::GitGateway;
use crate::ref_store::RefStore;

/// Information about a branch for display in TUIs.
///
/// Contains all the information needed to render a branch in a tree view,
/// including its position in the hierarchy and status indicators.
#[derive(Clone, Debug)]
pub struct BranchDisplay {
    /// The branch name
    pub name: String,
    /// Depth in the tree (0 = trunk, 1 = direct child of trunk, etc.)
    pub depth: usize,
    /// Relative commit time (e.g., "2 hours ago")
    pub commit_time: String,
    /// Whether this is the currently checked out branch
    pub is_current: bool,
    /// Whether this branch needs to be restacked
    pub needs_restack: bool,
}

/// Build a tree of branches for display in TUIs.
///
/// Returns branches in "stack order": children appear above their parents,
/// with trunk at the bottom. This matches the mental model of a stack where
/// you build upward from a base.
///
/// # Arguments
/// * `ref_store` - The ref store for branch relationships
/// * `current_branch` - The currently checked out branch name
/// * `gateway` - Git gateway for commit information
///
/// # Returns
/// A vector of `BranchDisplay` items in stack order (trunk at bottom)
pub fn build_branch_tree(
    ref_store: &RefStore,
    current_branch: &str,
    gateway: &GitGateway,
) -> Result<Vec<BranchDisplay>> {
    let mut rows = Vec::new();

    // Find roots (trunk)
    let trunk = ref_store.get_trunk()?;
    let roots: Vec<String> = match trunk {
        Some(t) => vec![t],
        None => vec![],
    };

    for root in roots {
        build_tree_recursive(ref_store, &root, current_branch, 0, &mut rows, gateway)?;
    }

    // Reverse so trunk is at bottom (standard stack visualization)
    rows.reverse();
    Ok(rows)
}

fn build_tree_recursive(
    ref_store: &RefStore,
    branch: &str,
    current_branch: &str,
    depth: usize,
    rows: &mut Vec<BranchDisplay>,
    gateway: &GitGateway,
) -> Result<()> {
    let commit_time = gateway.get_commit_time_relative(branch).unwrap_or_default();

    // Check if this branch needs restack (parent's tip is not an ancestor of this branch)
    let needs_restack = if let Ok(Some(parent)) = ref_store.get_parent(branch) {
        !gateway.is_ancestor(&parent, branch).unwrap_or(true)
    } else {
        false
    };

    rows.push(BranchDisplay {
        name: branch.to_string(),
        depth,
        commit_time,
        is_current: branch == current_branch,
        needs_restack,
    });

    // Get and sort children for consistent ordering
    let mut children: Vec<_> = ref_store.get_children(branch)?.into_iter().collect();
    children.sort();

    for child in children {
        build_tree_recursive(ref_store, &child, current_branch, depth + 1, rows, gateway)?;
    }

    Ok(())
}

/// Get detailed commit info for a branch.
///
/// Returns (short_hash, commit_subject, relative_time).
pub fn get_commit_info(gateway: &GitGateway, branch: &str) -> (String, String, String) {
    let hash = gateway.get_short_hash(branch).unwrap_or_default();
    let message = gateway.get_commit_subject(branch).unwrap_or_default();
    let time = gateway.get_commit_time_relative(branch).unwrap_or_default();
    (hash, message, time)
}

/// Branch marker for current vs other branches
pub const MARKER_CURRENT: &str = "◉";
/// Branch marker for non-current branches
pub const MARKER_OTHER: &str = "◯";

/// Format tree indentation for a given depth.
///
/// Returns a string of vertical lines representing the branch's depth in the tree.
pub fn format_indent(depth: usize) -> String {
    if depth > 0 {
        "│ ".repeat(depth)
    } else {
        String::new()
    }
}

/// Find the index of the current branch in a list of BranchDisplay items.
///
/// Returns 0 if not found (safe default for selection).
pub fn find_current_branch_index(branches: &[BranchDisplay]) -> usize {
    branches.iter().position(|b| b.is_current).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use crate::test_context::TestRepoContext;
    use super::*;
    use crate::git_gateway::GitGateway;

    use tempfile::tempdir;

    fn init_test_repo(path: &std::path::Path) -> anyhow::Result<git2::Repository> {
        let repo = git2::Repository::init(path)?;
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit message", &tree, &[])?;
        drop(tree);
        Ok(repo)
    }

    #[test]
    fn test_format_indent_zero_depth() {
        assert_eq!(format_indent(0), "");
    }

    #[test]
    fn test_format_indent_depth_one() {
        assert_eq!(format_indent(1), "│ ");
    }

    #[test]
    fn test_format_indent_depth_two() {
        assert_eq!(format_indent(2), "│ │ ");
    }

    #[test]
    fn test_format_indent_depth_three() {
        assert_eq!(format_indent(3), "│ │ │ ");
    }

    #[test]
    fn test_find_current_branch_index_found() {
        let branches = vec![
            BranchDisplay {
                name: "feature-1".to_string(),
                depth: 0,
                commit_time: String::new(),
                is_current: false,
                needs_restack: false,
            },
            BranchDisplay {
                name: "feature-2".to_string(),
                depth: 0,
                commit_time: String::new(),
                is_current: true,
                needs_restack: false,
            },
            BranchDisplay {
                name: "main".to_string(),
                depth: 0,
                commit_time: String::new(),
                is_current: false,
                needs_restack: false,
            },
        ];

        assert_eq!(find_current_branch_index(&branches), 1);
    }

    #[test]
    fn test_find_current_branch_index_not_found() {
        let branches = vec![BranchDisplay {
            name: "feature".to_string(),
            depth: 0,
            commit_time: String::new(),
            is_current: false,
            needs_restack: false,
        }];

        assert_eq!(find_current_branch_index(&branches), 0);
    }

    #[test]
    fn test_find_current_branch_index_empty() {
        let branches: Vec<BranchDisplay> = vec![];
        assert_eq!(find_current_branch_index(&branches), 0);
    }

    #[test]
    fn test_markers() {
        assert_eq!(MARKER_CURRENT, "◉");
        assert_eq!(MARKER_OTHER, "◯");
    }

    #[test]
    fn test_get_commit_info_valid_branch() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::from_path(dir.path())?;

        let (hash, message, time) = get_commit_info(&gateway, "main");

        // Hash should be non-empty and look like a short git hash
        assert!(!hash.is_empty(), "Hash should not be empty");
        assert!(hash.len() >= 7, "Hash should be at least 7 chars");

        // Message should match what we committed
        assert_eq!(message, "Initial commit message");

        // Time should be non-empty (relative time like "just now" or "X seconds ago")
        assert!(!time.is_empty(), "Time should not be empty");

        Ok(())
    }

    #[test]
    fn test_get_commit_info_nonexistent_branch() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::from_path(dir.path())?;

        // Request info for a branch that doesn't exist
        let (hash, message, time) = get_commit_info(&gateway, "nonexistent-branch");

        // Should return empty strings, not panic
        assert!(hash.is_empty(), "Hash should be empty for nonexistent branch");
        assert!(message.is_empty(), "Message should be empty for nonexistent branch");
        assert!(time.is_empty(), "Time should be empty for nonexistent branch");

        Ok(())
    }
}
