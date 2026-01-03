use anyhow::Result;
use colored::Colorize;

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Pop the current branch - delete it but keep working tree changes
///
/// This is useful when you:
/// - Created a branch with the wrong name
/// - Want to fold changes manually into another branch
/// - Accidentally committed to the wrong branch
pub fn run() -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    let current = gateway.get_current_branch_name()?;
    let trunk = ref_store.get_trunk()?;

    // Can't pop trunk
    if trunk.as_ref() == Some(&current) {
        anyhow::bail!("Cannot pop trunk branch '{}'", current);
    }

    // Get parent - required for pop
    let parent = ref_store.get_parent(&current)?.ok_or_else(|| {
        anyhow::anyhow!(
            "Branch '{}' has no parent. Cannot pop without a parent to return to.\n\
            Use '{} delete -f {}' to delete the branch instead.",
            current,
            program_name(),
            current
        )
    })?;

    // Check for children - can't pop if there are children (would orphan them)
    let children = ref_store.get_children(&current)?;
    if !children.is_empty() {
        let mut child_list: Vec<_> = children.into_iter().collect();
        child_list.sort();
        anyhow::bail!(
            "Cannot pop branch '{}' - it has children: {}\n\
            Use '{} delete --reparent {}' to delete and reparent children.",
            current,
            child_list.join(", "),
            program_name(),
            current
        );
    }

    // Get SHAs to determine if there are committed changes to preserve
    let current_sha = gateway.get_branch_sha(&current)?;
    let parent_sha = gateway.get_branch_sha(&parent)?;
    let has_commits = current_sha != parent_sha;

    // Stash any uncommitted changes
    let stashed = gateway.stash_push(&format!("{} pop: changes from {}", program_name(), current))?;
    if stashed {
        println!("{} Stashed uncommitted changes", "→".blue());
    }

    // Checkout parent
    gateway.checkout_branch(&parent)?;
    println!("{} Checked out {}", "→".blue(), parent.green());

    // Delete the branch from git
    gateway.delete_branch(&current)?;

    // Remove from diamond metadata
    ref_store.remove_parent(&current)?;

    println!("{} Popped branch '{}'", "✓".green().bold(), current.yellow());

    // Restore committed changes from the popped branch as uncommitted changes
    // Only if the branch had commits beyond parent
    if has_commits {
        gateway.restore_files_from_commit(&current_sha)?;
        println!("{} Restored committed changes to working tree", "✓".green());
    }

    // Pop the stash if we stashed (restore uncommitted changes on top)
    if stashed {
        gateway.stash_pop()?;
        println!("{} Restored uncommitted changes", "✓".green());
    }

    println!(
        "\nYou are now on '{}'. Changes from '{}' are preserved in your working tree.",
        parent.green(),
        current.yellow()
    );

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
        Ok(repo)
    }

    #[test]
    fn test_pop_basic() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set up: main -> feature
        ref_store.set_trunk("main")?;
        gateway.create_branch("feature")?;
        ref_store.set_parent("feature", "main")?;

        // We're on feature
        assert_eq!(gateway.get_current_branch_name()?, "feature");

        // Pop feature
        run()?;

        // Should be back on main
        assert_eq!(gateway.get_current_branch_name()?, "main");

        // Feature should be gone
        assert!(!gateway.branch_exists("feature")?);
        assert!(!ref_store.is_tracked("feature")?);

        Ok(())
    }

    #[test]
    fn test_pop_preserves_uncommitted_changes() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set up: main -> feature
        ref_store.set_trunk("main")?;
        gateway.create_branch("feature")?;
        ref_store.set_parent("feature", "main")?;

        // Create uncommitted changes
        fs::write(dir.path().join("test.txt"), "hello world")?;

        // Pop feature
        run()?;

        // Should be back on main
        assert_eq!(gateway.get_current_branch_name()?, "main");

        // Uncommitted changes should be preserved
        let content = fs::read_to_string(dir.path().join("test.txt"))?;
        assert_eq!(content, "hello world");

        Ok(())
    }

    #[test]
    fn test_pop_trunk_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        // Try to pop trunk
        let result = run();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot pop trunk"));

        Ok(())
    }

    #[test]
    fn test_pop_with_children_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set up: main -> middle -> leaf
        ref_store.set_trunk("main")?;
        gateway.create_branch("middle")?;
        ref_store.set_parent("middle", "main")?;
        gateway.create_branch("leaf")?;
        ref_store.set_parent("leaf", "middle")?;
        gateway.checkout_branch("middle")?;

        // Try to pop middle (which has child "leaf")
        let result = run();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("has children"));
        assert!(err.contains("leaf"));

        Ok(())
    }

    #[test]
    fn test_pop_untracked_branch_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create branch without tracking
        gateway.create_branch("untracked")?;
        ref_store.set_trunk("main")?;

        // Try to pop untracked branch
        let result = run();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("has no parent"));

        Ok(())
    }

    #[test]
    fn test_pop_preserves_committed_changes_as_uncommitted() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set up: main -> feature
        ref_store.set_trunk("main")?;
        gateway.create_branch("feature")?;
        ref_store.set_parent("feature", "main")?;

        // Create and commit a file on the feature branch
        fs::write(dir.path().join("feature-file.txt"), "feature content")?;
        gateway.stage_all()?;
        gateway.commit("Add feature file")?;

        // Pop the feature branch
        run()?;

        // We should be on main
        assert_eq!(gateway.get_current_branch_name()?, "main");

        // The committed file should be preserved as uncommitted changes
        let file_path = dir.path().join("feature-file.txt");
        assert!(file_path.exists(), "Committed file should be preserved after pop");

        // Verify the content is correct
        let content = fs::read_to_string(&file_path)?;
        assert_eq!(content, "feature content");

        Ok(())
    }
}
