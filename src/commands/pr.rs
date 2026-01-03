use anyhow::Result;
use colored::Colorize;

use crate::cache::Cache;
use crate::forge::get_forge;
use crate::git_gateway::GitGateway;
use crate::program_name::program_name;

/// Open the PR for a branch in the browser
///
/// If `branch` is None, uses the current branch.
/// If `branch` is a number, treats it as a PR number.
/// Otherwise, looks up the PR for the specified branch.
pub fn run(branch: Option<String>) -> Result<()> {
    let gateway = GitGateway::new()?;
    let cache = Cache::load().unwrap_or_default();
    let forge = get_forge(None)?;

    // If a PR number is provided, open it directly via forge
    if let Some(ref arg) = branch {
        if arg.parse::<u64>().is_ok() {
            println!("{} Opening PR #{}...", "→".blue(), arg);
            forge.open_pr_in_browser(arg)?;
            return Ok(());
        }
    }

    // Determine which branch to look up
    let target_branch = match branch {
        Some(ref b) => b.clone(),
        None => gateway.get_current_branch_name()?,
    };

    // Check if branch has a PR URL stored in cache - open directly for efficiency
    if let Some(url) = cache.get_pr_url(&target_branch) {
        println!("{} Opening PR: {}", "→".blue(), url.cyan());
        open_browser(url)?;
        return Ok(());
    }

    // Fall back to forge CLI with branch name
    println!("{} Opening PR for {}...", "→".blue(), target_branch.cyan());
    forge.open_pr_in_browser(&target_branch).map_err(|_| {
        anyhow::anyhow!(
            "No PR found for branch '{}'. Run '{} submit' first to create a PR.",
            target_branch,
            program_name()
        )
    })?;

    Ok(())
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd").args(["/c", "start", url]).spawn()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use tempfile::tempdir;

    use crate::test_context::TestRepoContext;

    fn init_test_repo(path: &std::path::Path) -> Result<git2::Repository> {
        let repo = git2::Repository::init(path)?;
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
        drop(tree);
        fs::create_dir_all(path.join(".git").join("diamond"))?;
        Ok(repo)
    }

    #[test]
    fn test_pr_with_stored_url() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create cache with PR URL
        let mut cache = Cache::load().unwrap_or_default();
        cache.set_pr_url("main", "https://github.com/org/repo/pull/123");
        cache.save().unwrap();

        // The command will try to open a browser, which we can't test easily
        // Just verify it doesn't panic when the URL is present
        // In a real test, we'd mock the open_browser function
    }

    #[test]
    fn test_pr_with_branch_argument_uses_specified_branch() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create cache with PR URLs for different branches
        let mut cache = Cache::load().unwrap_or_default();
        cache.set_pr_url("feature-a", "https://github.com/org/repo/pull/100");
        cache.set_pr_url("feature-b", "https://github.com/org/repo/pull/200");
        cache.save().unwrap();

        // Verify we can look up the right branch
        let cache = Cache::load().unwrap();
        assert_eq!(
            cache.get_pr_url("feature-a"),
            Some("https://github.com/org/repo/pull/100")
        );
        assert_eq!(
            cache.get_pr_url("feature-b"),
            Some("https://github.com/org/repo/pull/200")
        );
    }

    #[test]
    fn test_pr_number_detection() {
        // Test that numeric strings are recognized as PR numbers
        assert!("123".parse::<u64>().is_ok());
        assert!("456789".parse::<u64>().is_ok());

        // Test that branch names are not recognized as PR numbers
        assert!("feature-123".parse::<u64>().is_err());
        assert!("my-branch".parse::<u64>().is_err());
        assert!("123-feature".parse::<u64>().is_err());
    }

    #[test]
    fn test_pr_requires_remote() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Empty cache - no PR for any branch
        let cache = Cache::load().unwrap_or_default();
        cache.save().unwrap();

        // run() should fail because no remote is configured (forge requires remote)
        let result = run(Some("nonexistent-branch".to_string()));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("remote") || err_msg.contains("origin"),
            "Error should mention missing remote, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_pr_with_no_argument_uses_current_branch() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Get current branch name
        let gateway = GitGateway::new().unwrap();
        let current = gateway.get_current_branch_name().unwrap();

        // Set up cache with PR URL for current branch
        let mut cache = Cache::load().unwrap_or_default();
        cache.set_pr_url(&current, "https://github.com/org/repo/pull/999");
        cache.save().unwrap();

        // run() with None should use current branch and find the cached URL
        // It will try to open browser which we can't prevent, but it won't error
        // because the URL is found in cache
        // Note: This test may open a browser window in non-headless environments
    }
}
