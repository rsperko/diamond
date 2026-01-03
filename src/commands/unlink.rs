//! Unlink command - disassociates a branch from its PR
//!
//! This removes the cached PR URL for a branch without
//! deleting the branch or closing the PR on GitHub.

use anyhow::Result;
use colored::Colorize;

use crate::cache::Cache;
use crate::git_gateway::GitGateway;
use crate::program_name::program_name;

/// Unlink the current branch from its associated PR
pub fn run() -> Result<()> {
    let gateway = GitGateway::new()?;
    let current = gateway.get_current_branch_name()?;

    let mut cache = Cache::load().unwrap_or_default();

    // Check if branch has a PR URL
    let pr_url = cache.get_pr_url(&current).map(|s| s.to_string());

    if pr_url.is_none() {
        println!("{} Branch '{}' is not linked to any PR", "ℹ".blue(), current.yellow());
        return Ok(());
    }

    // Remove PR association
    cache.remove_pr_url(&current);
    cache.save()?;

    if let Some(url) = pr_url {
        println!(
            "{} Unlinked '{}' from {}",
            "✓".green().bold(),
            current.green(),
            url.blue()
        );
    }

    println!(
        "\n{} The PR still exists on GitHub. Run '{} submit' to create a new PR.",
        "ℹ".blue(),
        program_name()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_gateway::GitGateway;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_unlink_removes_pr_url() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Get current branch name (could be main or master)
        let gateway = GitGateway::new()?;
        let current = gateway.get_current_branch_name()?;

        // Set up cache with PR URL for current branch
        let mut cache = Cache::load().unwrap_or_default();
        cache.set_pr_url(&current, "https://github.com/org/repo/pull/42");
        cache.save()?;

        // Verify it exists
        let cache = Cache::load()?;
        assert!(cache.get_pr_url(&current).is_some());

        // Run unlink
        run()?;

        // Verify it's gone
        let cache = Cache::load()?;
        assert!(cache.get_pr_url(&current).is_none());

        Ok(())
    }

    #[test]
    fn test_unlink_succeeds_when_not_linked() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Run unlink on branch with no PR
        let result = run();
        assert!(result.is_ok(), "Unlink should succeed even when not linked");

        Ok(())
    }
}
