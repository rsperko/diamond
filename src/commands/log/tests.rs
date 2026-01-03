//! Tests for log command.

use super::*;
use crate::branch_tree::build_branch_tree;
use crate::test_context::TestRepoContext;
use tempfile::tempdir;

fn init_test_repo(path: &std::path::Path) -> anyhow::Result<git2::Repository> {
    let repo = git2::Repository::init(path)?;
    let sig = git2::Signature::now("Test User", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
    drop(tree);
    Ok(repo)
}

#[test]
fn test_find_roots_with_trunk() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("feature", "main")?;

    let roots = find_roots(&ref_store)?;
    assert_eq!(roots.len(), 1);
    assert!(roots.contains(&"main".to_string()));

    Ok(())
}

#[test]
fn test_find_roots_empty() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    // No trunk set

    let roots = find_roots(&ref_store)?;
    assert!(roots.is_empty());

    Ok(())
}

#[test]
fn test_build_branch_tree() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create branches in git
    let head = repo.head()?.peel_to_commit()?;
    repo.branch("feature", &head, false)?;

    let gateway = GitGateway::from_path(dir.path())?;

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("feature", "main")?;

    let rows = build_branch_tree(&ref_store, "main", &gateway)?;

    // After reversal, main should be at the bottom (last)
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "feature");
    assert_eq!(rows[1].name, "main");
    assert!(rows[1].is_current);

    Ok(())
}

#[test]
fn test_build_branch_tree_with_depth() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create branches in git
    let head = repo.head()?.peel_to_commit()?;
    repo.branch("level1", &head, false)?;
    repo.branch("level2", &head, false)?;

    let gateway = GitGateway::from_path(dir.path())?;
    gateway.checkout_branch("level1")?;

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("level1", "main")?;
    ref_store.set_parent("level2", "level1")?;

    let rows = build_branch_tree(&ref_store, "level1", &gateway)?;

    // After reversal: level2, level1, main
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].name, "level2");
    assert_eq!(rows[0].depth, 2);
    assert_eq!(rows[1].name, "level1");
    assert_eq!(rows[1].depth, 1);
    assert!(rows[1].is_current);
    assert_eq!(rows[2].name, "main");
    assert_eq!(rows[2].depth, 0);

    Ok(())
}

#[test]
fn test_needs_restack_indicator() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create main -> feature structure
    let head = repo.head()?.peel_to_commit()?;
    repo.branch("feature", &head, false)?;

    let gateway = GitGateway::from_path(dir.path())?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("feature", "main")?;

    // Feature is based on main - should NOT need restack
    let rows = build_branch_tree(&ref_store, "main", &gateway)?;
    let feature_row = rows.iter().find(|r| r.name == "feature").unwrap();
    assert!(!feature_row.needs_restack);

    // Now add a commit to main (simulate parent moving ahead)
    gateway.checkout_branch("main")?;
    std::fs::write(dir.path().join("new_file.txt"), "content")?;
    let mut index = repo.index()?;
    index.add_path(std::path::Path::new("new_file.txt"))?;
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let parent = repo.head()?.peel_to_commit()?;
    let sig = git2::Signature::now("Test", "test@example.com")?;
    repo.commit(Some("HEAD"), &sig, &sig, "New commit on main", &tree, &[&parent])?;

    // Now feature is NOT based on main's new tip - should need restack
    let rows = build_branch_tree(&ref_store, "main", &gateway)?;
    let feature_row = rows.iter().find(|r| r.name == "feature").unwrap();
    assert!(feature_row.needs_restack);

    Ok(())
}

#[test]
fn test_build_branch_tree_no_trunk() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::from_path(dir.path())?;
    let ref_store = RefStore::new()?;
    // No trunk set

    let rows = build_branch_tree(&ref_store, "main", &gateway)?;

    // Should return empty when no trunk is configured
    assert!(rows.is_empty());

    Ok(())
}

#[test]
fn test_build_branch_tree_trunk_only() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::from_path(dir.path())?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // No children - just trunk

    let rows = build_branch_tree(&ref_store, "main", &gateway)?;

    // Should have just trunk
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "main");
    assert_eq!(rows[0].depth, 0);
    assert!(rows[0].is_current);

    Ok(())
}

#[test]
fn test_build_branch_tree_multiple_children_sorted() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create branches in git (create in non-alphabetical order)
    let head = repo.head()?.peel_to_commit()?;
    repo.branch("zebra", &head, false)?;
    repo.branch("alpha", &head, false)?;
    repo.branch("middle", &head, false)?;

    let gateway = GitGateway::from_path(dir.path())?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // All are children of main
    ref_store.set_parent("zebra", "main")?;
    ref_store.set_parent("alpha", "main")?;
    ref_store.set_parent("middle", "main")?;

    let rows = build_branch_tree(&ref_store, "main", &gateway)?;

    // After reversal, trunk is at bottom, children above in alphabetical order
    // Stack order (bottom to top): main, then children in reverse-alpha order at top
    // Since we reverse, the DFS order (main, alpha, middle, zebra) becomes (zebra, middle, alpha, main)
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[3].name, "main"); // trunk at bottom

    // Children should be in alphabetical order (alpha, middle, zebra) before reversal
    // After reversal: zebra, middle, alpha (reverse alphabetical at top)
    assert_eq!(rows[0].name, "zebra");
    assert_eq!(rows[1].name, "middle");
    assert_eq!(rows[2].name, "alpha");

    Ok(())
}

#[test]
fn test_build_branch_tree_trunk_always_at_bottom() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a deep stack
    let head = repo.head()?.peel_to_commit()?;
    repo.branch("level1", &head, false)?;
    repo.branch("level2", &head, false)?;
    repo.branch("level3", &head, false)?;

    let gateway = GitGateway::from_path(dir.path())?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("level1", "main")?;
    ref_store.set_parent("level2", "level1")?;
    ref_store.set_parent("level3", "level2")?;

    let rows = build_branch_tree(&ref_store, "level2", &gateway)?;

    // Trunk must always be at the last position (bottom of stack)
    assert!(!rows.is_empty());
    assert_eq!(rows.last().unwrap().name, "main");
    assert_eq!(rows.last().unwrap().depth, 0);

    Ok(())
}

#[test]
fn test_build_branch_tree_child_above_parent() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let head = repo.head()?.peel_to_commit()?;
    repo.branch("child", &head, false)?;

    let gateway = GitGateway::from_path(dir.path())?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("child", "main")?;

    let rows = build_branch_tree(&ref_store, "main", &gateway)?;

    // Find positions
    let child_pos = rows.iter().position(|r| r.name == "child").unwrap();
    let parent_pos = rows.iter().position(|r| r.name == "main").unwrap();

    // Child should appear before (above) parent in the list
    // Lower index = higher in display = above
    assert!(
        child_pos < parent_pos,
        "Child should appear above parent: child at {}, parent at {}",
        child_pos,
        parent_pos
    );

    Ok(())
}

#[test]
fn test_build_branch_tree_untracked_current_branch() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a branch but don't track it
    let head = repo.head()?.peel_to_commit()?;
    repo.branch("tracked", &head, false)?;
    repo.branch("untracked", &head, false)?;

    let gateway = GitGateway::from_path(dir.path())?;
    gateway.checkout_branch("untracked")?; // Current branch is untracked

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("tracked", "main")?;
    // "untracked" is NOT tracked

    let rows = build_branch_tree(&ref_store, "untracked", &gateway)?;

    // Tree should only contain tracked branches
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|r| r.name == "main"));
    assert!(rows.iter().any(|r| r.name == "tracked"));
    assert!(!rows.iter().any(|r| r.name == "untracked"));

    // No branch should be marked as current since current is untracked
    assert!(!rows.iter().any(|r| r.is_current));

    Ok(())
}
