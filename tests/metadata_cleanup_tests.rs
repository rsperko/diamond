//! Integration tests for metadata cleanup (CRITICAL-7)
//!
//! Tests that Diamond automatically cleans up orphaned metadata refs when
//! branches are deleted via git (bypassing Diamond commands).

mod common;

use anyhow::Result;
use common::*;
use tempfile::TempDir;

/// Helper to check if a Diamond parent ref exists for a branch
fn parent_ref_exists(dir: &std::path::Path, branch: &str) -> Result<bool> {
    is_branch_tracked_in_refs(dir, branch)
}

#[test]
fn test_log_cleans_orphaned_refs() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create stack: main <- A <- B
    run_dm(dir.path(), &["create", "A"])?;
    create_file_and_commit(dir.path(), "A.txt", "A content", "Add A")?;

    run_dm(dir.path(), &["create", "B"])?;
    create_file_and_commit(dir.path(), "B.txt", "B content", "Add B")?;

    // Verify both branches are tracked
    assert!(parent_ref_exists(dir.path(), "A")?);
    assert!(parent_ref_exists(dir.path(), "B")?);

    // Delete both branches via git (bypass Diamond)
    run_git(dir.path(), &["checkout", "main"])?;
    run_git(dir.path(), &["branch", "-D", "A", "B"])?;

    // Verify branches are gone but refs still exist (orphaned)
    assert!(!git_branch_exists(dir.path(), "A")?);
    assert!(!git_branch_exists(dir.path(), "B")?);
    assert!(parent_ref_exists(dir.path(), "A")?);
    assert!(parent_ref_exists(dir.path(), "B")?);

    // Run dm log (should trigger cleanup)
    run_dm(dir.path(), &["log", "short"])?;

    // Verify refs removed
    assert!(!parent_ref_exists(dir.path(), "A")?);
    assert!(!parent_ref_exists(dir.path(), "B")?);

    Ok(())
}

#[test]
fn test_info_cleans_orphaned_refs() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create and delete branch
    run_dm(dir.path(), &["create", "feature"])?;
    create_file_and_commit(dir.path(), "feature.txt", "feature", "Add feature")?;

    assert!(parent_ref_exists(dir.path(), "feature")?);

    // Delete via git
    run_git(dir.path(), &["checkout", "main"])?;
    run_git(dir.path(), &["branch", "-D", "feature"])?;

    assert!(!git_branch_exists(dir.path(), "feature")?);
    assert!(parent_ref_exists(dir.path(), "feature")?);

    // Run dm info (should trigger cleanup)
    run_dm(dir.path(), &["info", "trunk"])?;

    // Verify ref removed
    assert!(!parent_ref_exists(dir.path(), "feature")?);

    Ok(())
}

#[test]
fn test_checkout_cleans_orphaned_refs() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create two branches
    run_dm(dir.path(), &["create", "A"])?;
    create_file_and_commit(dir.path(), "A.txt", "A", "Add A")?;

    run_dm(dir.path(), &["create", "B"])?;
    create_file_and_commit(dir.path(), "B.txt", "B", "Add B")?;

    assert!(parent_ref_exists(dir.path(), "A")?);
    assert!(parent_ref_exists(dir.path(), "B")?);

    // Delete A via git
    run_git(dir.path(), &["checkout", "main"])?;
    run_git(dir.path(), &["branch", "-D", "A"])?;

    assert!(!git_branch_exists(dir.path(), "A")?);
    assert!(parent_ref_exists(dir.path(), "A")?);

    // Run dm checkout (should trigger cleanup)
    run_dm(dir.path(), &["checkout", "B"])?;

    // Verify ref removed
    assert!(!parent_ref_exists(dir.path(), "A")?);
    // B should still exist
    assert!(parent_ref_exists(dir.path(), "B")?);

    Ok(())
}

#[test]
fn test_cleanup_handles_ide_deletion() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // IDE scenario: Create branch, delete via git branch -D (what IDEs do)
    run_dm(dir.path(), &["create", "ide-deleted"])?;
    create_file_and_commit(dir.path(), "ide.txt", "ide content", "IDE work")?;

    assert!(parent_ref_exists(dir.path(), "ide-deleted")?);

    // Simulate IDE deletion
    run_git(dir.path(), &["checkout", "main"])?;
    run_git(dir.path(), &["branch", "-D", "ide-deleted"])?;

    assert!(parent_ref_exists(dir.path(), "ide-deleted")?);

    // Next dm command cleans it up
    run_dm(dir.path(), &["log", "short"])?;

    assert!(!parent_ref_exists(dir.path(), "ide-deleted")?);

    Ok(())
}

#[test]
fn test_cleanup_handles_large_scale() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create 20 branches (testing performance)
    for i in 0..20 {
        run_dm(dir.path(), &["create", &format!("branch{}", i)])?;
        create_file_and_commit(
            dir.path(),
            &format!("file{}.txt", i),
            "content",
            &format!("Add file {}", i),
        )?;
        run_git(dir.path(), &["checkout", "main"])?;
    }

    // Verify all tracked
    for i in 0..20 {
        assert!(parent_ref_exists(dir.path(), &format!("branch{}", i))?);
    }

    // Delete all via git
    for i in 0..20 {
        run_git(dir.path(), &["branch", "-D", &format!("branch{}", i)])?;
    }

    // Run cleanup (should be fast)
    let start = std::time::Instant::now();
    run_dm(dir.path(), &["log", "short"])?;
    let duration = start.elapsed();

    // Verify all cleaned and fast
    for i in 0..20 {
        assert!(!parent_ref_exists(dir.path(), &format!("branch{}", i))?);
    }

    assert!(
        duration.as_millis() < 1000,
        "Cleanup took {}ms, expected <1000ms",
        duration.as_millis()
    );

    Ok(())
}

#[test]
fn test_cleanup_idempotent() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create and delete branch
    run_dm(dir.path(), &["create", "test-branch"])?;
    create_file_and_commit(dir.path(), "test.txt", "test", "Test")?;

    run_git(dir.path(), &["checkout", "main"])?;
    run_git(dir.path(), &["branch", "-D", "test-branch"])?;

    // Run cleanup multiple times
    run_dm(dir.path(), &["log", "short"])?;
    run_dm(dir.path(), &["log", "short"])?;
    run_dm(dir.path(), &["info", "trunk"])?;

    // Should not error, ref should stay gone
    assert!(!parent_ref_exists(dir.path(), "test-branch")?);

    Ok(())
}
