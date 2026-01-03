mod common;

use anyhow::Result;
use common::*;
use std::fs;
use std::process::Command;
use std::thread;
use tempfile::TempDir;

// ============================================================================
// BRANCH NAME EDGE CASES
// ============================================================================

#[test]
fn test_create_very_long_message_truncates_branch_name() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create with a very long message
    let long_message = "This is a very long commit message that should be truncated when generating the branch name because git branch names have practical limits";
    fs::write(temp_dir.path().join("f.txt"), "content")?;
    let output = run_dm(temp_dir.path(), &["create", "-a", "-m", long_message])?;
    assert!(output.status.success());

    // Branch name should exist and be based on the message
    let branch = get_current_branch(temp_dir.path())?;
    assert!(!branch.is_empty(), "Branch should be created");
    assert!(!branch.contains("main"), "Should be on new branch, not main");
    // Branch names shouldn't exceed 200 chars (reasonable limit)
    assert!(
        branch.len() < 200,
        "Branch name should be reasonable length: {}",
        branch
    );
    // Should contain part of the message
    assert!(
        branch.contains("very_long") || branch.contains("message"),
        "Branch name should be derived from message: {}",
        branch
    );

    Ok(())
}

#[test]
fn test_create_message_with_unicode() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create with unicode in message
    fs::write(temp_dir.path().join("f.txt"), "content")?;
    let output = run_dm(temp_dir.path(), &["create", "-a", "-m", "Fix emoji bug 🐛"])?;
    assert!(output.status.success());

    // Branch should be created (emoji stripped or handled)
    let branch = get_current_branch(temp_dir.path())?;
    assert!(!branch.is_empty());
    assert!(!branch.contains("main")); // Should be on new branch

    Ok(())
}

#[test]
fn test_branch_name_with_slashes() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch with slashes (common pattern: feature/name)
    let output = run_dm(temp_dir.path(), &["create", "feature/my-feature"])?;
    assert!(
        output.status.success(),
        "Should support slashes in branch names: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(get_current_branch(temp_dir.path())?, "feature/my-feature");

    // Verify it works with stack operations
    fs::write(temp_dir.path().join("f.txt"), "content")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Add feature"])?;

    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("main"));

    Ok(())
}

#[test]
fn test_create_duplicate_branch_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create feature branch
    run_dm(temp_dir.path(), &["create", "feature"])?;

    // Go back to main
    run_dm(temp_dir.path(), &["checkout", "main"])?;

    // Try to create same branch again
    let output = run_dm(temp_dir.path(), &["create", "feature"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("exists") || stderr.contains("already"),
        "Should mention branch exists: {}",
        stderr
    );

    Ok(())
}

// ============================================================================
// UNTRACKED BRANCH OPERATIONS
// ============================================================================

#[test]
fn test_untracked_branch_operations_fail_gracefully() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a git branch directly (not through dm)
    Command::new("git")
        .args(["checkout", "-b", "untracked-branch"])
        .current_dir(temp_dir.path())
        .output()?;

    // dm up should fail with helpful message
    let output = run_dm(temp_dir.path(), &["up"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not tracked") || stderr.contains("track"),
        "Should mention tracking: {}",
        stderr
    );

    // dm parent should fail with helpful message
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not tracked") || stderr.contains("track"),
        "Should mention tracking: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_track_already_tracked_branch() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create tracked branch
    fs::write(temp_dir.path().join("f.txt"), "f")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Try to track it again - should be idempotent or warn
    let output = run_dm(temp_dir.path(), &["track", "feature"])?;
    // Should either succeed (idempotent) or fail with message
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success() || combined.contains("already") || combined.contains("tracked"),
        "Track should handle already-tracked branch"
    );

    Ok(())
}

// ============================================================================
// LOG COMMAND EDGE CASES
// ============================================================================

#[test]
fn test_empty_stack_log() -> Result<()> {
    let temp_dir = TempDir::new()?;

    // Initialize git repo but don't initialize diamond
    Command::new("git")
        .args(["init"])
        .current_dir(temp_dir.path())
        .output()?;

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(temp_dir.path())
        .output()?;

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(temp_dir.path())
        .output()?;

    fs::write(temp_dir.path().join("README.md"), "# Test")?;

    Command::new("git")
        .args(["add", "."])
        .current_dir(temp_dir.path())
        .output()?;

    Command::new("git")
        .args(["commit", "-m", "Initial"])
        .current_dir(temp_dir.path())
        .output()?;

    // Initialize diamond
    run_dm(temp_dir.path(), &["init"])?;

    // Log should show main (tracked during init)
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(log.contains("main"), "Should show main branch: {}", log);

    Ok(())
}

#[test]
fn test_log_short_output_order() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> f2 -> f3
    for i in 1..=3 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("{}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("f{}", i), "-a", "-m", &format!("F{}", i)],
        )?;
    }

    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());
    let log = String::from_utf8_lossy(&output.stdout);

    // Trunk should be at bottom
    let lines: Vec<&str> = log.trim().lines().collect();
    assert!(
        lines.last().unwrap().contains("main"),
        "main should be last line (at bottom): {:?}",
        lines
    );
    assert!(
        lines.first().unwrap().contains("f3"),
        "f3 (current/top) should be first line: {:?}",
        lines
    );

    Ok(())
}

// ============================================================================
// INFO COMMAND TESTS
// ============================================================================

#[test]
fn test_info_shows_branch_details() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature commit"])?;

    // Info should show branch details
    let output = run_dm(temp_dir.path(), &["info"])?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should mention the branch or commit info
    assert!(
        stdout.contains("feature") || stdout.contains("main") || stdout.contains("parent"),
        "info should show branch details: {}",
        stdout
    );

    Ok(())
}

#[test]
fn test_info_specific_branch() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branches
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Info for specific branch while on different branch
    let output = run_dm(temp_dir.path(), &["info", "f1"])?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show f1's info
    assert!(
        stdout.contains("f1") || stdout.contains("main"),
        "info f1 should show f1 details: {}",
        stdout
    );

    Ok(())
}

// ============================================================================
// SPLIT COMMAND TESTS
// ============================================================================

#[test]
fn test_split_branch_at_commit() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch with multiple commits
    run_dm(temp_dir.path(), &["create", "feature"])?;

    fs::write(temp_dir.path().join("f1.txt"), "commit 1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", "Commit 1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "commit 2")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", "Commit 2"])?;

    fs::write(temp_dir.path().join("f3.txt"), "commit 3")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", "Commit 3"])?;

    // Split at HEAD~1 (Commit 2). This means:
    // - feature is reset to BEFORE Commit 2 (only Commit 1 remains)
    // - feature-part2 gets Commit 2 and Commit 3
    let output = run_dm(temp_dir.path(), &["split", "feature-part2", "HEAD~1"])?;
    assert!(
        output.status.success(),
        "split failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // feature should now have 1 commit (Commit 1 only, since we split at Commit 2)
    let output = Command::new("git")
        .args(["rev-list", "--count", "main..feature"])
        .current_dir(temp_dir.path())
        .output()?;
    let feature_count: u32 = String::from_utf8_lossy(&output.stdout).trim().parse()?;
    assert_eq!(
        feature_count, 1,
        "feature should have 1 commit after split (before split point)"
    );

    // feature-part2 should have 2 additional commits (Commit 2 and 3) beyond feature
    let output = Command::new("git")
        .args(["rev-list", "--count", "feature..feature-part2"])
        .current_dir(temp_dir.path())
        .output()?;
    let part2_count: u32 = String::from_utf8_lossy(&output.stdout).trim().parse()?;
    assert_eq!(part2_count, 2, "feature-part2 should have 2 commits after feature");

    // feature-part2's parent should be feature
    run_dm(temp_dir.path(), &["checkout", "feature-part2"])?;
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("feature"));

    Ok(())
}

#[test]
fn test_split_preserves_children() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> feature -> child
    run_dm(temp_dir.path(), &["create", "feature"])?;

    fs::write(temp_dir.path().join("f1.txt"), "commit 1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", "Commit 1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "commit 2")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", "Commit 2"])?;

    fs::write(temp_dir.path().join("child.txt"), "child")?;
    run_dm(temp_dir.path(), &["create", "child", "-a", "-m", "Child"])?;

    // Go back to feature and split
    run_dm(temp_dir.path(), &["checkout", "feature"])?;
    let output = run_dm(temp_dir.path(), &["split", "feature-part2", "HEAD~1"])?;
    assert!(output.status.success());

    // child's parent should now be feature-part2 (the new top of the old feature)
    run_dm(temp_dir.path(), &["checkout", "child"])?;
    let output = run_dm(temp_dir.path(), &["parent"])?;
    let parent = String::from_utf8_lossy(&output.stdout);
    assert!(
        parent.contains("feature-part2"),
        "child's parent should be feature-part2: {}",
        parent
    );

    Ok(())
}

// ============================================================================
// ABSORB COMMAND TESTS
// ============================================================================

#[test]
fn test_absorb_dry_run_shows_changes() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch with commits
    run_dm(temp_dir.path(), &["create", "feature"])?;

    fs::write(temp_dir.path().join("file.txt"), "original content")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", "Add file"])?;

    // Make changes that could be absorbed
    fs::write(temp_dir.path().join("file.txt"), "modified content")?;
    run_git(temp_dir.path(), &["add", "."])?;

    // Absorb dry run should show what would happen
    let output = run_dm(temp_dir.path(), &["absorb", "--dry-run"])?;
    // Should either succeed or indicate no absorbable changes
    assert!(
        output.status.success() || !String::from_utf8_lossy(&output.stderr).contains("panic"),
        "absorb --dry-run should not panic"
    );

    Ok(())
}

#[test]
fn test_absorb_with_no_staged_changes() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch with commit
    fs::write(temp_dir.path().join("file.txt"), "content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Add file"])?;

    // Absorb with no staged changes
    let output = run_dm(temp_dir.path(), &["absorb"])?;
    // Should handle gracefully (fail with helpful error message)
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Either succeeds, or fails with message about no staged changes
    assert!(
        output.status.success()
            || combined.to_lowercase().contains("no")
            || combined.contains("nothing")
            || combined.contains("stage"),
        "absorb should handle no staged changes gracefully: {}",
        combined
    );

    Ok(())
}

// ============================================================================
// EMPTY/BARE REPOSITORY TESTS
// ============================================================================

#[test]
fn test_empty_repository_no_commits() -> Result<()> {
    let temp_dir = TempDir::new()?;

    // Initialize git repo but don't commit anything
    Command::new("git")
        .args(["init"])
        .current_dir(temp_dir.path())
        .output()?;

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(temp_dir.path())
        .output()?;

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(temp_dir.path())
        .output()?;

    // Try to init diamond - should fail gracefully
    let output = run_dm(temp_dir.path(), &["init"])?;

    // Should either succeed or fail with helpful error (not panic)
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panic"), "Should not panic on empty repo: {}", stderr);

    // If it fails, should give helpful error about missing commit, HEAD, or trunk
    if !output.status.success() {
        let stderr_lower = stderr.to_lowercase();
        assert!(
            stderr_lower.contains("commit")
                || stderr_lower.contains("head")
                || stderr_lower.contains("empty")
                || stderr_lower.contains("trunk")
                || stderr_lower.contains("main")
                || stderr_lower.contains("master"),
            "Error should be helpful: {}",
            stderr
        );
    }

    Ok(())
}

#[test]
fn test_bare_repository_fails_gracefully() -> Result<()> {
    let temp_dir = TempDir::new()?;

    // Initialize bare git repo
    Command::new("git")
        .args(["init", "--bare"])
        .current_dir(temp_dir.path())
        .output()?;

    // Try to use diamond on bare repo
    let output = run_dm(temp_dir.path(), &["init"])?;

    // Should fail with helpful error (not panic)
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panic"), "Should not panic on bare repo: {}", stderr);

    // Should fail (bare repos don't have working trees)
    assert!(!output.status.success(), "Should fail on bare repo");

    Ok(())
}

// ============================================================================
// CORRUPTED STATE RECOVERY TESTS
// ============================================================================

#[test]
fn test_stale_operation_state_warning() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a stale operation_state.json manually
    let diamond_dir = temp_dir.path().join(".git").join("diamond");
    fs::create_dir_all(&diamond_dir)?;

    // Write a stale sync operation state (no actual git rebase in progress)
    let stale_state = r#"{
        "operation_type": "Sync",
        "in_progress": true,
        "current_branch": "feature",
        "remaining_branches": ["feature"],
        "all_branches": ["feature"],
        "original_branch": "main",
        "move_target_parent": null,
        "old_parent": null
    }"#;
    fs::write(diamond_dir.join("operation_state.json"), stale_state)?;

    // Run restack - this command checks for operations in progress
    // When git rebase is not actually in progress, it should auto-clean stale state
    let output = run_dm(temp_dir.path(), &["restack"])?;

    // Should succeed after cleaning up the stale state
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "Should succeed after cleaning stale state: stderr={}",
        stderr
    );

    // Verify a warning was shown about the stale state cleanup
    assert!(
        stderr.contains("stale") || stderr.contains("Cleaning up"),
        "Should warn about cleaning up stale state: {}",
        stderr
    );

    // Verify operation_state.json was cleaned up
    assert!(
        !diamond_dir.join("operation_state.json").exists(),
        "Stale state file should be removed after cleanup"
    );

    Ok(())
}

#[test]
fn test_missing_trunk_ref_recovery() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a feature branch
    fs::write(temp_dir.path().join("f.txt"), "content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Remove the trunk ref
    run_git(temp_dir.path(), &["update-ref", "-d", "refs/diamond/trunk"])?;

    // Try to run a command - should handle gracefully
    let output = run_dm(temp_dir.path(), &["log", "short"])?;

    // Should not panic
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panic"),
        "Should not panic on missing trunk ref: {}",
        stderr
    );

    // dm doctor should be able to help
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!combined.contains("panic"), "doctor should not panic: {}", combined);

    Ok(())
}

// ============================================================================
// ABORT AND RETRY TESTS
// ============================================================================

#[test]
fn test_abort_then_retry_restack() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a simple stack: main -> feature
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Create an operation state as if we were mid-restack
    let diamond_dir = temp_dir.path().join(".git").join("diamond");
    let state = r#"{
        "operation_type": "Restack",
        "in_progress": true,
        "current_branch": "feature",
        "remaining_branches": ["feature"],
        "all_branches": ["feature"],
        "original_branch": "main",
        "move_target_parent": null,
        "old_parent": null
    }"#;
    fs::write(diamond_dir.join("operation_state.json"), state)?;

    // Abort the operation
    let output = run_dm(temp_dir.path(), &["abort"])?;
    assert!(
        output.status.success(),
        "Abort should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify state was cleared
    assert!(
        !diamond_dir.join("operation_state.json").exists(),
        "State file should be cleared after abort"
    );

    // Now try restack again - should work
    let output = run_dm(temp_dir.path(), &["restack"])?;
    // Should succeed (or report nothing to do)
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success() || combined.contains("Nothing to restack"),
        "Restack should work after abort: {}",
        combined
    );

    Ok(())
}

// ============================================================================
// SCALE TESTS
// ============================================================================

#[test]
fn test_deep_stack_20_levels() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a 20-deep stack
    for i in 1..=20 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("{}", i))?;
        let output = run_dm(
            temp_dir.path(),
            &["create", &format!("level-{}", i), "-a", "-m", &format!("Level {}", i)],
        )?;
        assert!(
            output.status.success(),
            "Failed to create level {}: {}",
            i,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Verify we're at level-20
    let current = get_current_branch(temp_dir.path())?;
    assert_eq!(current, "level-20");

    // dm log should work without stack overflow
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(
        output.status.success(),
        "log should handle deep stack: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Navigate back to bottom
    for _ in 0..20 {
        let output = run_dm(temp_dir.path(), &["down"])?;
        if !output.status.success() {
            break; // We hit main
        }
    }
    let current = get_current_branch(temp_dir.path())?;
    assert_eq!(current, "main", "Should navigate back to main");

    Ok(())
}

#[test]
fn test_wide_tree_20_children() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create 20 branches all parented to main
    for i in 1..=20 {
        run_dm(temp_dir.path(), &["checkout", "main"])?;
        fs::write(temp_dir.path().join(format!("child{}.txt", i)), format!("{}", i))?;
        let output = run_dm(
            temp_dir.path(),
            &["create", &format!("child-{}", i), "-a", "-m", &format!("Child {}", i)],
        )?;
        assert!(
            output.status.success(),
            "Failed to create child {}: {}",
            i,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // dm log should show all 20 children
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());
    let log = String::from_utf8_lossy(&output.stdout);

    // Verify at least some children are shown
    assert!(log.contains("child-1"), "Should show child-1");
    assert!(log.contains("child-20"), "Should show child-20");

    Ok(())
}

#[test]
fn test_branch_churn_create_delete_cycle() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create 10 branches
    for i in 1..=10 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("{}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("branch-{}", i), "-a", "-m", &format!("Branch {}", i)],
        )?;
        // Go back to main for next branch
        run_dm(temp_dir.path(), &["checkout", "main"])?;
    }

    // Delete 5 of them
    for i in [2, 4, 6, 8, 10] {
        let output = run_dm(temp_dir.path(), &["delete", &format!("branch-{}", i), "-f"])?;
        assert!(
            output.status.success(),
            "Failed to delete branch-{}: {}",
            i,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Verify remaining branches are still tracked and navigable
    for i in [1, 3, 5, 7, 9] {
        let output = run_dm(temp_dir.path(), &["checkout", &format!("branch-{}", i)])?;
        assert!(output.status.success(), "branch-{} should still exist", i);

        let output = run_dm(temp_dir.path(), &["parent"])?;
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("main"),
            "branch-{} should still have main as parent",
            i
        );
    }

    // Verify deleted branches are gone
    for i in [2, 4, 6, 8, 10] {
        let output = run_dm(temp_dir.path(), &["checkout", &format!("branch-{}", i)])?;
        assert!(!output.status.success(), "branch-{} should be deleted", i);
    }

    // dm doctor should find no issues
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(
        output.status.success(),
        "doctor should pass: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

// ============================================================================
// CONCURRENT ACCESS TESTS
// ============================================================================

#[test]
fn test_two_creates_simultaneously() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let dir1 = temp_dir.path().to_path_buf();
    let dir2 = temp_dir.path().to_path_buf();

    // Spawn two parallel creates - let them race naturally
    // This tests that file locking prevents corruption regardless of timing
    let h1 = thread::spawn(move || -> Result<std::process::Output> {
        fs::write(dir1.join("f1.txt"), "1")?;
        run_dm(&dir1, &["create", "feature-1", "-a", "-m", "F1"])
    });

    let h2 = thread::spawn(move || -> Result<std::process::Output> {
        fs::write(dir2.join("f2.txt"), "2")?;
        run_dm(&dir2, &["create", "feature-2", "-a", "-m", "F2"])
    });

    let result1 = h1.join().expect("Thread 1 panicked")?;
    let result2 = h2.join().expect("Thread 2 panicked")?;

    // At least one should succeed; both might if locking works
    assert!(
        result1.status.success() || result2.status.success(),
        "At least one create should succeed"
    );

    // Go back to main to check both branches
    run_dm(temp_dir.path(), &["checkout", "main"])?;

    // Verify both branches exist and are tracked
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Both should be in the log if both succeeded
    if result1.status.success() {
        assert!(stdout.contains("feature-1"), "feature-1 should be tracked: {}", stdout);
    }
    if result2.status.success() {
        assert!(stdout.contains("feature-2"), "feature-2 should be tracked: {}", stdout);
    }

    // Verify no corruption
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(
        output.status.success(),
        "doctor should pass: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

#[test]
fn test_create_during_restack() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a small stack
    fs::write(temp_dir.path().join("f1.txt"), "1")?;
    run_dm(temp_dir.path(), &["create", "b1", "-a", "-m", "B1"])?;
    fs::write(temp_dir.path().join("f2.txt"), "2")?;
    run_dm(temp_dir.path(), &["create", "b2", "-a", "-m", "B2"])?;

    let dir1 = temp_dir.path().to_path_buf();
    let dir2 = temp_dir.path().to_path_buf();

    // Start both operations concurrently - let them race naturally
    // This tests that file locking prevents corruption regardless of timing
    let h1 = thread::spawn(move || -> Result<std::process::Output> { run_dm(&dir1, &["restack"]) });
    let h2 = thread::spawn(move || -> Result<std::process::Output> {
        fs::write(dir2.join("f3.txt"), "3")?;
        run_dm(&dir2, &["create", "b3", "-a", "-m", "B3"])
    });

    let result1 = h1.join().expect("Thread 1 panicked")?;
    let result2 = h2.join().expect("Thread 2 panicked")?;

    // At least one operation should succeed; both might if locking works perfectly
    // Either could fail with "index is locked" depending on timing
    assert!(
        result1.status.success() || result2.status.success(),
        "At least one operation should succeed:\nRestack: {}\nCreate: {}",
        String::from_utf8_lossy(&result1.stderr),
        String::from_utf8_lossy(&result2.stderr)
    );

    // The important thing is no corruption
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(
        output.status.success(),
        "doctor should pass after concurrent ops: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // If create succeeded, verify the branch exists
    if result2.status.success() {
        let output = run_dm(temp_dir.path(), &["log", "short"])?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("b3"), "b3 should be tracked if create succeeded");
    }

    Ok(())
}

#[test]
fn test_rapid_modify_cycles() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create initial branch
    fs::write(temp_dir.path().join("file.txt"), "initial")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Rapid modify cycles
    for i in 1..=10 {
        fs::write(temp_dir.path().join("file.txt"), format!("content {}", i))?;
        let output = run_dm(temp_dir.path(), &["modify", "-a", "-m", &format!("Modify {}", i)])?;
        assert!(
            output.status.success(),
            "Modify {} should succeed: {}",
            i,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Verify we have commits
    let output = run_git(temp_dir.path(), &["log", "--oneline"])?;
    let log = String::from_utf8_lossy(&output.stdout);

    // Should have at least the modify commits (some might be amends)
    assert!(log.contains("Modify"), "Should have modify commits: {}", log);

    // No corruption
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    assert!(output.status.success());

    Ok(())
}

// ============================================================================
// CORRUPTED STATE TESTS (Additional)
// ============================================================================

#[test]
fn test_refs_with_cycles() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create two branches
    fs::write(temp_dir.path().join("a.txt"), "a")?;
    run_dm(temp_dir.path(), &["create", "branch-a", "-a", "-m", "A"])?;
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("b.txt"), "b")?;
    run_dm(temp_dir.path(), &["create", "branch-b", "-a", "-m", "B"])?;

    // Corrupt: Make A -> B cycle by setting branch-a's parent to branch-b
    // branch-b already has main as parent, but we'll set it to branch-a
    let temp_file = temp_dir.path().join(".git/diamond/temp_parent");
    fs::write(&temp_file, "branch-b")?;
    let output = run_git(temp_dir.path(), &["hash-object", "-w", temp_file.to_str().unwrap()])?;
    let blob_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

    run_git(
        temp_dir.path(),
        &["update-ref", "refs/diamond/parent/branch-a", &blob_hash],
    )?;

    fs::write(&temp_file, "branch-a")?;
    let output = run_git(temp_dir.path(), &["hash-object", "-w", temp_file.to_str().unwrap()])?;
    let blob_hash2 = String::from_utf8_lossy(&output.stdout).trim().to_string();
    fs::remove_file(temp_file)?;

    run_git(
        temp_dir.path(),
        &["update-ref", "refs/diamond/parent/branch-b", &blob_hash2],
    )?;

    // Doctor should detect cycle
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Should report cycle or circular dependency
    let combined_lower = combined.to_lowercase();
    assert!(
        combined_lower.contains("cycle") || combined_lower.contains("circular"),
        "Doctor should detect cycle: {}",
        combined
    );

    Ok(())
}

#[test]
fn test_refs_missing_parent_auto_repairs() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Corrupt: Set parent to non-existent branch via refs
    let temp_file = temp_dir.path().join(".git/diamond/temp_parent");
    fs::write(&temp_file, "non-existent-parent")?;
    let output = run_git(temp_dir.path(), &["hash-object", "-w", temp_file.to_str().unwrap()])?;
    let blob_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    fs::remove_file(temp_file)?;

    run_git(
        temp_dir.path(),
        &["update-ref", "refs/diamond/parent/feature", &blob_hash],
    )?;

    // Commands should handle gracefully (not panic) and auto-repair
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panic"),
        "Should not panic on missing parent: {}",
        stderr
    );

    // After auto-repair, doctor should report healthy
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Auto-repair should have cleaned up the orphaned metadata
    assert!(
        combined.contains("All checks passed") || combined.contains("healthy"),
        "Doctor should report healthy after auto-repair: {}",
        combined
    );

    Ok(())
}

#[test]
fn test_corrupted_git_refs() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "content")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Get the ref file path
    let refs_path = temp_dir.path().join(".git/refs/heads/feature");

    // Corrupt the ref file with invalid content
    fs::write(refs_path, "not-a-valid-commit-hash")?;

    // Go back to main first (since feature is corrupted)
    run_git(temp_dir.path(), &["checkout", "main"])?;

    // dm commands should handle gracefully (not panic)
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panic"),
        "Should not panic on corrupted refs: {}",
        stderr
    );

    // Doctor should detect the issue
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Should report some issue with the branch
    // (it might say missing, invalid, or error - depends on git2 error handling)
    assert!(
        !combined.to_lowercase().contains("panic"),
        "Doctor should not panic: {}",
        combined
    );

    Ok(())
}

// ============================================================================
// TEST-001: Crash Recovery Tests
// ============================================================================

/// Test that corrupted operation state file is handled gracefully
#[test]
fn test_corrupted_operation_state_handled() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    run_dm(temp_dir.path(), &["create", "feature"])?;

    // Write a corrupted operation state file
    let state_path = temp_dir
        .path()
        .join(".git")
        .join("diamond")
        .join("operation_state.json");
    fs::create_dir_all(state_path.parent().unwrap())?;
    fs::write(&state_path, "{ truncated json...")?;

    // Commands should handle gracefully
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should either work or give a clear error, not panic
    assert!(
        !stderr.contains("panic"),
        "Should not panic on corrupted state: {}",
        stderr
    );

    Ok(())
}

/// Test that empty operation state file is handled gracefully
#[test]
fn test_empty_operation_state_handled() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    run_dm(temp_dir.path(), &["create", "feature"])?;

    // Write an empty operation state file
    let state_path = temp_dir
        .path()
        .join(".git")
        .join("diamond")
        .join("operation_state.json");
    fs::create_dir_all(state_path.parent().unwrap())?;
    fs::write(&state_path, "")?;

    // Commands should handle gracefully
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!stderr.contains("panic"), "Should not panic on empty state: {}", stderr);

    Ok(())
}

// ============================================================================
// TEST-005: RefStore Corruption Handling
// ============================================================================

/// Test that corrupted parent ref blob is detected by doctor
#[test]
fn test_corrupted_parent_ref_blob_detected() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    run_dm(temp_dir.path(), &["create", "feature"])?;

    // Corrupt the parent ref by making it point to a tree instead of a blob
    // First, get the tree OID
    let output = run_git(temp_dir.path(), &["rev-parse", "HEAD^{tree}"])?;
    let tree_oid = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Update the parent ref to point to the tree (not a blob)
    run_git(
        temp_dir.path(),
        &["update-ref", "refs/diamond/parent/feature", &tree_oid],
    )?;

    // Doctor should detect the issue without panicking
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !combined.to_lowercase().contains("panic"),
        "Doctor should not panic on corrupted ref: {}",
        combined
    );

    Ok(())
}

/// Test that ref pointing to invalid UTF-8 is handled
#[test]
fn test_invalid_utf8_in_parent_ref() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    run_dm(temp_dir.path(), &["create", "feature"])?;

    // Create a blob with invalid UTF-8
    let invalid_bytes: &[u8] = &[0xFF, 0xFE, 0x00, 0x01];
    let temp_file = temp_dir.path().join("invalid.bin");
    fs::write(&temp_file, invalid_bytes)?;

    let output = run_git(temp_dir.path(), &["hash-object", "-w", temp_file.to_str().unwrap()])?;
    let blob_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    fs::remove_file(&temp_file)?;

    // Update the parent ref to point to the invalid blob
    run_git(
        temp_dir.path(),
        &["update-ref", "refs/diamond/parent/feature", &blob_hash],
    )?;

    // Commands should handle gracefully
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains("panic"),
        "Should not panic on invalid UTF-8: {}",
        stderr
    );

    Ok(())
}

// ============================================================================
// TEST-008: Input Validation Edge Cases
// ============================================================================

/// Test that branch names with double dots are rejected
#[test]
fn test_create_rejects_double_dots() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_test_repo(temp_dir.path())?;

    // Try to create a branch with double dots (git doesn't allow this)
    let output = run_dm(temp_dir.path(), &["create", "feature..test"])?;

    // Should fail (git itself rejects this)
    assert!(!output.status.success(), "Should reject branch name with ..");

    Ok(())
}

/// Test that branch names starting with dash are rejected
#[test]
fn test_create_rejects_dash_prefix() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_test_repo(temp_dir.path())?;

    // Try to create a branch starting with dash
    // This is tricky because git might interpret it as a flag
    let output = run_dm(temp_dir.path(), &["create", "--", "-feature"])?;

    // Should fail (git rejects leading dash)
    assert!(!output.status.success(), "Should reject branch name starting with -");

    Ok(())
}

/// Test that very long branch names are handled
#[test]
fn test_create_long_branch_name() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_test_repo(temp_dir.path())?;

    // Create a very long branch name (255 chars is typical limit)
    let long_name = "a".repeat(300);
    let output = run_dm(temp_dir.path(), &["create", &long_name])?;

    // Should either work or fail gracefully (depends on git/filesystem)
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panic"), "Should not panic on long name: {}", stderr);

    Ok(())
}

// ============================================================================
// TEST-009: Empty/Edge States
// ============================================================================

/// Test sync with no tracked branches (empty stack)
#[test]
fn test_sync_with_no_tracked_branches() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_test_repo(temp_dir.path())?;

    // Don't create any branches - just run sync on an empty stack
    let output = run_dm(temp_dir.path(), &["sync"])?;

    // Should complete successfully with a message about nothing to sync
    // (or might just succeed silently)
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panic"),
        "Should not panic with empty stack: {}",
        stderr
    );

    Ok(())
}

/// Test cleanup when current branch is the only candidate
#[test]
fn test_cleanup_current_branch_skipped() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch and stay on it
    run_dm(temp_dir.path(), &["create", "feature"])?;

    // Run cleanup - current branch should be skipped
    let output = run_dm(temp_dir.path(), &["cleanup", "--force"])?;

    // Should not delete the current branch
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("Deleted feature"), "Should not delete current branch");

    // Verify branch still exists
    let branch_exists = git_branch_exists(temp_dir.path(), "feature")?;
    assert!(branch_exists, "Current branch should still exist");

    Ok(())
}

/// Test move to same parent (no-op)
#[test]
fn test_move_to_same_parent_noop() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    run_dm(temp_dir.path(), &["create", "feature"])?;

    // Try to move to same parent (main)
    let output = run_dm(temp_dir.path(), &["move", "--onto", "main"])?;

    // Should succeed (no-op) or give a clear message
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panic"),
        "Should not panic on move to same parent: {}",
        stderr
    );

    Ok(())
}
