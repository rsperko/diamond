use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;

use crate::cache::Cache;
use crate::forge::get_forge;
use crate::git_gateway::{BranchSyncState, GitGateway};
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::ui;

/// Rename the current branch
/// If force is true, allow renaming even when a PR is open
pub fn run(new_name: Option<String>, local_only: bool, force: bool) -> Result<()> {
    let gateway = GitGateway::new()?;
    let current = gateway.get_current_branch_name()?;
    let ref_store = RefStore::new()?;
    let trunk = ref_store.get_trunk()?;

    // Verify current branch is tracked
    let parent = ref_store.get_parent(&current)?;
    if parent.is_none() && trunk.as_ref() != Some(&current) {
        anyhow::bail!(
            "Branch '{}' is not tracked by Diamond. Run '{} track' first.",
            current,
            program_name()
        );
    }

    // Get new name (prompt if not provided)
    let new_name = match new_name {
        Some(n) => n,
        None => {
            if !std::io::stdin().is_terminal() {
                anyhow::bail!(
                    "New branch name is required. Usage: {} rename <new-name>",
                    program_name()
                );
            }
            ui::input("Enter new branch name", Some(&current))?
        }
    };

    // Verify new name doesn't already exist locally
    if gateway.branch_exists(&new_name)? {
        anyhow::bail!("Branch '{}' already exists", new_name);
    }

    // Check if branch has an open PR
    let cache = Cache::load().unwrap_or_default();
    let has_pr = cache.get_pr_url(&current).is_some();
    if has_pr && !force && !local_only {
        anyhow::bail!(
            "Branch '{}' has an open PR. Renaming will break the PR link.\n\
            Use --force (-f) to rename anyway, or --local to only rename locally.",
            current
        );
    }

    // Check if remote branch exists (for later cleanup)
    let has_remote = !matches!(
        gateway.check_remote_sync(&current),
        Ok(BranchSyncState::NoRemote) | Err(_)
    );

    // Rename in git
    gateway.rename_branch(&current, &new_name)?;

    // Update metadata
    update_metadata(&ref_store, &current, &new_name, trunk.as_deref())?;

    println!(
        "{} Renamed '{}' → '{}' locally",
        "✓".green().bold(),
        current.dimmed(),
        new_name.green()
    );

    // Handle remote if requested and remote exists
    if !local_only && has_remote {
        rename_remote(&gateway, &current, &new_name)?;
    } else if !local_only && !has_remote {
        // No remote to update
        println!("  {} No remote branch to update", "ℹ".blue());
    }

    Ok(())
}

/// Rename branch on remote by pushing new name and deleting old name
fn rename_remote(gateway: &GitGateway, old_name: &str, new_name: &str) -> Result<()> {
    // Get forge for pushing
    let forge = match get_forge(None) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("  {} Could not get forge for remote operations: {}", "!".yellow(), e);
            eprintln!("  To rename on remote manually:");
            eprintln!("    git push {} {}", gateway.remote(), new_name);
            eprintln!("    git push {} --delete {}", gateway.remote(), old_name);
            return Ok(());
        }
    };

    // Push new branch name
    print!("  Pushing {} to remote...", new_name.green());
    if let Err(e) = forge.push_branch(new_name, true) {
        println!(" {}", "failed".red());
        eprintln!("  {} Could not push new branch name: {}", "!".yellow(), e);
        eprintln!("  To push manually: git push {} {}", gateway.remote(), new_name);
        return Ok(());
    }
    println!(" {}", "✓".green());

    // Push updated diamond ref
    if let Err(e) = gateway.push_diamond_ref(new_name) {
        eprintln!("  {} Could not push diamond ref: {}", "!".yellow(), e);
    }

    // Delete old remote branch
    print!("  Deleting {} from remote...", old_name.dimmed());
    if let Err(e) = gateway.delete_remote_branch(old_name) {
        println!(" {}", "failed".red());
        eprintln!("  {} Could not delete old remote branch: {}", "!".yellow(), e);
        eprintln!(
            "  To delete manually: git push {} --delete {}",
            gateway.remote(),
            old_name
        );
        return Ok(());
    }
    println!(" {}", "✓".green());

    // Delete old diamond ref from remote
    if let Err(e) = gateway.delete_remote_diamond_ref(old_name) {
        // Non-fatal - ref might not exist
        eprintln!("  {} Could not delete old diamond ref: {}", "!".yellow(), e);
    }

    println!(
        "{} Renamed on remote: {} → {}",
        "✓".green().bold(),
        old_name.dimmed(),
        new_name.green()
    );

    Ok(())
}

fn update_metadata(ref_store: &RefStore, old_name: &str, new_name: &str, trunk: Option<&str>) -> Result<()> {
    // Get the old branch's parent
    let parent = ref_store.get_parent(old_name)?;

    // Get children of the old branch
    let children = ref_store.get_children(old_name)?;

    // Remove the old branch's parent ref
    ref_store.remove_parent(old_name)?;

    // Set the new branch's parent (if it has one)
    if let Some(p) = parent {
        ref_store.set_parent(new_name, &p)?;
    }

    // Update children to point to the new name
    for child in children {
        ref_store.set_parent(&child, new_name)?;
    }

    // If the old branch was trunk, update trunk
    if trunk == Some(old_name) {
        ref_store.set_trunk(new_name)?;
    }

    // Update cache (pr_url, base_sha)
    let mut cache = Cache::load().unwrap_or_default();
    if let Some(pr_url) = cache.get_pr_url(old_name).map(|s| s.to_string()) {
        cache.set_pr_url(new_name, &pr_url);
        cache.remove_pr_url(old_name);
    }
    if let Some(base_sha) = cache.get_base_sha(old_name).map(|s| s.to_string()) {
        cache.set_base_sha(new_name, &base_sha);
        cache.remove_base_sha(old_name);
    }
    cache.save()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::TestRepoContext;

    fn init_test_repo(path: &std::path::Path) -> Result<git2::Repository> {
        let repo = git2::Repository::init(path)?;
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
        drop(tree);
        Ok(repo)
    }

    fn create_branch(repo: &git2::Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[test]
    fn test_rename_untracked_branch_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Empty ref_store - branch not tracked
        let _ref_store = RefStore::new().unwrap();

        let result = run(Some("new-name".to_string()), true, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));
    }

    #[test]
    fn test_rename_requires_new_name() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Track main as trunk
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        let result = run(None, true, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("required"));
    }

    #[test]
    fn test_rename_with_pr_requires_force() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create feature branch
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &main_commit, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();

        // Set up tracking
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature", "main").unwrap();

        // Set a PR URL in cache
        let mut cache = Cache::load().unwrap_or_default();
        cache.set_pr_url("feature", "https://github.com/test/test/pull/123");
        cache.save().unwrap();

        // Try to rename without --force should fail
        let result = run(Some("new-feature".to_string()), false, false);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("has an open PR") || err_msg.contains("--force"),
            "Expected PR warning, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_rename_with_pr_succeeds_with_force() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create feature branch
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &main_commit, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();

        // Set up tracking
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature", "main").unwrap();

        // Set a PR URL in cache
        let mut cache = Cache::load().unwrap_or_default();
        cache.set_pr_url("feature", "https://github.com/test/test/pull/123");
        cache.save().unwrap();

        // With --force and --local should succeed
        let result = run(Some("new-feature".to_string()), true, true);
        assert!(result.is_ok(), "Rename with --force and --local should succeed");
    }

    #[test]
    fn test_rename_with_pr_succeeds_with_force_only() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create feature branch
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &main_commit, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();

        // Set up tracking
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature", "main").unwrap();

        // Set a PR URL in cache
        let mut cache = Cache::load().unwrap_or_default();
        cache.set_pr_url("feature", "https://github.com/test/test/pull/123");
        cache.save().unwrap();

        // With --force only (not --local) should succeed
        // local_only=false, force=true
        let result = run(Some("new-feature".to_string()), false, true);
        assert!(result.is_ok(), "Rename with --force only should succeed");
    }

    #[test]
    fn test_update_metadata() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        // Create git branches before setting parent relationships
        create_branch(&repo, "old-name")?;
        create_branch(&repo, "new-name")?;

        // Create structure: main -> old-name -> child
        ref_store.set_parent("old-name", "main")?;
        ref_store.set_parent("child", "old-name")?;

        // Rename old-name to new-name
        update_metadata(&ref_store, "old-name", "new-name", Some("main"))?;

        // Verify old name parent ref is gone
        assert!(ref_store.get_parent("old-name")?.is_none());

        // Verify new name has correct parent
        assert_eq!(ref_store.get_parent("new-name")?, Some("main".to_string()));

        // Verify child points to new name
        assert_eq!(ref_store.get_parent("child")?, Some("new-name".to_string()));

        // Verify parent's children list updated
        let main_children = ref_store.get_children("main")?;
        assert!(main_children.contains("new-name"));
        assert!(!main_children.contains("old-name"));

        Ok(())
    }

    #[test]
    fn test_update_metadata_multiple_children() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        // Create git branches before setting parent relationships
        create_branch(&repo, "old-name")?;
        create_branch(&repo, "new-name")?;

        // Create structure: old-name -> child-a, child-b
        ref_store.set_parent("old-name", "main")?;
        ref_store.set_parent("child-a", "old-name")?;
        ref_store.set_parent("child-b", "old-name")?;

        // Rename
        update_metadata(&ref_store, "old-name", "new-name", Some("main"))?;

        // Verify both children updated
        assert_eq!(ref_store.get_parent("child-a")?, Some("new-name".to_string()));
        assert_eq!(ref_store.get_parent("child-b")?, Some("new-name".to_string()));

        Ok(())
    }

    #[test]
    fn test_rename_without_name_in_non_tty_shows_helpful_error() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create feature branch
        let main_commit = repo.head()?.peel_to_commit()?;
        repo.branch("feature", &main_commit, false)?;
        repo.set_head("refs/heads/feature")?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;

        // Set up tracking
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Call without new name - should fail with helpful message
        let result = run(None, true, false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("required") || err.contains("non-interactive") || err.contains("Usage"),
            "Expected helpful error about missing name, got: {}",
            err
        );

        Ok(())
    }
}
