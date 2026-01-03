use anyhow::Result;
use colored::Colorize;

use crate::git_gateway::GitGateway;
use crate::ref_store::RefStore;
use crate::state::{OperationState, OperationType};

/// Abort an interrupted operation (sync, restack, move)
pub fn run() -> Result<()> {
    let gateway = GitGateway::new()?;

    // Check if there's an operation in progress
    let state = OperationState::load()?;
    if state.is_none() {
        anyhow::bail!("No operation in progress to abort.");
    }

    let state = state.unwrap();

    // Abort any git rebase in progress
    if gateway.rebase_in_progress()? {
        gateway.rebase_abort()?;
    }

    // For sync/restack operations, restore all branches from backups
    if state.operation_type == OperationType::Sync || state.operation_type == OperationType::Restack {
        restore_branches_from_backups(&gateway, &state)?;
    }

    // For move operations, rollback metadata changes using old_parent
    if state.operation_type == OperationType::Move {
        if let Some(old_parent) = &state.old_parent {
            rollback_move_metadata(&state.original_branch, old_parent)?;
            println!("  {} Metadata reverted", "✓".green());
        }
    }

    // For insert operations, revert the child's parent back to original
    if state.operation_type == OperationType::Insert {
        if let (Some(child), Some(original_parent)) = (state.current_branch.as_ref(), state.old_parent.as_ref()) {
            let ref_store = RefStore::new()?;
            ref_store.set_parent(child, original_parent)?;
            println!(
                "  {} Reverted '{}' parent back to '{}'",
                "✓".green(),
                child,
                original_parent
            );
        }
    }

    // Return to original branch
    gateway.checkout_branch(&state.original_branch)?;

    // Clear operation state
    OperationState::clear()?;

    // Capitalize the operation type for display
    let op_name = state.operation_type.to_string();
    let op_name_cap = op_name[..1].to_uppercase() + &op_name[1..];

    println!("{} {} aborted", "✗".red().bold(), op_name_cap);
    Ok(())
}

/// Restore all branches from backup refs created at the start of the operation
fn restore_branches_from_backups(gateway: &GitGateway, state: &OperationState) -> Result<()> {
    // Get all backup refs
    let backups = gateway.list_backup_refs()?;

    if backups.is_empty() {
        return Ok(());
    }

    // Use all_branches if available, otherwise fall back to remaining_branches
    let branches_to_restore = if !state.all_branches.is_empty() {
        &state.all_branches
    } else {
        // Backward compatibility: if all_branches is empty (old state file),
        // we can only restore remaining branches
        &state.remaining_branches
    };

    if branches_to_restore.is_empty() {
        return Ok(());
    }

    println!("{} Restoring branches from backups...", "→".blue());

    let mut restored_count = 0;
    let mut restore_failures = Vec::new();

    for branch in branches_to_restore {
        // Find the most recent backup for this branch
        if let Some(backup) = backups.iter().find(|b| b.branch_name == *branch) {
            // Restore the branch from backup (works whether branch exists or was deleted)
            match gateway.restore_from_backup(backup) {
                Ok(()) => {
                    println!(
                        "  {} Restored {} to {}",
                        "✓".green(),
                        branch,
                        &backup.commit_oid.to_string()[..7]
                    );
                    restored_count += 1;
                }
                Err(e) => {
                    restore_failures.push(format!("{}: {}", branch, e));
                }
            }
        }
    }

    // Report any failures but don't fail the abort operation
    if !restore_failures.is_empty() {
        eprintln!("  {} Warning: Failed to restore some branches:", "!".yellow());
        for failure in &restore_failures {
            eprintln!("    - {}", failure);
        }
    }

    if restored_count > 0 {
        println!("  {} Restored {} branches from backups", "✓".green(), restored_count);
    }

    Ok(())
}

/// Rollback move metadata by restoring the old parent
fn rollback_move_metadata(branch: &str, old_parent: &str) -> Result<()> {
    let ref_store = RefStore::new()?;

    // Simply set the parent back to the old value
    // RefStore handles the parent-child relationship automatically
    ref_store.set_parent(branch, old_parent)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_abort_no_operation_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // No operation in progress
        OperationState::clear().ok();

        let result = run();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No operation in progress"));
    }

    #[test]
    fn test_abort_sync_operation() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create a sync operation state
        let state = OperationState::new_sync("main".to_string(), vec!["feature-1".to_string()]);
        state.save().unwrap();

        // Abort should succeed
        let result = run();
        assert!(result.is_ok());

        // State should be cleared
        assert!(OperationState::load().unwrap().is_none());
    }

    #[test]
    fn test_abort_restack_operation() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create a restack operation state
        let state = OperationState::new_restack("main".to_string(), vec!["feature-1".to_string()]);
        state.save().unwrap();

        // Abort should succeed
        let result = run();
        assert!(result.is_ok());

        // State should be cleared
        assert!(OperationState::load().unwrap().is_none());
    }

    #[test]
    fn test_abort_move_operation() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create a move operation state
        let state = OperationState::new_move(
            "main".to_string(),
            vec!["feature-1".to_string()],
            "develop".to_string(),
            None, // No old_parent in this test
        );
        state.save().unwrap();

        // Abort should succeed
        let result = run();
        assert!(result.is_ok());

        // State should be cleared
        assert!(OperationState::load().unwrap().is_none());
    }

    #[test]
    fn test_abort_insert_operation_reverts_parent() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();
        let ref_store = RefStore::new().unwrap();

        // Set up: main -> child (original structure)
        ref_store.set_trunk("main").unwrap();
        gateway.create_branch("child").unwrap();
        ref_store.set_parent("child", "main").unwrap();

        // Create new-branch (the one being inserted)
        gateway.create_branch("new-branch").unwrap();
        gateway.checkout_branch("new-branch").unwrap();

        // Simulate insert: child now points to new-branch (already modified by create --insert)
        ref_store.set_parent("child", "new-branch").unwrap();
        ref_store.set_parent("new-branch", "main").unwrap();

        // Create insert operation state (as if we're mid-rebase)
        let state = OperationState::new_insert(
            "new-branch".to_string(),
            "child".to_string(),
            "main".to_string(), // original parent before insert
        );
        state.save().unwrap();

        // Abort should succeed and revert child's parent
        let result = run();
        assert!(result.is_ok());

        // State should be cleared
        assert!(OperationState::load().unwrap().is_none());

        // Child's parent should be reverted to original (main)
        let child_parent = ref_store.get_parent("child").unwrap();
        assert_eq!(child_parent, Some("main".to_string()));
    }

    #[test]
    fn test_abort_restack_restores_from_backups() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();

        // Create a feature branch
        gateway.create_branch("feature-1").unwrap();

        // Make a commit on feature-1
        fs::write(dir.path().join("feature1.txt"), "original content").unwrap();
        gateway.stage_all().unwrap();
        gateway.commit("Feature 1 commit").unwrap();

        // Record original commit for feature-1
        let original_commit = repo.head().unwrap().peel_to_commit().unwrap().id().to_string();

        // Create a backup (simulating what restack does)
        let backup = gateway.create_backup_ref("feature-1").unwrap();
        assert_eq!(backup.commit_oid, original_commit);

        // Simulate a rebase that changed the branch (make new commit)
        fs::write(dir.path().join("feature1.txt"), "modified content").unwrap();
        gateway.stage_all().unwrap();
        gateway.commit("Rebased commit").unwrap();

        // Verify commit changed
        let modified_commit = repo.head().unwrap().peel_to_commit().unwrap().id().to_string();
        assert_ne!(modified_commit, original_commit);

        // Go back to main
        gateway.checkout_branch("main").unwrap();

        // Create restack operation state with all_branches
        let state = OperationState::new_restack("main".to_string(), vec!["feature-1".to_string()]);
        state.save().unwrap();

        // Abort should restore from backup
        let result = run();
        assert!(result.is_ok());

        // Verify feature-1 was restored to original commit
        let restored_commit = repo
            .find_branch("feature-1", git2::BranchType::Local)
            .unwrap()
            .get()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string();
        assert_eq!(
            restored_commit, original_commit,
            "Branch should be restored to original commit from backup"
        );
    }

    #[test]
    fn test_operation_state_has_all_branches() {
        // Verify that new_sync and new_restack populate all_branches
        let sync_state = OperationState::new_sync(
            "main".to_string(),
            vec!["feature-1".to_string(), "feature-2".to_string()],
        );
        assert_eq!(sync_state.all_branches, vec!["feature-1", "feature-2"]);
        assert_eq!(sync_state.remaining_branches, vec!["feature-1", "feature-2"]);

        let restack_state = OperationState::new_restack("main".to_string(), vec!["branch-a".to_string()]);
        assert_eq!(restack_state.all_branches, vec!["branch-a"]);

        let move_state = OperationState::new_move(
            "main".to_string(),
            vec!["feature".to_string()],
            "develop".to_string(),
            Some("main".to_string()),
        );
        assert_eq!(move_state.all_branches, vec!["feature"]);
    }
}
