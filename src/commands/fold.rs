use anyhow::Result;
use colored::Colorize;

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Fold the current branch into its parent
pub fn run(keep_name: bool) -> Result<()> {
    let gateway = GitGateway::new()?;
    gateway.require_clean_working_tree("fold")?;

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

    // Get parent
    let parent = parent.ok_or_else(|| anyhow::anyhow!("Cannot fold: branch '{}' has no parent", current))?;

    // Verify parent is not trunk
    if trunk.as_ref() == Some(&parent) {
        anyhow::bail!(
            "Cannot fold into trunk branch '{}'. Fold is for combining feature branches.",
            parent
        );
    }

    println!("{} Folding {} into {}...", "→".blue(), current.green(), parent.green());

    // Get children of current branch
    let children: Vec<String> = ref_store.get_children(&current)?.into_iter().collect();

    if !children.is_empty() {
        println!("  {} Will restack {} descendant(s)", "→".blue(), children.len());
    }

    // Determine final branch name
    let final_name = if keep_name { current.clone() } else { parent.clone() };

    // Checkout parent and fast-forward to current
    gateway.checkout_branch_worktree_safe(&parent)?;
    gateway.merge_branch_ff(&current)?;

    // If keeping name, delete current branch then rename parent to current's name
    if keep_name {
        gateway.delete_branch(&current)?;
        gateway.rename_branch(&parent, &current)?;
    }

    // Update metadata
    if keep_name {
        // When keeping current's name:
        // 1. Get parent's parent (grandparent)
        // 2. Set current's parent to grandparent
        // 3. Update all children to point to current (which now replaces parent)
        // 4. Remove parent's metadata

        let grandparent = ref_store.get_parent(&parent)?;

        // Get parent's children (excluding current)
        let parent_children: Vec<String> = ref_store
            .get_children(&parent)?
            .into_iter()
            .filter(|c| c != &current)
            .collect();

        // Remove current's old parent ref
        ref_store.remove_parent(&current)?;

        // Set current's parent to grandparent (if it has one)
        if let Some(gp) = &grandparent {
            ref_store.set_parent(&current, gp)?;
        }

        // Update current's children to include parent's children
        for child in parent_children {
            ref_store.set_parent(&child, &current)?;
        }

        // Update original children to point to current
        for child in &children {
            ref_store.set_parent(child, &current)?;
        }

        // Remove parent from store
        ref_store.remove_parent(&parent)?;
    } else {
        // When keeping parent's name:
        // 1. Update current's children to point to parent
        // 2. Remove current from store

        // Update children to point to parent
        for child in &children {
            ref_store.set_parent(child, &parent)?;
        }

        // Remove current from store
        ref_store.remove_parent(&current)?;
    }

    // Delete the old branch (if not keep_name)
    if keep_name {
        // We already deleted current and renamed parent to current
        println!("  {} Folded {} into {}", "✓".green(), parent, current);
    } else {
        gateway.delete_branch(&current)?;
        println!("  {} Folded and deleted branch {}", "✓".green(), current);
    }

    // Warn about children that need restacking
    if !children.is_empty() {
        println!();
        println!(
            "{} {} child branch{} need restacking:",
            "!".yellow().bold(),
            children.len(),
            if children.len() == 1 { "" } else { "es" }
        );
        for child in &children {
            println!("    • {}", child.cyan());
        }
        println!();
        println!(
            "Run {} to rebase them onto the new base.",
            format!("{} restack", program_name()).cyan().bold()
        );
    }

    println!();
    println!(
        "{} Fold complete! Now on branch {}",
        "✓".green().bold(),
        final_name.green()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    fn create_commit(repo: &git2::Repository, message: &str, parent: &git2::Commit) -> Result<git2::Oid> {
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[parent])?;
        drop(tree);
        Ok(oid)
    }

    #[test]
    fn test_fold_basic() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create stack: main -> feature-a -> feature-b
        let main_commit = repo.head()?.peel_to_commit()?;

        // Create feature-a with one commit
        repo.branch("feature-a", &main_commit, false)?;
        repo.set_head("refs/heads/feature-a")?;
        let commit_a = create_commit(&repo, "Commit A", &main_commit)?;
        let commit_a = repo.find_commit(commit_a)?;

        // Create feature-b with one commit
        repo.branch("feature-b", &commit_a, false)?;
        repo.set_head("refs/heads/feature-b")?;
        create_commit(&repo, "Commit B", &commit_a)?;

        // Setup metadata
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature-a", "main")?;
        ref_store.set_parent("feature-b", "feature-a")?;

        // Fold feature-b into feature-a
        run(false)?;

        // Verify feature-b is deleted
        assert!(repo.find_branch("feature-b", git2::BranchType::Local).is_err());

        // Verify we're on feature-a
        let head = repo.head()?;
        let current = head.shorthand().ok_or_else(|| anyhow::anyhow!("No branch"))?;
        assert_eq!(current, "feature-a");

        // Verify metadata
        let ref_store = RefStore::new()?;
        assert!(ref_store.get_parent("feature-b")?.is_none());
        assert_eq!(ref_store.get_parent("feature-a")?, Some("main".to_string()));

        Ok(())
    }

    #[test]
    fn test_fold_with_keep_flag() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create stack: main -> feature-a -> feature-b
        let main_commit = repo.head()?.peel_to_commit()?;

        repo.branch("feature-a", &main_commit, false)?;
        repo.set_head("refs/heads/feature-a")?;
        let commit_a = create_commit(&repo, "Commit A", &main_commit)?;
        let commit_a = repo.find_commit(commit_a)?;

        repo.branch("feature-b", &commit_a, false)?;
        repo.set_head("refs/heads/feature-b")?;
        create_commit(&repo, "Commit B", &commit_a)?;

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature-a", "main")?;
        ref_store.set_parent("feature-b", "feature-a")?;

        // Fold with --keep flag
        run(true)?;

        // Verify feature-a no longer exists
        assert!(repo.find_branch("feature-a", git2::BranchType::Local).is_err());

        // Verify we're on feature-b
        let head = repo.head()?;
        let current = head.shorthand().ok_or_else(|| anyhow::anyhow!("No branch"))?;
        assert_eq!(current, "feature-b");

        // Verify metadata
        let ref_store = RefStore::new()?;
        assert!(ref_store.get_parent("feature-a")?.is_none());
        assert_eq!(ref_store.get_parent("feature-b")?, Some("main".to_string()));

        Ok(())
    }

    #[test]
    fn test_fold_cannot_fold_into_trunk() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();

        repo.branch("feature-a", &main_commit, false).unwrap();
        repo.set_head("refs/heads/feature-a").unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-a", "main").unwrap();

        let result = run(false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot fold into trunk"));
    }

    #[test]
    fn test_fold_untracked_branch_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let _ref_store = RefStore::new().unwrap();

        let result = run(false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));
    }
}
