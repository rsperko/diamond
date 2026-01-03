use anyhow::{Context, Result};
use colored::Colorize;

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Freeze a branch to prevent local modifications
pub fn run(branch: Option<String>) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    let trunk = ref_store.require_trunk()?;

    // Use specified branch or current branch
    let branch = match branch {
        Some(b) => b,
        None => gateway.get_current_branch_name()?,
    };

    // Can't freeze trunk
    if branch == trunk {
        anyhow::bail!("Cannot freeze trunk branch '{}'.", trunk);
    }

    // Check if already frozen
    if ref_store.is_frozen(&branch)? {
        println!("{} Branch '{}' is already frozen", "!".yellow(), branch);
        return Ok(());
    }

    // Freeze the branch
    ref_store
        .set_frozen(&branch, true)
        .context(format!("Failed to freeze '{}'", branch))?;

    println!("{} Froze branch '{}'", "✓".green(), branch.cyan());
    println!(
        "  {} Use '{} unfreeze' to allow modifications again",
        "→".dimmed(),
        program_name()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ref_store::RefStore;
    use anyhow::Result;
    use git2::Repository;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    fn create_and_checkout_branch(repo: &Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        repo.set_head(&format!("refs/heads/{}", name))?;
        Ok(())
    }

    #[test]
    fn test_freeze_current_branch() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let trunk = repo.head()?.shorthand().unwrap().to_string();

        // Initialize dm
        let ref_store = RefStore::from_path(dir.path())?;
        ref_store.set_trunk(&trunk)?;

        // Create and checkout a feature branch
        create_and_checkout_branch(&repo, "feature")?;
        ref_store.set_parent("feature", &trunk)?;

        // Change to the test directory
        let _ctx = TestRepoContext::new(dir.path());

        // Freeze with no argument (current branch)
        run(None)?;

        assert!(ref_store.is_frozen("feature")?);

        Ok(())
    }

    #[test]
    fn test_freeze_specified_branch() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let trunk = repo.head()?.shorthand().unwrap().to_string();

        let ref_store = RefStore::from_path(dir.path())?;
        ref_store.set_trunk(&trunk)?;

        // Create a feature branch
        let head = repo.head()?.peel_to_commit()?;
        repo.branch("feature", &head, false)?;
        ref_store.set_parent("feature", &trunk)?;

        let _ctx = TestRepoContext::new(dir.path());

        run(Some("feature".to_string()))?;

        assert!(ref_store.is_frozen("feature")?);

        Ok(())
    }

    #[test]
    fn test_freeze_trunk_fails() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let trunk = repo.head()?.shorthand().unwrap().to_string();

        let ref_store = RefStore::from_path(dir.path())?;
        ref_store.set_trunk(&trunk)?;

        let _ctx = TestRepoContext::new(dir.path());

        let result = run(Some(trunk.clone()));

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot freeze trunk"));

        Ok(())
    }

    #[test]
    fn test_freeze_already_frozen_is_noop() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let trunk = repo.head()?.shorthand().unwrap().to_string();

        let ref_store = RefStore::from_path(dir.path())?;
        ref_store.set_trunk(&trunk)?;

        let head = repo.head()?.peel_to_commit()?;
        repo.branch("feature", &head, false)?;
        ref_store.set_parent("feature", &trunk)?;

        // Pre-freeze the branch
        ref_store.set_frozen("feature", true)?;

        let _ctx = TestRepoContext::new(dir.path());

        // Freeze again should succeed (idempotent)
        run(Some("feature".to_string()))?;

        assert!(ref_store.is_frozen("feature")?);

        Ok(())
    }
}
