use crate::git_gateway::GitGateway;
use crate::ref_store::RefStore;
use anyhow::Result;

/// Navigate to the parent branch (down the stack)
///
/// If steps > 1, navigates multiple levels down.
pub fn run(steps: usize) -> Result<()> {
    if steps == 0 {
        return Ok(());
    }

    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    // Silent cleanup of orphaned refs (handles branches deleted via git/IDE)
    gateway.cleanup_orphaned_refs_silently();

    let mut current = gateway.get_current_branch_name()?;

    for step in 0..steps {
        // Get parent from refs
        let parent = ref_store.get_parent(&current)?.ok_or_else(|| {
            if step == 0 {
                anyhow::anyhow!("Already at stack root (no parent)")
            } else {
                anyhow::anyhow!("Reached stack root after {} step(s)", step)
            }
        })?;

        // Checkout parent safely (fail if uncommitted changes)
        gateway.checkout_branch_worktree_safe(&parent)?;

        if steps == 1 {
            println!("Switched to parent branch: {}", parent);
        } else {
            println!("Step {}: switched to {}", step + 1, parent);
        }

        current = parent;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_gateway::GitGateway;
    use git2::Repository;
    use tempfile::tempdir;

    use crate::test_context::{init_test_repo_with_branch, TestRepoContext};

    fn create_branch(repo: &Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[test]
    fn test_down_to_parent() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set up refs: main -> feature
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Create feature branch in git
        gateway.create_branch("feature")?;

        // Now on feature, go down to main
        run(1)?;

        assert_eq!(gateway.get_current_branch_name()?, "main");
        Ok(())
    }

    #[test]
    fn test_down_at_root_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Set up refs with main as root (no parent)
        ref_store.set_trunk("main")?;

        // Try to go down from root - main has no parent
        let result = run(1);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("root"));

        Ok(())
    }

    #[test]
    fn test_down_untracked_branch_has_no_parent() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create branch in git but don't track it
        gateway.create_branch("untracked")?;

        // Try to go down from untracked branch - it has no parent
        let result = run(1);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("root"));

        Ok(())
    }

    #[test]
    fn test_down_through_full_stack() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create all branches in git first (parents must exist before set_parent)
        create_branch(&repo, "level1")?;
        create_branch(&repo, "level2")?;

        // Set up refs: main -> level1 -> level2 -> level3
        ref_store.set_trunk("main")?;
        ref_store.set_parent("level1", "main")?;
        ref_store.set_parent("level2", "level1")?;
        ref_store.set_parent("level3", "level2")?;

        // Create level3 and checkout to it
        gateway.create_branch("level3")?;

        // Now on level3, navigate all the way down
        assert_eq!(gateway.get_current_branch_name()?, "level3");

        run(1)?;
        assert_eq!(gateway.get_current_branch_name()?, "level2");

        run(1)?;
        assert_eq!(gateway.get_current_branch_name()?, "level1");

        run(1)?;
        assert_eq!(gateway.get_current_branch_name()?, "main");

        // Can't go further
        let result = run(1);
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_down_multiple_steps() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create all branches in git first (parents must exist before set_parent)
        create_branch(&repo, "level1")?;
        create_branch(&repo, "level2")?;

        // Set up refs: main -> level1 -> level2 -> level3
        ref_store.set_trunk("main")?;
        ref_store.set_parent("level1", "main")?;
        ref_store.set_parent("level2", "level1")?;
        ref_store.set_parent("level3", "level2")?;

        // Create level3 and checkout to it
        gateway.create_branch("level3")?;

        // Now on level3, navigate down 2 steps
        assert_eq!(gateway.get_current_branch_name()?, "level3");

        run(2)?;
        assert_eq!(gateway.get_current_branch_name()?, "level1");

        Ok(())
    }

    #[test]
    fn test_down_too_many_steps_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set up refs: main -> feature
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Create feature branch in git
        gateway.create_branch("feature")?;

        // Try to go down 5 steps (only 1 parent exists)
        let result = run(5);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Reached stack root"));

        Ok(())
    }

    #[test]
    fn test_down_zero_steps_noop() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        gateway.create_branch("feature")?;

        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Zero steps should be a no-op
        run(0)?;
        assert_eq!(gateway.get_current_branch_name()?, "feature");

        Ok(())
    }

    #[test]
    fn test_down_with_uncommitted_changes_fails() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create and commit a tracked file on main
        std::fs::write(dir.path().join("tracked.txt"), "original")?;
        let mut index = repo.index()?;
        index.add_path(std::path::Path::new("tracked.txt"))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let parent = repo.head()?.peel_to_commit()?;
        let sig = git2::Signature::now("Test", "test@test.com")?;
        repo.commit(Some("HEAD"), &sig, &sig, "Add tracked file", &tree, &[&parent])?;

        // Set up stack: main -> feature
        ref_store.set_trunk("main")?;
        gateway.create_branch("feature")?;
        ref_store.set_parent("feature", "main")?;

        // Modify the tracked file without committing (on feature branch)
        std::fs::write(dir.path().join("tracked.txt"), "modified")?;

        // Try to navigate down - should fail with uncommitted changes
        let result = run(1);
        assert!(result.is_err(), "dm down should fail with uncommitted changes");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("uncommitted changes"),
            "Error should mention uncommitted changes: {}",
            err_msg
        );

        // Should still be on feature
        assert_eq!(gateway.get_current_branch_name()?, "feature");

        Ok(())
    }
}
