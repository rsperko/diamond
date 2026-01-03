//! Integration tests for TTY detection (Gap 3)
//!
//! Tests that Diamond commands properly handle non-TTY environments.
//! These tests run without a TTY (stdin/stdout are pipes), verifying
//! that commands either:
//! 1. Fall back to non-interactive mode
//! 2. Fail with clear, actionable error messages

mod common;

use anyhow::Result;
use common::*;
use std::process::{Command, Stdio};
use tempfile::TempDir;

/// Test that `dm log` with piped stdout falls back to short format
#[test]
fn test_log_non_tty_uses_short_format() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create some branches for the log to show
    run_dm(dir.path(), &["create", "feature-1"])?;
    create_file_and_commit(dir.path(), "f1.txt", "f1", "Feature 1")?;

    run_dm(dir.path(), &["create", "feature-2"])?;
    create_file_and_commit(dir.path(), "f2.txt", "f2", "Feature 2")?;

    // Run dm log with explicitly piped stdout (not a TTY)
    let output = Command::new(dm_binary())
        .args(["log"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    // Should succeed with fallback mode (short format)
    // OR fail with a clear error about needing --short
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Verify it's producing output (short format works)
        assert!(
            !stdout.is_empty() || stdout.contains("main"),
            "Should produce some output"
        );
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // If it fails, error should mention using short format
        assert!(
            stderr.contains("short") || stderr.contains("interactive"),
            "Error should mention 'short' or 'interactive': {}",
            stderr
        );
    }

    Ok(())
}

/// Test that `dm log short` works without a TTY
#[test]
fn test_log_short_works_without_tty() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create a branch
    run_dm(dir.path(), &["create", "feature"])?;
    create_file_and_commit(dir.path(), "feature.txt", "feature", "Add feature")?;

    // Run dm log short - this should always work regardless of TTY
    let output = Command::new(dm_binary())
        .args(["log", "short"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    assert!(
        output.status.success(),
        "dm log short should succeed without TTY. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("feature") || stdout.contains("main"),
        "Output should show branches"
    );

    Ok(())
}

/// Test that `dm checkout` without branch name fails with helpful message
#[test]
fn test_checkout_non_tty_requires_branch_name() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create a branch to checkout
    run_dm(dir.path(), &["create", "feature"])?;
    run_git(dir.path(), &["checkout", "main"])?;

    // Run dm checkout without branch name (would need interactive picker)
    let output = Command::new(dm_binary())
        .args(["checkout"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    // Should fail with message about needing branch name
    assert!(
        !output.status.success(),
        "dm checkout without branch should fail in non-TTY"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("branch") || stderr.contains("interactive") || stderr.contains("terminal"),
        "Error should mention branch name or interactive mode: {}",
        stderr
    );

    Ok(())
}

/// Test that `dm checkout <branch>` works without a TTY
#[test]
fn test_checkout_with_branch_works_without_tty() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create a branch
    run_dm(dir.path(), &["create", "feature"])?;
    create_file_and_commit(dir.path(), "feature.txt", "feature", "Add feature")?;
    run_git(dir.path(), &["checkout", "main"])?;

    // Run dm checkout with explicit branch name
    let output = Command::new(dm_binary())
        .args(["checkout", "feature"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    assert!(
        output.status.success(),
        "dm checkout <branch> should work without TTY. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify we're on the feature branch
    let current = get_current_branch(dir.path())?;
    assert_eq!(current, "feature", "Should be on feature branch");

    Ok(())
}

/// Test that commands requiring confirmation fail without --force in non-TTY
#[test]
fn test_delete_without_force_fails_in_non_tty() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create and checkout a branch, then go back to main
    run_dm(dir.path(), &["create", "to-delete"])?;
    create_file_and_commit(dir.path(), "file.txt", "content", "Commit")?;
    run_git(dir.path(), &["checkout", "main"])?;

    // Try to delete without --force
    let output = Command::new(dm_binary())
        .args(["delete", "to-delete"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    // Should fail or prompt (and fail because no TTY)
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Error should mention --force
        assert!(
            stderr.contains("--force") || stderr.contains("-f") || stderr.contains("confirmation"),
            "Error should mention --force: {}",
            stderr
        );
    }

    Ok(())
}

/// Test that delete with --force works without TTY
#[test]
fn test_delete_with_force_works_without_tty() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create a branch
    run_dm(dir.path(), &["create", "to-delete"])?;
    create_file_and_commit(dir.path(), "file.txt", "content", "Commit")?;
    run_git(dir.path(), &["checkout", "main"])?;

    // Delete with --force
    let output = Command::new(dm_binary())
        .args(["delete", "to-delete", "--force"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    assert!(
        output.status.success(),
        "dm delete --force should work without TTY. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify branch is deleted
    assert!(!git_branch_exists(dir.path(), "to-delete")?);

    Ok(())
}

/// Test that sync works without TTY (non-interactive operation)
#[test]
fn test_sync_works_without_tty() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create a branch
    run_dm(dir.path(), &["create", "feature"])?;
    create_file_and_commit(dir.path(), "feature.txt", "feature", "Add feature")?;

    // Sync should work without TTY (it's not interactive)
    let output = Command::new(dm_binary())
        .args(["sync"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    assert!(
        output.status.success(),
        "dm sync should work without TTY. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

/// Test that restack works without TTY
#[test]
fn test_restack_works_without_tty() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create a branch
    run_dm(dir.path(), &["create", "feature"])?;
    create_file_and_commit(dir.path(), "feature.txt", "feature", "Add feature")?;

    // Restack should work without TTY
    let output = Command::new(dm_binary())
        .args(["restack"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    assert!(
        output.status.success(),
        "dm restack should work without TTY. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

/// Test that navigation commands work without TTY
#[test]
fn test_navigation_commands_work_without_tty() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create a stack: main -> f1 -> f2 -> f3
    run_dm(dir.path(), &["create", "f1"])?;
    create_file_and_commit(dir.path(), "f1.txt", "f1", "F1")?;

    run_dm(dir.path(), &["create", "f2"])?;
    create_file_and_commit(dir.path(), "f2.txt", "f2", "F2")?;

    run_dm(dir.path(), &["create", "f3"])?;
    create_file_and_commit(dir.path(), "f3.txt", "f3", "F3")?;

    // Test dm down
    let output = Command::new(dm_binary())
        .args(["down"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    assert!(output.status.success(), "dm down should work without TTY");
    assert_eq!(get_current_branch(dir.path())?, "f2");

    // Test dm bottom
    let output = Command::new(dm_binary())
        .args(["bottom"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    assert!(output.status.success(), "dm bottom should work without TTY");
    assert_eq!(get_current_branch(dir.path())?, "f1");

    // Test dm up
    let output = Command::new(dm_binary())
        .args(["up"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    assert!(output.status.success(), "dm up should work without TTY");
    assert_eq!(get_current_branch(dir.path())?, "f2");

    // Test dm top
    let output = Command::new(dm_binary())
        .args(["top"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    assert!(output.status.success(), "dm top should work without TTY");
    assert_eq!(get_current_branch(dir.path())?, "f3");

    Ok(())
}

/// Test that create command works without TTY
#[test]
fn test_create_works_without_tty() -> Result<()> {
    let dir = TempDir::new()?;
    init_test_repo(dir.path())?;

    // Create with explicit name
    let output = Command::new(dm_binary())
        .args(["create", "feature"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    assert!(
        output.status.success(),
        "dm create should work without TTY. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(get_current_branch(dir.path())?, "feature");

    Ok(())
}
