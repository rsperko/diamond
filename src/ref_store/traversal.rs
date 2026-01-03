//! Traversal and listing operations for RefStore.

use anyhow::{Context, Result};
use std::collections::HashSet;

use super::{RefStore, PARENT_REF_PREFIX};

/// Maximum depth for walking the parent chain (prevents runaway on circular refs)
const MAX_ANCESTOR_DEPTH: usize = 1000;

/// Maximum depth for DFS traversal (prevents stack overflow on pathological structures)
const MAX_DFS_DEPTH: usize = 1000;

/// Non-breaking space character (prevents markdown from collapsing whitespace in tree prefixes)
const NBSP: char = '\u{00A0}';

/// Maximum depth for tree prefix computation (prevents runaway on circular refs)
const MAX_TREE_PREFIX_DEPTH: usize = 100;

impl RefStore {
    /// Compute tree prefix for branch visualization using box-drawing characters.
    ///
    /// Returns a string like "├─" or "│  └─" showing the branch's position in the tree.
    /// Uses non-breaking spaces to prevent markdown from collapsing whitespace.
    ///
    /// # Arguments
    /// * `branch` - The branch to compute the prefix for
    /// * `root` - The root of the stack (first branch, typically trunk's direct child)
    pub fn compute_tree_prefix(&self, branch: &str, root: &str) -> String {
        if branch == root {
            return String::new();
        }

        // Build path from this branch up to root (excluding root)
        let mut path: Vec<String> = Vec::new();
        let mut current = branch.to_string();

        while current != root {
            path.push(current.clone());
            if let Ok(Some(parent)) = self.get_parent(&current) {
                current = parent;
            } else {
                break;
            }
            // Safety limit
            if path.len() > MAX_TREE_PREFIX_DEPTH {
                break;
            }
        }

        if path.is_empty() {
            return String::new();
        }

        // Reverse to get root-to-branch order
        path.reverse();

        // For each node in path, determine if it's the last child of its parent
        let mut is_last_child: Vec<bool> = Vec::new();

        for node in &path {
            if let Ok(Some(parent)) = self.get_parent(node) {
                let siblings = self.get_children(&parent).unwrap_or_default();
                let mut sorted: Vec<_> = siblings.into_iter().collect();
                sorted.sort();
                is_last_child.push(sorted.last().map(|s| s.as_str()) == Some(node.as_str()));
            } else {
                is_last_child.push(true);
            }
        }

        // Build prefix using box-drawing characters
        let mut prefix = String::new();
        let len = is_last_child.len();

        for (i, &is_last) in is_last_child.iter().enumerate() {
            if i == len - 1 {
                // This is the current node - show connector
                if is_last {
                    prefix.push_str("└─");
                } else {
                    prefix.push_str("├─");
                }
            } else {
                // This is an ancestor - show continuation line or space
                if is_last {
                    // Ancestor was last child, no vertical line needed
                    prefix.push(NBSP);
                    prefix.push(NBSP);
                    prefix.push(NBSP);
                } else {
                    // Ancestor has siblings below, show vertical line
                    prefix.push('│');
                    prefix.push(NBSP);
                    prefix.push(NBSP);
                }
            }
        }

        prefix
    }

    /// Returns ancestors of a branch from trunk towards the branch.
    ///
    /// The returned list is ordered trunk-to-branch: the first element is the
    /// direct child of trunk, and the last element is the branch itself.
    ///
    /// This is the unified replacement for the various `collect_downstack()`
    /// functions scattered across command files.
    ///
    /// # Errors
    /// - Returns error if trunk is not configured
    /// - Returns error if a cycle is detected
    pub fn ancestors(&self, branch: &str) -> Result<Vec<String>> {
        let trunk = self
            .require_trunk()
            .context("ancestors() requires trunk to be configured")?;

        let mut result = vec![branch.to_string()];
        let mut current = branch.to_string();
        let mut seen = HashSet::new();
        seen.insert(current.clone());

        while let Some(parent) = self.get_parent(&current)? {
            // Stop at trunk (don't include trunk in result)
            if parent == trunk {
                break;
            }

            // Cycle detection
            if !seen.insert(parent.clone()) {
                anyhow::bail!("Circular parent reference detected: {} -> ... -> {}", branch, parent);
            }

            // Depth limit
            if result.len() >= MAX_ANCESTOR_DEPTH {
                anyhow::bail!("Parent chain exceeds maximum depth ({})", MAX_ANCESTOR_DEPTH);
            }

            result.push(parent.clone());
            current = parent;
        }

        // Reverse so trunk's child is first, current branch is last
        result.reverse();
        Ok(result)
    }

    /// Returns all descendants of a branch (children, grandchildren, etc).
    ///
    /// The branch itself is NOT included in the result. Branches are returned
    /// in DFS order with siblings sorted alphabetically.
    ///
    /// This is the unified replacement for the various `collect_descendants()`
    /// functions in delete.rs and move_cmd.rs.
    ///
    /// # Errors
    /// - Returns error if the tree exceeds MAX_DFS_DEPTH
    pub fn descendants(&self, branch: &str) -> Result<Vec<String>> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();

        // Get direct children to start
        let mut children: Vec<String> = self.get_children(branch)?.into_iter().collect();
        children.sort();

        // DFS through children
        for child in children {
            self.collect_descendants_recursive(&child, &mut result, &mut visited, 0)?;
        }

        Ok(result)
    }

    fn collect_descendants_recursive(
        &self,
        branch: &str,
        result: &mut Vec<String>,
        visited: &mut HashSet<String>,
        depth: usize,
    ) -> Result<()> {
        if depth >= MAX_DFS_DEPTH {
            anyhow::bail!("Descendant traversal exceeds maximum depth ({})", MAX_DFS_DEPTH);
        }

        if visited.contains(branch) {
            return Ok(());
        }
        visited.insert(branch.to_string());
        result.push(branch.to_string());

        let mut children: Vec<String> = self.get_children(branch)?.into_iter().collect();
        children.sort();

        for child in children {
            self.collect_descendants_recursive(&child, result, visited, depth + 1)?;
        }

        Ok(())
    }
    /// Walk up the parent chain from a branch to trunk, with cycle detection.
    ///
    /// Returns the path from `branch` toward trunk (not including trunk).
    /// If trunk is None, walks until no parent is found.
    ///
    /// # Errors
    /// Returns an error if a cycle is detected or the chain exceeds MAX_ANCESTOR_DEPTH.
    #[allow(dead_code)] // Tested, available for future stack operations
    pub fn walk_ancestors(&self, branch: &str, trunk: Option<&str>) -> Result<Vec<String>> {
        let mut path = Vec::new();
        let mut seen = HashSet::new();
        let mut current = branch.to_string();

        seen.insert(current.clone());

        while let Some(parent) = self.get_parent(&current)? {
            // Stop at trunk
            if trunk.is_some() && trunk == Some(parent.as_str()) {
                break;
            }

            // Cycle detection
            if !seen.insert(parent.clone()) {
                anyhow::bail!(
                    "Circular parent reference detected: {} -> ... -> {}. \
                     Run 'dm cleanup' to repair metadata.",
                    branch,
                    parent
                );
            }

            // Depth limit protection
            if path.len() >= MAX_ANCESTOR_DEPTH {
                anyhow::bail!(
                    "Parent chain exceeds maximum depth ({}). \
                     This may indicate corrupted metadata.",
                    MAX_ANCESTOR_DEPTH
                );
            }

            current = parent.clone();
            path.push(parent);
        }

        Ok(path)
    }

    /// Get all tracked branches (all that have parent refs)
    pub fn list_tracked_branches(&self) -> Result<Vec<String>> {
        let mut branches = Vec::new();

        let pattern = format!("{}*", PARENT_REF_PREFIX);
        for (ref_name, _) in self.gateway.list_references(&pattern)? {
            if let Some(branch) = ref_name.strip_prefix(PARENT_REF_PREFIX) {
                branches.push(branch.to_string());
            }
        }

        branches.sort();
        Ok(branches)
    }

    /// Collect branches in DFS order starting from given roots
    ///
    /// Returns branches in parent-first order suitable for rebasing.
    /// Siblings are sorted alphabetically for determinism.
    ///
    /// # Errors
    /// Returns an error if the traversal exceeds MAX_DFS_DEPTH (prevents stack overflow).
    pub fn collect_branches_dfs(&self, roots: &[String]) -> Result<Vec<String>> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();

        for root in roots {
            self.collect_dfs_recursive(root, &mut result, &mut visited, 0)?;
        }

        Ok(result)
    }

    fn collect_dfs_recursive(
        &self,
        branch: &str,
        result: &mut Vec<String>,
        visited: &mut HashSet<String>,
        depth: usize,
    ) -> Result<()> {
        // Depth limit protection (prevents stack overflow on pathological structures)
        if depth >= MAX_DFS_DEPTH {
            anyhow::bail!(
                "Stack traversal exceeds maximum depth ({}). \
                 This may indicate corrupted metadata or an extremely deep stack.",
                MAX_DFS_DEPTH
            );
        }

        if visited.contains(branch) {
            return Ok(());
        }
        visited.insert(branch.to_string());
        result.push(branch.to_string());

        // Get children sorted alphabetically for determinism
        let mut children: Vec<String> = self.get_children(branch)?.into_iter().collect();
        children.sort();

        for child in children {
            self.collect_dfs_recursive(&child, result, visited, depth + 1)?;
        }

        Ok(())
    }
}
