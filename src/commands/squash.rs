use anyhow::Result;
use colored::Colorize;

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Squash all commits in the current branch into a single commit
///
/// If message is provided, uses it as the commit message.
/// Otherwise, generates a message based on the branch name.
pub fn run(message: Option<String>) -> Result<()> {
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

    // Get parent branch
    let parent = parent.ok_or_else(|| anyhow::anyhow!("Cannot squash: branch '{}' has no parent", current))?;

    // Get commit count
    let commit_count = gateway.get_commit_count_since(&parent)?;

    if commit_count == 0 {
        println!(
            "{} Branch has no commits ahead of {}",
            "✓".green().bold(),
            parent.blue()
        );
        return Ok(());
    }

    if commit_count == 1 {
        println!("{} Branch has only 1 commit, nothing to squash", "✓".green().bold());
        return Ok(());
    }

    println!(
        "{} Squashing {} commits into one...",
        "→".blue(),
        commit_count.to_string().yellow()
    );

    // Collect original commit messages BEFORE resetting (they'll be lost after)
    let original_messages = gateway.get_commit_messages_since(&parent)?;

    // Soft reset to parent
    gateway.soft_reset_to(&parent)?;

    // Create new commit with provided or generated message
    let commit_message = message.unwrap_or_else(|| {
        // Build message that includes all original commit messages
        let title = current.replace(['-', '_'], " ");
        let mut msg = format!("{}\n\nSquashed {} commits:\n", title, commit_count);

        // Add each original message (oldest first for chronological order)
        for (i, original_msg) in original_messages.iter().rev().enumerate() {
            msg.push_str(&format!("\n{}. {}\n", i + 1, original_msg.replace('\n', "\n   ")));
        }

        msg
    });

    gateway.commit(&commit_message)?;

    println!("{} Squashed {} commits into 1", "✓".green().bold(), commit_count);

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
    fn test_squash_untracked_branch_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Empty ref_store - branch not tracked
        let _ref_store = RefStore::new().unwrap();

        let result = run(None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));
    }

    #[test]
    fn test_squash_no_parent_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Track main as trunk (has no parent)
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        let result = run(None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no parent"));
    }

    #[test]
    fn test_squash_no_commits_ahead() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();

        // Create a branch on the same commit as main (0 commits ahead)
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Create feature branch but don't make any commits
        gateway.create_branch("feature").unwrap();
        ref_store.set_parent("feature", "main").unwrap();

        // Should succeed but report no commits
        let result = run(None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_squash_with_custom_message() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();

        // Create feature branch with multiple commits
        gateway.create_branch("feature").unwrap();

        // Add commits
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        for i in 1..=3 {
            let filename = format!("file{}.txt", i);
            fs::write(dir.path().join(&filename), format!("content {}", i)).unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new(&filename)).unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let parent = repo.head().unwrap().peel_to_commit().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, &format!("Commit {}", i), &tree, &[&parent])
                .unwrap();
        }

        // Set up ref_store
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature", "main").unwrap();

        // Squash with custom message
        let result = run(Some("My custom squash message".to_string()));
        assert!(result.is_ok());

        // Verify the commit message
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.summary().unwrap(), "My custom squash message");
    }

    #[test]
    fn test_squash_auto_message_preserves_original_commits() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();

        // Create feature branch with multiple commits with distinct messages
        gateway.create_branch("feature-auth").unwrap();

        // Add commits with meaningful messages
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let commit_messages = ["Add user model", "Implement password hashing", "Add login endpoint"];

        for (i, msg) in commit_messages.iter().enumerate() {
            let filename = format!("file{}.txt", i);
            fs::write(dir.path().join(&filename), format!("content {}", i)).unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new(&filename)).unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let parent = repo.head().unwrap().peel_to_commit().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[&parent]).unwrap();
        }

        // Set up ref_store
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-auth", "main").unwrap();

        // Squash WITHOUT custom message (auto-generate)
        let result = run(None);
        assert!(result.is_ok());

        // Verify the commit message contains all original messages
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let message = head.message().unwrap();

        // Should contain branch name transformed to title
        assert!(message.contains("feature auth"), "Should contain branch name as title");

        // Should contain count
        assert!(message.contains("3 commits"), "Should mention commit count");

        // Should preserve all original commit messages
        for original_msg in &commit_messages {
            assert!(
                message.contains(original_msg),
                "Should contain original message: {}",
                original_msg
            );
        }
    }
}
