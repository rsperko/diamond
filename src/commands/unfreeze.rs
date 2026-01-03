use anyhow::{Context, Result};
use colored::Colorize;

use crate::git_gateway::GitGateway;
use crate::ref_store::RefStore;

/// Unfreeze a branch (and optionally its upstack branches)
pub fn run(branch: Option<String>, upstack: bool) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    // Use specified branch or current branch
    let branch = match branch {
        Some(b) => b,
        None => gateway.get_current_branch_name()?,
    };

    // Check if the branch is frozen
    if !ref_store.is_frozen(&branch)? {
        println!("{} Branch '{}' is not frozen", "!".yellow(), branch);
        return Ok(());
    }

    if upstack {
        // Collect upstack branches (DFS from current branch, inclusive)
        let to_unfreeze = ref_store.collect_branches_dfs(std::slice::from_ref(&branch))?;

        // Filter to only frozen branches
        let frozen_branches: Vec<_> = to_unfreeze
            .iter()
            .filter(|b| ref_store.is_frozen(b).unwrap_or(false))
            .cloned()
            .collect();

        if frozen_branches.is_empty() {
            println!("{} No frozen branches to unfreeze", "!".yellow());
            return Ok(());
        }

        // Unfreeze all
        for b in &frozen_branches {
            ref_store
                .set_frozen(b, false)
                .context(format!("Failed to unfreeze '{}'", b))?;
            println!("{} Unfroze '{}'", "✓".green(), b.cyan());
        }

        println!(
            "\n{} Unfroze {} branch{}",
            "✓".green().bold(),
            frozen_branches.len(),
            if frozen_branches.len() == 1 { "" } else { "es" }
        );
    } else {
        // Just unfreeze the single branch
        ref_store
            .set_frozen(&branch, false)
            .context(format!("Failed to unfreeze '{}'", branch))?;

        println!("{} Unfroze branch '{}'", "✓".green(), branch.cyan());
    }

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

    fn create_branch(repo: &Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[test]
    fn test_unfreeze_single_branch() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let trunk = repo.head()?.shorthand().unwrap().to_string();

        let ref_store = RefStore::from_path(dir.path())?;
        ref_store.set_trunk(&trunk)?;

        create_branch(&repo, "feature")?;
        ref_store.set_parent("feature", &trunk)?;

        // Freeze the branch
        ref_store.set_frozen("feature", true)?;
        assert!(ref_store.is_frozen("feature")?);

        let _ctx = TestRepoContext::new(dir.path());

        // Unfreeze without upstack flag
        run(Some("feature".to_string()), false)?;

        assert!(!ref_store.is_frozen("feature")?);

        Ok(())
    }

    #[test]
    fn test_unfreeze_not_frozen_is_noop() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let trunk = repo.head()?.shorthand().unwrap().to_string();

        let ref_store = RefStore::from_path(dir.path())?;
        ref_store.set_trunk(&trunk)?;

        create_branch(&repo, "feature")?;
        ref_store.set_parent("feature", &trunk)?;

        let _ctx = TestRepoContext::new(dir.path());

        run(Some("feature".to_string()), false)?;

        Ok(())
    }

    #[test]
    fn test_unfreeze_upstack() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let trunk = repo.head()?.shorthand().unwrap().to_string();

        let ref_store = RefStore::from_path(dir.path())?;
        ref_store.set_trunk(&trunk)?;

        // Create stack: trunk -> feature -> child1, child2
        create_branch(&repo, "feature")?;
        create_branch(&repo, "child1")?;
        create_branch(&repo, "child2")?;

        ref_store.set_parent("feature", &trunk)?;
        ref_store.set_parent("child1", "feature")?;
        ref_store.set_parent("child2", "feature")?;

        // Freeze all
        ref_store.set_frozen("feature", true)?;
        ref_store.set_frozen("child1", true)?;
        ref_store.set_frozen("child2", true)?;

        let _ctx = TestRepoContext::new(dir.path());

        // Unfreeze with upstack=true
        run(Some("feature".to_string()), true)?;

        assert!(!ref_store.is_frozen("feature")?);
        assert!(!ref_store.is_frozen("child1")?);
        assert!(!ref_store.is_frozen("child2")?);

        Ok(())
    }

    #[test]
    fn test_unfreeze_upstack_only_frozen_ones() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let trunk = repo.head()?.shorthand().unwrap().to_string();

        let ref_store = RefStore::from_path(dir.path())?;
        ref_store.set_trunk(&trunk)?;

        create_branch(&repo, "feature")?;
        create_branch(&repo, "child")?;

        ref_store.set_parent("feature", &trunk)?;
        ref_store.set_parent("child", "feature")?;

        // Only freeze parent, not child
        ref_store.set_frozen("feature", true)?;

        let _ctx = TestRepoContext::new(dir.path());

        run(Some("feature".to_string()), true)?;

        assert!(!ref_store.is_frozen("feature")?);
        assert!(!ref_store.is_frozen("child")?); // Was never frozen

        Ok(())
    }
}
