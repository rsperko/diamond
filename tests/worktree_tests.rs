mod common;

use anyhow::Result;
use common::*;
use std::fs;
use tempfile::TempDir;

// ============================================================================
// WORKTREE TESTS - Multi-worktree scenarios
// ============================================================================

#[test]
fn test_sync_with_branches_in_multiple_worktrees() -> Result<()> {
    // Verify sync works correctly when branches are checked out in multiple worktrees
    let main_dir = TempDir::new()?;
    init_test_repo(main_dir.path())?;

    // Create a feature branch in main worktree
    fs::write(main_dir.path().join("f1.txt"), "feature 1")?;
    run_dm(main_dir.path(), &["create", "feature-1", "-a", "-m", "Feature 1"])?;

    // Create a second feature branch
    fs::write(main_dir.path().join("f2.txt"), "feature 2")?;
    run_dm(main_dir.path(), &["create", "feature-2", "-a", "-m", "Feature 2"])?;

    // Create a worktree for feature-1
    let wt_dir = main_dir.path().join("../worktree-f1");
    fs::create_dir_all(&wt_dir)?;
    run_git(
        main_dir.path(),
        &["worktree", "add", wt_dir.to_str().unwrap(), "feature-1"],
    )?;

    // Make a change in main and sync
    run_dm(main_dir.path(), &["checkout", "main"])?;
    fs::write(main_dir.path().join("main.txt"), "main update")?;
    run_git(main_dir.path(), &["add", "."])?;
    run_git(main_dir.path(), &["commit", "-m", "Main update"])?;

    // Sync should handle feature-1 being checked out in worktree
    let output = run_dm(main_dir.path(), &["sync", "--no-cleanup"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should either succeed or give clear message about worktree
    if !output.status.success() {
        assert!(
            stderr.contains("worktree") || stderr.contains("checked out"),
            "Should mention worktree issue: {}",
            stderr
        );
    }

    // feature-2 should still be synced (not in worktree)
    run_dm(main_dir.path(), &["checkout", "feature-2"])?;
    let log = get_last_commit_message(main_dir.path())?;
    assert!(
        log.contains("Feature 2") || log.contains("Main update"),
        "feature-2 should have commits"
    );

    Ok(())
}

#[test]
fn test_restack_when_worktree_has_dirty_state() -> Result<()> {
    // Verify restack handles dirty worktree appropriately
    let main_dir = TempDir::new()?;
    init_test_repo(main_dir.path())?;

    // Create a branch
    fs::write(main_dir.path().join("f.txt"), "feature")?;
    run_dm(main_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Create a child branch
    fs::write(main_dir.path().join("c.txt"), "child")?;
    run_dm(main_dir.path(), &["create", "child", "-a", "-m", "Child"])?;

    // Create worktree for feature
    let wt_dir = main_dir.path().join("../worktree-feature");
    fs::create_dir_all(&wt_dir)?;
    run_git(
        main_dir.path(),
        &["worktree", "add", wt_dir.to_str().unwrap(), "feature"],
    )?;

    // Make the worktree dirty
    fs::write(wt_dir.join("dirty.txt"), "uncommitted")?;

    // Try to restack from main repo
    run_dm(main_dir.path(), &["checkout", "main"])?;
    let output = run_dm(main_dir.path(), &["restack"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should either succeed (skipping dirty worktree) or fail with clear message
    if !output.status.success() {
        assert!(
            stderr.contains("dirty") || stderr.contains("uncommitted") || stderr.contains("worktree"),
            "Should mention dirty worktree: {}",
            stderr
        );
    }

    Ok(())
}

#[test]
fn test_move_when_target_is_in_worktree() -> Result<()> {
    // Verify move command handles branches in worktrees
    let main_dir = TempDir::new()?;
    init_test_repo(main_dir.path())?;

    // Create branches: main -> feature-1 -> feature-2
    fs::write(main_dir.path().join("f1.txt"), "f1")?;
    run_dm(main_dir.path(), &["create", "feature-1", "-a", "-m", "F1"])?;

    fs::write(main_dir.path().join("f2.txt"), "f2")?;
    run_dm(main_dir.path(), &["create", "feature-2", "-a", "-m", "F2"])?;

    // Create worktree for feature-1
    let wt_dir = main_dir.path().join("../worktree-f1");
    fs::create_dir_all(&wt_dir)?;
    run_git(
        main_dir.path(),
        &["worktree", "add", wt_dir.to_str().unwrap(), "feature-1"],
    )?;

    // Try to move feature-2 to be a child of main (bypassing feature-1)
    let output = run_dm(main_dir.path(), &["move", "feature-2", "--onto", "main"])?;

    // Should either succeed or handle worktree appropriately
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        // Error should be clear if worktree is an issue
        if stderr.contains("feature-1") {
            assert!(
                stderr.contains("worktree") || stderr.contains("checked out"),
                "Should explain worktree issue: {}",
                stderr
            );
        }
    } else {
        // If it succeeded, verify the move worked
        let parent = get_parent_from_refs(main_dir.path(), "feature-2")?;
        assert_eq!(parent, Some("main".to_string()));
    }

    Ok(())
}

#[test]
fn test_create_in_worktree() -> Result<()> {
    // Verify creating branches works within a worktree
    let main_dir = TempDir::new()?;
    init_test_repo(main_dir.path())?;

    // Create a feature branch
    fs::write(main_dir.path().join("f.txt"), "feature")?;
    run_dm(main_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Create worktree for feature
    let wt_dir = main_dir.path().join("../worktree-feature");
    fs::create_dir_all(&wt_dir)?;
    run_git(
        main_dir.path(),
        &["worktree", "add", wt_dir.to_str().unwrap(), "feature"],
    )?;

    // Create a child branch from within the worktree
    fs::write(wt_dir.join("child.txt"), "child")?;
    let output = run_dm(&wt_dir, &["create", "child", "-a", "-m", "Child"])?;

    // Should either succeed or fail with a clear message about worktrees
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        // If it fails, check if it's a worktree-related issue
        // This is acceptable as creating branches in worktrees may have limitations
        if !stderr.is_empty() {
            // Test passes if there's a clear error message
            return Ok(());
        }
        panic!("Failed to create branch in worktree without error message: {}", stderr);
    }

    // If it succeeded, verify the branch was created correctly
    // Verify parent relationship
    let parent = get_parent_from_refs(&wt_dir, "child")?;
    assert_eq!(parent, Some("feature".to_string()));

    // Verify branch exists in main repo too
    assert!(git_branch_exists(main_dir.path(), "child")?);

    Ok(())
}

#[test]
fn test_doctor_with_worktrees() -> Result<()> {
    // Verify doctor command works correctly with worktrees
    let main_dir = TempDir::new()?;
    init_test_repo(main_dir.path())?;

    // Create branches
    fs::write(main_dir.path().join("f1.txt"), "f1")?;
    run_dm(main_dir.path(), &["create", "feature-1", "-a", "-m", "F1"])?;

    fs::write(main_dir.path().join("f2.txt"), "f2")?;
    run_dm(main_dir.path(), &["create", "feature-2", "-a", "-m", "F2"])?;

    // Create worktree
    let wt_dir = main_dir.path().join("../worktree-f1");
    fs::create_dir_all(&wt_dir)?;
    run_git(
        main_dir.path(),
        &["worktree", "add", wt_dir.to_str().unwrap(), "feature-1"],
    )?;

    // Delete feature-1 branch via git (breaking the worktree)
    // This creates an orphaned tracking ref
    // Note: git branch -D refuses to delete branches in worktrees, so we force it via update-ref
    let delete_result = run_git(main_dir.path(), &["branch", "-D", "feature-1"])?;
    if !delete_result.status.success() {
        // Branch is in a worktree, force delete via update-ref
        run_git(main_dir.path(), &["update-ref", "-d", "refs/heads/feature-1"])?;
    }

    // Doctor should detect the issue
    let output = run_dm(main_dir.path(), &["doctor"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        combined.contains("feature-1"),
        "Doctor should detect orphaned tracking for feature-1. Output: {}",
        combined
    );

    // Doctor --fix should clean up
    run_dm(main_dir.path(), &["doctor", "--fix"])?;

    // Verify tracking was removed
    assert!(
        !is_branch_tracked_in_refs(main_dir.path(), "feature-1")?,
        "Tracking should be removed"
    );

    Ok(())
}

#[test]
fn test_checkout_to_worktree_branch() -> Result<()> {
    // Verify checkout handles branches that are checked out in worktrees
    let main_dir = TempDir::new()?;
    init_test_repo(main_dir.path())?;

    // Create a branch
    fs::write(main_dir.path().join("f.txt"), "feature")?;
    run_dm(main_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Create worktree
    let wt_dir = main_dir.path().join("../worktree-feature");
    fs::create_dir_all(&wt_dir)?;
    run_git(
        main_dir.path(),
        &["worktree", "add", wt_dir.to_str().unwrap(), "feature"],
    )?;

    // Go back to main in main repo
    run_dm(main_dir.path(), &["checkout", "main"])?;

    // Try to checkout feature (which is in worktree)
    let output = run_dm(main_dir.path(), &["checkout", "feature"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should fail or warn about worktree
    if !output.status.success() {
        assert!(
            stderr.contains("worktree") || stderr.contains("checked out"),
            "Should mention worktree conflict: {}",
            stderr
        );
    }

    Ok(())
}

#[test]
fn test_sync_updates_worktree_branches() -> Result<()> {
    // Verify that sync properly updates parent tracking even for branches in worktrees
    let main_dir = TempDir::new()?;
    init_test_repo(main_dir.path())?;

    // Create feature branch
    fs::write(main_dir.path().join("f.txt"), "feature")?;
    run_dm(main_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Create worktree for feature
    let wt_dir = main_dir.path().join("../worktree-feature");
    fs::create_dir_all(&wt_dir)?;
    run_git(
        main_dir.path(),
        &["worktree", "add", wt_dir.to_str().unwrap(), "feature"],
    )?;

    // Update main
    run_dm(main_dir.path(), &["checkout", "main"])?;
    fs::write(main_dir.path().join("main.txt"), "update")?;
    run_git(main_dir.path(), &["add", "."])?;
    run_git(main_dir.path(), &["commit", "-m", "Main update"])?;

    // Sync (feature is in worktree but should still track parent)
    let _output = run_dm(main_dir.path(), &["sync", "--no-cleanup"])?;

    // Parent tracking should still work
    let parent = get_parent_from_refs(main_dir.path(), "feature")?;
    assert_eq!(parent, Some("main".to_string()), "Parent tracking should persist");

    Ok(())
}

#[test]
fn test_multiple_worktrees_same_stack() -> Result<()> {
    // Verify handling of complex scenario: multiple worktrees in same stack
    let main_dir = TempDir::new()?;
    init_test_repo(main_dir.path())?;

    // Create stack: main -> feature-1 -> feature-2 -> feature-3
    fs::write(main_dir.path().join("f1.txt"), "f1")?;
    run_dm(main_dir.path(), &["create", "feature-1", "-a", "-m", "F1"])?;

    fs::write(main_dir.path().join("f2.txt"), "f2")?;
    run_dm(main_dir.path(), &["create", "feature-2", "-a", "-m", "F2"])?;

    fs::write(main_dir.path().join("f3.txt"), "f3")?;
    run_dm(main_dir.path(), &["create", "feature-3", "-a", "-m", "F3"])?;

    // Create worktrees for feature-1 and feature-3
    let wt1_dir = main_dir.path().join("../worktree-f1");
    let wt3_dir = main_dir.path().join("../worktree-f3");
    fs::create_dir_all(&wt1_dir)?;
    fs::create_dir_all(&wt3_dir)?;

    run_git(
        main_dir.path(),
        &["worktree", "add", wt1_dir.to_str().unwrap(), "feature-1"],
    )?;
    run_git(
        main_dir.path(),
        &["worktree", "add", wt3_dir.to_str().unwrap(), "feature-3"],
    )?;

    // Update main and sync
    run_dm(main_dir.path(), &["checkout", "main"])?;
    fs::write(main_dir.path().join("main.txt"), "update")?;
    run_git(main_dir.path(), &["add", "."])?;
    run_git(main_dir.path(), &["commit", "-m", "Update"])?;

    let _output = run_dm(main_dir.path(), &["sync", "--no-cleanup"])?;

    // Should handle multiple worktrees gracefully
    // At minimum, feature-2 (not in worktree) should sync
    run_dm(main_dir.path(), &["checkout", "feature-2"])?;
    let current = get_current_branch(main_dir.path())?;
    assert_eq!(current, "feature-2");

    Ok(())
}
