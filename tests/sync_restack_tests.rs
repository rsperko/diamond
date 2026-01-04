mod common;

use anyhow::Result;
use common::*;
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

// ============================================================================
// RESTACK TESTS
// ============================================================================

#[test]
fn test_restack_after_parent_amend() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1 original")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Get f2's original commit hash
    let original_f2_hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let original_f2_hash = String::from_utf8_lossy(&original_f2_hash.stdout).trim().to_string();

    // Go back to f1 and amend
    run_dm(temp_dir.path(), &["checkout", "f1"])?;
    fs::write(temp_dir.path().join("f1.txt"), "f1 amended content")?;
    run_dm(temp_dir.path(), &["modify", "-a"])?;

    // Restack should have been automatic, but let's verify f2 was rebased
    run_dm(temp_dir.path(), &["checkout", "f2"])?;
    let new_f2_hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let new_f2_hash = String::from_utf8_lossy(&new_f2_hash.stdout).trim().to_string();

    // f2's hash should have changed (was rebased)
    assert_ne!(
        original_f2_hash, new_f2_hash,
        "f2 should have been rebased after f1 amend"
    );

    // f2 should still have f1's amended changes in its history
    let output = Command::new("git")
        .args(["show", "HEAD~1:f1.txt"])
        .current_dir(temp_dir.path())
        .output()?;
    let f1_content = String::from_utf8_lossy(&output.stdout);
    assert!(
        f1_content.contains("amended"),
        "f2 should be based on amended f1: {}",
        f1_content
    );

    Ok(())
}

#[test]
fn test_restack_only_current_branch() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> f1 -> f2 -> f3
    for i in 1..=3 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("{}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("f{}", i), "-a", "-m", &format!("F{}", i)],
        )?;
    }

    // Modify f1 via git (simulating divergence)
    run_git(temp_dir.path(), &["checkout", "f1"])?;
    fs::write(temp_dir.path().join("f1_extra.txt"), "extra")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "--amend", "--no-edit"])?;

    let f2_before = get_commit_hash(temp_dir.path(), "f2")?;
    let f3_before = get_commit_hash(temp_dir.path(), "f3")?;

    // Restack only f2
    run_dm(temp_dir.path(), &["checkout", "f2"])?;
    let output = run_dm(temp_dir.path(), &["restack", "--only"])?;
    assert!(output.status.success());

    let f2_after = get_commit_hash(temp_dir.path(), "f2")?;
    let f3_after = get_commit_hash(temp_dir.path(), "f3")?;

    // f2 should have changed (restacked onto new f1)
    assert_ne!(f2_before, f2_after, "f2 should be restacked");

    // f3 should NOT have changed (--only flag)
    assert_eq!(f3_before, f3_after, "f3 should NOT be restacked with --only");

    Ok(())
}

#[test]
fn test_restack_downstack() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> f1 -> f2 -> f3
    for i in 1..=3 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("{}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("f{}", i), "-a", "-m", &format!("F{}", i)],
        )?;
    }

    // Add commit to main
    run_git(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("main_update.txt"), "update")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "-m", "Main update"])?;

    let f1_before = get_commit_hash(temp_dir.path(), "f1")?;

    // Restack downstack from f3
    run_dm(temp_dir.path(), &["checkout", "f3"])?;
    let output = run_dm(temp_dir.path(), &["restack", "--downstack"])?;
    assert!(output.status.success());

    let f1_after = get_commit_hash(temp_dir.path(), "f1")?;

    // f1 should have been rebased onto updated main
    assert_ne!(f1_before, f1_after, "f1 should be restacked onto main");

    Ok(())
}

#[test]
fn test_restack_specific_branch() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Modify f1 via git
    run_git(temp_dir.path(), &["checkout", "f1"])?;
    fs::write(temp_dir.path().join("f1_extra.txt"), "extra")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "--amend", "--no-edit"])?;

    let f2_before = get_commit_hash(temp_dir.path(), "f2")?;

    // Stay on f1 but restack f2 specifically
    let output = run_dm(temp_dir.path(), &["restack", "-b", "f2"])?;
    assert!(output.status.success());

    let f2_after = get_commit_hash(temp_dir.path(), "f2")?;
    assert_ne!(f2_before, f2_after, "f2 should be restacked");

    Ok(())
}

// ============================================================================
// SYNC TESTS
// ============================================================================

#[test]
fn test_sync_rebases_stack_onto_updated_main() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a stack: main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Get original commit hashes
    let original_f1 = get_commit_hash(temp_dir.path(), "f1")?;
    let original_f2 = get_commit_hash(temp_dir.path(), "f2")?;

    // Add a commit to main (simulating remote updates)
    run_git(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("main_update.txt"), "main update")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "-m", "Main update"])?;

    // Go back to f2 and run sync
    run_dm(temp_dir.path(), &["checkout", "f2"])?;
    let output = run_dm(temp_dir.path(), &["sync"])?;
    assert!(
        output.status.success(),
        "sync failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Both branches should have been rebased (new commit hashes)
    let new_f1 = get_commit_hash(temp_dir.path(), "f1")?;
    let new_f2 = get_commit_hash(temp_dir.path(), "f2")?;

    assert_ne!(original_f1, new_f1, "f1 should have been rebased onto updated main");
    assert_ne!(original_f2, new_f2, "f2 should have been rebased onto updated main");

    // Verify main_update.txt is in f1's history
    run_dm(temp_dir.path(), &["checkout", "f1"])?;
    let output = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(temp_dir.path())
        .output()?;
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(log.contains("Main update"), "f1 should include main update: {}", log);

    Ok(())
}

#[test]
fn test_sync_abort_stops_operation() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Create operation state as if sync is in progress
    let op_state = json!({
        "operation_type": "Sync",
        "in_progress": true,
        "current_branch": "feature",
        "remaining_branches": ["feature"],
        "original_branch": "main",
        "move_target_parent": null
    });
    create_operation_state(temp_dir.path(), &op_state)?;

    // Abort should clear the operation
    let output = run_dm(temp_dir.path(), &["sync", "--abort"])?;
    assert!(
        output.status.success(),
        "sync --abort failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Operation state should be cleared
    let op_state_path = temp_dir.path().join(".git/diamond/operation_state.json");
    if op_state_path.exists() {
        let content = fs::read_to_string(&op_state_path)?;
        let state: serde_json::Value = serde_json::from_str(&content)?;
        assert!(
            !state["in_progress"].as_bool().unwrap_or(false),
            "Operation should be cleared after abort"
        );
    }

    Ok(())
}

#[test]
fn test_sync_with_no_updates_is_noop() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a simple stack
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    let original_hash = get_commit_hash(temp_dir.path(), "feature")?;

    // Sync when there are no updates
    let output = run_dm(temp_dir.path(), &["sync"])?;
    assert!(output.status.success());

    // Hash should be the same (no rebase needed)
    let new_hash = get_commit_hash(temp_dir.path(), "feature")?;
    assert_eq!(original_hash, new_hash, "No rebase needed when main hasn't changed");

    Ok(())
}

#[test]
fn test_sync_multiple_independent_stacks() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create two independent stacks from main
    fs::write(temp_dir.path().join("stack1.txt"), "stack1")?;
    run_dm(temp_dir.path(), &["create", "stack1", "-a", "-m", "Stack1"])?;

    run_dm(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("stack2.txt"), "stack2")?;
    run_dm(temp_dir.path(), &["create", "stack2", "-a", "-m", "Stack2"])?;

    // Update main
    run_git(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("main_new.txt"), "new")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "-m", "Main new"])?;

    // Sync from stack2
    run_dm(temp_dir.path(), &["checkout", "stack2"])?;
    let output = run_dm(temp_dir.path(), &["sync"])?;
    assert!(output.status.success());

    // Both stacks should be rebased
    run_dm(temp_dir.path(), &["checkout", "stack1"])?;
    let output = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(temp_dir.path())
        .output()?;
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(log.contains("Main new"), "stack1 should include main update");

    run_dm(temp_dir.path(), &["checkout", "stack2"])?;
    let output = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(temp_dir.path())
        .output()?;
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(log.contains("Main new"), "stack2 should include main update");

    Ok(())
}

#[test]
fn test_sync_then_cleanup_workflow() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Merge f1 into main (simulating PR merge)
    run_git(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["merge", "f1", "--no-ff", "-m", "Merge f1"])?;

    // Sync should update stacks
    run_dm(temp_dir.path(), &["checkout", "f2"])?;
    run_dm(temp_dir.path(), &["sync"])?;

    // Cleanup should detect and remove merged branch
    let output = run_dm(temp_dir.path(), &["cleanup", "--force"])?;
    assert!(output.status.success());

    // f1 should be deleted
    assert!(!git_branch_exists(temp_dir.path(), "f1")?);

    // f2 should still exist and have main as parent
    assert!(git_branch_exists(temp_dir.path(), "f2")?);
    run_dm(temp_dir.path(), &["checkout", "f2"])?;
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("main"));

    Ok(())
}

// ============================================================================
// CLEANUP TESTS
// ============================================================================

#[test]
fn test_cleanup_merged_branch_restacks_children() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Get f2's original commit hash
    let original_f2_hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let original_f2_hash = String::from_utf8_lossy(&original_f2_hash.stdout).trim().to_string();

    // Merge f1 into main (simulating PR merge)
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(temp_dir.path())
        .output()?;
    Command::new("git")
        .args(["merge", "f1", "--no-ff", "-m", "Merge f1"])
        .current_dir(temp_dir.path())
        .output()?;

    // Run cleanup with --force
    let output = run_dm(temp_dir.path(), &["cleanup", "--force"])?;
    assert!(
        output.status.success(),
        "dm cleanup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify f1 is deleted
    let output = Command::new("git")
        .args(["branch", "--list", "f1"])
        .current_dir(temp_dir.path())
        .output()?;
    assert!(
        String::from_utf8_lossy(&output.stdout).trim().is_empty(),
        "f1 should be deleted"
    );

    // f2 should have been restacked - its commit hash should change
    let new_f2_hash = Command::new("git")
        .args(["rev-parse", "f2"])
        .current_dir(temp_dir.path())
        .output()?;
    let new_f2_hash = String::from_utf8_lossy(&new_f2_hash.stdout).trim().to_string();

    assert_ne!(
        original_f2_hash, new_f2_hash,
        "f2 should have been restacked (commit hash changed)"
    );

    // f2's parent should now be main
    run_dm(temp_dir.path(), &["checkout", "f2"])?;
    let output = run_dm(temp_dir.path(), &["parent"])?;
    let parent = String::from_utf8_lossy(&output.stdout);
    assert!(parent.contains("main"), "f2's parent should be main: {}", parent);

    Ok(())
}

#[test]
fn test_cleanup_multiple_merged_with_children() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> [f1a, f1b]
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f1a.txt"), "f1a")?;
    run_dm(temp_dir.path(), &["create", "f1a", "-a", "-m", "F1A"])?;

    run_dm(temp_dir.path(), &["checkout", "f1"])?;
    fs::write(temp_dir.path().join("f1b.txt"), "f1b")?;
    run_dm(temp_dir.path(), &["create", "f1b", "-a", "-m", "F1B"])?;

    // Merge f1 into main
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(temp_dir.path())
        .output()?;
    Command::new("git")
        .args(["merge", "f1", "--no-ff", "-m", "Merge f1"])
        .current_dir(temp_dir.path())
        .output()?;

    // Run cleanup
    let output = run_dm(temp_dir.path(), &["cleanup", "--force"])?;
    assert!(output.status.success());

    // Verify f1 is deleted
    let output = Command::new("git")
        .args(["branch", "--list", "f1"])
        .current_dir(temp_dir.path())
        .output()?;
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    // Both f1a and f1b should have main as parent now
    run_dm(temp_dir.path(), &["checkout", "f1a"])?;
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("main"));

    run_dm(temp_dir.path(), &["checkout", "f1b"])?;
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("main"));

    // main should have both f1a and f1b as children
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    let output = run_dm(temp_dir.path(), &["children"])?;
    let children = String::from_utf8_lossy(&output.stdout);
    assert!(children.contains("f1a"), "Should have f1a child: {}", children);
    assert!(children.contains("f1b"), "Should have f1b child: {}", children);

    Ok(())
}

#[test]
fn test_cleanup_middle_of_stack_merged() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> f2 -> f3
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    fs::write(temp_dir.path().join("f3.txt"), "f3")?;
    run_dm(temp_dir.path(), &["create", "f3", "-a", "-m", "F3"])?;

    // Merge f2 into main (skipping f1!)
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(temp_dir.path())
        .output()?;
    Command::new("git")
        .args(["merge", "f2", "--no-ff", "-m", "Merge f2"])
        .current_dir(temp_dir.path())
        .output()?;

    // Run cleanup - should clean up f2 and f1 (both are now merged)
    let output = run_dm(temp_dir.path(), &["cleanup", "--force"])?;
    assert!(output.status.success());

    // f3 should be reparented to main (since f1 and f2 are cleaned)
    run_dm(temp_dir.path(), &["checkout", "f3"])?;
    let output = run_dm(temp_dir.path(), &["parent"])?;
    let parent = String::from_utf8_lossy(&output.stdout);
    // f3's parent should be main or f1 depending on what got cleaned
    // Since we merged f2 into main, both f1 and f2 should show as merged
    assert!(
        parent.contains("main") || parent.contains("f1"),
        "f3's parent should be main or f1: {}",
        parent
    );

    Ok(())
}

// ============================================================================
// MID-STACK MODIFICATION AUTO-RESTACK TESTS
// ============================================================================

#[test]
fn test_modify_mid_stack_auto_restacks_all_descendants() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a 5-level stack
    for i in 1..=5 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("f{}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("f{}", i), "-a", "-m", &format!("F{}", i)],
        )?;
    }

    // Record original hashes for f3, f4, f5
    let original_f3 = get_commit_hash(temp_dir.path(), "f3")?;
    let original_f4 = get_commit_hash(temp_dir.path(), "f4")?;
    let original_f5 = get_commit_hash(temp_dir.path(), "f5")?;

    // Go to f2 and modify it (this should auto-restack f3, f4, f5)
    run_dm(temp_dir.path(), &["checkout", "f2"])?;
    fs::write(temp_dir.path().join("f2_extra.txt"), "extra content")?;
    let output = run_dm(temp_dir.path(), &["modify", "-a"])?;
    assert!(
        output.status.success(),
        "modify failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // All descendants should have new commit hashes (were rebased)
    let new_f3 = get_commit_hash(temp_dir.path(), "f3")?;
    let new_f4 = get_commit_hash(temp_dir.path(), "f4")?;
    let new_f5 = get_commit_hash(temp_dir.path(), "f5")?;

    assert_ne!(original_f3, new_f3, "f3 should have been restacked after f2 modify");
    assert_ne!(original_f4, new_f4, "f4 should have been restacked after f2 modify");
    assert_ne!(original_f5, new_f5, "f5 should have been restacked after f2 modify");

    // Verify f5 still has all the expected files in its history
    run_dm(temp_dir.path(), &["checkout", "f5"])?;
    let output = Command::new("git")
        .args(["ls-tree", "-r", "--name-only", "HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let files = String::from_utf8_lossy(&output.stdout);
    assert!(
        files.contains("f2_extra.txt"),
        "f5 should include f2's new file: {}",
        files
    );

    Ok(())
}

#[test]
fn test_modify_bottom_of_stack_restacks_entire_stack() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> f1 -> f2 -> f3
    for i in 1..=3 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("{}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("f{}", i), "-a", "-m", &format!("F{}", i)],
        )?;
    }

    let original_f2 = get_commit_hash(temp_dir.path(), "f2")?;
    let original_f3 = get_commit_hash(temp_dir.path(), "f3")?;

    // Modify f1 (bottom of stack, should restack f2 and f3)
    run_dm(temp_dir.path(), &["checkout", "f1"])?;
    fs::write(temp_dir.path().join("f1_new.txt"), "new")?;
    run_dm(temp_dir.path(), &["modify", "-a"])?;

    let new_f2 = get_commit_hash(temp_dir.path(), "f2")?;
    let new_f3 = get_commit_hash(temp_dir.path(), "f3")?;

    assert_ne!(original_f2, new_f2, "f2 should be restacked");
    assert_ne!(original_f3, new_f3, "f3 should be restacked");

    Ok(())
}

// ============================================================================
// PARTIAL FAILURE RECOVERY TESTS
// ============================================================================

#[test]
fn test_sync_fails_midway_preserves_partial_progress() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a stack of 5 branches: main -> b1 -> b2 -> b3 -> b4 -> b5
    // Each branch modifies a DIFFERENT file to avoid conflicts between branches
    for i in 1..=5 {
        let filename = format!("branch{}.txt", i);
        fs::write(temp_dir.path().join(&filename), format!("content {}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("b{}", i), "-a", "-m", &format!("Branch {}", i)],
        )?;
    }

    // Record original commit hashes
    let original_hashes: Vec<String> = (1..=5)
        .map(|i| get_commit_hash(temp_dir.path(), &format!("b{}", i)).unwrap())
        .collect();

    // Update main with a commit that will conflict with b3
    run_git(temp_dir.path(), &["checkout", "main"])?;
    // Create a file that conflicts with branch3.txt
    create_file_and_commit(
        temp_dir.path(),
        "branch3.txt",
        "conflicting content from main",
        "Main update that conflicts with b3",
    )?;

    // Start sync from b5 - it should fail when it reaches b3
    run_dm(temp_dir.path(), &["checkout", "b5"])?;
    let _output = run_dm(temp_dir.path(), &["sync"])?;

    // The command may succeed (conflicts are not errors, just pause the operation)
    // Check that a rebase is in progress at b3
    assert!(
        git_rebase_in_progress(temp_dir.path())?,
        "Should have a rebase in progress"
    );

    // Verify operation state exists and shows remaining branches
    let state = get_operation_state(temp_dir.path())?;
    assert!(state.is_some(), "Operation state should exist");
    let state = state.unwrap();
    assert!(
        state["in_progress"].as_bool().unwrap_or(false),
        "Operation should be in progress"
    );

    // b1 and b2 should have been rebased (different commit hashes)
    let new_b1 = get_commit_hash(temp_dir.path(), "b1")?;
    let new_b2 = get_commit_hash(temp_dir.path(), "b2")?;
    assert_ne!(
        original_hashes[0], new_b1,
        "b1 should have been rebased onto updated main"
    );
    assert_ne!(original_hashes[1], new_b2, "b2 should have been rebased onto b1");

    // b4 and b5 should be unchanged (still at original hashes)
    let current_b4 = get_commit_hash(temp_dir.path(), "b4")?;
    let current_b5 = get_commit_hash(temp_dir.path(), "b5")?;
    assert_eq!(original_hashes[3], current_b4, "b4 should NOT have been rebased yet");
    assert_eq!(original_hashes[4], current_b5, "b5 should NOT have been rebased yet");

    // Cleanup: abort the rebase
    run_git(temp_dir.path(), &["rebase", "--abort"])?;

    Ok(())
}

#[test]
fn test_sync_continue_after_conflict_completes_remaining_branches() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a stack: main -> b1 -> b2
    // b1 adds a unique file, b2 adds shared.txt which will conflict with main
    fs::write(temp_dir.path().join("branch1.txt"), "content 1")?;
    run_dm(temp_dir.path(), &["create", "b1", "-a", "-m", "Branch 1"])?;

    fs::write(temp_dir.path().join("shared.txt"), "from branch")?;
    run_dm(temp_dir.path(), &["create", "b2", "-a", "-m", "Branch 2"])?;

    // Update main with a conflicting change on shared.txt
    // This will only conflict at b2 (the leaf), so no cascade
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "shared.txt", "from main", "Main update")?;

    // Sync from b2
    run_dm(temp_dir.path(), &["checkout", "b2"])?;
    run_dm(temp_dir.path(), &["sync"])?;

    // b1 should rebase cleanly, but b2 should conflict on shared.txt
    assert!(
        git_rebase_in_progress(temp_dir.path())?,
        "Should have rebase in progress at b2"
    );

    // Resolve the conflict
    fs::write(temp_dir.path().join("shared.txt"), "resolved")?;
    run_git(temp_dir.path(), &["add", "shared.txt"])?;

    // Continue the sync
    let output = run_dm(temp_dir.path(), &["continue"])?;
    assert!(
        output.status.success(),
        "continue should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // No rebase should be in progress now (b2 was the leaf, no more branches)
    assert!(!git_rebase_in_progress(temp_dir.path())?, "Rebase should be complete");

    // Operation state should be cleared
    let state = get_operation_state(temp_dir.path())?;
    if let Some(state) = state {
        assert!(
            !state["in_progress"].as_bool().unwrap_or(true),
            "Operation should not be in progress after completion"
        );
    }

    // Verify both branches include the main update
    for branch in &["b1", "b2"] {
        run_dm(temp_dir.path(), &["checkout", branch])?;
        let output = run_git(temp_dir.path(), &["log", "--oneline"])?;
        let log = String::from_utf8_lossy(&output.stdout);
        assert!(
            log.contains("Main update"),
            "{} should include main update: {}",
            branch,
            log
        );
    }

    Ok(())
}

#[test]
fn test_sync_abort_after_partial_progress_restores_branches() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a stack: main -> b1 -> b2 -> b3
    for i in 1..=3 {
        let filename = format!("branch{}.txt", i);
        fs::write(temp_dir.path().join(&filename), format!("content {}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("b{}", i), "-a", "-m", &format!("Branch {}", i)],
        )?;
    }

    // Record original commit hashes
    let original_b1 = get_commit_hash(temp_dir.path(), "b1")?;

    // Update main to conflict with b2
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "branch2.txt", "conflict from main", "Main update")?;

    // Sync from b3 - will fail at b2
    run_dm(temp_dir.path(), &["checkout", "b3"])?;
    run_dm(temp_dir.path(), &["sync"])?;

    // Verify we're in a conflict state
    assert!(git_rebase_in_progress(temp_dir.path())?);

    // b1 was already rebased before the conflict
    let current_b1 = get_commit_hash(temp_dir.path(), "b1")?;
    assert_ne!(original_b1, current_b1, "b1 should have been rebased");

    // Abort the sync
    let output = run_dm(temp_dir.path(), &["abort"])?;
    assert!(
        output.status.success(),
        "abort failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Rebase should no longer be in progress
    assert!(!git_rebase_in_progress(temp_dir.path())?);

    // Operation state should be cleared
    let state = get_operation_state(temp_dir.path())?;
    if let Some(state) = state {
        assert!(
            !state["in_progress"].as_bool().unwrap_or(true),
            "Operation state should be cleared after abort"
        );
    }

    // NOTE: Diamond uses backup refs to restore. Let's check b3 is accessible
    // The branches should still exist
    assert!(git_branch_exists(temp_dir.path(), "b1")?, "b1 should still exist");
    assert!(git_branch_exists(temp_dir.path(), "b2")?, "b2 should still exist");
    assert!(git_branch_exists(temp_dir.path(), "b3")?, "b3 should still exist");

    Ok(())
}

#[test]
fn test_restack_fails_midway_state_is_consistent() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> b1 -> b2 -> b3 -> b4
    for i in 1..=4 {
        let filename = format!("feature{}.txt", i);
        fs::write(temp_dir.path().join(&filename), format!("feature {}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("b{}", i), "-a", "-m", &format!("Feature {}", i)],
        )?;
    }

    // Modify b1 to create a change that will conflict with b3 during restack
    run_dm(temp_dir.path(), &["checkout", "b1"])?;
    fs::write(temp_dir.path().join("feature3.txt"), "conflict from b1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "B1 modifies f3"])?;

    // Now modify b3 with the same file
    run_dm(temp_dir.path(), &["checkout", "b3"])?;
    fs::write(temp_dir.path().join("feature3.txt"), "original b3 content")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "B3 has f3"])?;

    // Record original hashes
    let original_b4 = get_commit_hash(temp_dir.path(), "b4")?;

    // Trigger restack from b1 - this should rebase b2, then conflict on b3
    run_dm(temp_dir.path(), &["checkout", "b1"])?;
    let _output = run_dm(temp_dir.path(), &["restack"])?;

    // Check if we have a conflict (rebase in progress)
    if git_rebase_in_progress(temp_dir.path())? {
        // Verify operation state
        let state = get_operation_state(temp_dir.path())?;
        assert!(state.is_some(), "Operation state should exist during conflict");

        // b4 should be unchanged since we haven't gotten to it yet
        let current_b4 = get_commit_hash(temp_dir.path(), "b4")?;
        assert_eq!(
            original_b4, current_b4,
            "b4 should not be modified during partial restack"
        );

        // Cleanup
        run_git(temp_dir.path(), &["rebase", "--abort"])?;
    }

    // Branches should still exist
    for i in 1..=4 {
        assert!(
            git_branch_exists(temp_dir.path(), &format!("b{}", i))?,
            "b{} should still exist",
            i
        );
    }

    Ok(())
}

#[test]
fn test_delete_mid_stack_restacks_children_atomically() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> b1 -> b2 -> b3
    for i in 1..=3 {
        let filename = format!("del{}.txt", i);
        fs::write(temp_dir.path().join(&filename), format!("delete test {}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("b{}", i), "-a", "-m", &format!("Del test {}", i)],
        )?;
    }

    // Verify stack structure before using refs
    assert_eq!(
        get_parent_from_refs(temp_dir.path(), "b2")?,
        Some("b1".to_string()),
        "b2 parent should be b1"
    );
    assert_eq!(
        get_parent_from_refs(temp_dir.path(), "b3")?,
        Some("b2".to_string()),
        "b3 parent should be b2"
    );

    // Delete b2 (middle of stack) - requires --force since not merged
    run_dm(temp_dir.path(), &["checkout", "b1"])?;
    let output = run_dm(temp_dir.path(), &["delete", "b2", "--force"])?;
    assert!(
        output.status.success(),
        "delete should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify refs are updated atomically:
    // - b2 should not be tracked
    // - b3 should now have b1 as parent
    assert!(
        !is_branch_tracked_in_refs(temp_dir.path(), "b2")?,
        "b2 should be removed from refs"
    );
    assert_eq!(
        get_parent_from_refs(temp_dir.path(), "b3")?,
        Some("b1".to_string()),
        "b3 parent should now be b1"
    );

    // Verify git state matches metadata
    assert!(!git_branch_exists(temp_dir.path(), "b2")?, "b2 should not exist in git");
    assert!(git_branch_exists(temp_dir.path(), "b3")?, "b3 should still exist");
    assert!(git_branch_exists(temp_dir.path(), "b1")?, "b1 should still exist");

    Ok(())
}

#[test]
fn test_merge_order_child_before_parent() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Merge f2 into main FIRST (out of order - child before parent)
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(temp_dir.path())
        .output()?;
    Command::new("git")
        .args(["merge", "f2", "--no-ff", "-m", "Merge f2"])
        .current_dir(temp_dir.path())
        .output()?;

    // Run cleanup - both f1 and f2 should be detected as merged
    // (f2 was explicitly merged, f1's changes are in main through f2)
    let output = run_dm(temp_dir.path(), &["cleanup", "--force"])?;
    assert!(output.status.success());

    // Both branches should be deleted
    let output = Command::new("git")
        .args(["branch", "--list", "f2"])
        .current_dir(temp_dir.path())
        .output()?;
    assert!(
        String::from_utf8_lossy(&output.stdout).trim().is_empty(),
        "f2 should be deleted"
    );

    // f1 should also be deleted (its commits are in main via f2)
    let output = Command::new("git")
        .args(["branch", "--list", "f1"])
        .current_dir(temp_dir.path())
        .output()?;
    assert!(
        String::from_utf8_lossy(&output.stdout).trim().is_empty(),
        "f1 should be deleted (its commits are in main via f2)"
    );

    Ok(())
}

// ============================================================================
// RECOVERY PATH TESTS
// ============================================================================

#[test]
fn test_abort_after_partial_sync_restores_all() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create 5-branch stack with different files
    for i in 1..=5 {
        fs::write(
            temp_dir.path().join(format!("f{}.txt", i)),
            format!("branch {} content", i),
        )?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("b{}", i), "-a", "-m", &format!("B{}", i)],
        )?;
    }

    // Record original commit hashes
    let mut original_hashes = Vec::new();
    for i in 1..=5 {
        let hash = get_commit_hash(temp_dir.path(), &format!("b{}", i))?;
        original_hashes.push(hash);
    }

    // Go to b3, create conflicting change for b3's file on main
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("f3.txt"), "conflict on main")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "-m", "Conflict on main"])?;

    // Go to b5 (top of stack) and sync - should fail at b3 (it's in our stack)
    run_dm(temp_dir.path(), &["checkout", "b5"])?;
    let _output = run_dm(temp_dir.path(), &["sync"])?;

    // Check if rebase is in progress (conflict happened)
    if git_rebase_in_progress(temp_dir.path())? {
        // Abort the git rebase first
        run_git(temp_dir.path(), &["rebase", "--abort"])?;
    }

    // Now abort the dm operation
    run_dm(temp_dir.path(), &["abort"])?;

    // Verify ALL branches are restored to original hashes
    for i in 1..=5 {
        let current_hash = get_commit_hash(temp_dir.path(), &format!("b{}", i))?;
        assert_eq!(
            current_hash,
            original_hashes[i - 1],
            "b{} should be restored to original hash",
            i
        );
    }

    // Doctor should find no issues
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(output.status.success());

    Ok(())
}

#[test]
fn test_continue_after_resolving_conflict() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a simple 2-branch stack
    fs::write(temp_dir.path().join("shared.txt"), "original content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Add shared file"])?;

    // Go to main and create conflict
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("shared.txt"), "main's version")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "-m", "Main changes shared"])?;

    // Sync - will conflict
    run_dm(temp_dir.path(), &["sync"])?;

    // Check if rebase is in progress
    if git_rebase_in_progress(temp_dir.path())? {
        // Resolve conflict by accepting feature's version
        fs::write(temp_dir.path().join("shared.txt"), "resolved content")?;
        run_git(temp_dir.path(), &["add", "shared.txt"])?;

        // Continue the operation
        let output = run_dm(temp_dir.path(), &["continue"])?;
        assert!(
            output.status.success(),
            "Continue should succeed after resolution: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Operation state should be cleared
    let state = get_operation_state(temp_dir.path())?;
    assert!(
        state.is_none(),
        "Operation state should be cleared after successful continue"
    );

    // Doctor should find no issues
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(output.status.success());

    Ok(())
}

#[test]
fn test_abort_restores_exact_commit_hashes() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a 3-branch stack
    for i in 1..=3 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("content {}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("b{}", i), "-a", "-m", &format!("B{}", i)],
        )?;
    }

    // Record exact commit hashes before any operation
    let hash1_before = get_commit_hash(temp_dir.path(), "b1")?;
    let hash2_before = get_commit_hash(temp_dir.path(), "b2")?;
    let hash3_before = get_commit_hash(temp_dir.path(), "b3")?;

    // Go to main and make a change
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("main.txt"), "main update")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "-m", "Main update"])?;

    // Start restack (this will rebase all branches)
    run_dm(temp_dir.path(), &["checkout", "b1"])?;
    let _output = run_dm(temp_dir.path(), &["restack"])?;

    // Even if restack succeeded, we can still abort (within window)
    // If it's still in progress due to conflict, abort that first
    if git_rebase_in_progress(temp_dir.path())? {
        run_git(temp_dir.path(), &["rebase", "--abort"])?;
    }

    // Check if operation state exists
    let state = get_operation_state(temp_dir.path())?;
    if state.is_some() {
        // Abort the operation
        run_dm(temp_dir.path(), &["abort"])?;

        // Verify exact commit hashes are restored
        let hash1_after = get_commit_hash(temp_dir.path(), "b1")?;
        let hash2_after = get_commit_hash(temp_dir.path(), "b2")?;
        let hash3_after = get_commit_hash(temp_dir.path(), "b3")?;

        assert_eq!(hash1_before, hash1_after, "b1 hash should be restored");
        assert_eq!(hash2_before, hash2_after, "b2 hash should be restored");
        assert_eq!(hash3_before, hash3_after, "b3 hash should be restored");
    }

    // Doctor should find no issues
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(output.status.success());

    Ok(())
}

// ============================================================================
// EXTERNAL CHANGE HANDLING TESTS
// ============================================================================

#[test]
fn test_sync_handles_external_changes_gracefully() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch with Diamond
    fs::write(temp_dir.path().join("feature.txt"), "feature content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Go back to main
    run_git(temp_dir.path(), &["checkout", "main"])?;

    // Modify the branch externally (outside Diamond)
    run_git(temp_dir.path(), &["checkout", "feature"])?;
    fs::write(temp_dir.path().join("external.txt"), "external change")?;
    run_git(temp_dir.path(), &["add", "external.txt"])?;
    run_git(temp_dir.path(), &["commit", "-m", "External change"])?;
    run_git(temp_dir.path(), &["checkout", "main"])?;

    // Sync should succeed - external changes are handled gracefully
    // (no blocking warning, just rebase onto correct parent)
    let output = run_dm(temp_dir.path(), &["sync"])?;
    assert!(
        output.status.success(),
        "Sync should succeed with external changes: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

#[test]
fn test_restack_handles_external_changes_gracefully() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> feature
    fs::write(temp_dir.path().join("feature.txt"), "feature content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Modify the branch externally
    fs::write(temp_dir.path().join("external.txt"), "external change")?;
    run_git(temp_dir.path(), &["add", "external.txt"])?;
    run_git(temp_dir.path(), &["commit", "-m", "External change"])?;

    // Restack should succeed - external changes are common in stacked PR workflows
    let output = run_dm(temp_dir.path(), &["restack"])?;
    assert!(
        output.status.success(),
        "Restack should succeed with external changes: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

#[test]
fn test_no_external_changes_detected_for_fresh_branches() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch with Diamond (this sets base_sha)
    fs::write(temp_dir.path().join("feature.txt"), "feature content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Go back to main
    run_git(temp_dir.path(), &["checkout", "main"])?;

    // Sync should work without --force (no external changes)
    let output = run_dm(temp_dir.path(), &["sync"])?;
    assert!(
        output.status.success(),
        "Sync should succeed when no external changes: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Output should NOT mention external changes
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("External changes detected"),
        "Should not mention external changes when there are none"
    );

    Ok(())
}

#[test]
fn test_sync_records_sync_state() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a simple stack
    fs::write(temp_dir.path().join("feature.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Run sync
    let output = run_dm(temp_dir.path(), &["sync"])?;
    assert!(output.status.success(), "Sync should succeed");

    // Verify cache.json was created with sync state (merged from sync_state.rs)
    let cache_path = temp_dir.path().join(".git/diamond/cache.json");
    assert!(cache_path.exists(), "cache.json should exist after sync");

    // Verify the content is valid JSON with last_sync_at
    let content = fs::read_to_string(&cache_path)?;
    let cache: serde_json::Value = serde_json::from_str(&content)?;
    assert!(
        cache.get("last_sync_at").is_some(),
        "cache.json should contain last_sync_at timestamp"
    );

    Ok(())
}

// ============================================================================
// UNDO TESTS
// ============================================================================

#[test]
fn test_undo_restores_last_sync() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch: main -> feature
    fs::write(temp_dir.path().join("feature.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Get the commit hash before sync
    let pre_sync_hash = get_commit_hash(temp_dir.path(), "feature")?;

    // Make a change on main that will affect the sync
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main content", "Main update")?;

    // Sync - this will rebase feature onto the new main
    run_dm(temp_dir.path(), &["checkout", "feature"])?;
    let output = run_dm(temp_dir.path(), &["sync"])?;
    assert!(output.status.success(), "Sync should succeed");

    // Get the commit hash after sync (should be different due to rebase)
    let post_sync_hash = get_commit_hash(temp_dir.path(), "feature")?;
    assert_ne!(pre_sync_hash, post_sync_hash, "Feature branch should have been rebased");

    // Undo the sync with --force to skip confirmation
    let output = run_dm(temp_dir.path(), &["undo", "--force"])?;
    assert!(
        output.status.success(),
        "Undo should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the output mentions the sync operation
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("sync") || stdout.contains("Restored"),
        "Undo output should mention sync or restoration: {}",
        stdout
    );

    // The branch should be restored to its pre-sync state
    let restored_hash = get_commit_hash(temp_dir.path(), "feature")?;
    assert_eq!(
        pre_sync_hash, restored_hash,
        "Feature branch should be restored to pre-sync state"
    );

    Ok(())
}

#[test]
fn test_undo_chain_multiple_operations() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branches: main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Get original hashes
    let original_f1 = get_commit_hash(temp_dir.path(), "f1")?;
    let original_f2 = get_commit_hash(temp_dir.path(), "f2")?;

    // First operation: Sync (after updating main)
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main1.txt", "main1", "Main 1")?;
    run_dm(temp_dir.path(), &["checkout", "f2"])?;
    run_dm(temp_dir.path(), &["sync"])?;

    // Second operation: Restack (after modifying f1)
    run_dm(temp_dir.path(), &["checkout", "f1"])?;
    fs::write(temp_dir.path().join("f1.txt"), "f1 modified")?;
    run_dm(temp_dir.path(), &["modify", "-a"])?;

    // Now undo twice
    // First undo should restore from restack
    let output = run_dm(temp_dir.path(), &["undo", "--force"])?;
    assert!(
        output.status.success(),
        "First undo should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("restack") || stdout.contains("Restored"),
        "First undo should mention restack: {}",
        stdout
    );

    // Second undo should restore from sync
    let output = run_dm(temp_dir.path(), &["undo", "--force"])?;
    assert!(
        output.status.success(),
        "Second undo should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("sync") || stdout.contains("Restored"),
        "Second undo should mention sync: {}",
        stdout
    );

    // Branches should be back to original state
    let restored_f1 = get_commit_hash(temp_dir.path(), "f1")?;
    let restored_f2 = get_commit_hash(temp_dir.path(), "f2")?;
    assert_eq!(original_f1, restored_f1, "f1 should be restored");
    assert_eq!(original_f2, restored_f2, "f2 should be restored");

    // Third undo should report no more operations
    let output = run_dm(temp_dir.path(), &["undo", "--force"])?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No undoable operations"),
        "Third undo should report no operations: {}",
        stdout
    );

    Ok(())
}

#[test]
fn test_undo_without_force_requires_tty() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create and sync a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "m.txt", "main", "Main")?;

    run_dm(temp_dir.path(), &["checkout", "feature"])?;
    run_dm(temp_dir.path(), &["sync"])?;

    // Undo without --force should fail in non-TTY (test) environment
    let output = run_dm(temp_dir.path(), &["undo"])?;

    // It should fail with an error about requiring --force
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("--force"),
        "Undo without --force should fail or mention --force in non-TTY: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        stderr
    );

    Ok(())
}

// ============================================================================
// MERGE-AWARE SYNC TESTS (GitHub squash merge workflow)
// ============================================================================

/// Tests that sync works correctly after a parent branch is "merged" on GitHub.
/// This simulates the common workflow:
/// 1. Create stack: main → A → B
/// 2. A is merged on GitHub (squash merge - A's branch is deleted, main has A's content)
/// 3. Run sync - B should rebase cleanly onto main using fork-point
#[test]
fn test_sync_after_parent_merged_uses_fork_point() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> branch_a -> branch_b
    fs::write(temp_dir.path().join("a.txt"), "content from A")?;
    run_dm(temp_dir.path(), &["create", "branch_a", "-a", "-m", "Add A"])?;

    fs::write(temp_dir.path().join("b.txt"), "content from B")?;
    run_dm(temp_dir.path(), &["create", "branch_b", "-a", "-m", "Add B"])?;

    // Record B's original commit hash
    let original_b = get_commit_hash(temp_dir.path(), "branch_b")?;

    // Simulate GitHub squash merge of A into main:
    // 1. Cherry-pick A's changes to main (like squash merge)
    run_git(temp_dir.path(), &["checkout", "main"])?;
    // Create the same file with same content (simulating squash merge result)
    fs::write(temp_dir.path().join("a.txt"), "content from A")?;
    run_git(temp_dir.path(), &["add", "a.txt"])?;
    run_git(temp_dir.path(), &["commit", "-m", "Squash merge: Add A"])?;

    // 2. Delete branch_a locally (as if cleaned up after merge)
    run_git(temp_dir.path(), &["branch", "-D", "branch_a"])?;

    // 3. Update Diamond metadata: reparent branch_b to main and remove branch_a tracking
    // This simulates what cleanup_merged_branches_for_sync does
    remove_branch_tracking(temp_dir.path(), "branch_a")?;
    set_parent_in_refs(temp_dir.path(), "branch_b", "main")?;

    // Run sync from branch_b - this should work without conflicts
    // because we should use fork-point rebasing
    run_dm(temp_dir.path(), &["checkout", "branch_b"])?;
    let output = run_dm(temp_dir.path(), &["sync"])?;

    // The sync should succeed (no conflicts)
    assert!(
        output.status.success(),
        "Sync should succeed after parent merge. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // branch_b should have been rebased (new commit hash)
    let new_b = get_commit_hash(temp_dir.path(), "branch_b")?;
    assert_ne!(original_b, new_b, "branch_b should have been rebased onto main");

    // branch_b should now include the squash-merged changes from main
    run_dm(temp_dir.path(), &["checkout", "branch_b"])?;
    let output = run_git(temp_dir.path(), &["log", "--oneline"])?;
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(
        log.contains("Squash merge: Add A"),
        "branch_b should include squash-merged A: {}",
        log
    );

    // b.txt should still exist
    assert!(
        temp_dir.path().join("b.txt").exists(),
        "b.txt should still exist after rebase"
    );

    Ok(())
}

/// Tests that sync handles a 3-level stack where the first branch is merged.
/// Stack: main → A → B → C
/// After A is merged: main → B → C
#[test]
fn test_sync_three_level_stack_first_merged() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> a -> b -> c
    fs::write(temp_dir.path().join("a.txt"), "a content")?;
    run_dm(temp_dir.path(), &["create", "a", "-a", "-m", "Add A"])?;

    fs::write(temp_dir.path().join("b.txt"), "b content")?;
    run_dm(temp_dir.path(), &["create", "b", "-a", "-m", "Add B"])?;

    fs::write(temp_dir.path().join("c.txt"), "c content")?;
    run_dm(temp_dir.path(), &["create", "c", "-a", "-m", "Add C"])?;

    // Record original hashes
    let original_b = get_commit_hash(temp_dir.path(), "b")?;
    let original_c = get_commit_hash(temp_dir.path(), "c")?;

    // Simulate squash merge of A into main
    run_git(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("a.txt"), "a content")?;
    run_git(temp_dir.path(), &["add", "a.txt"])?;
    run_git(temp_dir.path(), &["commit", "-m", "Squash: Add A"])?;

    // Delete branch a and update metadata
    run_git(temp_dir.path(), &["branch", "-D", "a"])?;
    remove_branch_tracking(temp_dir.path(), "a")?;
    set_parent_in_refs(temp_dir.path(), "b", "main")?;

    // Run sync
    run_dm(temp_dir.path(), &["checkout", "c"])?;
    let output = run_dm(temp_dir.path(), &["sync"])?;

    assert!(
        output.status.success(),
        "Sync should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Both b and c should be rebased
    let new_b = get_commit_hash(temp_dir.path(), "b")?;
    let new_c = get_commit_hash(temp_dir.path(), "c")?;
    assert_ne!(original_b, new_b, "b should be rebased");
    assert_ne!(original_c, new_c, "c should be rebased");

    // All files should exist
    assert!(temp_dir.path().join("a.txt").exists());
    assert!(temp_dir.path().join("b.txt").exists());
    assert!(temp_dir.path().join("c.txt").exists());

    Ok(())
}

/// Tests that sync handles multiple merged parents correctly.
/// Stack: main → A → B → C → D
/// After A and B are merged: main → C → D
#[test]
fn test_sync_multiple_parents_merged() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> a -> b -> c -> d
    for (name, content) in [("a", "a"), ("b", "b"), ("c", "c"), ("d", "d")] {
        fs::write(temp_dir.path().join(format!("{}.txt", name)), content)?;
        run_dm(
            temp_dir.path(),
            &["create", name, "-a", "-m", &format!("Add {}", name.to_uppercase())],
        )?;
    }

    let original_c = get_commit_hash(temp_dir.path(), "c")?;
    let original_d = get_commit_hash(temp_dir.path(), "d")?;

    // Simulate squash merge of A and B into main
    run_git(temp_dir.path(), &["checkout", "main"])?;
    // First A's content
    fs::write(temp_dir.path().join("a.txt"), "a")?;
    run_git(temp_dir.path(), &["add", "a.txt"])?;
    run_git(temp_dir.path(), &["commit", "-m", "Squash: Add A"])?;
    // Then B's content
    fs::write(temp_dir.path().join("b.txt"), "b")?;
    run_git(temp_dir.path(), &["add", "b.txt"])?;
    run_git(temp_dir.path(), &["commit", "-m", "Squash: Add B"])?;

    // Delete branches a and b, update metadata
    run_git(temp_dir.path(), &["branch", "-D", "a"])?;
    run_git(temp_dir.path(), &["branch", "-D", "b"])?;
    remove_branch_tracking(temp_dir.path(), "a")?;
    remove_branch_tracking(temp_dir.path(), "b")?;
    set_parent_in_refs(temp_dir.path(), "c", "main")?;

    // Run sync
    run_dm(temp_dir.path(), &["checkout", "d"])?;
    let output = run_dm(temp_dir.path(), &["sync"])?;

    assert!(
        output.status.success(),
        "Sync should succeed with multiple merged parents: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Both c and d should be rebased
    let new_c = get_commit_hash(temp_dir.path(), "c")?;
    let new_d = get_commit_hash(temp_dir.path(), "d")?;
    assert_ne!(original_c, new_c, "c should be rebased");
    assert_ne!(original_d, new_d, "d should be rebased");

    // All files should exist
    for name in ["a", "b", "c", "d"] {
        assert!(
            temp_dir.path().join(format!("{}.txt", name)).exists(),
            "{}.txt should exist",
            name
        );
    }

    Ok(())
}

// ============================================================================
// SYNC --no-restack FLAG TESTS
// ============================================================================

#[test]
fn test_sync_no_restack_flag_is_recognized() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Just verify the flag is recognized (doesn't cause an error)
    let output = run_dm(temp_dir.path(), &["sync", "--no-restack"])?;

    // Should not fail with "unknown flag" error
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("unknown"),
        "--no-restack flag should be recognized: {}",
        stderr
    );

    Ok(())
}

// ============================================================================
// ORPHANED BRANCH AUTO-REPAIR TESTS
// ============================================================================

/// Test that sync automatically repairs orphaned branches when parent is deleted
#[test]
fn test_sync_auto_repairs_orphaned_branches() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> parent -> child
    fs::write(temp_dir.path().join("parent.txt"), "parent")?;
    run_dm(temp_dir.path(), &["create", "parent-branch", "-a", "-m", "Parent"])?;

    fs::write(temp_dir.path().join("child.txt"), "child")?;
    run_dm(temp_dir.path(), &["create", "child-branch", "-a", "-m", "Child"])?;

    // Delete parent via git, leaving child orphaned
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["branch", "-D", "parent-branch"])?;

    // Sync should detect and fix the orphaned branch
    let output = run_dm(temp_dir.path(), &["sync"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify sync detected and fixed the orphan
    assert!(
        stdout.contains("Fixed") && stdout.contains("orphaned"),
        "Sync should report fixing orphaned branches: {}",
        stdout
    );
    assert!(
        stdout.contains("child-branch") && stdout.contains("parent-branch"),
        "Sync should mention the orphaned child and deleted parent: {}",
        stdout
    );

    // Verify child is now parented to main
    assert_eq!(
        get_parent_from_refs(temp_dir.path(), "child-branch")?,
        Some("main".to_string()),
        "child-branch should be reparented to main"
    );

    // Doctor should show no issues
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let doctor_stdout = String::from_utf8_lossy(&output.stdout);
    let doctor_stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "Doctor should pass after sync auto-repair. stdout: {}, stderr: {}",
        doctor_stdout,
        doctor_stderr
    );

    Ok(())
}

/// Test that restack automatically repairs orphaned branches when parent is deleted
#[test]
fn test_restack_auto_repairs_orphaned_branches() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> parent -> child
    fs::write(temp_dir.path().join("parent.txt"), "parent")?;
    run_dm(temp_dir.path(), &["create", "parent-branch", "-a", "-m", "Parent"])?;

    fs::write(temp_dir.path().join("child.txt"), "child")?;
    run_dm(temp_dir.path(), &["create", "child-branch", "-a", "-m", "Child"])?;

    // Delete parent via git, leaving child orphaned
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["branch", "-D", "parent-branch"])?;

    // Restack should detect and fix the orphaned branch
    let output = run_dm(temp_dir.path(), &["restack"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify restack detected and fixed the orphan
    assert!(
        stdout.contains("Fixed") && stdout.contains("orphaned"),
        "Restack should report fixing orphaned branches: {}",
        stdout
    );

    // Verify child is now parented to main
    assert_eq!(
        get_parent_from_refs(temp_dir.path(), "child-branch")?,
        Some("main".to_string()),
        "child-branch should be reparented to main"
    );

    // Doctor should show no issues
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(output.status.success(), "Doctor should pass after restack auto-repair");

    Ok(())
}

/// Test that sync repairs multiple orphaned branches in a chain
#[test]
fn test_sync_repairs_orphaned_chain() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> parent -> child1 -> child2
    fs::write(temp_dir.path().join("parent.txt"), "parent")?;
    run_dm(temp_dir.path(), &["create", "parent-branch", "-a", "-m", "Parent"])?;

    fs::write(temp_dir.path().join("child1.txt"), "child1")?;
    run_dm(temp_dir.path(), &["create", "child1-branch", "-a", "-m", "Child1"])?;

    fs::write(temp_dir.path().join("child2.txt"), "child2")?;
    run_dm(temp_dir.path(), &["create", "child2-branch", "-a", "-m", "Child2"])?;

    // Delete parent via git, leaving both children orphaned from main
    // (child1 directly orphaned, child2 indirectly through child1)
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    run_git(temp_dir.path(), &["branch", "-D", "parent-branch"])?;

    // Sync should detect and fix the orphaned branch (child1)
    // child2 remains parented to child1 (which still exists)
    let output = run_dm(temp_dir.path(), &["sync"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("Fixed") && stdout.contains("orphaned"),
        "Sync should report fixing orphaned branches: {}",
        stdout
    );

    // child1 should be reparented to main (its parent was deleted)
    assert_eq!(
        get_parent_from_refs(temp_dir.path(), "child1-branch")?,
        Some("main".to_string()),
        "child1-branch should be reparented to main"
    );

    // child2 should still be parented to child1 (child1 exists)
    assert_eq!(
        get_parent_from_refs(temp_dir.path(), "child2-branch")?,
        Some("child1-branch".to_string()),
        "child2-branch should still be parented to child1-branch"
    );

    // Doctor should show no issues
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(output.status.success(), "Doctor should pass after sync auto-repair");

    Ok(())
}

// ============================================================================
// Stack-Aware Conflict Handling Tests (Sync)
// ============================================================================

/// Test 1: Sync from trunk skips all branch conflicts
/// When on trunk (main), there is no "stack concept" - all branches are "other"
/// Expected: Skip all conflicted branches, return to main cleanly
#[test]
fn test_sync_from_trunk_skips_all_branch_conflicts() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> stack1-a -> stack1-b
    // Create stack1-a and add a.txt ON THE BRANCH
    run_dm(temp_dir.path(), &["create", "stack1-a"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "stack1-a content", "Add a on stack1-a")?;

    // Create stack1-b and add b.txt
    run_dm(temp_dir.path(), &["create", "stack1-b"])?;
    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b")?;

    // Get original hashes before conflict
    let original_a = get_commit_hash(temp_dir.path(), "stack1-a")?;
    let original_b = get_commit_hash(temp_dir.path(), "stack1-b")?;

    // Go to main and create conflict with stack1-a (modify same file a.txt)
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "main conflicts", "Main modifies a.txt")?;

    // Run sync from main - should skip both branches (a has conflict, b is child of a)
    let output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Debug output
    eprintln!("\n=== SYNC OUTPUT ===");
    eprintln!("Exit code: {:?}", output.status.code());
    eprintln!("STDOUT:\n{}", stdout);
    eprintln!("STDERR:\n{}", stderr);

    let current = get_current_branch(temp_dir.path())?;
    eprintln!("Current branch: '{}'", current);
    eprintln!("Rebase in progress: {}", git_rebase_in_progress(temp_dir.path())?);

    // Verify: should be on main, no rebase in progress
    assert_eq!(current, "main", "Should return to main branch");
    assert!(
        !git_rebase_in_progress(temp_dir.path())?,
        "Should not be in rebase state"
    );

    // Verify: branches unchanged (not rebased)
    assert_eq!(
        get_commit_hash(temp_dir.path(), "stack1-a")?,
        original_a,
        "stack1-a should not be rebased (has conflicts)"
    );
    assert_eq!(
        get_commit_hash(temp_dir.path(), "stack1-b")?,
        original_b,
        "stack1-b should not be rebased (parent was skipped)"
    );

    // Verify: operation state cleared
    assert!(
        get_operation_state(temp_dir.path())?.is_none(),
        "Operation state should be cleared"
    );

    // Verify: output mentions skipped branches
    assert!(
        stdout.contains("skipped") || stdout.contains("Skipped"),
        "Output should mention skipped branches: stderr={}, stdout={}",
        stderr,
        stdout
    );

    Ok(())
}

/// Test 2: Sync stops when ancestor in current stack has conflicts
/// When on feature-c, feature-a (ancestor) is in YOUR dependency chain
/// Expected: Stop at feature-a (leave in rebase state)
#[test]
fn test_sync_stops_when_ancestor_in_current_stack_has_conflicts() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> feature-a -> feature-b -> feature-c
    run_dm(temp_dir.path(), &["create", "feature-a"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a on feature-a")?;

    run_dm(temp_dir.path(), &["create", "feature-b"])?;
    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b on feature-b")?;

    run_dm(temp_dir.path(), &["create", "feature-c"])?;
    create_file_and_commit(temp_dir.path(), "c.txt", "c content", "Add c on feature-c")?;

    // Go to main and create conflict with feature-a
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "CONFLICT", "Main conflicts with a")?;

    // Go back to feature-c (the top of the stack)
    run_git(temp_dir.path(), &["checkout", "feature-c"])?;

    // Run sync from feature-c - should stop at feature-a (it's in your stack)
    let _output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;

    // Verify: should be in rebase state
    assert!(
        git_rebase_in_progress(temp_dir.path())?,
        "Should be in rebase state (ancestor in current stack has conflicts)"
    );

    // Verify: operation state exists (sync paused)
    let state = get_operation_state(temp_dir.path())?;
    assert!(
        state.is_some(),
        "Operation state should exist (sync paused on conflict)"
    );

    // Verify: state indicates Sync operation
    let state = state.unwrap();
    assert!(
        state
            .get("operation_type")
            .and_then(|v| v.as_str())
            .map(|s| s == "Sync")
            .unwrap_or(false),
        "Operation type should be Sync"
    );

    Ok(())
}

/// Test 3: Sync stops when current branch has conflicts
/// When on feature-b, and feature-b conflicts, it's obviously in YOUR stack
/// Expected: feature-a rebases, feature-b stops in conflict state
#[test]
fn test_sync_stops_when_current_branch_has_conflicts() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> feature-a -> feature-b
    run_dm(temp_dir.path(), &["create", "feature-a"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a on feature-a")?;

    run_dm(temp_dir.path(), &["create", "feature-b"])?;
    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b on feature-b")?;

    // Get feature-a hash
    let original_a = get_commit_hash(temp_dir.path(), "feature-a")?;

    // Go to main and advance it (no conflict with feature-a)
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main content", "Advance main")?;

    // Then create conflict with feature-b
    create_file_and_commit(temp_dir.path(), "b.txt", "CONFLICT", "Main conflicts with b")?;

    // Go back to feature-b
    run_git(temp_dir.path(), &["checkout", "feature-b"])?;

    // Run sync from feature-b - should rebase feature-a, then stop at feature-b
    let _output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;

    // Verify: feature-a was rebased (hash changed)
    let new_a = get_commit_hash(temp_dir.path(), "feature-a")?;
    assert_ne!(new_a, original_a, "feature-a should be rebased (no conflicts)");

    // Verify: should be in rebase state on feature-b
    assert!(
        git_rebase_in_progress(temp_dir.path())?,
        "Should be in rebase state (current branch has conflicts)"
    );

    // Verify: operation state exists
    assert!(
        get_operation_state(temp_dir.path())?.is_some(),
        "Operation state should exist (sync paused)"
    );

    Ok(())
}

/// Test 4: Sync stops when child in current stack has conflicts
/// When on feature-a, feature-c (descendant) is in YOUR stack
/// Expected: feature-a and feature-b rebase, feature-c stops
#[test]
fn test_sync_stops_when_child_in_current_stack_has_conflicts() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> feature-a -> feature-b -> feature-c
    run_dm(temp_dir.path(), &["create", "feature-a"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a on feature-a")?;

    run_dm(temp_dir.path(), &["create", "feature-b"])?;
    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b on feature-b")?;

    run_dm(temp_dir.path(), &["create", "feature-c"])?;
    create_file_and_commit(temp_dir.path(), "c.txt", "c content", "Add c on feature-c")?;

    // Get original hashes
    let original_a = get_commit_hash(temp_dir.path(), "feature-a")?;
    let original_b = get_commit_hash(temp_dir.path(), "feature-b")?;

    // Go to main and advance it, then create conflict with feature-c
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main content", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "c.txt", "CONFLICT", "Main conflicts with c")?;

    // Go to feature-a (the root of the stack)
    run_git(temp_dir.path(), &["checkout", "feature-a"])?;

    // Run sync from feature-a - should rebase a and b, then stop at c
    let _output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;

    // Verify: feature-a and feature-b were rebased
    let new_a = get_commit_hash(temp_dir.path(), "feature-a")?;
    let new_b = get_commit_hash(temp_dir.path(), "feature-b")?;
    assert_ne!(new_a, original_a, "feature-a should be rebased");
    assert_ne!(new_b, original_b, "feature-b should be rebased");

    // Verify: should be in rebase state on feature-c
    assert!(
        git_rebase_in_progress(temp_dir.path())?,
        "Should be in rebase state (child in current stack has conflicts)"
    );

    // Verify: operation state exists
    assert!(
        get_operation_state(temp_dir.path())?.is_some(),
        "Operation state should exist (sync paused)"
    );

    Ok(())
}

/// Test 5: Sync skips unrelated stack conflicts
/// Two stacks: stack1 (unrelated) and stack2 (you're on stack2-b)
/// Expected: Skip stack1-a, rebase stack2-a and stack2-b, return to stack2-b cleanly
#[test]
fn test_sync_skips_unrelated_stack_conflicts() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create first stack: main -> stack1-a
    run_dm(temp_dir.path(), &["create", "stack1-a"])?;
    create_file_and_commit(temp_dir.path(), "s1.txt", "stack1 content", "Add s1 on stack1-a")?;

    // Go back to main and create second stack: main -> stack2-a -> stack2-b
    run_git(temp_dir.path(), &["checkout", "main"])?;
    run_dm(temp_dir.path(), &["create", "stack2-a"])?;
    create_file_and_commit(temp_dir.path(), "s2a.txt", "stack2a content", "Add s2a on stack2-a")?;

    run_dm(temp_dir.path(), &["create", "stack2-b"])?;
    create_file_and_commit(temp_dir.path(), "s2b.txt", "stack2b content", "Add s2b on stack2-b")?;

    // Get original hashes
    let original_stack1_a = get_commit_hash(temp_dir.path(), "stack1-a")?;
    let original_stack2_a = get_commit_hash(temp_dir.path(), "stack2-a")?;
    let original_stack2_b = get_commit_hash(temp_dir.path(), "stack2-b")?;

    // Go to main and create conflict with stack1-a (the unrelated stack)
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "s1.txt", "CONFLICT", "Main conflicts with stack1")?;

    // Go to stack2-b (your current stack)
    run_git(temp_dir.path(), &["checkout", "stack2-b"])?;

    // Run sync - should skip stack1-a, rebase stack2-a and stack2-b
    let output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify: should be on stack2-b, no rebase in progress
    assert_eq!(
        get_current_branch(temp_dir.path())?,
        "stack2-b",
        "Should return to stack2-b"
    );
    assert!(
        !git_rebase_in_progress(temp_dir.path())?,
        "Should not be in rebase state (unrelated stack conflict)"
    );

    // Verify: stack1-a unchanged (skipped)
    assert_eq!(
        get_commit_hash(temp_dir.path(), "stack1-a")?,
        original_stack1_a,
        "stack1-a should be unchanged (unrelated stack, skipped)"
    );

    // Verify: stack2-a and stack2-b rebased
    let new_stack2_a = get_commit_hash(temp_dir.path(), "stack2-a")?;
    let new_stack2_b = get_commit_hash(temp_dir.path(), "stack2-b")?;
    assert_ne!(
        new_stack2_a, original_stack2_a,
        "stack2-a should be rebased (in your stack)"
    );
    assert_ne!(
        new_stack2_b, original_stack2_b,
        "stack2-b should be rebased (in your stack)"
    );

    // Verify: operation state cleared
    assert!(
        get_operation_state(temp_dir.path())?.is_none(),
        "Operation state should be cleared"
    );

    // Verify: output mentions skipped branch
    assert!(
        stdout.contains("skipped") || stdout.contains("Skipped"),
        "Output should mention skipped branches: {}",
        stdout
    );

    Ok(())
}

/// Test 6: Sync from trunk skips multiple unrelated stacks with conflicts
/// From trunk, both stack1 and stack2 have conflicts
/// Expected: Skip all conflicted stacks, return to main cleanly
#[test]
fn test_sync_skips_multiple_unrelated_stacks_from_trunk() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack1: main -> a1 -> a2
    run_dm(temp_dir.path(), &["create", "a1"])?;
    create_file_and_commit(temp_dir.path(), "a1.txt", "a1 content", "Add a1 on a1")?;

    run_dm(temp_dir.path(), &["create", "a2"])?;
    create_file_and_commit(temp_dir.path(), "a2.txt", "a2 content", "Add a2 on a2")?;

    // Go back to main and create stack2: main -> b1 -> b2
    run_git(temp_dir.path(), &["checkout", "main"])?;
    run_dm(temp_dir.path(), &["create", "b1"])?;
    create_file_and_commit(temp_dir.path(), "b1.txt", "b1 content", "Add b1 on b1")?;

    run_dm(temp_dir.path(), &["create", "b2"])?;
    create_file_and_commit(temp_dir.path(), "b2.txt", "b2 content", "Add b2 on b2")?;

    // Get original hashes
    let original_a1 = get_commit_hash(temp_dir.path(), "a1")?;
    let original_a2 = get_commit_hash(temp_dir.path(), "a2")?;
    let original_b1 = get_commit_hash(temp_dir.path(), "b1")?;
    let original_b2 = get_commit_hash(temp_dir.path(), "b2")?;

    // Go to main and create conflicts with both a1 and b1
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "a1.txt", "CONFLICT_A", "Main conflicts with a1")?;
    create_file_and_commit(temp_dir.path(), "b1.txt", "CONFLICT_B", "Main conflicts with b1")?;

    // Run sync from trunk - should skip all branches
    let output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify: should be on main, no rebase in progress
    assert_eq!(get_current_branch(temp_dir.path())?, "main");
    assert!(!git_rebase_in_progress(temp_dir.path())?);

    // Verify: all branches unchanged (skipped)
    assert_eq!(get_commit_hash(temp_dir.path(), "a1")?, original_a1);
    assert_eq!(get_commit_hash(temp_dir.path(), "a2")?, original_a2);
    assert_eq!(get_commit_hash(temp_dir.path(), "b1")?, original_b1);
    assert_eq!(get_commit_hash(temp_dir.path(), "b2")?, original_b2);

    // Verify: operation state cleared
    assert!(get_operation_state(temp_dir.path())?.is_none());

    // Verify: output mentions skipped branches
    assert!(
        stdout.contains("skipped") || stdout.contains("Skipped"),
        "Output should mention skipped branches: {}",
        stdout
    );

    Ok(())
}

/// Test 7: Sync from trunk with parent and child conflicts in different stacks
/// Stack1: a1 has conflict (so a2 also skipped). Stack2: b2 has conflict (so b3 also skipped)
/// Expected: Skip all branches with dependency chain logic
#[test]
fn test_sync_from_trunk_with_multiple_stacks_skips_all_conflicts() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack1: main -> a1 -> a2
    run_dm(temp_dir.path(), &["create", "a1"])?;
    create_file_and_commit(temp_dir.path(), "a1.txt", "a1 content", "Add a1 on a1")?;

    run_dm(temp_dir.path(), &["create", "a2"])?;
    create_file_and_commit(temp_dir.path(), "a2.txt", "a2 content", "Add a2 on a2")?;

    // Go back to main and create stack2: main -> b1 -> b2 -> b3
    run_git(temp_dir.path(), &["checkout", "main"])?;
    run_dm(temp_dir.path(), &["create", "b1"])?;
    create_file_and_commit(temp_dir.path(), "b1.txt", "b1 content", "Add b1 on b1")?;

    run_dm(temp_dir.path(), &["create", "b2"])?;
    create_file_and_commit(temp_dir.path(), "b2.txt", "b2 content", "Add b2 on b2")?;

    run_dm(temp_dir.path(), &["create", "b3"])?;
    create_file_and_commit(temp_dir.path(), "b3.txt", "b3 content", "Add b3 on b3")?;

    // Get original hashes
    let original_a1 = get_commit_hash(temp_dir.path(), "a1")?;
    let original_a2 = get_commit_hash(temp_dir.path(), "a2")?;
    let _original_b1 = get_commit_hash(temp_dir.path(), "b1")?;
    let original_b2 = get_commit_hash(temp_dir.path(), "b2")?;
    let original_b3 = get_commit_hash(temp_dir.path(), "b3")?;

    // Go to main and create conflicts: a1 and b2
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "a1.txt", "CONFLICT_A", "Main conflicts with a1")?;
    create_file_and_commit(temp_dir.path(), "b2.txt", "CONFLICT_B", "Main conflicts with b2")?;

    // Run sync from trunk - should skip a1+a2 (parent failed), b1 might rebase, b2+b3 skipped
    let output = run_dm(temp_dir.path(), &["sync", "--no-cleanup"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify: should be on main, no rebase in progress
    assert_eq!(get_current_branch(temp_dir.path())?, "main");
    assert!(!git_rebase_in_progress(temp_dir.path())?);

    // Verify: a1 and a2 unchanged (a1 conflict, a2 is child)
    assert_eq!(get_commit_hash(temp_dir.path(), "a1")?, original_a1);
    assert_eq!(get_commit_hash(temp_dir.path(), "a2")?, original_a2);

    // Verify: b1 might be rebased (no conflict), but b2 and b3 unchanged
    let _new_b1 = get_commit_hash(temp_dir.path(), "b1")?;
    // b1 could be rebased or skipped depending on conflict in b2
    // For now, just check b2 and b3 are unchanged
    assert_eq!(get_commit_hash(temp_dir.path(), "b2")?, original_b2);
    assert_eq!(get_commit_hash(temp_dir.path(), "b3")?, original_b3);

    // Verify: operation state cleared
    assert!(get_operation_state(temp_dir.path())?.is_none());

    // Verify: output mentions skipped branches
    assert!(
        stdout.contains("skipped") || stdout.contains("Skipped"),
        "Output should mention skipped branches: {}",
        stdout
    );

    Ok(())
}

// ============================================================================
// Restack Regression Tests (Always Stop on Conflicts)
// ============================================================================

/// Restack Test R1: Restack stops on current branch conflict
/// Setup: Stack main -> feature-a -> feature-b, run restack from feature-b, make feature-b conflict
/// Expected: STOP at feature-b
#[test]
fn test_restack_stops_on_current_branch_conflict() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> feature-a -> feature-b
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a")?;
    run_dm(temp_dir.path(), &["create", "feature-a", "-a", "-m", "Feature A"])?;

    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b")?;
    run_dm(temp_dir.path(), &["create", "feature-b", "-a", "-m", "Feature B"])?;

    // Go to main and create conflict with feature-b
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "b.txt", "CONFLICT", "Main conflicts with b")?;

    // Go back to feature-b
    run_git(temp_dir.path(), &["checkout", "feature-b"])?;

    // Run restack - should STOP at feature-b
    let _output = run_dm(temp_dir.path(), &["restack"])?;

    // Verify: should be in rebase state
    assert!(
        git_rebase_in_progress(temp_dir.path())?,
        "Restack should stop in rebase state (current branch conflict)"
    );

    // Verify: operation state exists
    let state = get_operation_state(temp_dir.path())?;
    assert!(state.is_some(), "Operation state should exist");

    // Verify: state indicates Restack operation
    let state = state.unwrap();
    assert!(
        state
            .get("operation_type")
            .and_then(|v| v.as_str())
            .map(|s| s == "Restack")
            .unwrap_or(false),
        "Operation type should be Restack"
    );

    Ok(())
}

/// Restack Test R2: Restack stops on ancestor conflict
/// Setup: Stack main -> feature-a -> feature-b -> feature-c, run restack from feature-c, make feature-a conflict
/// Expected: STOP at feature-a (not skip like sync does)
#[test]
fn test_restack_stops_on_ancestor_conflict() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> feature-a -> feature-b -> feature-c
    run_dm(temp_dir.path(), &["create", "feature-a"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a on feature-a")?;

    run_dm(temp_dir.path(), &["create", "feature-b"])?;
    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b on feature-b")?;

    run_dm(temp_dir.path(), &["create", "feature-c"])?;
    create_file_and_commit(temp_dir.path(), "c.txt", "c content", "Add c on feature-c")?;

    // Go to main and create conflict with feature-a
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "a.txt", "CONFLICT", "Main conflicts with a")?;

    // Go to feature-c
    run_git(temp_dir.path(), &["checkout", "feature-c"])?;

    // Get hashes to verify feature-b and feature-c were NOT rebased
    let original_b = get_commit_hash(temp_dir.path(), "feature-b")?;
    let original_c = get_commit_hash(temp_dir.path(), "feature-c")?;

    // Run restack - should STOP at feature-a
    let _output = run_dm(temp_dir.path(), &["restack"])?;

    // Verify: should be in rebase state
    assert!(
        git_rebase_in_progress(temp_dir.path())?,
        "Restack should stop in rebase state (ancestor conflict)"
    );

    // Verify: feature-b and feature-c were not yet rebased
    assert_eq!(
        get_commit_hash(temp_dir.path(), "feature-b")?,
        original_b,
        "feature-b should not be rebased yet (stopped at ancestor)"
    );
    assert_eq!(
        get_commit_hash(temp_dir.path(), "feature-c")?,
        original_c,
        "feature-c should not be rebased yet (stopped at ancestor)"
    );

    Ok(())
}

/// Restack Test R3: Restack stops on child conflict
/// Setup: Stack main -> feature-a -> feature-b -> feature-c, run restack from feature-a, make feature-c conflict
/// Expected: STOP at feature-c (not skip)
#[test]
fn test_restack_stops_on_child_conflict() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> feature-a -> feature-b -> feature-c
    run_dm(temp_dir.path(), &["create", "feature-a"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a on feature-a")?;

    run_dm(temp_dir.path(), &["create", "feature-b"])?;
    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b on feature-b")?;

    run_dm(temp_dir.path(), &["create", "feature-c"])?;
    create_file_and_commit(temp_dir.path(), "c.txt", "c content", "Add c on feature-c")?;

    // Get original hashes
    let original_a = get_commit_hash(temp_dir.path(), "feature-a")?;
    let original_b = get_commit_hash(temp_dir.path(), "feature-b")?;

    // Go to main and create conflict with feature-c
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "c.txt", "CONFLICT", "Main conflicts with c")?;

    // Go to feature-a
    run_git(temp_dir.path(), &["checkout", "feature-a"])?;

    // Run restack - should rebase a and b, then STOP at c
    let _output = run_dm(temp_dir.path(), &["restack"])?;

    // Verify: feature-a and feature-b were rebased
    let new_a = get_commit_hash(temp_dir.path(), "feature-a")?;
    let new_b = get_commit_hash(temp_dir.path(), "feature-b")?;
    assert_ne!(new_a, original_a, "feature-a should be rebased");
    assert_ne!(new_b, original_b, "feature-b should be rebased");

    // Verify: should be in rebase state on feature-c
    assert!(
        git_rebase_in_progress(temp_dir.path())?,
        "Restack should stop in rebase state (child conflict)"
    );

    Ok(())
}

/// Restack Test R4: Restack stops on first of multiple conflicts
/// Setup: Stack main -> a -> b -> c, make both a and c conflict
/// Expected: STOP at a (first conflict)
#[test]
fn test_restack_stops_on_first_of_multiple_conflicts() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> a -> b -> c
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a")?;
    run_dm(temp_dir.path(), &["create", "a", "-a", "-m", "A"])?;

    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b")?;
    run_dm(temp_dir.path(), &["create", "b", "-a", "-m", "B"])?;

    create_file_and_commit(temp_dir.path(), "c.txt", "c content", "Add c")?;
    run_dm(temp_dir.path(), &["create", "c", "-a", "-m", "C"])?;

    // Get original hashes
    let original_b = get_commit_hash(temp_dir.path(), "b")?;
    let original_c = get_commit_hash(temp_dir.path(), "c")?;

    // Go to main and create conflicts with both a and c
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "a.txt", "CONFLICT_A", "Main conflicts with a")?;
    create_file_and_commit(temp_dir.path(), "c.txt", "CONFLICT_C", "Main conflicts with c")?;

    // Go to c and run restack - should STOP at a (first conflict)
    run_git(temp_dir.path(), &["checkout", "c"])?;
    let _output = run_dm(temp_dir.path(), &["restack"])?;

    // Verify: should be in rebase state
    assert!(git_rebase_in_progress(temp_dir.path())?);

    // Verify: b and c not processed yet
    assert_eq!(get_commit_hash(temp_dir.path(), "b")?, original_b);
    assert_eq!(get_commit_hash(temp_dir.path(), "c")?, original_c);

    Ok(())
}

/// Restack Test R5: Restack with --only flag still stops on conflict
/// Setup: Stack main -> a -> b -> c, run restack --only from b, make b conflict
/// Expected: STOP at b (--only doesn't skip conflicts)
#[test]
fn test_restack_only_stops_on_current_branch_conflict() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> a -> b -> c
    run_dm(temp_dir.path(), &["create", "a"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a on a")?;

    run_dm(temp_dir.path(), &["create", "b"])?;
    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b on b")?;

    run_dm(temp_dir.path(), &["create", "c"])?;
    create_file_and_commit(temp_dir.path(), "c.txt", "c content", "Add c on c")?;

    // Go to main and create conflict with b
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "b.txt", "CONFLICT", "Main conflicts with b")?;

    // First, rebase a onto new main (so a picks up the conflicting b.txt from main)
    run_git(temp_dir.path(), &["checkout", "a"])?;
    run_git(temp_dir.path(), &["rebase", "main"])?;

    // Get hashes AFTER manual rebase (these are the "original" hashes before dm restack)
    let original_a = get_commit_hash(temp_dir.path(), "a")?;
    let original_c = get_commit_hash(temp_dir.path(), "c")?;

    // Now go to b and run restack --only - should STOP at b (conflict when rebasing onto updated a)
    run_git(temp_dir.path(), &["checkout", "b"])?;
    let _output = run_dm(temp_dir.path(), &["restack", "--only"])?;

    // Verify: should be in rebase state
    assert!(git_rebase_in_progress(temp_dir.path())?);

    // Verify: a and c not touched (--only only restacks b)
    assert_eq!(get_commit_hash(temp_dir.path(), "a")?, original_a);
    assert_eq!(get_commit_hash(temp_dir.path(), "c")?, original_c);

    Ok(())
}

/// Restack Test R6: Restack with --upstack flag still stops on child conflict
/// Setup: Stack main -> a -> b -> c, run restack --upstack from b, make c conflict
/// Expected: STOP at c (--upstack includes descendants)
#[test]
fn test_restack_upstack_stops_on_child_conflict() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> a -> b -> c
    run_dm(temp_dir.path(), &["create", "a"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a on a")?;

    run_dm(temp_dir.path(), &["create", "b"])?;
    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b on b")?;

    run_dm(temp_dir.path(), &["create", "c"])?;
    create_file_and_commit(temp_dir.path(), "c.txt", "c content", "Add c on c")?;

    // Go to main and create conflict with c
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "c.txt", "CONFLICT", "Main conflicts with c")?;

    // First, rebase a and b onto new main (so they pick up the conflicting c.txt)
    run_git(temp_dir.path(), &["checkout", "a"])?;
    run_git(temp_dir.path(), &["rebase", "main"])?;
    run_git(temp_dir.path(), &["checkout", "b"])?;
    run_git(temp_dir.path(), &["rebase", "a"])?; // Rebase b onto updated a

    // Get hash after manual rebase
    let original_a = get_commit_hash(temp_dir.path(), "a")?;
    let original_b = get_commit_hash(temp_dir.path(), "b")?;

    // Now run restack --upstack from b - should skip b (already based on a), then STOP at c
    let _output = run_dm(temp_dir.path(), &["restack", "--upstack"])?;

    // Verify: b was NOT rebased again (already based on a)
    let new_b = get_commit_hash(temp_dir.path(), "b")?;
    assert_eq!(new_b, original_b, "b should not be rebased again (already up to date)");

    // Verify: should be in rebase state on c
    assert!(git_rebase_in_progress(temp_dir.path())?);

    // Verify: a not touched (--upstack doesn't include ancestors)
    assert_eq!(get_commit_hash(temp_dir.path(), "a")?, original_a);

    Ok(())
}

/// Restack Test R7: Restack with --downstack flag still stops on ancestor conflict
/// Setup: Stack main -> a -> b -> c, run restack --downstack from c, make a conflict
/// Expected: STOP at a (--downstack includes ancestors)
#[test]
fn test_restack_downstack_stops_on_ancestor_conflict() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> a -> b -> c
    run_dm(temp_dir.path(), &["create", "a"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a on a")?;

    run_dm(temp_dir.path(), &["create", "b"])?;
    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b on b")?;

    run_dm(temp_dir.path(), &["create", "c"])?;
    create_file_and_commit(temp_dir.path(), "c.txt", "c content", "Add c on c")?;

    // Get original hashes
    let original_b = get_commit_hash(temp_dir.path(), "b")?;
    let original_c = get_commit_hash(temp_dir.path(), "c")?;

    // Go to main and create conflict with a
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "a.txt", "CONFLICT", "Main conflicts with a")?;

    // Go to c and run restack --downstack - should STOP at a
    run_git(temp_dir.path(), &["checkout", "c"])?;
    let _output = run_dm(temp_dir.path(), &["restack", "--downstack"])?;

    // Verify: should be in rebase state on a
    assert!(git_rebase_in_progress(temp_dir.path())?);

    // Verify: b and c not processed yet
    assert_eq!(get_commit_hash(temp_dir.path(), "b")?, original_b);
    assert_eq!(get_commit_hash(temp_dir.path(), "c")?, original_c);

    Ok(())
}

/// Restack Test R8: Restack from trunk still stops on branch conflict
/// Setup: Stack main -> a -> b, run restack from main, make a conflict
/// Expected: STOP at a (even from trunk, restack should stop)
#[test]
fn test_restack_from_trunk_stops_on_branch_conflict() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> a -> b
    run_dm(temp_dir.path(), &["create", "a"])?;
    create_file_and_commit(temp_dir.path(), "a.txt", "a content", "Add a on a")?;

    run_dm(temp_dir.path(), &["create", "b"])?;
    create_file_and_commit(temp_dir.path(), "b.txt", "b content", "Add b on b")?;

    // Get original hashes
    let original_b = get_commit_hash(temp_dir.path(), "b")?;

    // Go to main and create conflict with a
    run_git(temp_dir.path(), &["checkout", "main"])?;
    create_file_and_commit(temp_dir.path(), "main.txt", "main advance", "Advance main")?;
    create_file_and_commit(temp_dir.path(), "a.txt", "CONFLICT", "Main conflicts with a")?;

    // Run restack from main - should STOP at a
    let _output = run_dm(temp_dir.path(), &["restack"])?;

    // Verify: should be in rebase state
    assert!(git_rebase_in_progress(temp_dir.path())?);

    // Verify: b not processed yet
    assert_eq!(get_commit_hash(temp_dir.path(), "b")?, original_b);

    Ok(())
}
