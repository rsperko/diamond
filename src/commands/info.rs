use anyhow::Result;
use colored::Colorize;

use crate::cache::Cache;
use crate::git_gateway::{BranchSyncState, GitGateway};
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Show the parent of the current branch
pub fn run_parent() -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    let current = gateway.get_current_branch_name()?;
    let trunk = ref_store.get_trunk()?;

    // Check if tracked (has parent or is trunk)
    let parent = ref_store.get_parent(&current)?;
    if parent.is_none() && trunk.as_ref() != Some(&current) {
        anyhow::bail!(
            "Branch '{}' is not tracked by Diamond. Run '{} track' to add it to a stack.",
            current,
            program_name()
        );
    }

    match parent {
        Some(p) => println!("{}", p),
        None => println!("(none)"),
    }
    Ok(())
}

/// Show the children of the current branch
pub fn run_children() -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    let current = gateway.get_current_branch_name()?;
    let trunk = ref_store.get_trunk()?;

    // Check if tracked (has parent or is trunk)
    let parent = ref_store.get_parent(&current)?;
    if parent.is_none() && trunk.as_ref() != Some(&current) {
        anyhow::bail!(
            "Branch '{}' is not tracked by Diamond. Run '{} track' to add it to a stack.",
            current,
            program_name()
        );
    }

    let children = ref_store.get_children(&current)?;
    // Output nothing when no children
    let mut sorted_children: Vec<_> = children.into_iter().collect();
    sorted_children.sort();
    for child in sorted_children {
        println!("{}", child);
    }
    Ok(())
}

/// Show or set the trunk branch
/// If set is Some(branch), sets that branch as the trunk
/// Otherwise, displays the current trunk
pub fn run_trunk(set: Option<String>) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    // Handle --set: set a new trunk
    if let Some(new_trunk) = set {
        // Verify branch exists
        if !gateway.branch_exists(&new_trunk)? {
            anyhow::bail!("Branch '{}' does not exist", new_trunk);
        }

        ref_store.set_trunk(&new_trunk)?;
        println!("{} Set trunk to: {}", "✓".green().bold(), new_trunk.green());
        return Ok(());
    }

    // Default: show current trunk
    let trunk = ref_store
        .get_trunk()?
        .ok_or_else(|| anyhow::anyhow!("No trunk configured. Run '{} init' first.", program_name()))?;
    println!("{}", trunk);
    Ok(())
}

/// Show details about a branch including PR status
///
/// Special subcommands (deprecated - use `dm parent`, `dm children`, `dm trunk`):
/// - `dm info trunk` - print the trunk branch name
/// - `dm info parent` - print the current branch's parent
/// - `dm info children` - print the current branch's children
pub fn run(branch: Option<String>) -> Result<()> {
    // Silent cleanup of orphaned refs before displaying info
    let gateway = GitGateway::new()?;
    if let Err(_e) = crate::validation::silent_cleanup_orphaned_refs(&gateway) {
        // Non-fatal: if cleanup fails, still show info
    }

    let ref_store = RefStore::new()?;
    let cache = Cache::load().unwrap_or_default();
    let trunk = ref_store.get_trunk()?;

    // Handle special subcommands
    if let Some(ref cmd) = branch {
        match cmd.as_str() {
            "trunk" => {
                let trunk = trunk
                    .ok_or_else(|| anyhow::anyhow!("No trunk configured. Run '{} init' first.", program_name()))?;
                println!("{}", trunk);
                return Ok(());
            }
            "parent" => {
                let current = gateway.get_current_branch_name()?;
                let parent = ref_store.get_parent(&current)?;
                if parent.is_none() && trunk.as_ref() != Some(&current) {
                    anyhow::bail!(
                        "Branch '{}' is not tracked by Diamond. Run '{} track' to add it to a stack.",
                        current,
                        program_name()
                    );
                }
                match parent {
                    Some(p) => println!("{}", p),
                    None => println!("(none)"),
                }
                return Ok(());
            }
            "children" => {
                let current = gateway.get_current_branch_name()?;
                let parent = ref_store.get_parent(&current)?;
                if parent.is_none() && trunk.as_ref() != Some(&current) {
                    anyhow::bail!(
                        "Branch '{}' is not tracked by Diamond. Run '{} track' to add it to a stack.",
                        current,
                        program_name()
                    );
                }
                let children = ref_store.get_children(&current)?;
                // Output nothing when no children
                let mut sorted_children: Vec<_> = children.into_iter().collect();
                sorted_children.sort();
                for child in sorted_children {
                    println!("{}", child);
                }
                return Ok(());
            }
            _ => {} // Not a special command, treat as branch name
        }
    }

    let target = branch.unwrap_or_else(|| gateway.get_current_branch_name().unwrap_or_default());

    // Check if tracked
    let parent = ref_store.get_parent(&target)?;
    if parent.is_none() && trunk.as_ref() != Some(&target) {
        anyhow::bail!(
            "Branch '{}' is not tracked by Diamond. Run '{} track {}' first.",
            target,
            program_name(),
            target
        );
    }

    // Branch name
    println!("{}", target.green().bold());
    println!();

    // Parent
    let parent_str = parent
        .as_ref()
        .map(|p| p.blue().to_string())
        .unwrap_or_else(|| "(none - root)".dimmed().to_string());
    println!("  {}: {}", "Parent".bold(), parent_str);

    // Children
    let children = ref_store.get_children(&target)?;
    if children.is_empty() {
        println!("  {}: {}", "Children".bold(), "(none - leaf)".dimmed());
    } else {
        let mut sorted_children: Vec<_> = children.into_iter().collect();
        sorted_children.sort();
        let children_str = sorted_children
            .iter()
            .map(|c| c.green().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!("  {}: {}", "Children".bold(), children_str);
    }

    // Remote sync status
    let remote_status = match gateway.check_remote_sync(&target) {
        Ok(BranchSyncState::InSync) => "✓ in sync".green().to_string(),
        Ok(BranchSyncState::Ahead(n)) => {
            let s = if n == 1 { "" } else { "s" };
            format!("{} commit{} ahead", n, s).yellow().to_string()
        }
        Ok(BranchSyncState::Behind(n)) => {
            let s = if n == 1 { "" } else { "s" };
            format!("{} commit{} behind", n, s).red().to_string()
        }
        Ok(BranchSyncState::Diverged {
            local_ahead,
            remote_ahead,
        }) => format!("diverged (+{} local, +{} remote)", local_ahead, remote_ahead)
            .red()
            .to_string(),
        Ok(BranchSyncState::NoRemote) => "not pushed".dimmed().to_string(),
        Err(_) => "unknown".dimmed().to_string(),
    };
    println!("  {}: {}", "Remote".bold(), remote_status);

    // PR URL (from cache)
    if let Some(url) = cache.get_pr_url(&target) {
        println!("  {}: {}", "PR".bold(), url.cyan());
    } else {
        println!("  {}: {}", "PR".bold(), "(not submitted)".dimmed());
    }

    // Frozen status
    let is_frozen = ref_store.is_frozen(&target)?;
    if is_frozen {
        println!("  {}: {}", "Status".bold(), "frozen".cyan());
    }

    // Commit count (if we have a parent)
    if let Some(p) = &parent {
        match gateway.get_commit_count_since(p) {
            Ok(count) => {
                let commit_word = if count == 1 { "commit" } else { "commits" };
                println!(
                    "  {}: {} {} ahead of {}",
                    "Commits".bold(),
                    count.to_string().yellow(),
                    commit_word,
                    p.blue()
                );
            }
            Err(_) => {
                // Silently skip if we can't get commit count
            }
        }
    }

    // Base SHA (from cache)
    if let Some(sha) = cache.get_base_sha(&target) {
        println!("  {}: {}", "Base SHA".bold(), sha.dimmed());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_gateway::GitGateway;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_info_untracked_branch_fails() {
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
    fn test_info_tracked_branch() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Track main as trunk
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Should succeed for tracked branch
        let result = run(Some("main".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_info_with_pr_url() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Track main as trunk with PR URL in cache
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        let mut cache = Cache::load().unwrap_or_default();
        cache.set_pr_url("main", "https://github.com/org/repo/pull/123");
        cache.save().unwrap();

        // Should succeed
        let result = run(Some("main".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_info_with_children() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Track main with children
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-a", "main").unwrap();
        ref_store.set_parent("feature-b", "main").unwrap();

        // Should succeed
        let result = run(Some("main".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_info_trunk_subcommand() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Should succeed and print trunk name
        let result = run(Some("trunk".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_info_trunk_no_trunk_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Empty ref_store (no trunk)
        let _ref_store = RefStore::new().unwrap();

        let result = run(Some("trunk".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No trunk"));
    }

    #[test]
    fn test_info_parent_subcommand() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();
        gateway.create_branch("feature").unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature", "main").unwrap();

        // Should succeed and print parent name
        let result = run(Some("parent".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_info_children_subcommand() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Set up: main -> feature-a, feature-b
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-a", "main").unwrap();
        ref_store.set_parent("feature-b", "main").unwrap();

        // Should succeed and print children
        let result = run(Some("children".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_info_children_empty() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();
        gateway.create_branch("feature").unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature", "main").unwrap();

        // Feature has no children - should succeed
        let result = run(Some("children".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_info_shows_remote_status() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Track main as trunk
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Info should succeed and include remote status (no remote = "not pushed")
        let result = run(Some("main".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_trunk_set_branch() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();
        gateway.create_branch("develop").unwrap();

        // Set develop as trunk
        let result = run_trunk(Some("develop".to_string()));
        assert!(result.is_ok());

        // Verify trunk was set
        let ref_store = RefStore::new().unwrap();
        assert_eq!(ref_store.get_trunk().unwrap(), Some("develop".to_string()));
    }

    #[test]
    fn test_trunk_set_nonexistent_branch_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Try to set nonexistent branch as trunk
        let result = run_trunk(Some("nonexistent".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }
}
