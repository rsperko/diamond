//! Tests for RefStore.

use super::*;
use anyhow::Result;
use git2::Repository;
use std::path::Path;
use tempfile::tempdir;

fn init_test_repo(path: &Path) -> Result<Repository> {
    let repo = Repository::init(path)?;

    // Configure user for commits
    let mut config = repo.config()?;
    config.set_str("user.name", "Test User")?;
    config.set_str("user.email", "test@example.com")?;

    // Make initial commit so HEAD is valid
    let sig = git2::Signature::now("Test User", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;

    // Drop tree explicitly before returning repo
    drop(tree);

    Ok(repo)
}

/// Get the current branch name (the default branch after init)
fn get_current_branch(repo: &Repository) -> Result<String> {
    let head = repo.head()?;
    let name = head.shorthand().ok_or_else(|| anyhow::anyhow!("No branch name"))?;
    Ok(name.to_string())
}

fn create_branch(repo: &Repository, name: &str) -> Result<()> {
    let head = repo.head()?.peel_to_commit()?;
    repo.branch(name, &head, false)?;
    Ok(())
}

#[test]
fn test_set_and_get_parent() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-1")?;

    let store = RefStore::from_path(dir.path())?;

    // Initially no parent
    assert_eq!(store.get_parent("feature-1")?, None);

    // Set parent
    store.set_parent("feature-1", &trunk)?;

    // Verify parent is set
    assert_eq!(store.get_parent("feature-1")?, Some(trunk));

    Ok(())
}

#[test]
fn test_update_parent() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-1")?;
    create_branch(&repo, "develop")?;

    let store = RefStore::from_path(dir.path())?;

    // Set initial parent
    store.set_parent("feature-1", &trunk)?;
    assert_eq!(store.get_parent("feature-1")?, Some(trunk));

    // Update parent
    store.set_parent("feature-1", "develop")?;
    assert_eq!(store.get_parent("feature-1")?, Some("develop".to_string()));

    Ok(())
}

#[test]
fn test_remove_parent() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-1")?;

    let store = RefStore::from_path(dir.path())?;

    // Set parent
    store.set_parent("feature-1", &trunk)?;
    assert_eq!(store.get_parent("feature-1")?, Some(trunk));

    // Remove parent
    store.remove_parent("feature-1")?;
    assert_eq!(store.get_parent("feature-1")?, None);

    // Removing again is ok (idempotent)
    store.remove_parent("feature-1")?;

    Ok(())
}

#[test]
fn test_get_children() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-1")?;
    create_branch(&repo, "feature-2")?;
    create_branch(&repo, "feature-1-sub")?;

    let store = RefStore::from_path(dir.path())?;

    // Set up tree: trunk -> feature-1, feature-2
    //              feature-1 -> feature-1-sub
    store.set_parent("feature-1", &trunk)?;
    store.set_parent("feature-2", &trunk)?;
    store.set_parent("feature-1-sub", "feature-1")?;

    // Check children of trunk
    let trunk_children = store.get_children(&trunk)?;
    assert_eq!(trunk_children.len(), 2);
    assert!(trunk_children.contains("feature-1"));
    assert!(trunk_children.contains("feature-2"));

    // Check children of feature-1
    let f1_children = store.get_children("feature-1")?;
    assert_eq!(f1_children.len(), 1);
    assert!(f1_children.contains("feature-1-sub"));

    // Check children of leaf (none)
    let leaf_children = store.get_children("feature-1-sub")?;
    assert_eq!(leaf_children.len(), 0);

    Ok(())
}

#[test]
fn test_list_tracked_branches() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;
    create_branch(&repo, "feature-c")?;

    let store = RefStore::from_path(dir.path())?;

    // Initially no tracked branches
    assert_eq!(store.list_tracked_branches()?, Vec::<String>::new());

    // Track some branches
    store.set_parent("feature-c", &trunk)?;
    store.set_parent("feature-a", &trunk)?;
    store.set_parent("feature-b", "feature-a")?;

    // Should be sorted alphabetically
    let tracked = store.list_tracked_branches()?;
    assert_eq!(tracked, vec!["feature-a", "feature-b", "feature-c"]);

    Ok(())
}

#[test]
fn test_set_and_get_trunk() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;

    let store = RefStore::from_path(dir.path())?;

    // Initially no trunk (Diamond not initialized)
    assert_eq!(store.get_trunk()?, None);
    assert!(!store.is_initialized()?);

    // Set trunk
    store.set_trunk(&trunk)?;

    // Verify trunk is set
    assert_eq!(store.get_trunk()?, Some(trunk.clone()));
    assert!(store.is_initialized()?);

    Ok(())
}

#[test]
fn test_require_trunk() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;

    let store = RefStore::from_path(dir.path())?;

    // Should fail when not initialized
    assert!(store.require_trunk().is_err());

    // Set trunk
    store.set_trunk(&trunk)?;

    // Should succeed now
    assert_eq!(store.require_trunk()?, trunk);

    Ok(())
}

#[test]
fn test_set_trunk_nonexistent_branch_fails() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;

    let store = RefStore::from_path(dir.path())?;

    // Attempt to set trunk to a branch that doesn't exist
    let result = store.set_trunk("nonexistent-branch");

    // Should fail with clear error message
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("does not exist") || err.contains("nonexistent"),
        "Error should mention branch doesn't exist. Got: {}",
        err
    );

    Ok(())
}

#[test]
fn test_collect_branches_dfs() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;
    create_branch(&repo, "feature-a-1")?;
    create_branch(&repo, "feature-a-2")?;

    let store = RefStore::from_path(dir.path())?;

    // Set up tree:
    // trunk -> feature-a -> feature-a-1
    //                    -> feature-a-2
    //       -> feature-b
    store.set_parent("feature-a", &trunk)?;
    store.set_parent("feature-b", &trunk)?;
    store.set_parent("feature-a-1", "feature-a")?;
    store.set_parent("feature-a-2", "feature-a")?;

    // Collect from roots (children of trunk)
    let branches = store.collect_branches_dfs(&["feature-a".to_string(), "feature-b".to_string()])?;

    // Should be DFS order: feature-a, feature-a-1, feature-a-2, feature-b
    // (children sorted alphabetically within each level)
    assert_eq!(branches.len(), 4);
    assert_eq!(branches[0], "feature-a");
    // feature-a's children come next (sorted)
    assert!(branches[1..3].contains(&"feature-a-1".to_string()));
    assert!(branches[1..3].contains(&"feature-a-2".to_string()));
    assert_eq!(branches[3], "feature-b");

    Ok(())
}

#[test]
fn test_collect_branches_dfs_deep_stack() -> Result<()> {
    // Test that moderately deep stacks work correctly
    // (validates depth tracking works without hitting the 1000 limit)
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;

    let store = RefStore::from_path(dir.path())?;

    // Create a linear chain of 50 branches
    const DEPTH: usize = 50;
    let mut prev = trunk.clone();
    for i in 0..DEPTH {
        let name = format!("branch-{}", i);
        create_branch(&repo, &name)?;
        store.set_parent(&name, &prev)?;
        prev = name;
    }

    // Collect from root
    let branches = store.collect_branches_dfs(&["branch-0".to_string()])?;

    // Should have all 50 branches in order
    assert_eq!(branches.len(), DEPTH);
    for (i, branch) in branches.iter().enumerate() {
        assert_eq!(branch, &format!("branch-{}", i));
    }

    Ok(())
}

#[test]
fn test_reparent() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "develop")?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;

    // Initial parent
    store.set_parent("feature", &trunk)?;
    assert_eq!(store.get_parent("feature")?, Some(trunk.clone()));

    // Reparent
    store.reparent("feature", "develop")?;
    assert_eq!(store.get_parent("feature")?, Some("develop".to_string()));

    // Verify children updated correctly
    let trunk_children = store.get_children(&trunk)?;
    let develop_children = store.get_children("develop")?;

    assert!(!trunk_children.contains("feature"));
    assert!(develop_children.contains("feature"));

    Ok(())
}

#[test]
fn test_is_tracked() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "tracked")?;
    create_branch(&repo, "untracked")?;

    let store = RefStore::from_path(dir.path())?;

    // Nothing tracked yet
    assert!(!store.is_tracked("tracked")?);
    assert!(!store.is_tracked("untracked")?);

    // Track one
    store.set_parent("tracked", &trunk)?;

    // Check
    assert!(store.is_tracked("tracked")?);
    assert!(!store.is_tracked("untracked")?);

    Ok(())
}

#[test]
fn test_special_branch_names() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature/auth")?;
    create_branch(&repo, "fix-bug-123")?;

    let store = RefStore::from_path(dir.path())?;

    // Branch names with slashes and hyphens
    store.set_parent("feature/auth", &trunk)?;
    store.set_parent("fix-bug-123", &trunk)?;

    assert_eq!(store.get_parent("feature/auth")?, Some(trunk.clone()));
    assert_eq!(store.get_parent("fix-bug-123")?, Some(trunk));

    let tracked = store.list_tracked_branches()?;
    assert!(tracked.contains(&"feature/auth".to_string()));
    assert!(tracked.contains(&"fix-bug-123".to_string()));

    Ok(())
}

#[test]
fn test_blob_content_is_just_branch_name() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_parent("feature", &trunk)?;

    // Verify the blob contains just the branch name (not refs/heads/...)
    let reference = repo.find_reference("refs/diamond/parent/feature")?;
    let blob = repo.find_blob(reference.target().unwrap())?;
    let content = String::from_utf8(blob.content().to_vec())?;

    assert_eq!(content, trunk);
    assert!(!content.contains("refs/heads/"));

    Ok(())
}

#[test]
fn test_remove_branch_reparent() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "parent")?;
    create_branch(&repo, "middle")?;
    create_branch(&repo, "child-1")?;
    create_branch(&repo, "child-2")?;

    let store = RefStore::from_path(dir.path())?;

    // Set up: trunk -> parent -> middle -> child-1, child-2
    store.set_parent("parent", &trunk)?;
    store.set_parent("middle", "parent")?;
    store.set_parent("child-1", "middle")?;
    store.set_parent("child-2", "middle")?;

    // Remove middle - children should reparent to "parent"
    store.remove_branch_reparent("middle")?;

    // Verify middle is untracked
    assert!(!store.is_tracked("middle")?);

    // Verify children now point to parent (grandparent)
    assert_eq!(store.get_parent("child-1")?, Some("parent".to_string()));
    assert_eq!(store.get_parent("child-2")?, Some("parent".to_string()));

    // Verify parent's children include the reparented ones
    let parent_children = store.get_children("parent")?;
    assert!(parent_children.contains("child-1"));
    assert!(parent_children.contains("child-2"));
    assert!(!parent_children.contains("middle"));

    Ok(())
}

#[test]
fn test_remove_branch_reparent_leaf() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_parent("feature", &trunk)?;

    // Remove leaf branch (no children to reparent)
    store.remove_branch_reparent("feature")?;

    assert!(!store.is_tracked("feature")?);
    assert!(store.get_children(&trunk)?.is_empty());

    Ok(())
}

#[test]
fn test_register_branch_with_parent() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;

    // Register with parent
    store.register_branch("feature", Some(&trunk))?;
    assert_eq!(store.get_parent("feature")?, Some(trunk));

    Ok(())
}

#[test]
fn test_register_branch_with_none_removes_parent() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;

    // Set parent first
    store.set_parent("feature", &trunk)?;
    assert!(store.is_tracked("feature")?);

    // Register with None removes parent
    store.register_branch("feature", None)?;
    assert!(!store.is_tracked("feature")?);

    Ok(())
}

#[test]
fn test_remove_branch() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature")?;
    create_branch(&repo, "child")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_parent("feature", &trunk)?;
    store.set_parent("child", "feature")?;

    // remove_branch does NOT reparent children (unlike remove_branch_reparent)
    store.remove_branch("feature")?;

    // Feature is untracked
    assert!(!store.is_tracked("feature")?);

    // Child still points to feature (orphaned)
    assert_eq!(store.get_parent("child")?, Some("feature".to_string()));

    Ok(())
}

#[test]
fn test_get_parent_nonexistent_branch() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;

    let store = RefStore::from_path(dir.path())?;

    // Getting parent of non-existent branch returns None (not error)
    assert_eq!(store.get_parent("does-not-exist")?, None);

    Ok(())
}

#[test]
fn test_get_children_nonexistent_branch() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;

    let store = RefStore::from_path(dir.path())?;

    // Getting children of non-existent branch returns empty set
    let children = store.get_children("does-not-exist")?;
    assert!(children.is_empty());

    Ok(())
}

#[test]
fn test_fresh_repo_is_not_initialized() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;

    let store = RefStore::from_path(dir.path())?;

    // Fresh repo has no trunk set
    assert!(!store.is_initialized()?);
    assert_eq!(store.get_trunk()?, None);
    assert!(store.list_tracked_branches()?.is_empty());

    Ok(())
}

// ===== Frozen Branch Tests =====

#[test]
fn test_is_frozen_default_false() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;

    // Branches are not frozen by default
    assert!(!store.is_frozen("feature")?);

    Ok(())
}

#[test]
fn test_set_frozen_and_check() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;

    // Freeze branch
    store.set_frozen("feature", true)?;
    assert!(store.is_frozen("feature")?);

    // Unfreeze branch
    store.set_frozen("feature", false)?;
    assert!(!store.is_frozen("feature")?);

    Ok(())
}

#[test]
fn test_freeze_is_idempotent() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;

    // Freezing multiple times is ok
    store.set_frozen("feature", true)?;
    store.set_frozen("feature", true)?;
    assert!(store.is_frozen("feature")?);

    // Unfreezing multiple times is ok
    store.set_frozen("feature", false)?;
    store.set_frozen("feature", false)?;
    assert!(!store.is_frozen("feature")?);

    Ok(())
}

#[test]
fn test_list_frozen_branches() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;
    create_branch(&repo, "feature-c")?;

    let store = RefStore::from_path(dir.path())?;

    // No frozen branches initially
    assert!(store.list_frozen_branches()?.is_empty());

    // Freeze some
    store.set_frozen("feature-c", true)?;
    store.set_frozen("feature-a", true)?;

    // Should be sorted
    let frozen = store.list_frozen_branches()?;
    assert_eq!(frozen, vec!["feature-a", "feature-c"]);

    Ok(())
}

#[test]
fn test_clear_all_removes_everything() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;

    let store = RefStore::from_path(dir.path())?;

    // Set up some state
    store.set_trunk(&trunk)?;
    store.set_parent("feature-a", &trunk)?;
    store.set_parent("feature-b", "feature-a")?;
    store.set_frozen("feature-a", true)?;

    // Verify state is set
    assert!(store.is_initialized()?);
    assert_eq!(store.list_tracked_branches()?.len(), 2);
    assert!(store.is_frozen("feature-a")?);

    // Clear all
    store.clear_all()?;

    // Verify everything is cleared
    assert!(!store.is_initialized()?);
    assert!(store.list_tracked_branches()?.is_empty());
    assert!(!store.is_frozen("feature-a")?);
    assert!(store.list_frozen_branches()?.is_empty());

    Ok(())
}

// ============================================================================
// walk_ancestors() tests
// ============================================================================

#[test]
fn test_walk_ancestors_linear_chain() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;
    create_branch(&repo, "feature-c")?;

    let store = RefStore::from_path(dir.path())?;

    // Set up: trunk <- feature-a <- feature-b <- feature-c
    store.set_trunk(&trunk)?;
    store.set_parent("feature-a", &trunk)?;
    store.set_parent("feature-b", "feature-a")?;
    store.set_parent("feature-c", "feature-b")?;

    // Walk from feature-c to trunk
    let ancestors = store.walk_ancestors("feature-c", Some(&trunk))?;

    // Should return [feature-b, feature-a] (not including trunk)
    assert_eq!(ancestors.len(), 2);
    assert_eq!(ancestors[0], "feature-b");
    assert_eq!(ancestors[1], "feature-a");

    Ok(())
}

#[test]
fn test_walk_ancestors_stops_at_trunk() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-a")?;

    let store = RefStore::from_path(dir.path())?;

    // Set up: trunk <- feature-a
    store.set_trunk(&trunk)?;
    store.set_parent("feature-a", &trunk)?;

    // Walk from feature-a - should stop at trunk (not include it)
    let ancestors = store.walk_ancestors("feature-a", Some(&trunk))?;

    assert!(ancestors.is_empty());

    Ok(())
}

#[test]
fn test_walk_ancestors_no_parent() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "orphan")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;

    // orphan has no parent set
    let ancestors = store.walk_ancestors("orphan", Some(&trunk))?;

    assert!(ancestors.is_empty());

    Ok(())
}

#[test]
fn test_walk_ancestors_without_trunk_walks_to_end() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;
    create_branch(&repo, "feature-c")?;

    let store = RefStore::from_path(dir.path())?;

    // Set up chain without trunk: feature-a <- feature-b <- feature-c
    // (feature-a has no parent)
    store.set_parent("feature-b", "feature-a")?;
    store.set_parent("feature-c", "feature-b")?;

    // Walk with trunk=None - should walk all the way
    let ancestors = store.walk_ancestors("feature-c", None)?;

    assert_eq!(ancestors.len(), 2);
    assert_eq!(ancestors[0], "feature-b");
    assert_eq!(ancestors[1], "feature-a");

    Ok(())
}

#[test]
fn test_walk_ancestors_detects_cycle() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;

    let store = RefStore::from_path(dir.path())?;

    // Create a cycle: feature-a -> feature-b -> feature-a
    store.set_parent("feature-a", "feature-b")?;
    store.set_parent("feature-b", "feature-a")?;

    // Walking should detect the cycle and error
    let result = store.walk_ancestors("feature-a", None);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Circular parent reference"));

    Ok(())
}

#[test]
fn test_walk_ancestors_detects_self_cycle() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    create_branch(&repo, "feature-a")?;

    let store = RefStore::from_path(dir.path())?;

    // Create a self-referential cycle by directly manipulating refs
    // (simulating corrupted metadata - set_parent now rejects this)
    let ref_name = "refs/diamond/parent/feature-a";
    let blob_oid = repo.blob("feature-a".as_bytes())?;
    repo.reference(ref_name, blob_oid, true, "test: create self-cycle")?;

    // Walking should detect the cycle and error
    let result = store.walk_ancestors("feature-a", None);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Circular parent reference"));

    Ok(())
}

// ============================================================================
// RefStore::lock() tests
// ============================================================================

#[test]
fn test_ref_store_lock_acquires_and_releases() -> Result<()> {
    let dir = tempdir()?;
    init_test_repo(dir.path())?;

    let store = RefStore::from_path(dir.path())?;

    // Acquire lock through RefStore
    let guard = store.lock()?;

    // Lock file should exist
    let lock_path = dir.path().join(".git").join("diamond").join("lock");
    assert!(lock_path.exists());

    // Drop and verify we can re-acquire
    drop(guard);
    let _guard2 = store.lock()?;

    Ok(())
}

#[test]
fn test_ref_store_try_lock_succeeds_when_unlocked() -> Result<()> {
    let dir = tempdir()?;
    init_test_repo(dir.path())?;

    let store = RefStore::from_path(dir.path())?;

    // Try lock should succeed
    let guard = store.try_lock()?;
    assert!(guard.is_some());

    Ok(())
}

// ============================================================================
// Circular Reference Prevention Tests
// ============================================================================

#[test]
fn test_set_parent_rejects_self_reference() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;

    // Attempting to set a branch as its own parent should fail
    let result = store.set_parent("feature", "feature");
    assert!(result.is_err(), "set_parent should reject self-referential parent");

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("cannot be its own parent") || err_msg.contains("self-referential"),
        "Error message should mention self-reference: {}",
        err_msg
    );

    Ok(())
}

#[test]
fn test_walk_ancestors_detects_three_node_cycle() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;
    create_branch(&repo, "feature-c")?;

    let store = RefStore::from_path(dir.path())?;

    // Create a 3-node cycle: A -> B -> C -> A
    // First set up the chain: B's parent = A, C's parent = B
    store.set_parent("feature-b", "feature-a")?;
    store.set_parent("feature-c", "feature-b")?;

    // Now create the cycle by making A's parent point to C
    // We bypass set_parent validation to simulate corrupted metadata
    let ref_name = "refs/diamond/parent/feature-a";
    let blob_oid = repo.blob("feature-c".as_bytes())?;
    repo.reference(ref_name, blob_oid, true, "test: create cycle")?;

    // Walking should detect the cycle
    let result = store.walk_ancestors("feature-a", None);
    assert!(result.is_err(), "walk_ancestors should detect 3-node cycle");

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Circular parent reference"),
        "Error message should mention circular reference: {}",
        err_msg
    );

    Ok(())
}

// ============================================================================
// Concurrent Access Tests
// ============================================================================

#[test]
fn test_concurrent_set_parent_with_locking() -> Result<()> {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-1")?;
    create_branch(&repo, "feature-2")?;

    let path = dir.path().to_path_buf();
    let barrier = Arc::new(Barrier::new(2));
    let trunk_clone = trunk.clone();

    // Spawn two threads that try to set parents simultaneously
    let path1 = path.clone();
    let barrier1 = Arc::clone(&barrier);
    let trunk1 = trunk_clone.clone();
    let handle1 = thread::spawn(move || -> Result<()> {
        let store = RefStore::from_path(&path1)?;
        barrier1.wait(); // Synchronize thread start

        // Use locking to ensure atomic operation
        let _lock = store.lock()?;
        store.set_parent("feature-1", &trunk1)?;
        Ok(())
    });

    let path2 = path.clone();
    let barrier2 = Arc::clone(&barrier);
    let trunk2 = trunk_clone;
    let handle2 = thread::spawn(move || -> Result<()> {
        let store = RefStore::from_path(&path2)?;
        barrier2.wait(); // Synchronize thread start

        // Use locking to ensure atomic operation
        let _lock = store.lock()?;
        store.set_parent("feature-2", &trunk2)?;
        Ok(())
    });

    // Both threads should complete successfully
    handle1.join().expect("Thread 1 panicked")?;
    handle2.join().expect("Thread 2 panicked")?;

    // Verify final state is consistent
    let store = RefStore::from_path(&path)?;
    assert_eq!(store.get_parent("feature-1")?, Some(trunk.clone()));
    assert_eq!(store.get_parent("feature-2")?, Some(trunk));

    Ok(())
}

#[test]
fn test_concurrent_traversal_during_modification() -> Result<()> {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;
    create_branch(&repo, "feature-c")?;

    let store = RefStore::from_path(dir.path())?;

    // Set up initial chain: trunk <- a <- b <- c
    store.set_trunk(&trunk)?;
    store.set_parent("feature-a", &trunk)?;
    store.set_parent("feature-b", "feature-a")?;
    store.set_parent("feature-c", "feature-b")?;

    let path = dir.path().to_path_buf();
    let barrier = Arc::new(Barrier::new(2));

    // Thread 1: Traverse ancestors
    let path1 = path.clone();
    let barrier1 = Arc::clone(&barrier);
    let trunk1 = trunk.clone();
    let handle1 = thread::spawn(move || -> Result<Vec<String>> {
        let store = RefStore::from_path(&path1)?;
        barrier1.wait();

        // Perform multiple traversals
        let mut results = Vec::new();
        for _ in 0..10 {
            match store.walk_ancestors("feature-c", Some(&trunk1)) {
                Ok(ancestors) => results.push(ancestors.len().to_string()),
                Err(e) => results.push(format!("error: {}", e)),
            }
        }
        Ok(results)
    });

    // Thread 2: Modify parent relationships
    let path2 = path;
    let barrier2 = Arc::clone(&barrier);
    let handle2 = thread::spawn(move || -> Result<()> {
        let store = RefStore::from_path(&path2)?;
        barrier2.wait();

        // Rapidly change parent relationships
        for _ in 0..10 {
            store.set_parent("feature-b", "feature-a")?;
        }
        Ok(())
    });

    // Both threads should complete without panics or infinite loops
    let results = handle1.join().expect("Thread 1 panicked")?;
    handle2.join().expect("Thread 2 panicked")?;

    // Verify no panics occurred and results are valid
    for result in results {
        // Each result should either be a number (ancestor count) or a cycle error
        // It should NOT be empty or cause a panic
        assert!(
            result.parse::<usize>().is_ok() || result.starts_with("error:"),
            "Unexpected result: {}",
            result
        );
    }

    Ok(())
}

// ============================================================================
// ancestors() and descendants() tests - Unified Traversal API
// ============================================================================

#[test]
fn test_ancestors_linear_stack() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;
    create_branch(&repo, "feature-c")?;

    let store = RefStore::from_path(dir.path())?;

    // Set up: trunk <- feature-a <- feature-b <- feature-c
    store.set_trunk(&trunk)?;
    store.set_parent("feature-a", &trunk)?;
    store.set_parent("feature-b", "feature-a")?;
    store.set_parent("feature-c", "feature-b")?;

    // Get ancestors of feature-c (should return trunk-to-branch order)
    let ancestors = store.ancestors("feature-c")?;

    // Should be [feature-a, feature-b, feature-c] - trunk's child first, current last
    assert_eq!(ancestors.len(), 3);
    assert_eq!(ancestors[0], "feature-a");
    assert_eq!(ancestors[1], "feature-b");
    assert_eq!(ancestors[2], "feature-c");

    Ok(())
}

#[test]
fn test_ancestors_direct_child_of_trunk() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;
    store.set_parent("feature", &trunk)?;

    // Only branch in stack - should return just itself
    let ancestors = store.ancestors("feature")?;

    assert_eq!(ancestors.len(), 1);
    assert_eq!(ancestors[0], "feature");

    Ok(())
}

#[test]
fn test_ancestors_requires_trunk() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;
    // No trunk set

    let result = store.ancestors("feature");
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_ancestors_orphan_branch() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "orphan")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;
    // orphan has no parent set

    // Should return just the branch itself (walks 0 parents)
    let ancestors = store.ancestors("orphan")?;
    assert_eq!(ancestors.len(), 1);
    assert_eq!(ancestors[0], "orphan");

    Ok(())
}

#[test]
fn test_descendants_with_children() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature")?;
    create_branch(&repo, "child-a")?;
    create_branch(&repo, "child-b")?;
    create_branch(&repo, "grandchild")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;
    store.set_parent("feature", &trunk)?;
    store.set_parent("child-a", "feature")?;
    store.set_parent("child-b", "feature")?;
    store.set_parent("grandchild", "child-a")?;

    // Get descendants of feature (should NOT include feature itself)
    let descendants = store.descendants("feature")?;

    // Should be [child-a, grandchild, child-b] in DFS order (sorted children)
    assert_eq!(descendants.len(), 3);
    assert!(descendants.contains(&"child-a".to_string()));
    assert!(descendants.contains(&"child-b".to_string()));
    assert!(descendants.contains(&"grandchild".to_string()));
    // Should NOT contain feature
    assert!(!descendants.contains(&"feature".to_string()));

    Ok(())
}

#[test]
fn test_descendants_leaf_branch() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "leaf")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;
    store.set_parent("leaf", &trunk)?;

    // Leaf branch has no descendants
    let descendants = store.descendants("leaf")?;
    assert!(descendants.is_empty());

    Ok(())
}

#[test]
fn test_descendants_nonexistent_branch() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;

    let store = RefStore::from_path(dir.path())?;

    // Non-existent branch has no descendants (just returns empty)
    let descendants = store.descendants("does-not-exist")?;
    assert!(descendants.is_empty());

    Ok(())
}

// ============================================================================
// compute_tree_prefix() tests - Visualization Tree Prefixes
// ============================================================================

#[test]
fn test_compute_tree_prefix_root_returns_empty() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;
    store.set_parent("feature", &trunk)?;

    // Root of the stack should have no prefix
    let prefix = store.compute_tree_prefix("feature", "feature");
    assert_eq!(prefix, "");

    Ok(())
}

#[test]
fn test_compute_tree_prefix_linear_stack() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;
    create_branch(&repo, "feature-c")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;
    store.set_parent("feature-a", &trunk)?;
    store.set_parent("feature-b", "feature-a")?;
    store.set_parent("feature-c", "feature-b")?;

    // In a linear stack A -> B -> C with A as root:
    // - A has no prefix (it's the root)
    // - B shows "└─" (only child of A)
    // - C shows "   └─" (continuation from B which was last child)
    let prefix_a = store.compute_tree_prefix("feature-a", "feature-a");
    let prefix_b = store.compute_tree_prefix("feature-b", "feature-a");
    let prefix_c = store.compute_tree_prefix("feature-c", "feature-a");

    assert_eq!(prefix_a, "");
    assert_eq!(prefix_b, "└─");
    // C's prefix has 3 non-breaking spaces (B was last child) then └─
    assert!(prefix_c.ends_with("└─"));
    assert!(prefix_c.len() > 2); // Has some spacing before the connector

    Ok(())
}

#[test]
fn test_compute_tree_prefix_branching_stack() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "feature")?;
    create_branch(&repo, "child-a")?;
    create_branch(&repo, "child-b")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;
    store.set_parent("feature", &trunk)?;
    store.set_parent("child-a", "feature")?;
    store.set_parent("child-b", "feature")?;

    // With two children (child-a and child-b, sorted alphabetically):
    // - child-a (first child): "├─"
    // - child-b (last child): "└─"
    let prefix_a = store.compute_tree_prefix("child-a", "feature");
    let prefix_b = store.compute_tree_prefix("child-b", "feature");

    assert_eq!(prefix_a, "├─"); // Not last child
    assert_eq!(prefix_b, "└─"); // Last child

    Ok(())
}

#[test]
fn test_compute_tree_prefix_deep_branching() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "root")?;
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "a1")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;
    store.set_parent("root", &trunk)?;
    store.set_parent("a", "root")?;
    store.set_parent("b", "root")?;
    store.set_parent("a1", "a")?;

    // Tree structure:
    // root
    // ├─ a
    // │  └─ a1
    // └─ b

    // a1's prefix: "│" (a has sibling b) + spaces + "└─" (a1 is a's only child)
    let prefix_a1 = store.compute_tree_prefix("a1", "root");

    // Should contain a vertical line for the continuation from root->a level
    assert!(prefix_a1.contains('│'));
    assert!(prefix_a1.ends_with("└─"));

    Ok(())
}

#[test]
fn test_compute_tree_prefix_orphan_branch() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "orphan")?;
    create_branch(&repo, "root")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;
    store.set_parent("root", &trunk)?;
    // orphan has no parent relationship to root

    // Orphan branch that can't reach root still gets a connector
    // (the function treats it as a leaf node in its own mini-tree)
    let prefix = store.compute_tree_prefix("orphan", "root");
    assert_eq!(prefix, "└─");

    Ok(())
}

#[test]
fn test_compute_tree_prefix_wide_tree_many_siblings() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let trunk = get_current_branch(&repo)?;
    create_branch(&repo, "root")?;
    create_branch(&repo, "child-a")?;
    create_branch(&repo, "child-b")?;
    create_branch(&repo, "child-c")?;
    create_branch(&repo, "child-d")?;
    create_branch(&repo, "grandchild-b1")?;

    let store = RefStore::from_path(dir.path())?;
    store.set_trunk(&trunk)?;
    store.set_parent("root", &trunk)?;
    // Four children of root (sorted: a, b, c, d)
    store.set_parent("child-a", "root")?;
    store.set_parent("child-b", "root")?;
    store.set_parent("child-c", "root")?;
    store.set_parent("child-d", "root")?;
    // Grandchild of child-b
    store.set_parent("grandchild-b1", "child-b")?;

    // Tree structure:
    // root
    // ├─ child-a       (first, not last → ├─)
    // ├─ child-b       (middle, not last → ├─)
    // │  └─ grandchild-b1  (child of middle sibling → │ continuation)
    // ├─ child-c       (middle, not last → ├─)
    // └─ child-d       (last → └─)

    let prefix_a = store.compute_tree_prefix("child-a", "root");
    let prefix_b = store.compute_tree_prefix("child-b", "root");
    let prefix_c = store.compute_tree_prefix("child-c", "root");
    let prefix_d = store.compute_tree_prefix("child-d", "root");
    let prefix_grandchild = store.compute_tree_prefix("grandchild-b1", "root");

    // First three children should have ├─ (not last)
    assert_eq!(prefix_a, "├─", "child-a should be ├─ (not last child)");
    assert_eq!(prefix_b, "├─", "child-b should be ├─ (not last child)");
    assert_eq!(prefix_c, "├─", "child-c should be ├─ (not last child)");
    // Last child should have └─
    assert_eq!(prefix_d, "└─", "child-d should be └─ (last child)");

    // Grandchild of child-b:
    // - child-b is NOT last, so needs vertical line (│) continuation
    // - grandchild-b1 IS last child of child-b, so gets └─
    assert!(
        prefix_grandchild.contains('│'),
        "grandchild should have │ continuation (parent child-b has siblings below)"
    );
    assert!(
        prefix_grandchild.ends_with("└─"),
        "grandchild should end with └─ (only child of child-b)"
    );

    Ok(())
}
