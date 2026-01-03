use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use anyhow::{Context, Result};
use colored::Colorize;

pub fn run_track(branch: Option<String>, parent: Option<String>) -> Result<()> {
    let branch_name = match branch {
        Some(b) => b,
        None => {
            let gateway = GitGateway::new()?;
            gateway
                .get_current_branch_name()
                .context("Failed to get current branch")?
        }
    };

    let ref_store = RefStore::new()?;

    // Check if already tracked (read from refs)
    if ref_store.is_tracked(&branch_name)? {
        println!("{} Branch '{}' is already tracked", "Note:".yellow(), branch_name);
        return Ok(());
    }

    // Determine parent: explicit parent or default to trunk
    let parent_name = match parent {
        Some(p) => p,
        None => {
            // Default to trunk
            let trunk = ref_store.get_trunk()?.ok_or_else(|| {
                anyhow::anyhow!(
                    "No trunk branch configured. Run '{} init' first or use '{} track --parent <parent>'.",
                    program_name(),
                    program_name()
                )
            })?;
            trunk
        }
    };

    // Set the parent relationship
    ref_store.set_parent(&branch_name, &parent_name)?;
    println!(
        "{} Now tracking branch '{}' with parent '{}'",
        "Success:".green(),
        branch_name,
        parent_name
    );
    Ok(())
}

pub fn run_untrack(branch: Option<String>) -> Result<()> {
    let branch_name = match branch {
        Some(b) => b,
        None => {
            let gateway = GitGateway::new()?;
            gateway
                .get_current_branch_name()
                .context("Failed to get current branch")?
        }
    };

    // Remove from refs
    let ref_store = RefStore::new()?;
    ref_store.remove_parent(&branch_name)?;

    println!("{} Untracked branch '{}'", "Success:".green(), branch_name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_gateway::GitGateway;
    use crate::ref_store::RefStore;
    use anyhow::Result;
    use git2::Repository;

    use tempfile::tempdir;

    use crate::test_context::TestRepoContext;

    fn init_test_repo(path: &std::path::Path) -> Result<Repository> {
        let repo = Repository::init(path)?;
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
        drop(tree);
        Ok(repo)
    }

    #[test]
    fn test_track_without_trunk_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Track current branch without trunk configured - should fail
        let result = run_track(None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("trunk") || err.contains("init"));

        Ok(())
    }

    #[test]
    fn test_track_already_tracked_branch() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Set up a tracked branch via refs
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Track should report it's already tracked
        run_track(Some("feature".to_string()), None)?;

        // It should still be tracked
        assert!(ref_store.is_tracked("feature")?);

        Ok(())
    }

    #[test]
    fn test_track_idempotent() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        let gateway = GitGateway::new()?;

        // Set trunk and create branch
        ref_store.set_trunk("main")?;
        gateway.create_branch("branch")?;

        // Track same branch twice - second call should report already tracked
        run_track(Some("branch".to_string()), None)?;
        run_track(Some("branch".to_string()), None)?;

        // Verify it's still tracked (idempotent)
        assert!(ref_store.is_tracked("branch")?);
        assert_eq!(ref_store.get_parent("branch")?, Some("main".to_string()));

        Ok(())
    }

    #[test]
    fn test_untrack_tracked_branch() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Set up a tracked branch via refs
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Verify it's tracked
        assert!(ref_store.is_tracked("feature")?);

        // Untrack
        run_untrack(Some("feature".to_string()))?;

        // Verify it's no longer tracked
        assert!(!ref_store.is_tracked("feature")?);

        Ok(())
    }

    #[test]
    fn test_untrack_removes_from_parent() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create structure: main -> feature via refs
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Verify main has feature as child
        assert!(ref_store.get_children("main")?.contains("feature"));

        // Untrack feature
        run_untrack(Some("feature".to_string()))?;

        // Verify main no longer has feature as child
        assert!(!ref_store.get_children("main")?.contains("feature"));

        Ok(())
    }

    #[test]
    fn test_untrack_nonexistent_branch() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Untrack something that doesn't exist - should not fail
        run_untrack(Some("does-not-exist".to_string()))?;

        Ok(())
    }

    #[test]
    fn test_track_with_parent() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Set up trunk first
        ref_store.set_trunk("main")?;

        // Track a branch with explicit parent
        run_track(Some("feature".to_string()), Some("main".to_string()))?;

        // Verify parent-child relationship
        assert_eq!(ref_store.get_parent("feature")?, Some("main".to_string()));
        assert!(ref_store.get_children("main")?.contains("feature"));

        Ok(())
    }

    #[test]
    fn test_track_with_parent_creates_relationship() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        // Create and switch to a feature branch first
        let gateway = GitGateway::new()?;
        gateway.create_branch("feature")?;

        // Track current branch (feature) with parent (main)
        run_track(None, Some("main".to_string()))?;

        // Verify it's tracked with the parent
        let current = gateway.get_current_branch_name()?;
        assert_eq!(current, "feature");
        assert_eq!(ref_store.get_parent(&current)?, Some("main".to_string()));

        Ok(())
    }

    // TDD: Default to trunk behavior

    #[test]
    fn test_track_defaults_to_trunk() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        let gateway = GitGateway::new()?;

        // Set trunk
        ref_store.set_trunk("main")?;

        // Create and checkout a feature branch
        gateway.create_branch("feature")?;
        gateway.checkout_branch("feature")?;

        // Track without parent - should default to trunk
        run_track(None, None)?;

        // Verify it's tracked with trunk as parent
        assert!(ref_store.is_tracked("feature")?);
        assert_eq!(ref_store.get_parent("feature")?, Some("main".to_string()));

        Ok(())
    }

    #[test]
    fn test_track_explicit_parent_overrides_default() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        let gateway = GitGateway::new()?;

        // Set trunk
        ref_store.set_trunk("main")?;

        // Create feature branches
        gateway.create_branch("feature-1")?;
        gateway.create_branch("feature-2")?;
        ref_store.set_parent("feature-1", "main")?; // Track feature-1 first

        gateway.checkout_branch("feature-2")?;

        // Track with explicit parent (not trunk)
        run_track(None, Some("feature-1".to_string()))?;

        // Verify it's tracked with explicit parent, not trunk
        assert!(ref_store.is_tracked("feature-2")?);
        assert_eq!(ref_store.get_parent("feature-2")?, Some("feature-1".to_string()));

        Ok(())
    }

    #[test]
    fn test_track_trunk_itself_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Set trunk to "main"
        ref_store.set_trunk("main")?;

        // Try to track trunk itself (self-referential)
        let result = run_track(Some("main".to_string()), None);

        // Should fail with clear error about self-referential parent
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("cannot be its own parent") || err.contains("circular"),
            "Error should mention self-referential/circular, got: {}",
            err
        );

        Ok(())
    }

    #[test]
    fn test_track_when_trunk_missing_in_git() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        let gateway = GitGateway::new()?;

        // Set trunk to "main"
        ref_store.set_trunk("main")?;

        // Delete the main branch from git (but trunk is still configured)
        // Note: We can't delete current branch, so create and switch to feature first
        gateway.create_branch("feature")?;
        gateway.checkout_branch("feature")?;

        // Now delete main branch
        let mut main_branch = repo.find_branch("main", git2::BranchType::Local)?;
        main_branch.delete()?;

        // Try to track feature with missing trunk
        let result = run_track(None, None);

        // Should fail with clear error about parent not existing
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does not exist") || err.contains("main"),
            "Error should mention parent/trunk doesn't exist, got: {}",
            err
        );

        Ok(())
    }
}
