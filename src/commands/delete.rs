use crate::context::ExecutionContext;
use crate::git_gateway::{BackupRef, GitGateway};
use crate::operation_log::{Operation, OperationRecorder};
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::ui;
use crate::worktree;
use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;

/// Delete a branch from git and Diamond metadata
///
/// If name is None, would show interactive TUI (not yet implemented).
/// If reparent is true, children are re-parented to the grandparent AND rebased.
/// If force is true, delete even if branch is not merged (skip confirmation).
/// If upstack is true, delete branch and all descendants.
/// If downstack is true, delete branch and all ancestors (except trunk).
pub fn run(name: Option<String>, reparent: bool, force: bool, upstack: bool, downstack: bool) -> Result<()> {
    // Validate mutually exclusive flags
    if upstack && downstack {
        anyhow::bail!("--upstack and --downstack are mutually exclusive");
    }

    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    let name = match name {
        Some(n) => n,
        None => {
            // Check TTY before interactive mode
            if !std::io::stdin().is_terminal() {
                anyhow::bail!(
                    "delete requires a branch name when running non-interactively. \
                     Usage: {} delete <branch>",
                    program_name()
                );
            }

            // Get all tracked branches except trunk
            let trunk = ref_store.get_trunk()?;
            let trunk_name = trunk.as_deref().unwrap_or("main");
            let all_branches = ref_store.collect_branches_dfs(&[trunk_name.to_string()])?;
            let branches: Vec<String> = all_branches
                .into_iter()
                .filter(|b| Some(b.as_str()) != trunk.as_deref())
                .collect();

            if branches.is_empty() {
                anyhow::bail!("No branches available to delete");
            }

            // Show selection prompt
            let idx = ui::select("Select branch to delete", &branches)?;
            branches[idx].clone()
        }
    };

    let current = gateway.get_current_branch_name()?;

    // Don't allow deleting trunk (read from refs)
    if let Some(trunk) = ref_store.get_trunk()? {
        if trunk == name {
            anyhow::bail!("Cannot delete trunk branch '{}'", name);
        }
    }

    // Check if branch exists in git
    if !gateway.branch_exists(&name)? {
        anyhow::bail!("Branch '{}' does not exist in git", name);
    }

    // Handle --upstack: delete branch and all descendants
    if upstack {
        return delete_upstack(&gateway, &ref_store, &name, &current, force);
    }

    // Handle --downstack: delete branch and all ancestors (except trunk)
    if downstack {
        return delete_downstack(&gateway, &ref_store, &name, &current, force);
    }

    // Check if branch is merged (if not force, require confirmation)
    if !force {
        let trunk = ref_store.require_trunk()?;
        match gateway.is_branch_merged(&name, &trunk) {
            Ok(true) => {} // Branch is merged, proceed
            Ok(false) => {
                anyhow::bail!(
                    "Branch '{}' is not merged into '{}'.\n\
                    Use -f or --force to delete anyway.",
                    name,
                    trunk
                );
            }
            Err(e) => {
                // Git command failed - don't silently assume not merged
                anyhow::bail!(
                    "Could not determine if branch '{}' is merged: {}\n\
                    Use -f or --force to delete anyway.",
                    name,
                    e
                );
            }
        }
    }

    // Confirm deletion (unless --force)
    if !force && !ui::confirm(&format!("Delete branch '{}'?", name), false)? {
        println!("Delete cancelled.");
        return Ok(());
    }

    // Get branch info before modifying (read from refs - canonical source)
    let grandparent = ref_store.get_parent(&name)?;
    let children: Vec<String> = ref_store.get_children(&name)?.into_iter().collect();

    // Handle dry-run mode
    if ExecutionContext::is_dry_run() {
        println!(
            "{} Dry run - would delete branch: {}",
            "[preview]".yellow().bold(),
            name.green()
        );
        if !children.is_empty() {
            if reparent {
                if let Some(ref gp) = grandparent {
                    println!(
                        "  • Reparent {} children to {}",
                        children.len().to_string().yellow(),
                        gp.blue()
                    );
                    for child in &children {
                        println!("    - {}", child.green());
                    }
                } else {
                    println!("  {} Cannot reparent children without grandparent", "!".yellow());
                }
            } else {
                println!(
                    "  {} {} children would be orphaned",
                    "!".yellow(),
                    children.len().to_string().yellow()
                );
                for child in &children {
                    println!("    - {}", child.green());
                }
            }
        }
        if current == name {
            if let Some(parent) = &grandparent {
                println!("  • Checkout parent: {}", parent.blue());
            }
        }
        println!("  • Delete git branch: {}", name);
        println!("  • Remove from Diamond metadata");
        println!();
        println!("{} No changes made (dry-run mode)", "✓".green().bold());
        return Ok(());
    }

    // If deleting current branch, checkout parent first
    if current == name {
        if let Some(parent) = &grandparent {
            gateway.checkout_branch(parent)?;
            println!("Checked out parent branch: {}", parent);
        } else {
            anyhow::bail!("Cannot delete current branch '{}' without a parent to checkout", name);
        }
    }

    // Track if we need to return to a different branch after rebasing
    let return_to_branch = if current != name { Some(current.clone()) } else { None };

    // If reparenting with children, we need atomic operations with rollback capability
    if reparent && !children.is_empty() {
        // Check for worktree conflicts before rebasing children
        worktree::check_branches_for_worktree_conflicts(&children)?;

        if let Some(ref gp) = grandparent {
            // PHASE 1: Create backup refs for all children BEFORE any modifications
            println!("{} Creating backups...", "→".blue());
            let recorder = OperationRecorder::new()?;
            let mut backups: Vec<(String, BackupRef)> = Vec::new();

            for child in &children {
                if gateway.branch_exists(child)? {
                    let backup = gateway.create_backup_ref(child)?;
                    println!(
                        "  {} Backed up {} @ {}",
                        "✓".green(),
                        child,
                        &backup.commit_oid.to_string()[..7]
                    );
                    recorder.record(Operation::BackupCreated {
                        branch: child.clone(),
                        backup_ref: backup.ref_name.clone(),
                    })?;
                    backups.push((child.clone(), backup));
                }
            }

            // PHASE 2: Update metadata FIRST (atomic operation)
            // This ensures metadata is always consistent - if we fail during rebase,
            // at least the parent-child relationships are correct
            println!("{} Updating metadata...", "→".blue());
            {
                // Acquire lock for metadata updates to prevent race conditions
                let _lock = ref_store.lock()?;
                for child in &children {
                    ref_store.reparent(child, gp)?;
                }
                ref_store.remove_parent(&name)?;
            } // Lock released here

            // PHASE 3: Rebase children onto grandparent
            println!("{} Rebasing children...", "→".blue());
            for child in &children {
                if gateway.branch_exists(child)? {
                    println!("  Restacking {} on {}...", child.green(), gp.blue());
                    let outcome = gateway.rebase_onto_from(child, gp, &name)?;
                    if outcome.has_conflicts() {
                        // Rebase failed - rollback everything
                        println!(
                            "\n{} Conflicts while rebasing '{}'. Rolling back...",
                            "!".yellow().bold(),
                            child
                        );

                        // Abort the current rebase
                        if gateway.rebase_in_progress()? {
                            gateway.rebase_abort()?;
                        }

                        // Restore all children from backups, collecting any errors
                        let mut rollback_errors: Vec<String> = Vec::new();

                        for (branch, backup) in &backups {
                            println!("  Restoring {} from backup...", branch);
                            if let Err(e) = gateway.restore_from_backup(backup) {
                                rollback_errors.push(format!("Failed to restore {}: {}", branch, e));
                            }
                        }

                        // Revert ref changes, collecting any errors
                        // Acquire lock for metadata rollback
                        if let Ok(_lock) = ref_store.lock() {
                            for child in &children {
                                if let Err(e) = ref_store.set_parent(child, &name) {
                                    rollback_errors.push(format!("Failed to restore parent for {}: {}", child, e));
                                }
                            }
                            if let Some(ref gp_ref) = grandparent {
                                if let Err(e) = ref_store.set_parent(&name, gp_ref) {
                                    rollback_errors.push(format!("Failed to restore parent for {}: {}", name, e));
                                }
                            }
                        } else {
                            rollback_errors.push("Failed to acquire lock for metadata rollback".to_string());
                        }

                        if rollback_errors.is_empty() {
                            println!(
                                "\n{} Rollback complete. Branch '{}' was NOT deleted.",
                                "✓".green(),
                                name
                            );
                        } else {
                            eprintln!(
                                "\n{} Rollback completed with errors:\n{}",
                                "!".yellow().bold(),
                                rollback_errors.join("\n")
                            );
                            eprintln!("\nManual intervention may be required.");
                        }
                        anyhow::bail!(
                            "Rebase conflict in '{}'. Resolve conflicts manually or use -f to force delete.",
                            child
                        );
                    }
                    println!("    {} Rebased {}", "✓".green(), child);
                }
            }

            // Return to original branch if we were not on the deleted branch
            if let Some(ref branch) = return_to_branch {
                gateway.checkout_branch(branch)?;
            }

            // PHASE 4: Delete from git (metadata already updated)
            gateway.delete_branch(&name)?;
            println!("{} Deleted branch: {}", "✓".green().bold(), name);
            return Ok(());
        }
    }

    // Simple case: no reparenting or no children
    ref_store.remove_parent(&name)?;

    // Delete from git
    gateway.delete_branch(&name)?;

    println!("{} Deleted branch: {}", "✓".green().bold(), name);
    Ok(())
}

/// Delete a branch and all its descendants (upstack deletion)
fn delete_upstack(gateway: &GitGateway, ref_store: &RefStore, name: &str, current: &str, force: bool) -> Result<()> {
    // Collect all branches to delete (target + all descendants)
    let mut branches_to_delete = vec![name.to_string()];
    branches_to_delete.extend(ref_store.descendants(name)?);

    let trunk = ref_store.get_trunk()?;

    // If current branch is in the delete list, checkout trunk first
    if branches_to_delete.contains(&current.to_string()) {
        if let Some(ref t) = trunk {
            gateway.checkout_branch(t)?;
            println!("Checked out trunk branch: {}", t);
        } else {
            anyhow::bail!("Cannot delete current branch without trunk to checkout");
        }
    }

    // Check if branches are merged (if not force)
    if !force {
        if let Some(ref t) = trunk {
            for branch in &branches_to_delete {
                if let Ok(false) = gateway.is_branch_merged(branch, t) {
                    anyhow::bail!("Branch '{}' is not merged. Use --force to delete anyway.", branch);
                }
            }
        }
    }

    // Confirm deletion (unless --force)
    if !force {
        let desc_count = branches_to_delete.len() - 1;
        let msg = if desc_count > 0 {
            format!(
                "Delete '{}' and {} descendant(s)? ({})",
                name,
                desc_count,
                branches_to_delete[1..].join(", ")
            )
        } else {
            format!("Delete '{}'?", name)
        };
        if !ui::confirm(&msg, false)? {
            println!("Delete cancelled.");
            return Ok(());
        }
    }

    // Delete all branches
    let count = branches_to_delete.len();
    for branch in &branches_to_delete {
        ref_store.remove_parent(branch)?;
        if gateway.branch_exists(branch)? {
            gateway.delete_branch(branch)?;
        }
    }

    println!(
        "{} Deleted {} branch(es) upstack from '{}'",
        "✓".green().bold(),
        count,
        name
    );

    Ok(())
}

/// Delete a branch and all its ancestors up to (but not including) trunk
fn delete_downstack(gateway: &GitGateway, ref_store: &RefStore, name: &str, current: &str, force: bool) -> Result<()> {
    let trunk = ref_store.require_trunk()?;

    // Collect all branches to delete (target + all ancestors except trunk), with cycle detection
    let mut branches_to_delete = vec![name.to_string()];
    let mut seen = std::collections::HashSet::new();
    seen.insert(name.to_string());
    let mut current_branch = name.to_string();
    while let Some(parent) = ref_store.get_parent(&current_branch)? {
        if parent == trunk {
            break; // Don't delete trunk
        }
        // Cycle detection
        if !seen.insert(parent.clone()) {
            anyhow::bail!(
                "Circular parent reference detected at '{}'. Run 'dm cleanup' to repair metadata.",
                parent
            );
        }
        branches_to_delete.push(parent.clone());
        current_branch = parent;
    }

    // If current branch is in the delete list, checkout trunk first
    if branches_to_delete.contains(&current.to_string()) {
        gateway.checkout_branch(&trunk)?;
        println!("Checked out trunk branch: {}", trunk);
    }

    // Check if branches are merged (if not force)
    if !force {
        for branch in &branches_to_delete {
            if let Ok(false) = gateway.is_branch_merged(branch, &trunk) {
                anyhow::bail!("Branch '{}' is not merged. Use --force to delete anyway.", branch);
            }
        }
    }

    // Confirm deletion (unless --force)
    if !force {
        let anc_count = branches_to_delete.len() - 1;
        let msg = if anc_count > 0 {
            format!(
                "Delete '{}' and {} ancestor(s)? ({})",
                name,
                anc_count,
                branches_to_delete[1..].join(", ")
            )
        } else {
            format!("Delete '{}'?", name)
        };
        if !ui::confirm(&msg, false)? {
            println!("Delete cancelled.");
            return Ok(());
        }
    }

    // Reparent children of the deepest deleted branch to trunk
    let deepest = branches_to_delete.last().unwrap();
    let deepest_children: Vec<String> = ref_store.get_children(deepest)?.into_iter().collect();
    for child in &deepest_children {
        if !branches_to_delete.contains(child) {
            ref_store.reparent(child, &trunk)?;
        }
    }

    // Delete all branches (in reverse order to avoid orphan issues)
    let count = branches_to_delete.len();
    for branch in &branches_to_delete {
        ref_store.remove_parent(branch)?;
        if gateway.branch_exists(branch)? {
            gateway.delete_branch(branch)?;
        }
    }

    println!(
        "{} Deleted {} branch(es) downstack from '{}'",
        "✓".green().bold(),
        count,
        name
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;

    use std::path::Path;
    use tempfile::tempdir;

    use crate::test_context::TestRepoContext;

    fn init_test_repo_with_branch(path: &Path, branch_name: &str) -> Result<Repository> {
        let repo = Repository::init(path)?;

        let mut config = repo.config()?;
        config.set_str("init.defaultBranch", branch_name)?;

        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        let refname = format!("refs/heads/{}", branch_name);
        repo.commit(Some(&refname), &sig, &sig, "Initial commit", &tree, &[])?;
        drop(tree);

        repo.set_head(&refname)?;

        Ok(repo)
    }

    #[test]
    fn test_delete_branch_success() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create feature branch
        gateway.create_branch("feature")?;
        gateway.checkout_branch("main")?;

        // Set up refs
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Delete feature branch
        run(Some("feature".to_string()), true, true, false, false)?;

        // Verify deleted from git
        assert!(!gateway.branch_exists("feature")?);

        // Verify deleted from refs
        assert!(!ref_store.is_tracked("feature")?);

        Ok(())
    }

    #[test]
    fn test_delete_nonexistent_branch_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let result = run(Some("nonexistent".to_string()), true, true, false, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));

        Ok(())
    }

    #[test]
    fn test_delete_current_branch_checks_out_parent() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create and stay on feature branch
        gateway.create_branch("feature")?;

        // Set up refs
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Currently on feature
        assert_eq!(gateway.get_current_branch_name()?, "feature");

        // Delete current branch - should checkout parent first
        run(Some("feature".to_string()), true, true, false, false)?;

        // Should now be on main
        assert_eq!(gateway.get_current_branch_name()?, "main");
        assert!(!gateway.branch_exists("feature")?);

        Ok(())
    }

    #[test]
    fn test_delete_reparents_children() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create branch hierarchy: main -> middle -> leaf
        gateway.create_branch("middle")?;
        gateway.create_branch("leaf")?;
        gateway.checkout_branch("main")?;

        // Set up refs
        ref_store.set_trunk("main")?;
        ref_store.set_parent("middle", "main")?;
        ref_store.set_parent("leaf", "middle")?;

        // Delete middle branch (with reparent)
        run(Some("middle".to_string()), true, true, false, false)?;

        // Verify leaf is now child of main (check refs)
        assert_eq!(ref_store.get_parent("leaf")?, Some("main".to_string()));
        assert!(!ref_store.is_tracked("middle")?);

        Ok(())
    }

    #[test]
    fn test_delete_trunk_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create feature and go to it
        gateway.create_branch("feature")?;

        // Set up refs
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Try to delete trunk
        let result = run(Some("main".to_string()), true, true, false, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot delete trunk"));

        Ok(())
    }

    #[test]
    fn test_delete_without_reparent() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create branch hierarchy: main -> middle -> leaf
        gateway.create_branch("middle")?;
        gateway.create_branch("leaf")?;
        gateway.checkout_branch("main")?;

        // Set up refs
        ref_store.set_trunk("main")?;
        ref_store.set_parent("middle", "main")?;
        ref_store.set_parent("leaf", "middle")?;

        // Delete middle branch (without reparent)
        run(Some("middle".to_string()), false, true, false, false)?;

        // Verify leaf is orphaned - parent ref still points to deleted branch
        // (RefStore doesn't prevent this without --reparent)
        assert_eq!(ref_store.get_parent("leaf")?, Some("middle".to_string()));
        assert!(!ref_store.is_tracked("middle")?);

        Ok(())
    }

    #[test]
    fn test_delete_upstack() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create branch hierarchy: main -> middle -> child1, child2
        gateway.create_branch("middle")?;
        gateway.create_branch("child1")?;
        gateway.checkout_branch("middle")?;
        gateway.create_branch("child2")?;
        gateway.checkout_branch("main")?;

        // Set up refs
        ref_store.set_trunk("main")?;
        ref_store.set_parent("middle", "main")?;
        ref_store.set_parent("child1", "middle")?;
        ref_store.set_parent("child2", "middle")?;

        // Delete middle branch with --upstack (should delete middle, child1, child2)
        run(Some("middle".to_string()), false, true, true, false)?;

        // Verify all branches are deleted
        assert!(!gateway.branch_exists("middle")?);
        assert!(!gateway.branch_exists("child1")?);
        assert!(!gateway.branch_exists("child2")?);
        assert!(!ref_store.is_tracked("middle")?);
        assert!(!ref_store.is_tracked("child1")?);
        assert!(!ref_store.is_tracked("child2")?);

        Ok(())
    }

    #[test]
    fn test_delete_downstack() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Create branch hierarchy: main -> parent1 -> parent2 -> leaf
        gateway.create_branch("parent1")?;
        gateway.create_branch("parent2")?;
        gateway.create_branch("leaf")?;
        gateway.checkout_branch("main")?;

        // Set up refs
        ref_store.set_trunk("main")?;
        ref_store.set_parent("parent1", "main")?;
        ref_store.set_parent("parent2", "parent1")?;
        ref_store.set_parent("leaf", "parent2")?;

        // Delete leaf with --downstack (should delete leaf, parent2, parent1, but NOT trunk)
        run(Some("leaf".to_string()), false, true, false, true)?;

        // Verify branches are deleted (except trunk)
        assert!(!gateway.branch_exists("leaf")?);
        assert!(!gateway.branch_exists("parent2")?);
        assert!(!gateway.branch_exists("parent1")?);
        assert!(gateway.branch_exists("main")?); // trunk should still exist
        assert!(!ref_store.is_tracked("leaf")?);
        assert!(!ref_store.is_tracked("parent2")?);
        assert!(!ref_store.is_tracked("parent1")?);

        Ok(())
    }

    #[test]
    fn test_delete_upstack_and_downstack_mutually_exclusive() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        let gateway = GitGateway::new()?;
        gateway.create_branch("feature")?;
        gateway.checkout_branch("main")?;

        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // Both upstack and downstack should fail
        let result = run(Some("feature".to_string()), false, true, true, true);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("mutually exclusive") || err_msg.contains("cannot use both"),
            "Expected mutual exclusivity error, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_delete_without_name_in_non_tty_shows_helpful_error() -> Result<()> {
        // stdin is not TTY in test environment
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        // Call without name - should fail with helpful message
        let result = run(None, false, false, false, false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("delete requires a branch name") || err.contains("non-interactive"),
            "Expected helpful non-TTY error, got: {}",
            err
        );

        Ok(())
    }
}
