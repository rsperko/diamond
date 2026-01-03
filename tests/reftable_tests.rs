//! Integration tests for reftable repository support.
//!
//! These tests verify that Diamond works correctly on repositories using
//! the reftable format (Git 2.45+). Tests are skipped on older Git versions.
//!
//! ## Status
//!
//! Diamond now fully supports reftable repositories through subprocess fallbacks.
//! All git operations (branch, commit, rebase, etc.) work on both "files" format
//! and "reftable" format repositories.

mod common;

use anyhow::Result;
use common::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Check if git version supports reftable (2.45+)
fn git_supports_reftable() -> bool {
    let output = Command::new("git").args(["--version"]).output().ok();

    if let Some(output) = output {
        let version = String::from_utf8_lossy(&output.stdout);
        // Parse "git version 2.45.0" or similar
        if let Some(v) = version.strip_prefix("git version ") {
            let parts: Vec<&str> = v.trim().split('.').collect();
            if parts.len() >= 2 {
                let major: u32 = parts[0].parse().unwrap_or(0);
                let minor: u32 = parts[1].parse().unwrap_or(0);
                return major > 2 || (major == 2 && minor >= 45);
            }
        }
    }
    false
}

/// Initialize a reftable test repository
fn init_reftable_repo(dir: &std::path::Path) -> Result<()> {
    // Initialize git repo with reftable format (use -b main for consistency)
    let status = Command::new("git")
        .args(["init", "-b", "main", "--ref-format=reftable"])
        .current_dir(dir)
        .status()?;
    assert!(status.success(), "Failed to create reftable repo");

    // Configure git
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(dir)
        .output()?;

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(dir)
        .output()?;

    // Prevent editors from blocking tests
    Command::new("git")
        .args(["config", "core.editor", "true"])
        .current_dir(dir)
        .output()?;

    Command::new("git")
        .args(["config", "sequence.editor", "true"])
        .current_dir(dir)
        .output()?;

    // Create initial commit
    fs::write(dir.join("README.md"), "# Test Repo")?;
    Command::new("git").args(["add", "."]).current_dir(dir).output()?;

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(dir)
        .output()?;

    // Initialize diamond
    let output = Command::new(dm_binary()).args(["init"]).current_dir(dir).output()?;
    assert!(
        output.status.success(),
        "dm init failed on reftable repo: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

/// Verify the repo is actually using reftable format
fn verify_reftable_format(dir: &std::path::Path) -> Result<bool> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-ref-format"])
        .current_dir(dir)
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).trim() == "reftable")
}

// === Basic Command Tests ===
// These tests verify that all basic Diamond CLI commands work on reftable repos.
// Tests are automatically skipped on Git versions < 2.45 that don't support reftable.

#[test]
fn test_reftable_init() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;

    // Create reftable repo (use -b main for consistency)
    Command::new("git")
        .args(["init", "-b", "main", "--ref-format=reftable"])
        .current_dir(temp_dir.path())
        .status()?;

    // Configure git
    run_git(temp_dir.path(), &["config", "user.name", "Test User"])?;
    run_git(temp_dir.path(), &["config", "user.email", "test@example.com"])?;

    // Create initial commit
    fs::write(temp_dir.path().join("README.md"), "# Test")?;
    run_git(temp_dir.path(), &["add", "."])?;
    run_git(temp_dir.path(), &["commit", "-m", "Initial"])?;

    // Verify it's reftable
    assert!(verify_reftable_format(temp_dir.path())?);

    // dm init should work
    let output = run_dm(temp_dir.path(), &["init"])?;
    assert!(
        output.status.success(),
        "dm init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

#[test]
fn test_reftable_create_branch() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Verify it's reftable
    assert!(verify_reftable_format(temp_dir.path())?);

    // Create a branch
    let output = run_dm(temp_dir.path(), &["create", "feature-1"])?;
    assert!(
        output.status.success(),
        "dm create failed on reftable: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify we're on the new branch
    let current = get_current_branch(temp_dir.path())?;
    assert_eq!(current, "feature-1");

    Ok(())
}

#[test]
fn test_reftable_create_stack() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create a stack of branches
    let output = run_dm(temp_dir.path(), &["create", "feature-1"])?;
    assert!(output.status.success());

    fs::write(temp_dir.path().join("f1.txt"), "feature 1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 1 work"])?;

    let output = run_dm(temp_dir.path(), &["create", "feature-2"])?;
    assert!(output.status.success());

    fs::write(temp_dir.path().join("f2.txt"), "feature 2")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 2 work"])?;

    let output = run_dm(temp_dir.path(), &["create", "feature-3"])?;
    assert!(output.status.success());

    // Verify stack structure with dm log
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());

    let log_output = String::from_utf8_lossy(&output.stdout);
    assert!(log_output.contains("feature-1"), "Stack should contain feature-1");
    assert!(log_output.contains("feature-2"), "Stack should contain feature-2");
    assert!(log_output.contains("feature-3"), "Stack should contain feature-3");

    Ok(())
}

#[test]
fn test_reftable_navigation() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create a stack
    run_dm(temp_dir.path(), &["create", "feature-1"])?;
    run_dm(temp_dir.path(), &["create", "feature-2"])?;
    run_dm(temp_dir.path(), &["create", "feature-3"])?;

    // Navigate down
    let output = run_dm(temp_dir.path(), &["down"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-2");

    // Navigate to bottom
    let output = run_dm(temp_dir.path(), &["bottom"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-1");

    // Navigate to top
    let output = run_dm(temp_dir.path(), &["top"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-3");

    Ok(())
}

#[test]
fn test_reftable_delete_branch() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create and then delete a branch
    run_dm(temp_dir.path(), &["create", "to-delete"])?;
    assert_eq!(get_current_branch(temp_dir.path())?, "to-delete");

    let output = run_dm(temp_dir.path(), &["delete", "to-delete", "--force"])?;
    assert!(
        output.status.success(),
        "dm delete failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should be back on main
    assert_eq!(get_current_branch(temp_dir.path())?, "main");

    // Branch should be gone
    assert!(!git_branch_exists(temp_dir.path(), "to-delete")?);

    Ok(())
}

#[test]
fn test_reftable_trunk_branch() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // dm trunk should return main
    let output = run_dm(temp_dir.path(), &["trunk"])?;
    assert!(output.status.success());

    let trunk = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(trunk, "main");

    Ok(())
}

#[test]
fn test_reftable_modify_commit() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create branch with commit
    run_dm(temp_dir.path(), &["create", "feature-1"])?;
    fs::write(temp_dir.path().join("test.txt"), "initial content")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Initial feature work"])?;

    // Modify and amend
    fs::write(temp_dir.path().join("test.txt"), "updated content")?;
    let output = run_dm(temp_dir.path(), &["modify", "-a", "-m", "Updated feature work"])?;
    assert!(
        output.status.success(),
        "dm modify failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify commit message was updated
    let msg = get_last_commit_message(temp_dir.path())?;
    assert_eq!(msg, "Updated feature work");

    Ok(())
}

#[test]
fn test_reftable_restack() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create a stack
    run_dm(temp_dir.path(), &["create", "feature-1"])?;
    fs::write(temp_dir.path().join("f1.txt"), "feature 1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 1"])?;

    run_dm(temp_dir.path(), &["create", "feature-2"])?;
    fs::write(temp_dir.path().join("f2.txt"), "feature 2")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 2"])?;

    // Go back to feature-1 and amend (using a new file to avoid conflicts)
    run_dm(temp_dir.path(), &["down"])?;
    fs::write(temp_dir.path().join("f1_extra.txt"), "extra content")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 1 updated"])?;

    // Restack should work
    let output = run_dm(temp_dir.path(), &["restack"])?;
    assert!(
        output.status.success(),
        "dm restack failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

#[test]
fn test_reftable_log_output() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create some branches
    run_dm(temp_dir.path(), &["create", "feature-a"])?;
    run_dm(temp_dir.path(), &["create", "feature-b"])?;

    // Log short mode should work
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());

    // Log long mode should work
    let output = run_dm(temp_dir.path(), &["log", "long"])?;
    assert!(output.status.success());

    Ok(())
}
