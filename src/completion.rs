use anyhow::Result;
use std::collections::HashSet;

use crate::ref_store::RefStore;

/// Get a list of all tracked branch names from git refs
/// Returns an empty list if:
/// - Not in a git repository
/// - No tracked branches exist
#[allow(dead_code)]
pub fn complete_tracked_branches() -> Vec<String> {
    match complete_tracked_branches_impl() {
        Ok(branches) => branches,
        Err(e) => {
            // Silently fail for completions - log to stderr if it's an unexpected error
            if !e.to_string().contains("Not inside a git repository") && !e.to_string().contains("ref store") {
                eprintln!("Warning: Failed to load tracked branches: {}", e);
            }
            Vec::new()
        }
    }
}

#[allow(dead_code)]
fn complete_tracked_branches_impl() -> Result<Vec<String>> {
    let ref_store = RefStore::new()?;
    let trunk = ref_store.get_trunk()?.unwrap_or_default();
    let mut branches = ref_store.collect_branches_dfs(std::slice::from_ref(&trunk))?;
    // Remove trunk from list since it's not a "tracked" branch in the completion sense
    branches.retain(|b| b != &trunk);
    branches.sort();
    Ok(branches)
}

/// Get a list of all git branch names
/// Returns an empty list if:
/// - Not in a git repository
/// - git2 operation fails
#[allow(dead_code)]
pub fn complete_git_branches() -> Vec<String> {
    let gateway = match crate::git_gateway::GitGateway::new() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    match gateway.list_branches() {
        Ok(mut branches) => {
            branches.sort();
            branches
        }
        Err(_) => Vec::new(), // Silently fail for completions
    }
}

/// Get completion suggestions for a specific command and argument
/// This routes to the appropriate completion source based on the command
#[allow(dead_code)]
pub fn complete_for_command(cmd: &str, _arg: &str) -> Vec<String> {
    match cmd {
        // Commands that complete tracked branches
        "checkout" | "delete" | "untrack" | "info" | "undo" | "move" => complete_tracked_branches(),
        // track completes all git branches
        "track" => {
            // For track, show git branches that aren't already tracked
            let git_branches: HashSet<String> = complete_git_branches().into_iter().collect();
            let tracked: HashSet<String> = complete_tracked_branches().into_iter().collect();
            let mut untracked: Vec<String> = git_branches.difference(&tracked).cloned().collect();
            untracked.sort();
            untracked
        }
        // Unknown command - no completions
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use crate::test_context::TestRepoContext;
    use super::*;
    use crate::git_gateway::GitGateway;
    use anyhow::Result;

    use std::path::Path;
    use tempfile::tempdir;

    // Helper to initialize a test git repo with an initial commit
    fn init_test_repo(path: &Path) -> Result<git2::Repository> {
        let repo = git2::Repository::init(path)?;

        // Make initial commit so HEAD is valid
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
        drop(tree);

        Ok(repo)
    }

    // Helper to set up tracked branches using RefStore
    fn setup_tracked_branches(branches: &[&str]) -> Result<()> {
        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set trunk
        ref_store.set_trunk("main")?;

        // Create branches and track them
        for branch in branches {
            gateway.create_branch(branch)?;
            ref_store.set_parent(branch, "main")?;
        }

        Ok(())
    }

    #[test]
    fn test_complete_tracked_branches_sorted() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Set up tracked branches (unsorted order)
        setup_tracked_branches(&["zulu", "alpha", "bravo"])?;

        let branches = complete_tracked_branches();

        // Should be sorted alphabetically
        assert_eq!(branches, vec!["alpha", "bravo", "zulu"]);

        Ok(())
    }

    #[test]
    fn test_complete_outside_repo_returns_empty() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());

        // No git repo initialized
        let branches = complete_tracked_branches();

        assert_eq!(branches, Vec::<String>::new());

        Ok(())
    }

    #[test]
    fn test_complete_no_tracked_branches_returns_empty() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Git repo exists but no tracked branches
        let branches = complete_tracked_branches();

        assert_eq!(branches, Vec::<String>::new());

        Ok(())
    }

    #[test]
    fn test_complete_git_branches() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create some git branches
        let head = repo.head()?.peel_to_commit()?;
        repo.branch("feature-a", &head, false)?;
        repo.branch("feature-b", &head, false)?;

        let branches = complete_git_branches();

        // Should include master/main plus our created branches
        assert!(branches.contains(&"feature-a".to_string()));
        assert!(branches.contains(&"feature-b".to_string()));

        // Should be sorted
        let mut sorted = branches.clone();
        sorted.sort();
        assert_eq!(branches, sorted);

        Ok(())
    }

    #[test]
    fn test_complete_for_checkout() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        setup_tracked_branches(&["feat-1", "feat-2"])?;

        let completions = complete_for_command("checkout", "");

        assert_eq!(completions, vec!["feat-1", "feat-2"]);

        Ok(())
    }

    #[test]
    fn test_complete_for_track() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create git branches
        let head = repo.head()?.peel_to_commit()?;
        repo.branch("branch-a", &head, false)?;
        repo.branch("branch-b", &head, false)?;
        repo.branch("branch-c", &head, false)?;

        // Track one of them
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("branch-a", "main")?;

        // track should only complete untracked git branches
        let completions = complete_for_command("track", "");

        assert!(!completions.contains(&"branch-a".to_string()));
        assert!(completions.contains(&"branch-b".to_string()));
        assert!(completions.contains(&"branch-c".to_string()));

        Ok(())
    }

    #[test]
    fn test_complete_special_chars_in_names() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches with special characters
        setup_tracked_branches(&["feature/foo-bar", "fix_bug_123"])?;

        let branches = complete_tracked_branches();

        assert!(branches.contains(&"feature/foo-bar".to_string()));
        assert!(branches.contains(&"fix_bug_123".to_string()));

        Ok(())
    }

    #[test]
    fn test_complete_unknown_command() {
        let completions = complete_for_command("unknown", "");
        assert_eq!(completions, Vec::<String>::new());
    }
}
