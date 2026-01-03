use crate::git_gateway::GitGateway;
use crate::ref_store::RefStore;
use anyhow::Result;

/// Initialize Diamond in a git repository
///
/// If trunk is not specified, attempts to detect main or master branch.
/// If reset is true, clears all existing tracking data first.
pub fn run(trunk: Option<String>, reset: bool) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    // Handle reset flag
    if reset {
        ref_store.clear_all()?;
        println!("Reinitializing Diamond...");
        println!("All branches have been untracked");
    } else {
        // Check if already initialized (only when not resetting)
        if ref_store.is_initialized()? {
            let current_trunk = ref_store.require_trunk()?;
            println!("Diamond is already initialized with trunk: {}", current_trunk);
            return Ok(());
        }
    }

    // Determine trunk branch
    let trunk_name = if let Some(name) = trunk {
        // Verify the specified trunk exists
        if !gateway.branch_exists(&name)? {
            anyhow::bail!("Branch '{}' does not exist", name);
        }
        name
    } else {
        // Auto-detect main or master
        detect_trunk(&gateway)?
    };

    // Set trunk in refs
    ref_store.set_trunk(&trunk_name)?;

    println!("Trunk set to {}", trunk_name);

    // Configure fetch refspec for diamond metadata (if remote exists)
    // This allows `git fetch` to automatically include diamond refs
    if gateway.has_remote(gateway.remote())? {
        if let Err(e) = gateway.configure_diamond_refspec() {
            // Non-fatal: just warn, user can still fetch manually
            eprintln!("Note: Could not configure fetch refspec for diamond refs: {}", e);
        }
    }

    Ok(())
}

/// Detect the trunk branch (main or master)
fn detect_trunk(gateway: &GitGateway) -> Result<String> {
    let branches = gateway.list_branches()?;

    // Prefer "main" over "master"
    if branches.contains(&"main".to_string()) {
        return Ok("main".to_string());
    }
    if branches.contains(&"master".to_string()) {
        return Ok("master".to_string());
    }

    anyhow::bail!("Could not detect trunk branch. Neither 'main' nor 'master' exists. Use --trunk to specify.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;

    use std::path::Path;
    use tempfile::tempdir;

    use crate::test_context::TestRepoContext;

    fn init_test_repo(path: &Path) -> Result<Repository> {
        let repo = Repository::init(path)?;

        // Make initial commit so HEAD is valid
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
        drop(tree);

        Ok(repo)
    }

    fn init_test_repo_with_branch(path: &Path, branch_name: &str) -> Result<Repository> {
        // Set config to use the desired branch name as default
        let repo = Repository::init(path)?;

        // Configure the repo to use our branch name
        let mut config = repo.config()?;
        config.set_str("init.defaultBranch", branch_name)?;

        // Make initial commit with the branch name we want
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        // Create branch directly with desired name
        let refname = format!("refs/heads/{}", branch_name);
        repo.commit(Some(&refname), &sig, &sig, "Initial commit", &tree, &[])?;
        drop(tree);

        // Set HEAD to point to our branch
        repo.set_head(&refname)?;

        Ok(repo)
    }

    #[test]
    fn test_init_detects_main_branch() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        run(None, false)?;

        let ref_store = RefStore::new()?;
        assert!(ref_store.is_initialized()?);
        assert_eq!(ref_store.get_trunk()?, Some("main".to_string()));

        Ok(())
    }

    #[test]
    fn test_init_detects_master_branch() -> Result<()> {
        let dir = tempdir()?;
        // Default git init creates master branch
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Only run if we have master (git default varies)
        let gateway = GitGateway::new()?;
        let branches = gateway.list_branches()?;
        if branches.contains(&"master".to_string()) {
            run(None, false)?;

            let ref_store = RefStore::new()?;
            assert!(ref_store.is_initialized()?);
            assert_eq!(ref_store.get_trunk()?, Some("master".to_string()));
        }

        Ok(())
    }

    #[test]
    fn test_init_with_explicit_trunk() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "develop")?;
        let _ctx = TestRepoContext::new(dir.path());

        run(Some("develop".to_string()), false)?;

        let ref_store = RefStore::new()?;
        assert!(ref_store.is_initialized()?);
        assert_eq!(ref_store.get_trunk()?, Some("develop".to_string()));

        Ok(())
    }

    #[test]
    fn test_init_with_nonexistent_trunk_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let result = run(Some("nonexistent".to_string()), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));

        Ok(())
    }

    #[test]
    fn test_init_already_initialized_warns() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        // Initialize first time
        run(None, false)?;

        // Initialize second time - should not error
        let result = run(None, false);
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_init_sets_trunk_ref() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        run(None, false)?;

        // Verify trunk is set via refs
        let ref_store = RefStore::new()?;
        assert_eq!(ref_store.get_trunk()?, Some("main".to_string()));

        Ok(())
    }

    #[test]
    fn test_init_reset_clears_tracking() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        // Initialize and create some tracked branches
        run(None, false)?;

        let ref_store = RefStore::new()?;
        let gateway = GitGateway::new()?;

        // Create and track a branch
        gateway.create_branch("feature-1")?;
        ref_store.set_parent("feature-1", "main")?;
        assert!(ref_store.is_tracked("feature-1")?);

        // Reset
        run(None, true)?;

        // Verify all tracking is cleared
        let ref_store = RefStore::new()?;
        assert!(!ref_store.is_tracked("feature-1")?);
        assert!(ref_store.is_initialized()?); // Trunk should be re-set
        assert_eq!(ref_store.get_trunk()?, Some("main".to_string()));

        Ok(())
    }
}
