use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use anyhow::Result;

/// Navigate to a child branch (up the stack)
///
/// If there are multiple children, picks the first one alphabetically.
/// If steps > 1, navigates multiple levels up.
/// If `to` is specified, navigates directly to that specific descendant branch.
pub fn run(steps: usize, to: Option<String>) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    // Silent cleanup of orphaned refs (handles branches deleted via git/IDE)
    if let Err(_e) = crate::validation::silent_cleanup_orphaned_refs(&gateway) {}

    let current = gateway.get_current_branch_name()?;

    // Handle --to flag: navigate directly to a specific descendant
    if let Some(ref target) = to {
        return navigate_to_branch(&gateway, &ref_store, &current, target);
    }

    // Standard step-based navigation
    if steps == 0 {
        return Ok(());
    }

    let mut current_branch = current;

    for step in 0..steps {
        // Get children from refs
        let children_set = ref_store.get_children(&current_branch)?;

        if children_set.is_empty() {
            if step == 0 {
                // Check if current branch is tracked
                let trunk = ref_store.get_trunk()?.unwrap_or_default();
                if current_branch != trunk && !ref_store.is_tracked(&current_branch)? {
                    anyhow::bail!(
                        "Branch '{}' is not tracked. Run '{} track' to add it to a stack.",
                        current_branch,
                        program_name()
                    );
                }
                anyhow::bail!("No child branches");
            } else {
                anyhow::bail!("Reached stack top after {} step(s)", step);
            }
        }

        // Pick first child (alphabetically)
        let mut children: Vec<_> = children_set.iter().collect();
        children.sort();
        let child = children[0];

        // Checkout child safely (fail if uncommitted changes)
        gateway.checkout_branch_safe(child)?;

        if steps == 1 {
            if children.len() > 1 {
                println!("Switched to child branch: {} (of {} children)", child, children.len());
            } else {
                println!("Switched to child branch: {}", child);
            }
        } else {
            println!("Step {}: switched to {}", step + 1, child);
        }

        current_branch.clone_from(child);
    }

    Ok(())
}

/// Navigate directly to a specific descendant branch
fn navigate_to_branch(gateway: &GitGateway, ref_store: &RefStore, from: &str, target: &str) -> Result<()> {
    // Verify target is a descendant of current branch
    if !is_descendant(ref_store, target, from)? {
        anyhow::bail!(
            "Branch '{}' is not a descendant of '{}'. Use '{} checkout {}' instead.",
            target,
            from,
            program_name(),
            target
        );
    }

    // Checkout the target branch safely (fail if uncommitted changes)
    gateway.checkout_branch_safe(target)?;
    println!("Switched to descendant branch: {}", target);

    Ok(())
}

/// Check if `branch` is a descendant of `ancestor` (i.e., ancestor is in branch's parent chain)
fn is_descendant(ref_store: &RefStore, branch: &str, ancestor: &str) -> Result<bool> {
    let mut current = branch.to_string();
    let mut visited = std::collections::HashSet::new();

    while let Some(parent) = ref_store.get_parent(&current)? {
        if parent == ancestor {
            return Ok(true);
        }

        // Cycle detection
        if !visited.insert(current.clone()) {
            return Ok(false);
        }

        current = parent;
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_gateway::GitGateway;
    use tempfile::tempdir;

    use crate::test_context::{init_test_repo_with_branch, TestRepoContext};

    #[test]
    fn test_up_single_child() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create feature branch in git
        gateway.create_branch("feature")?;

        // Go back to main
        gateway.checkout_branch("main")?;

        // Set up refs: main -> feature
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Now on main, go up to feature
        run(1, None)?;

        assert_eq!(gateway.get_current_branch_name()?, "feature");
        Ok(())
    }

    #[test]
    fn test_up_multiple_children_picks_first() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create branches in git (in reverse alphabetical order)
        gateway.create_branch("zebra")?;
        gateway.checkout_branch("main")?;
        gateway.create_branch("apple")?;
        gateway.checkout_branch("main")?;
        gateway.create_branch("middle")?;
        gateway.checkout_branch("main")?;

        // Set up refs: main -> apple, middle, zebra
        ref_store.set_trunk("main")?;
        ref_store.set_parent("zebra", "main")?;
        ref_store.set_parent("apple", "main")?;
        ref_store.set_parent("middle", "main")?;

        // Now on main, should go to "apple" (first alphabetically)
        run(1, None)?;

        assert_eq!(gateway.get_current_branch_name()?, "apple");
        Ok(())
    }

    #[test]
    fn test_up_no_children_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create feature branch (which has no children)
        gateway.create_branch("feature")?;

        // Set up refs: main -> feature (feature has no children)
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Try to go up from feature (no children)
        let result = run(1, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No child"));

        Ok(())
    }

    #[test]
    fn test_up_untracked_branch_shows_helpful_message() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set trunk so we have a reference point
        ref_store.set_trunk("main")?;

        // Create branch in git but don't track it
        gateway.create_branch("untracked")?;

        // Try to go up from untracked branch - should give helpful message
        let result = run(1, None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not tracked") || err_msg.contains("track"),
            "Expected tracking message, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_up_through_full_stack() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create branches in git
        gateway.create_branch("level1")?;
        gateway.create_branch("level2")?;
        gateway.create_branch("level3")?;

        // Go back to main
        gateway.checkout_branch("main")?;

        // Set up refs: main -> level1 -> level2 -> level3
        ref_store.set_trunk("main")?;
        ref_store.set_parent("level1", "main")?;
        ref_store.set_parent("level2", "level1")?;
        ref_store.set_parent("level3", "level2")?;

        // Navigate all the way up
        assert_eq!(gateway.get_current_branch_name()?, "main");

        run(1, None)?;
        assert_eq!(gateway.get_current_branch_name()?, "level1");

        run(1, None)?;
        assert_eq!(gateway.get_current_branch_name()?, "level2");

        run(1, None)?;
        assert_eq!(gateway.get_current_branch_name()?, "level3");

        // Can't go further
        let result = run(1, None);
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_up_multiple_steps() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create branches in git
        gateway.create_branch("level1")?;
        gateway.create_branch("level2")?;
        gateway.create_branch("level3")?;

        // Go back to main
        gateway.checkout_branch("main")?;

        // Set up refs: main -> level1 -> level2 -> level3
        ref_store.set_trunk("main")?;
        ref_store.set_parent("level1", "main")?;
        ref_store.set_parent("level2", "level1")?;
        ref_store.set_parent("level3", "level2")?;

        // From main, navigate up 2 steps
        assert_eq!(gateway.get_current_branch_name()?, "main");

        run(2, None)?;
        assert_eq!(gateway.get_current_branch_name()?, "level2");

        Ok(())
    }

    #[test]
    fn test_up_too_many_steps_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create feature branch
        gateway.create_branch("feature")?;
        gateway.checkout_branch("main")?;

        // Set up refs: main -> feature
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Try to go up 5 steps (only 1 child exists)
        let result = run(5, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Reached stack top"));

        Ok(())
    }

    #[test]
    fn test_up_zero_steps_noop() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        gateway.create_branch("feature")?;
        gateway.checkout_branch("main")?;

        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Zero steps should be a no-op
        run(0, None)?;
        assert_eq!(gateway.get_current_branch_name()?, "main");

        Ok(())
    }

    #[test]
    fn test_up_to_specific_branch() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create branches: main -> level1 -> level2 -> level3
        gateway.create_branch("level1")?;
        gateway.create_branch("level2")?;
        gateway.create_branch("level3")?;
        gateway.checkout_branch("main")?;

        ref_store.set_trunk("main")?;
        ref_store.set_parent("level1", "main")?;
        ref_store.set_parent("level2", "level1")?;
        ref_store.set_parent("level3", "level2")?;

        // Navigate directly to level3 using --to
        run(1, Some("level3".to_string()))?;
        assert_eq!(gateway.get_current_branch_name()?, "level3");

        Ok(())
    }

    #[test]
    fn test_up_to_nonexistent_branch_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        gateway.create_branch("level1")?;
        gateway.checkout_branch("main")?;

        ref_store.set_trunk("main")?;
        ref_store.set_parent("level1", "main")?;

        // Try to navigate to a branch that doesn't exist
        let result = run(1, Some("nonexistent".to_string()));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found") || err_msg.contains("not a descendant"),
            "Expected error about branch not found or not being descendant, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_up_to_non_descendant_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create two separate stacks: main -> stack1, main -> stack2
        gateway.create_branch("stack1")?;
        gateway.checkout_branch("main")?;
        gateway.create_branch("stack2")?;
        gateway.checkout_branch("stack1")?;

        ref_store.set_trunk("main")?;
        ref_store.set_parent("stack1", "main")?;
        ref_store.set_parent("stack2", "main")?;

        // From stack1, try to go to stack2 (not a descendant, it's a sibling)
        let result = run(1, Some("stack2".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a descendant"));

        Ok(())
    }

    #[test]
    fn test_up_with_uncommitted_changes_fails() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create and commit a tracked file on main
        let file_path = dir.path().join("tracked.txt");
        std::fs::write(&file_path, "original content")?;
        let mut index = repo.index()?;
        index.add_path(std::path::Path::new("tracked.txt"))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let parent = repo.head()?.peel_to_commit()?;
        let sig = git2::Signature::now("Test", "test@test.com")?;
        repo.commit(Some("HEAD"), &sig, &sig, "Add tracked file", &tree, &[&parent])?;

        // Set up stack: main -> child
        ref_store.set_trunk("main")?;
        ref_store.set_parent("child", "main")?;

        // Create child branch
        gateway.create_branch("child")?;

        // Go back to main
        let obj = repo.revparse_single("main")?;
        repo.checkout_tree(&obj, None)?;
        repo.set_head("refs/heads/main")?;

        // Modify the tracked file without committing
        std::fs::write(&file_path, "modified uncommitted content")?;

        // Try to navigate up - should fail with uncommitted changes
        let result = run(1, None);
        assert!(result.is_err(), "dm up should fail with uncommitted changes");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("uncommitted changes"),
            "Error should mention uncommitted changes: {}",
            err_msg
        );

        // Verify we're still on main (didn't switch)
        assert_eq!(gateway.get_current_branch_name()?, "main");

        // Verify uncommitted changes are preserved
        let content = std::fs::read_to_string(&file_path)?;
        assert_eq!(
            content, "modified uncommitted content",
            "Uncommitted changes should be preserved"
        );

        Ok(())
    }
}
