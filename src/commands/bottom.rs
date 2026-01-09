use anyhow::Result;
use colored::Colorize;

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Jump to the bottom of the current stack (closest to trunk)
pub fn run() -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    // Silent cleanup of orphaned refs (handles branches deleted via git/IDE)
    gateway.cleanup_orphaned_refs_silently();

    let current = gateway.get_current_branch_name()?;
    let trunk = ref_store.get_trunk()?;

    // Verify current branch is tracked
    if ref_store.get_parent(&current)?.is_none() && trunk.as_ref() != Some(&current) {
        anyhow::bail!(
            "Branch '{}' is not tracked by Diamond. Run '{} track' first.",
            current,
            program_name()
        );
    }

    // Walk up parents until we hit trunk or a root
    let bottom = find_stack_bottom(&ref_store, &current, trunk.as_deref())?;

    if bottom == current {
        println!("{} Already at stack bottom", "✓".green().bold());
    } else {
        gateway.checkout_branch_worktree_safe(&bottom)?;
        println!("{} Jumped to bottom: {}", "✓".green().bold(), bottom.green());
    }

    Ok(())
}

/// Find the bottommost branch (closest to trunk) starting from the given branch
fn find_stack_bottom(ref_store: &RefStore, start: &str, trunk: Option<&str>) -> Result<String> {
    let mut branch = start.to_string();

    loop {
        match ref_store.get_parent(&branch)? {
            Some(parent) if trunk != Some(&parent) => {
                // Parent exists and isn't trunk, keep going up
                branch = parent;
            }
            _ => break, // Reached trunk or root
        }
    }

    Ok(branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    fn create_branch(repo: &git2::Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[test]
    fn test_find_stack_bottom_single_branch() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create branch before setting parent
        create_branch(&repo, "feature")?;

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        let bottom = find_stack_bottom(&ref_store, "feature", Some("main"))?;
        assert_eq!(bottom, "feature"); // feature's parent is trunk, so feature is bottom

        Ok(())
    }

    #[test]
    fn test_find_stack_bottom_linear_stack() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches before setting parents
        create_branch(&repo, "feature-1")?;
        create_branch(&repo, "feature-2")?;
        create_branch(&repo, "feature-3")?;

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature-1", "main")?;
        ref_store.set_parent("feature-2", "feature-1")?;
        ref_store.set_parent("feature-3", "feature-2")?;

        let bottom = find_stack_bottom(&ref_store, "feature-3", Some("main"))?;
        assert_eq!(bottom, "feature-1"); // feature-1's parent is trunk

        let bottom = find_stack_bottom(&ref_store, "feature-2", Some("main"))?;
        assert_eq!(bottom, "feature-1");

        let bottom = find_stack_bottom(&ref_store, "feature-1", Some("main"))?;
        assert_eq!(bottom, "feature-1"); // Already at bottom

        Ok(())
    }

    #[test]
    fn test_find_stack_bottom_no_trunk() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches before setting parents
        create_branch(&repo, "feature-1")?;
        create_branch(&repo, "feature-2")?;

        let ref_store = RefStore::new()?;
        // No trunk set
        // feature-1 has no parent
        ref_store.set_parent("feature-2", "feature-1")?;

        let bottom = find_stack_bottom(&ref_store, "feature-2", None)?;
        assert_eq!(bottom, "feature-1"); // feature-1 has no parent

        Ok(())
    }

    #[test]
    fn test_bottom_untracked_branch_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Empty ref_store - branch not tracked
        let _ref_store = RefStore::new().unwrap();
        // Don't set any parent refs

        let result = run();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));
    }

    #[test]
    fn test_bottom_already_at_bottom() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Track main as trunk
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Should succeed (already at bottom since it's trunk)
        let result = run();
        assert!(result.is_ok());
    }

    #[test]
    fn test_bottom_with_uncommitted_changes_fails() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();
        let ref_store = RefStore::new().unwrap();

        // Create and commit a tracked file on main
        std::fs::write(dir.path().join("tracked.txt"), "original").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Add tracked file", &tree, &[&parent])
            .unwrap();

        // Set up stack: main -> middle -> top
        ref_store.set_trunk("main").unwrap();
        gateway.create_branch("middle").unwrap();
        ref_store.set_parent("middle", "main").unwrap();
        gateway.create_branch("top").unwrap();
        ref_store.set_parent("top", "middle").unwrap();

        // Modify the tracked file without committing (on top branch)
        std::fs::write(dir.path().join("tracked.txt"), "modified").unwrap();

        // Try to navigate to bottom - should fail with uncommitted changes
        let result = run();
        assert!(result.is_err(), "dm bottom should fail with uncommitted changes");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("uncommitted changes"),
            "Error should mention uncommitted changes: {}",
            err_msg
        );

        // Should still be on top
        assert_eq!(gateway.get_current_branch_name().unwrap(), "top");
    }
}
