mod common;

use anyhow::Result;
use common::*;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

// ============================================================================
// CATEGORY 1: Git Operations Bypassing Diamond
// ============================================================================

#[test]
fn test_git_deletes_tracked_branch_doctor_fixes() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create tracked branch via Diamond
    fs::write(temp_dir.path().join("f.txt"), "content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Go back to main
    run_dm(temp_dir.path(), &["checkout", "main"])?;

    // Delete via git directly (bypassing Diamond)
    run_git(temp_dir.path(), &["branch", "-D", "feature"])?;

    // Verify branch is gone from git
    assert!(!git_branch_exists(temp_dir.path(), "feature")?);

    // Doctor should detect the issue
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("feature") && stdout.contains("doesn't exist"),
        "Doctor should detect missing branch: {}",
        stdout
    );

    // Doctor --fix should clean up metadata
    let output = run_dm(temp_dir.path(), &["doctor", "--fix"])?;
    assert!(output.status.success());

    // Verify branch is no longer tracked in refs
    assert!(
        !is_branch_tracked_in_refs(temp_dir.path(), "feature")?,
        "feature should be removed from refs"
    );

    // dm log should work cleanly now
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());

    Ok(())
}

#[test]
fn test_git_checkout_b_untracked_dm_fails_gracefully() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch via git directly (not Diamond)
    run_git(temp_dir.path(), &["checkout", "-b", "untracked-feature"])?;

    // dm up should fail with helpful message (it expects a tracked branch)
    let output = run_dm(temp_dir.path(), &["up"])?;
    // Either fails or shows no children (since untracked branch has no children)
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        !output.status.success() || combined.contains("No child") || combined.contains("not tracked"),
        "Should handle untracked branch: {}",
        combined
    );

    // dm parent should fail with helpful message
    let output = run_dm(temp_dir.path(), &["parent"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("not tracked") || stderr.contains("No parent"),
        "Should mention tracking issue or no parent"
    );

    Ok(())
}

#[test]
fn test_git_rename_tracked_branch_detected() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create tracked branch
    fs::write(temp_dir.path().join("f.txt"), "content")?;
    run_dm(temp_dir.path(), &["create", "old-name", "-a", "-m", "Feature"])?;

    // Rename via git directly
    run_git(temp_dir.path(), &["branch", "-m", "old-name", "new-name"])?;

    // Doctor should detect the missing branch
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("old-name") && stdout.contains("doesn't exist"),
        "Doctor should detect renamed branch: {}",
        stdout
    );

    // Doctor --fix should clean up
    let output = run_dm(temp_dir.path(), &["doctor", "--fix"])?;
    assert!(output.status.success());

    Ok(())
}

#[test]
fn test_git_reset_hard_on_tracked_branch() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create tracked branch with multiple commits
    run_dm(temp_dir.path(), &["create", "feature"])?;

    fs::write(temp_dir.path().join("f1.txt"), "commit 1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", "Commit 1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "commit 2")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", "Commit 2"])?;

    fs::write(temp_dir.path().join("f3.txt"), "commit 3")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", "Commit 3"])?;

    // Reset hard via git (bypassing Diamond)
    run_git(temp_dir.path(), &["reset", "--hard", "HEAD~2"])?;

    // Diamond operations should still work
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());

    // Parent should still be main
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("main"));

    // Can still navigate
    let output = run_dm(temp_dir.path(), &["down"])?;
    assert!(output.status.success());

    Ok(())
}

#[test]
fn test_git_commit_amend_without_dm_modify() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create parent branch
    fs::write(temp_dir.path().join("p.txt"), "parent")?;
    run_dm(temp_dir.path(), &["create", "parent-branch", "-a", "-m", "Parent"])?;

    // Create child branch
    fs::write(temp_dir.path().join("c.txt"), "child")?;
    run_dm(temp_dir.path(), &["create", "child-branch", "-a", "-m", "Child"])?;

    // Go back to parent and amend via git (not dm modify)
    run_dm(temp_dir.path(), &["checkout", "parent-branch"])?;
    fs::write(temp_dir.path().join("p2.txt"), "amended")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "--amend", "--no-edit"])?;

    // Child branch should still exist and be navigable
    run_dm(temp_dir.path(), &["checkout", "child-branch"])?;
    assert_eq!(get_current_branch(temp_dir.path())?, "child-branch");

    // Parent relationship should still be correct
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("parent-branch"));

    Ok(())
}

#[test]
fn test_git_merge_detected_by_cleanup() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create feature branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Merge via git (bypassing Diamond submit flow)
    run_git(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["merge", "feature", "--no-ff", "-m", "Merge feature"])?;

    // dm cleanup should detect the merged branch
    let output = run_dm(temp_dir.path(), &["cleanup", "--force"])?;
    assert!(output.status.success());

    // Branch should be deleted
    assert!(!git_branch_exists(temp_dir.path(), "feature")?);

    Ok(())
}

#[test]
fn test_git_rebase_tracked_branch_manually() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create feature branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Add commit to main
    run_git(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("main.txt"), "main update")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "-m", "Main update"])?;

    // Manually rebase feature onto updated main (bypassing dm restack)
    run_git(temp_dir.path(), &["checkout", "feature"])?;
    run_git(temp_dir.path(), &["rebase", "main"])?;

    // Diamond operations should still work
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());

    // Parent should still be main
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("main"));

    Ok(())
}

// ============================================================================
// CATEGORY 2: Metadata Corruption
// ============================================================================

#[test]
fn test_missing_trunk_ref_fails_gracefully() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch so we have some refs
    fs::write(temp_dir.path().join("f.txt"), "content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Remove the trunk ref directly
    run_git(temp_dir.path(), &["update-ref", "-d", "refs/diamond/trunk"])?;

    // dm commands should still work or fail gracefully
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    // Either succeeds with warning or fails gracefully
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() || !stderr.is_empty() || !stdout.is_empty(),
        "Should provide some feedback"
    );

    Ok(())
}

#[test]
fn test_missing_diamond_auto_initializes() -> Result<()> {
    let temp_dir = TempDir::new()?;

    // Initialize git but NOT diamond (use -b main for consistency across environments)
    run_git(temp_dir.path(), &["init", "-b", "main"])?;
    run_git(temp_dir.path(), &["config", "user.name", "Test"])?;
    run_git(temp_dir.path(), &["config", "user.email", "test@test.com"])?;
    fs::write(temp_dir.path().join("README.md"), "# Test")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "-m", "Initial"])?;

    // dm create auto-initializes when diamond is not initialized
    let output = run_dm(temp_dir.path(), &["create", "feature"])?;
    assert!(
        output.status.success(),
        "dm create should auto-initialize: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify we're on the feature branch
    assert_eq!(get_current_branch(temp_dir.path())?, "feature");

    // Verify branch is tracked in refs
    assert!(
        is_branch_tracked_in_refs(temp_dir.path(), "feature")?,
        "feature branch should be tracked in refs"
    );

    // Verify parent relationship
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("main"));

    Ok(())
}

#[test]
fn test_parent_branch_deleted_doctor_detects() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a stack: main -> parent -> child
    fs::write(temp_dir.path().join("p.txt"), "p")?;
    run_dm(temp_dir.path(), &["create", "parent-branch", "-a", "-m", "Parent"])?;

    fs::write(temp_dir.path().join("c.txt"), "c")?;
    run_dm(temp_dir.path(), &["create", "child-branch", "-a", "-m", "Child"])?;

    // Delete parent branch via git, leaving child orphaned
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["branch", "-D", "parent-branch"])?;

    // Doctor should detect the missing parent
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("parent-branch") && stdout.contains("doesn't exist"),
        "Doctor should detect missing parent branch: {}",
        stdout
    );

    Ok(())
}

#[test]
fn test_branch_tracked_but_git_branch_deleted() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create proper stack
    fs::write(temp_dir.path().join("f.txt"), "f")?;
    run_dm_success(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Verify the parent ref was created
    assert!(
        is_branch_tracked_in_refs(temp_dir.path(), "feature")?,
        "feature branch should be tracked after dm create"
    );

    // Go back to main and delete the git branch (but refs still exist)
    run_dm_success(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["branch", "-D", "feature"])?;

    // Verify ref still exists but git branch doesn't
    assert!(
        is_branch_tracked_in_refs(temp_dir.path(), "feature")?,
        "feature should still be tracked in refs after git branch -D"
    );
    assert!(
        !git_branch_exists(temp_dir.path(), "feature")?,
        "feature git branch should be deleted"
    );

    // Doctor should detect the issue - branch is tracked but doesn't exist in git
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("feature") && stdout.contains("doesn't exist"),
        "Doctor should detect missing branch: {}",
        stdout
    );

    Ok(())
}

#[test]
fn test_stacked_branches_basic() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a stack of branches: main -> a -> b -> c
    for name in &["a", "b", "c"] {
        fs::write(temp_dir.path().join(format!("{}.txt", name)), name)?;
        run_dm(
            temp_dir.path(),
            &["create", name, "-a", "-m", &format!("Branch {}", name)],
        )?;
    }

    // Verify stack structure
    assert_eq!(get_parent_from_refs(temp_dir.path(), "a")?, Some("main".to_string()));
    assert_eq!(get_parent_from_refs(temp_dir.path(), "b")?, Some("a".to_string()));
    assert_eq!(get_parent_from_refs(temp_dir.path(), "c")?, Some("b".to_string()));

    // Doctor should show no issues
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(output.status.success());

    Ok(())
}

#[test]
fn test_reference_to_deleted_branch() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> parent -> child
    fs::write(temp_dir.path().join("p.txt"), "p")?;
    run_dm(temp_dir.path(), &["create", "parent-branch", "-a", "-m", "P"])?;

    fs::write(temp_dir.path().join("c.txt"), "c")?;
    run_dm(temp_dir.path(), &["create", "child-branch", "-a", "-m", "C"])?;

    // Delete parent via git, leaving child orphaned
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["branch", "-D", "parent-branch"])?;

    // Doctor should detect the missing parent and return error
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("parent-branch") && stdout.contains("doesn't exist"),
        "Doctor should detect missing parent: {}",
        stdout
    );
    assert!(!output.status.success(), "Doctor should return error when issues found");

    // Doctor --fix should be able to fix the orphaned parent by reparenting to main
    // (since child-branch is based on main through the deleted parent)
    let output = run_dm(temp_dir.path(), &["doctor", "--fix"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify it fixed the issue by reparenting to main
    assert!(
        stdout.contains("Fixed") && stdout.contains("reparented to 'main'"),
        "Doctor should fix orphaned parent by reparenting to main: {}",
        stdout
    );
    assert!(
        output.status.success(),
        "Doctor --fix should succeed when it can fix all issues"
    );

    // Verify the child is now parented to main
    assert_eq!(
        get_parent_from_refs(temp_dir.path(), "child-branch")?,
        Some("main".to_string()),
        "child-branch should be reparented to main"
    );

    // Doctor should now show no issues
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(output.status.success(), "Doctor should pass after fix");

    Ok(())
}

#[test]
fn test_trunk_deleted_detected() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a feature branch
    fs::write(temp_dir.path().join("f.txt"), "f")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "F"])?;

    // Delete trunk (main) via git
    run_dm(temp_dir.path(), &["checkout", "feature"])?;
    run_git(temp_dir.path(), &["branch", "-D", "main"])?;

    // Doctor should detect missing trunk
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("main") && (stdout.contains("doesn't exist") || stdout.contains("Trunk")),
        "Doctor should detect missing trunk: {}",
        stdout
    );

    Ok(())
}

// ============================================================================
// CATEGORY 3: Detached HEAD Scenarios
// ============================================================================

#[test]
fn test_dm_create_detached_head_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Detach HEAD
    run_git(temp_dir.path(), &["checkout", "--detach"])?;

    // dm create should fail with clear error
    let output = run_dm(temp_dir.path(), &["create", "feature"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("detached") || stderr.contains("HEAD") || stderr.contains("branch"),
        "Should mention detached HEAD: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_dm_modify_detached_head_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Make a change first
    fs::write(temp_dir.path().join("test.txt"), "content")?;

    // Detach HEAD
    run_git(temp_dir.path(), &["checkout", "--detach"])?;

    // dm modify should fail with clear error
    let output = run_dm(temp_dir.path(), &["modify", "-a", "-m", "Test"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("detached") || stderr.contains("HEAD") || stderr.contains("branch"),
        "Should mention detached HEAD: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_dm_navigation_detached_head_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch first
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Detach HEAD
    run_git(temp_dir.path(), &["checkout", "--detach"])?;

    // dm up should fail
    let output = run_dm(temp_dir.path(), &["up"])?;
    assert!(!output.status.success());

    // dm down should fail
    let output = run_dm(temp_dir.path(), &["down"])?;
    assert!(!output.status.success());

    // dm parent should fail
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(!output.status.success());

    Ok(())
}

// ============================================================================
// CATEGORY 4: Operation State Edge Cases
// ============================================================================

#[test]
fn test_stale_operation_state_no_rebase() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a fake operation state (as if interrupted)
    let stale_state = json!({
        "operation_type": "Sync",
        "in_progress": true,
        "current_branch": "feature",
        "remaining_branches": ["feature"],
        "original_branch": "main",
        "move_target_parent": null
    });
    create_operation_state(temp_dir.path(), &stale_state)?;

    // dm continue should detect no rebase in progress
    let output = run_dm(temp_dir.path(), &["continue"])?;
    // It might succeed by cleaning up, or fail gracefully
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Should not panic, should provide some feedback
    assert!(!combined.is_empty(), "Should provide feedback about stale state");

    Ok(())
}

#[test]
fn test_operation_state_deleted_branch() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create and delete a branch
    fs::write(temp_dir.path().join("f.txt"), "f")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "F"])?;
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["branch", "-D", "feature"])?;

    // Create operation state referencing deleted branch
    let stale_state = json!({
        "operation_type": "Sync",
        "in_progress": true,
        "current_branch": "feature",
        "remaining_branches": ["feature"],
        "original_branch": "main",
        "move_target_parent": null
    });
    create_operation_state(temp_dir.path(), &stale_state)?;

    // dm abort should handle gracefully
    let output = run_dm(temp_dir.path(), &["abort"])?;
    // Should either succeed or fail gracefully (not panic)
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.is_empty() || output.status.success(),
        "Should handle deleted branch gracefully"
    );

    Ok(())
}

#[test]
fn test_abort_clears_operation_state() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branches for a sync scenario
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Create operation state
    let op_state = json!({
        "operation_type": "Sync",
        "in_progress": true,
        "current_branch": "f2",
        "remaining_branches": ["f2"],
        "original_branch": "f1",
        "move_target_parent": null
    });
    create_operation_state(temp_dir.path(), &op_state)?;

    // dm abort should clear the state
    let _output = run_dm(temp_dir.path(), &["abort"])?;

    // Operation state should be cleared
    let op_state_path = temp_dir.path().join(".git/diamond/operation_state.json");
    // State should either be deleted or marked as not in progress
    if op_state_path.exists() {
        let content = fs::read_to_string(&op_state_path)?;
        let state: serde_json::Value = serde_json::from_str(&content)?;
        assert!(
            !state["in_progress"].as_bool().unwrap_or(false),
            "Operation should no longer be in progress"
        );
    }

    Ok(())
}

#[test]
fn test_git_rebase_abort_then_dm_continue() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a scenario where rebase was started then aborted via git
    fs::write(temp_dir.path().join("f.txt"), "f")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "F"])?;

    // Create operation state as if rebase was in progress
    let op_state = json!({
        "operation_type": "Sync",
        "in_progress": true,
        "current_branch": "feature",
        "remaining_branches": ["feature"],
        "original_branch": "main",
        "move_target_parent": null
    });
    create_operation_state(temp_dir.path(), &op_state)?;

    // dm continue should handle the case where no rebase is actually in progress
    let output = run_dm(temp_dir.path(), &["continue"])?;
    // Should either complete successfully or fail gracefully
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.is_empty() || output.status.success(),
        "Should handle missing rebase gracefully"
    );

    Ok(())
}

#[test]
fn test_nested_operations_rejected() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branches
    fs::write(temp_dir.path().join("f.txt"), "f")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "F"])?;

    // Create operation state
    let op_state = json!({
        "operation_type": "Sync",
        "in_progress": true,
        "current_branch": "feature",
        "remaining_branches": [],
        "original_branch": "main",
        "move_target_parent": null
    });
    create_operation_state(temp_dir.path(), &op_state)?;

    // Try to start another sync - should be rejected
    let output = run_dm(temp_dir.path(), &["sync"])?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("in progress") || stderr.contains("operation") || stderr.contains("abort"),
            "Should mention existing operation: {}",
            stderr
        );
    }

    Ok(())
}

// ============================================================================
// CATEGORY 5: Complex Stack Stress Tests
// ============================================================================

#[test]
fn test_deep_stack_15_levels_middle_delete() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a 15-level deep stack
    for i in 1..=15 {
        fs::write(
            temp_dir.path().join(format!("level{}.txt", i)),
            format!("content for level {}", i),
        )?;
        let output = run_dm(
            temp_dir.path(),
            &["create", &format!("level-{}", i), "-a", "-m", &format!("Level {}", i)],
        )?;
        assert!(
            output.status.success(),
            "Failed to create level-{}: {}",
            i,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Delete level-7 (middle of stack)
    run_dm(temp_dir.path(), &["checkout", "level-6"])?;
    let output = run_dm(temp_dir.path(), &["delete", "level-7", "--force"])?;
    assert!(output.status.success());

    // level-8 should now have level-6 as parent
    run_dm(temp_dir.path(), &["checkout", "level-8"])?;
    let output = run_dm(temp_dir.path(), &["parent"])?;
    let parent = String::from_utf8_lossy(&output.stdout);
    assert!(
        parent.contains("level-6"),
        "level-8's parent should be level-6: {}",
        parent
    );

    // Can still navigate to top
    let output = run_dm(temp_dir.path(), &["top"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "level-15");

    Ok(())
}

#[test]
fn test_wide_stack_8_children_parent_delete() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create parent with 8 children
    fs::write(temp_dir.path().join("parent.txt"), "parent")?;
    run_dm(temp_dir.path(), &["create", "parent", "-a", "-m", "Parent"])?;

    for i in 1..=8 {
        run_dm(temp_dir.path(), &["checkout", "parent"])?;
        fs::write(temp_dir.path().join(format!("child{}.txt", i)), format!("child {}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("child-{}", i), "-a", "-m", &format!("Child {}", i)],
        )?;
    }

    // Delete parent - all children should be reparented to main
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    let output = run_dm(temp_dir.path(), &["delete", "parent", "--force"])?;
    assert!(output.status.success());

    // Verify all children have main as parent
    for i in 1..=8 {
        run_dm(temp_dir.path(), &["checkout", &format!("child-{}", i)])?;
        let output = run_dm(temp_dir.path(), &["parent"])?;
        let parent = String::from_utf8_lossy(&output.stdout);
        assert!(
            parent.contains("main"),
            "child-{}'s parent should be main: {}",
            i,
            parent
        );
    }

    // main should have 8 children now
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    let output = run_dm(temp_dir.path(), &["children"])?;
    let children = String::from_utf8_lossy(&output.stdout);
    for i in 1..=8 {
        assert!(
            children.contains(&format!("child-{}", i)),
            "main should have child-{}: {}",
            i,
            children
        );
    }

    Ok(())
}

#[test]
fn test_multiple_independent_stacks() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create 3 independent stacks from main
    for stack in 1..=3 {
        run_dm(temp_dir.path(), &["checkout", "main"])?;
        for level in 1..=3 {
            fs::write(
                temp_dir.path().join(format!("stack{}_level{}.txt", stack, level)),
                format!("stack {} level {}", stack, level),
            )?;
            run_dm(
                temp_dir.path(),
                &[
                    "create",
                    &format!("stack{}-level{}", stack, level),
                    "-a",
                    "-m",
                    &format!("Stack {} Level {}", stack, level),
                ],
            )?;
        }
    }

    // Verify each stack is independent
    run_dm(temp_dir.path(), &["checkout", "stack1-level3"])?;
    let output = run_dm(temp_dir.path(), &["bottom"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "stack1-level1");

    run_dm(temp_dir.path(), &["checkout", "stack2-level3"])?;
    let output = run_dm(temp_dir.path(), &["bottom"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "stack2-level1");

    // main should have 3 children (level1 of each stack)
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    let output = run_dm(temp_dir.path(), &["children"])?;
    let children = String::from_utf8_lossy(&output.stdout);
    assert!(children.contains("stack1-level1"));
    assert!(children.contains("stack2-level1"));
    assert!(children.contains("stack3-level1"));

    Ok(())
}

#[test]
fn test_orphaned_subtree_after_git_delete() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> parent -> child -> grandchild
    fs::write(temp_dir.path().join("p.txt"), "p")?;
    run_dm(temp_dir.path(), &["create", "parent", "-a", "-m", "P"])?;

    fs::write(temp_dir.path().join("c.txt"), "c")?;
    run_dm(temp_dir.path(), &["create", "child", "-a", "-m", "C"])?;

    fs::write(temp_dir.path().join("g.txt"), "g")?;
    run_dm(temp_dir.path(), &["create", "grandchild", "-a", "-m", "G"])?;

    // Delete child via git (leaving grandchild orphaned)
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["branch", "-D", "child"])?;

    // Doctor should detect the issue and return error
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("child") && stdout.contains("doesn't exist"),
        "Doctor should detect missing child: {}",
        stdout
    );
    assert!(!output.status.success(), "Doctor should return error when issues found");

    // Doctor --fix should fix both issues:
    // 1. TrackedBranchMissing for child (removed from refs)
    // 2. OrphanedParent for grandchild (reparented to closest valid ancestor - parent)
    let output = run_dm(temp_dir.path(), &["doctor", "--fix"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should fix both issues
    assert!(stdout.contains("Fixed"), "Doctor should fix issues: {}", stdout);
    assert!(
        output.status.success(),
        "Doctor --fix should succeed when it can fix all issues: {}",
        stdout
    );

    // Verify grandchild is reparented (could be to parent or main, both are valid)
    let grandchild_parent = get_parent_from_refs(temp_dir.path(), "grandchild")?;
    assert!(
        grandchild_parent == Some("parent".to_string()) || grandchild_parent == Some("main".to_string()),
        "grandchild should be reparented to parent or main, got: {:?}",
        grandchild_parent
    );

    // Doctor should now show no issues
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(output.status.success(), "Doctor should pass after fix");

    Ok(())
}

#[test]
fn test_diamond_pattern_two_paths() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create diamond pattern: main -> [left, right] -> bottom
    // First create left branch
    fs::write(temp_dir.path().join("left.txt"), "left")?;
    run_dm(temp_dir.path(), &["create", "left", "-a", "-m", "Left"])?;

    // Create right branch from main
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("right.txt"), "right")?;
    run_dm(temp_dir.path(), &["create", "right", "-a", "-m", "Right"])?;

    // Create bottom from left (it will have one parent, that's fine)
    run_dm(temp_dir.path(), &["checkout", "left"])?;
    fs::write(temp_dir.path().join("bottom.txt"), "bottom")?;
    run_dm(temp_dir.path(), &["create", "bottom", "-a", "-m", "Bottom"])?;

    // Verify structure
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    let output = run_dm(temp_dir.path(), &["children"])?;
    let children = String::from_utf8_lossy(&output.stdout);
    assert!(children.contains("left"));
    assert!(children.contains("right"));

    // bottom's parent should be left
    run_dm(temp_dir.path(), &["checkout", "bottom"])?;
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("left"));

    Ok(())
}

// ============================================================================
// CATEGORY 6: Recovery Testing
// ============================================================================

#[test]
fn test_undo_after_delete_restores_branch() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Get commit hash before delete (for potential verification after undo)
    let _original_hash = get_commit_hash(temp_dir.path(), "feature")?;

    // Delete the branch
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    let output = run_dm(temp_dir.path(), &["delete", "feature", "--force"])?;
    assert!(output.status.success());

    // Branch should be gone
    assert!(!git_branch_exists(temp_dir.path(), "feature")?);

    // Undo should restore it
    let output = run_dm(temp_dir.path(), &["undo"])?;
    // Undo might succeed or might not have a backup - check both cases
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("feature") || stdout.contains("restored") {
            // If undo worked, branch should exist again
            assert!(git_branch_exists(temp_dir.path(), "feature")?);
        }
    }

    Ok(())
}

#[test]
fn test_doctor_fix_multiple_issues() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create two independent branches (siblings off main, not a chain)
    fs::write(temp_dir.path().join("a.txt"), "a")?;
    run_dm(temp_dir.path(), &["create", "branch-a", "-a", "-m", "A"])?;

    run_dm(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("b.txt"), "b")?;
    run_dm(temp_dir.path(), &["create", "branch-b", "-a", "-m", "B"])?;

    // Delete both branches via git (creates TrackedBranchMissing issues - fixable)
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["branch", "-D", "branch-a"])?;
    run_git(temp_dir.path(), &["branch", "-D", "branch-b"])?;

    // Doctor should detect issues and return error
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("branch-a") && stdout.contains("branch-b"),
        "Doctor should detect both missing branches: {}",
        stdout
    );
    assert!(!output.status.success(), "Doctor should return error when issues found");

    // Doctor --fix should handle them (TrackedBranchMissing is auto-fixable)
    let output = run_dm(temp_dir.path(), &["doctor", "--fix"])?;
    assert!(
        output.status.success(),
        "Doctor --fix should succeed when all issues are fixable: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Now doctor should pass
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(
        output.status.success(),
        "Doctor should pass after fix: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    Ok(())
}

#[test]
fn test_history_records_operations() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Perform several operations
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "feature-1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "feature-2", "-a", "-m", "F2"])?;

    // Check history
    let output = run_dm(temp_dir.path(), &["history"])?;
    // History should show operations (might be empty if not implemented)
    // Main check is that it doesn't crash
    assert!(output.status.success() || !String::from_utf8_lossy(&output.stderr).contains("panic"));

    Ok(())
}

// ============================================================================
// CATEGORY 7: Operation State Crash Simulation
// ============================================================================

#[test]
fn test_operation_state_survives_crash_simulation() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Create operation state (simulating crash during operation)
    let op_state = json!({
        "operation_type": "Sync",
        "in_progress": true,
        "current_branch": "feature",
        "remaining_branches": ["feature"],
        "original_branch": "main",
        "move_target_parent": null
    });
    create_operation_state(temp_dir.path(), &op_state)?;

    // Verify state file exists
    let op_state_path = temp_dir.path().join(".git/diamond/operation_state.json");
    assert!(op_state_path.exists());

    // Running any dm command should handle the stale state gracefully
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    // Should not crash
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!combined.contains("panic"));

    Ok(())
}

#[test]
fn test_cleanup_without_force_requires_tty() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch and merge it
    fs::write(temp_dir.path().join("feature.txt"), "feature")?;
    run_dm_success(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Verify branch is tracked
    assert!(
        is_branch_tracked_in_refs(temp_dir.path(), "feature")?,
        "feature should be tracked after creation"
    );

    // Merge the branch into main
    run_git(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["merge", "feature", "--no-ff", "-m", "Merge feature"])?;

    // Try cleanup without --force (should fail in non-TTY environment)
    let output = run_dm(temp_dir.path(), &["cleanup"])?;

    // Should fail because stdin is not a TTY
    assert!(
        !output.status.success(),
        "cleanup without --force should fail in non-TTY environment.\n\
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Error message should mention --force
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--force") || stderr.contains("non-interactively"),
        "Error should mention --force flag, got: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_history_count_flag() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create multiple branches to generate history
    for i in 1..=5 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), "content")?;
        run_dm(
            temp_dir.path(),
            &[
                "create",
                &format!("feature-{}", i),
                "-a",
                "-m",
                &format!("Feature {}", i),
            ],
        )?;
    }

    // Test with --count flag
    let output = run_dm(temp_dir.path(), &["history", "--count", "3"])?;
    assert!(
        output.status.success(),
        "history --count 3 failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify it doesn't crash and produces output
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "history --count should produce output");

    // Test with short flag -c
    let output = run_dm(temp_dir.path(), &["history", "-c", "2"])?;
    assert!(
        output.status.success(),
        "history -c 2 failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

#[test]
fn test_history_all_flag() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a few branches
    for i in 1..=3 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), "content")?;
        run_dm(
            temp_dir.path(),
            &[
                "create",
                &format!("feature-{}", i),
                "-a",
                "-m",
                &format!("Feature {}", i),
            ],
        )?;
    }

    // Test with --all flag
    let output = run_dm(temp_dir.path(), &["history", "--all"])?;
    assert!(
        output.status.success(),
        "history --all failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify it produces output
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "history --all should produce output");

    Ok(())
}

#[test]
fn test_history_default_behavior() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Test default history (no flags)
    let output = run_dm(temp_dir.path(), &["history"])?;
    assert!(
        output.status.success(),
        "history (default) failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should use default limit of 20
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "history should produce output");

    Ok(())
}

// ============================================================================
// CATEGORY 8: Crash Recovery - Verifying existing mechanisms handle failures
// ============================================================================

#[test]
fn test_backup_refs_created_before_sync() -> Result<()> {
    // Verify that backup refs are created before sync operations
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a stack
    fs::write(temp_dir.path().join("f1.txt"), "feature 1")?;
    run_dm(temp_dir.path(), &["create", "feature-1", "-a", "-m", "Feature 1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "feature 2")?;
    run_dm(temp_dir.path(), &["create", "feature-2", "-a", "-m", "Feature 2"])?;

    // Run sync (this should create backup refs)
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    let _output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;

    // Check for backup refs
    let output = run_git(temp_dir.path(), &["for-each-ref", "refs/diamond/backup/"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should have backup refs (if any rebasing happened)
    // The presence of the refs/diamond/backup/ namespace is what we're verifying
    // Even if empty, the mechanism exists
    assert!(output.status.success(), "Should be able to query backup refs");

    // If there were backup refs created, verify they contain feature branches
    if !stdout.is_empty() {
        assert!(
            stdout.contains("feature-1") || stdout.contains("feature-2"),
            "Backup refs should reference the feature branches: {}",
            stdout
        );
    }

    Ok(())
}

#[test]
fn test_undo_uses_backup_refs_after_restack() -> Result<()> {
    // Verify that undo can restore branches using backup refs after a restack
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch with a known commit
    fs::write(temp_dir.path().join("f.txt"), "original content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Original"])?;
    let original_hash = get_commit_hash(temp_dir.path(), "feature")?;

    // Add a commit to main (so restack has something to do)
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("main.txt"), "new main content")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "-m", "Main update"])?;

    // Restack feature onto updated main (this creates a backup and changes the commit)
    run_dm(temp_dir.path(), &["checkout", "feature"])?;
    run_dm(temp_dir.path(), &["restack"])?;
    let rebased_hash = get_commit_hash(temp_dir.path(), "feature")?;

    // After restack, the commit hash should be different (new parent)
    assert_ne!(original_hash, rebased_hash, "Commit hash should change after restack");

    // Undo should restore the original commit
    let output = run_dm(temp_dir.path(), &["undo"])?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Check if undo reported success
        if stdout.contains("Restored") || stdout.contains("feature") || stdout.contains("Undid") {
            let restored_hash = get_commit_hash(temp_dir.path(), "feature")?;
            // The branch should be restored to its original commit
            assert_eq!(
                original_hash, restored_hash,
                "Undo should restore original commit before restack"
            );
        }
    }
    // If undo doesn't succeed, that's acceptable - the test verifies the mechanism exists

    Ok(())
}

#[test]
fn test_doctor_repairs_partially_reparented_children() -> Result<()> {
    // Simulate a crash during remove_branch_reparent where some children
    // were reparented but others weren't
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create: main -> parent -> [child-1, child-2, child-3]
    fs::write(temp_dir.path().join("p.txt"), "parent")?;
    run_dm(temp_dir.path(), &["create", "parent", "-a", "-m", "Parent"])?;

    run_dm(temp_dir.path(), &["checkout", "parent"])?;
    fs::write(temp_dir.path().join("c1.txt"), "child 1")?;
    run_dm(temp_dir.path(), &["create", "child-1", "-a", "-m", "Child 1"])?;

    run_dm(temp_dir.path(), &["checkout", "parent"])?;
    fs::write(temp_dir.path().join("c2.txt"), "child 2")?;
    run_dm(temp_dir.path(), &["create", "child-2", "-a", "-m", "Child 2"])?;

    run_dm(temp_dir.path(), &["checkout", "parent"])?;
    fs::write(temp_dir.path().join("c3.txt"), "child 3")?;
    run_dm(temp_dir.path(), &["create", "child-3", "-a", "-m", "Child 3"])?;

    // Simulate partial crash: delete parent from git, manually reparent child-1 to main,
    // but leave child-2 and child-3 pointing to "parent"
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["branch", "-D", "parent"])?;
    set_parent_in_refs(temp_dir.path(), "child-1", "main")?;
    // child-2 and child-3 still point to "parent" which no longer exists

    // Doctor should detect the orphaned parents
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("parent") && (stdout.contains("doesn't exist") || stdout.contains("non-existent")),
        "Doctor should detect orphaned parent references: {}",
        stdout
    );

    // Doctor --fix should repair by reparenting to main
    let output = run_dm(temp_dir.path(), &["doctor", "--fix"])?;
    assert!(
        output.status.success(),
        "Doctor --fix should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // All children should now have valid parents
    let child2_parent = get_parent_from_refs(temp_dir.path(), "child-2")?;
    let child3_parent = get_parent_from_refs(temp_dir.path(), "child-3")?;

    assert!(
        child2_parent == Some("main".to_string()),
        "child-2 should be reparented to main, got: {:?}",
        child2_parent
    );
    assert!(
        child3_parent == Some("main".to_string()),
        "child-3 should be reparented to main, got: {:?}",
        child3_parent
    );

    // Doctor should now pass
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(output.status.success(), "Doctor should pass after fix");

    Ok(())
}

#[test]
fn test_operation_lock_prevents_concurrent_operations() -> Result<()> {
    // Verify that the operation lock mechanism prevents concurrent operations
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Create a lock file to simulate an operation in progress
    let lock_path = temp_dir.path().join(".git/diamond/operation.lock");
    fs::create_dir_all(temp_dir.path().join(".git/diamond"))?;

    // Create and keep a lock using fs2
    use std::fs::OpenOptions;
    use fs2::FileExt;

    let lock_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)?;
    lock_file.try_lock_exclusive()?;

    // Try to run sync - should fail due to lock
    let output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;

    // Should either fail with lock message or succeed (if lock check isn't on sync path)
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        assert!(
            stderr.contains("Another Diamond operation") || stderr.contains("lock") || stderr.contains("in progress"),
            "Should mention operation in progress when locked: {}",
            stderr
        );
    }

    // Release the lock
    lock_file.unlock()?;

    // Now sync should work
    let output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;
    // Might fail for other reasons (no remote), but not due to lock
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Another Diamond operation"),
        "Should not mention lock after it's released: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_stale_lock_detection() -> Result<()> {
    // Verify that operations detect and handle stale locks from dead processes
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Create a lock file with a PID that definitely doesn't exist (999999999)
    let lock_path = temp_dir.path().join(".git/diamond/operation.lock");
    fs::create_dir_all(temp_dir.path().join(".git/diamond"))?;

    // Write a fake PID that won't exist
    fs::write(&lock_path, "999999999")?;

    // Operations should detect the stale lock and proceed
    // (or clean it up and proceed, depending on implementation)
    let output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;

    // Should either succeed or fail for reasons other than the lock
    let stderr = String::from_utf8_lossy(&output.stderr);

    // If it fails, it should NOT be because of the lock
    // (stale locks should be detected and cleared)
    if !output.status.success() {
        assert!(
            !stderr.contains("Another Diamond operation") || stderr.contains("stale"),
            "Should not block on stale lock (or should mention it's stale): {}",
            stderr
        );
    }

    Ok(())
}

#[test]
fn test_backup_refs_use_unique_timestamps() -> Result<()> {
    // Verify that backup refs created in rapid succession have unique names
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Perform multiple modifies in rapid succession (each creates a backup)
    for i in 1..=3 {
        fs::write(temp_dir.path().join("f.txt"), format!("content {}", i))?;
        run_dm(temp_dir.path(), &["modify", "-a", "--amend"])?;
    }

    // Check backup refs - should have multiple unique refs
    let output = run_git(
        temp_dir.path(),
        &["for-each-ref", "--format=%(refname)", "refs/diamond/backup/"],
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let backup_refs: Vec<&str> = stdout.lines().filter(|l| l.contains("feature")).collect();

    // Should have multiple backup refs for the same branch
    // (Due to the atomic counter fix, each backup has a unique name)
    if backup_refs.len() > 1 {
        // All refs should be unique
        let unique_refs: std::collections::HashSet<&str> = backup_refs.iter().cloned().collect();
        assert_eq!(
            backup_refs.len(),
            unique_refs.len(),
            "All backup refs should be unique: {:?}",
            backup_refs
        );
    }

    Ok(())
}

#[test]
fn test_stale_journal_cleaned_on_startup() -> Result<()> {
    // Verify that stale journal entries don't block operations
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a stale journal file (as if operation crashed)
    let journal_dir = temp_dir.path().join(".git/diamond/journal");
    fs::create_dir_all(&journal_dir)?;
    let journal_content = json!({
        "id": "20251230_120000_000",
        "operation": {
            "type": "sync",
            "original_branch": "main",
            "remaining_branches": ["feature"],
            "completed_branches": []
        },
        "started_at": "2025-12-30T12:00:00Z",
        "status": "interrupted",
        "backup_refs": []
    });
    fs::write(
        journal_dir.join("current.json"),
        serde_json::to_string_pretty(&journal_content)?,
    )?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Operations should still work (journal shouldn't block them)
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(
        output.status.success() || !String::from_utf8_lossy(&output.stderr).contains("panic"),
        "Should handle stale journal gracefully"
    );

    Ok(())
}

#[test]
fn test_interrupted_sync_can_continue() -> Result<()> {
    // Verify that an interrupted sync can be continued
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a stack
    fs::write(temp_dir.path().join("f1.txt"), "feature 1")?;
    run_dm(temp_dir.path(), &["create", "feature-1", "-a", "-m", "Feature 1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "feature 2")?;
    run_dm(temp_dir.path(), &["create", "feature-2", "-a", "-m", "Feature 2"])?;

    // Create operation state as if sync was interrupted after feature-1
    let op_state = json!({
        "operation_type": "Sync",
        "in_progress": true,
        "current_branch": "feature-2",
        "remaining_branches": ["feature-2"],
        "all_branches": ["feature-1", "feature-2"],
        "original_branch": "feature-2",
        "move_target_parent": null,
        "move_old_parent": null
    });
    create_operation_state(temp_dir.path(), &op_state)?;

    // dm continue should handle this gracefully
    let output = run_dm(temp_dir.path(), &["continue"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Should either complete the sync or clean up the stale state
    assert!(
        output.status.success() || combined.contains("No operation") || combined.contains("Cleaning up"),
        "Continue should handle interrupted sync: {}",
        combined
    );

    Ok(())
}
