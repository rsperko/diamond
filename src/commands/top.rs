use anyhow::Result;
use colored::Colorize;

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Jump to the top of the current stack (furthest from trunk)
pub fn run() -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    // Silent cleanup of orphaned refs (handles branches deleted via git/IDE)
    if let Err(_e) = crate::validation::silent_cleanup_orphaned_refs(&gateway) {}

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

    // Walk down to find the topmost leaf in the current stack
    let top = find_stack_top(&ref_store, &current)?;

    if top == current {
        println!("{} Already at stack top", "✓".green().bold());
    } else {
        gateway.checkout_branch(&top)?;
        println!("{} Jumped to top: {}", "✓".green().bold(), top.green());
    }

    Ok(())
}

/// Find the topmost branch (leaf) starting from the given branch
/// If multiple children exist, picks the first one alphabetically
fn find_stack_top(ref_store: &RefStore, start: &str) -> Result<String> {
    let mut branch = start.to_string();

    loop {
        let children = ref_store.get_children(&branch)?;
        if children.is_empty() {
            break; // Found a leaf
        }
        // Pick first child alphabetically for determinism
        let mut sorted_children: Vec<_> = children.into_iter().collect();
        sorted_children.sort();
        branch = sorted_children[0].clone();
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
    fn test_find_stack_top_single_branch() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        // main has no children

        let top = find_stack_top(&ref_store, "main")?;
        assert_eq!(top, "main");

        Ok(())
    }

    #[test]
    fn test_find_stack_top_linear_stack() -> Result<()> {
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

        let top = find_stack_top(&ref_store, "main")?;
        assert_eq!(top, "feature-3");

        let top = find_stack_top(&ref_store, "feature-1")?;
        assert_eq!(top, "feature-3");

        Ok(())
    }

    #[test]
    fn test_find_stack_top_multiple_children() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches before setting parents
        create_branch(&repo, "feature-a")?;
        create_branch(&repo, "feature-b")?;

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature-a", "main")?;
        ref_store.set_parent("feature-b", "main")?;

        // Should pick alphabetically first child
        let top = find_stack_top(&ref_store, "main")?;
        assert_eq!(top, "feature-a");

        Ok(())
    }

    #[test]
    fn test_top_untracked_branch_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Empty ref_store - branch not tracked
        let _ref_store = RefStore::new().unwrap();

        let result = run();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));
    }

    #[test]
    fn test_top_already_at_top() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Track main as trunk (it's a leaf)
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Should succeed (already at top)
        let result = run();
        assert!(result.is_ok());
    }
}
