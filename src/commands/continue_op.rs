use anyhow::{bail, Result};

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::state::{OperationState, OperationType};
use crate::ui;

/// Verify that the current git state matches the expected state from the operation
fn verify_git_state_matches(state: &OperationState, gateway: &GitGateway) -> Result<()> {
    // Check current branch matches if not in rebase
    if !gateway.rebase_in_progress()? {
        if let Some(expected) = &state.current_branch {
            let actual = gateway.get_current_branch_name()?;
            if &actual != expected {
                bail!(
                    "State mismatch: expected to be on '{}', but currently on '{}'.\n\
                    The git state may have been modified manually. Use '{} abort' to reset.",
                    expected,
                    actual,
                    program_name()
                );
            }
        }
    }

    // Verify remaining branches exist
    for branch in &state.remaining_branches {
        if !gateway.branch_exists(branch)? {
            bail!(
                "Branch '{}' no longer exists.\n\
                Run '{} abort' to cancel the operation.",
                branch,
                program_name()
            );
        }
    }

    Ok(())
}

/// Continue an interrupted operation (sync, restack, move)
pub fn run() -> Result<()> {
    let gateway = GitGateway::new()?;

    // Check if there's an operation in progress
    let state = OperationState::load()?;
    if state.is_none() {
        anyhow::bail!("No operation in progress to continue.");
    }

    let mut state = state.unwrap();

    // Verify git state matches expected state
    verify_git_state_matches(&state, &gateway)?;

    // Dispatch to appropriate handler based on operation type
    let ref_store = RefStore::new()?;

    // If there's a rebase in progress, continue it first
    if gateway.rebase_in_progress()? {
        let result = gateway.rebase_continue()?;
        if result.has_conflicts() {
            // Get parent branch for conflict context
            let current = state
                .current_branch
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Operation state missing current branch"))?;

            let parent = ref_store.get_parent(current)?.unwrap_or_else(|| "trunk".to_string());

            // Show rich conflict message
            crate::ui::display_conflict_message(
                current,
                &parent,
                &state.remaining_branches,
                &ref_store,
                &gateway,
                true, // continue attempt
            )?;

            return Ok(());
        } else {
            // Conflicts resolved! Show success message
            if let Some(current) = &state.current_branch {
                crate::ui::success(&format!("Resolved conflicts in {}", current));
            }
        }
    }
    match state.operation_type {
        OperationType::Sync => {
            // Skip cleanup on continue (was already handled/skipped in initial sync)
            // Map SyncOutcome to Result<()> - we don't need the outcome details here
            crate::commands::sync::continue_sync_from_state(&mut state, &ref_store, true, false).map(|_| ())
        }
        OperationType::Restack => crate::commands::restack::continue_restack_from_state(&mut state, &ref_store),
        OperationType::Move => crate::commands::move_cmd::continue_move_from_state(&mut state, &ref_store),
        OperationType::Insert => continue_insert_from_state(&state, &gateway),
    }
}

/// Continue an insert operation after conflicts are resolved
fn continue_insert_from_state(state: &OperationState, gateway: &GitGateway) -> Result<()> {
    // The child branch has been rebased onto the new branch (original_branch)
    // Just need to return to the new branch and clear state

    // Return to the new branch (the one that was inserted)
    gateway.checkout_branch_worktree_safe(&state.original_branch)?;

    // Clear operation state
    OperationState::clear()?;

    ui::success_bold(&format!(
        "Insert complete! '{}' is now between the parent and child",
        state.original_branch
    ));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_continue_no_operation_fails() {
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
    fn test_continue_validates_current_branch() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();

        // Create branches
        gateway.create_branch("feature").unwrap();
        gateway.checkout_branch_worktree_safe("main").unwrap();

        // Create operation state saying we should be on "feature"
        let mut state = OperationState::new_restack("main".to_string(), vec!["feature".to_string()]);
        state.current_branch = Some("feature".to_string());
        state.save().unwrap();

        // We're on main but state says we should be on feature - should fail
        let result = run();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("State mismatch") || err.contains("expected"),
            "Error should mention state mismatch: {}",
            err
        );

        // Clean up
        OperationState::clear().ok();
        drop(repo);
    }

    #[test]
    fn test_continue_fails_if_branch_deleted() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();

        // Create a branch
        gateway.create_branch("feature").unwrap();
        gateway.checkout_branch_worktree_safe("main").unwrap();

        // Create operation state with "feature" in remaining branches
        let state = OperationState::new_restack(
            "main".to_string(),
            vec!["feature".to_string(), "deleted-branch".to_string()],
        );
        state.save().unwrap();

        // "deleted-branch" doesn't exist in git - should fail
        let result = run();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no longer exists") || err.contains("deleted-branch"),
            "Error should mention missing branch: {}",
            err
        );

        // Clean up
        OperationState::clear().ok();
        drop(repo);
    }

    #[test]
    fn test_verify_git_state_matches_passes_when_correct() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();

        // Create branches
        gateway.create_branch("feature-1").unwrap();
        gateway.create_branch("feature-2").unwrap();
        gateway.checkout_branch_worktree_safe("main").unwrap();

        // State with current_branch = None (not in middle of rebase)
        let state = OperationState::new_restack(
            "main".to_string(),
            vec!["feature-1".to_string(), "feature-2".to_string()],
        );

        // Should pass - no current_branch to check, and both branches exist
        let result = verify_git_state_matches(&state, &gateway);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_git_state_matches_current_branch_matches() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();

        // Create and checkout feature
        gateway.create_branch("feature").unwrap();

        // State says we should be on feature, and we are
        let mut state = OperationState::new_restack("main".to_string(), vec![]);
        state.current_branch = Some("feature".to_string());

        let result = verify_git_state_matches(&state, &gateway);
        assert!(result.is_ok());
    }

    #[test]
    fn test_continue_insert_clears_state_and_returns_to_new_branch() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();

        // Create new-branch (the one being inserted) and child branch
        gateway.create_branch("new-branch").unwrap();
        gateway.create_branch("child").unwrap();

        // Simulate being on child after a successful rebase
        gateway.checkout_branch_worktree_safe("child").unwrap();

        // Create insert operation state
        let state = OperationState::new_insert("new-branch".to_string(), "child".to_string(), "main".to_string());
        state.save().unwrap();

        // Continue should work (returns to new-branch and clears state)
        let result = continue_insert_from_state(&state, &gateway);
        assert!(result.is_ok());

        // State should be cleared
        assert!(OperationState::load().unwrap().is_none());

        // Should be on new-branch now
        assert_eq!(gateway.get_current_branch_name().unwrap(), "new-branch");
    }
}
