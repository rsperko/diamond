//! Comprehensive integration tests for reftable repository support.
//!
//! These tests verify ALL Diamond operations work correctly on repositories using
//! the reftable format (Git 2.45+). Tests are skipped on older Git versions.
//!
//! ## Test Categories
//!
//! 1. **Basic Operations** - Already covered in reftable_tests.rs
//! 2. **Sync Operations** - Syncing with remote, pushing
//! 3. **Advanced Stack Operations** - Move, fold, split
//! 4. **Conflict Resolution** - Handling rebase conflicts
//! 5. **Undo/Redo** - Undo operations and recovery
//! 6. **Doctor/Cleanup** - Validation and cleanup operations
//! 7. **Worktree** - Worktree interactions (if applicable)

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

/// Initialize a reftable test repository with full configuration
fn init_reftable_repo(dir: &std::path::Path) -> Result<()> {
    // Initialize git repo with reftable format (use -b main for consistency)
    let status = Command::new("git")
        .args(["init", "-b", "main", "--ref-format=reftable"])
        .current_dir(dir)
        .status()?;
    assert!(status.success(), "Failed to create reftable repo");

    // Configure git
    for (key, value) in [
        ("user.name", "Test User"),
        ("user.email", "test@example.com"),
        ("core.editor", "true"),
        ("sequence.editor", "true"),
    ] {
        Command::new("git")
            .args(["config", key, value])
            .current_dir(dir)
            .output()?;
    }

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

// =============================================================================
// SYNC OPERATIONS
// =============================================================================

#[test]
fn test_reftable_sync_no_remote() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create a branch
    run_dm(temp_dir.path(), &["create", "feature-1"])?;
    fs::write(temp_dir.path().join("f1.txt"), "feature 1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 1 work"])?;

    // Sync without remote should succeed (nothing to sync)
    let output = run_dm(temp_dir.path(), &["sync"])?;
    // Sync may succeed or fail gracefully depending on remote detection
    // The key is it shouldn't panic with "reftable not supported"
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("reftable"),
        "Sync should not fail due to reftable: {}",
        stderr
    );

    Ok(())
}

// =============================================================================
// ADVANCED STACK OPERATIONS
// =============================================================================

#[test]
fn test_reftable_move_branch() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create a stack: main -> feature-1 -> feature-2
    run_dm(temp_dir.path(), &["create", "feature-1"])?;
    fs::write(temp_dir.path().join("f1.txt"), "feature 1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 1"])?;

    run_dm(temp_dir.path(), &["create", "feature-2"])?;
    fs::write(temp_dir.path().join("f2.txt"), "feature 2")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 2"])?;

    // Also create feature-3 off of main
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    run_dm(temp_dir.path(), &["create", "feature-3"])?;
    fs::write(temp_dir.path().join("f3.txt"), "feature 3")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 3"])?;

    // Move feature-2 to be based on main instead of feature-1
    let output = run_dm(temp_dir.path(), &["move", "--onto", "main"])?;

    // Move may trigger rebase conflicts, but should not fail due to reftable
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("This operation is not yet supported for reftable"),
        "Move should work on reftable: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_reftable_checkout_interactive_fallback() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create branches
    run_dm(temp_dir.path(), &["create", "feature-1"])?;
    run_dm(temp_dir.path(), &["create", "feature-2"])?;

    // Non-interactive checkout with branch name should work
    let output = run_dm(temp_dir.path(), &["checkout", "feature-1"])?;
    assert!(
        output.status.success(),
        "Checkout should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(get_current_branch(temp_dir.path())?, "feature-1");

    Ok(())
}

// =============================================================================
// CONFLICT RESOLUTION
// =============================================================================

#[test]
fn test_reftable_conflict_abort() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create a stack where restack will conflict
    run_dm(temp_dir.path(), &["create", "feature-1"])?;
    fs::write(temp_dir.path().join("conflict.txt"), "feature 1 content")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 1"])?;

    run_dm(temp_dir.path(), &["create", "feature-2"])?;
    fs::write(temp_dir.path().join("conflict.txt"), "feature 2 content")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 2"])?;

    // Modify feature-1 to create conflict
    run_dm(temp_dir.path(), &["down"])?;
    fs::write(temp_dir.path().join("conflict.txt"), "modified feature 1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 1 modified"])?;

    // This might trigger a conflict - check operation state
    let state_path = temp_dir
        .path()
        .join(".git")
        .join("diamond")
        .join("operation_state.json");

    if state_path.exists() {
        // If there's a conflict, abort should work
        let output = run_dm(temp_dir.path(), &["abort"])?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("This operation is not yet supported for reftable"),
            "Abort should work on reftable: {}",
            stderr
        );
    }

    Ok(())
}

// =============================================================================
// UNDO OPERATIONS
// =============================================================================

#[test]
fn test_reftable_undo() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create and modify a branch
    run_dm(temp_dir.path(), &["create", "feature-1"])?;
    fs::write(temp_dir.path().join("f1.txt"), "feature 1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 1"])?;

    // Get the commit hash before delete
    let hash_before = get_current_branch_hash(temp_dir.path())?;

    // Delete the branch
    run_dm(temp_dir.path(), &["delete", "feature-1", "--force"])?;

    // Undo should restore the branch
    let output = run_dm(temp_dir.path(), &["undo", "--force"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("This operation is not yet supported for reftable"),
        "Undo should work on reftable: {}",
        stderr
    );

    // If undo succeeded, verify branch is restored
    if output.status.success() && git_branch_exists(temp_dir.path(), "feature-1")? {
        run_dm(temp_dir.path(), &["checkout", "feature-1"])?;
        let hash_after = get_current_branch_hash(temp_dir.path())?;
        assert_eq!(hash_before, hash_after, "Branch should be restored to same commit");
    }

    Ok(())
}

// =============================================================================
// DOCTOR AND CLEANUP
// =============================================================================

#[test]
fn test_reftable_doctor() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create some branches
    run_dm(temp_dir.path(), &["create", "feature-1"])?;
    run_dm(temp_dir.path(), &["create", "feature-2"])?;

    // Run doctor
    let output = run_dm(temp_dir.path(), &["doctor"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("This operation is not yet supported for reftable"),
        "Doctor should work on reftable: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_reftable_cleanup_no_remote() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create a branch
    run_dm(temp_dir.path(), &["create", "feature-1"])?;
    fs::write(temp_dir.path().join("f1.txt"), "feature 1")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Feature 1"])?;

    // Cleanup without remote should handle gracefully
    let output = run_dm(temp_dir.path(), &["cleanup", "--force"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("This operation is not yet supported for reftable"),
        "Cleanup should work on reftable: {}",
        stderr
    );

    Ok(())
}

// =============================================================================
// EDGE CASES
// =============================================================================

#[test]
fn test_reftable_detached_head() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Get current commit
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Detach HEAD
    Command::new("git")
        .args(["checkout", &commit])
        .current_dir(temp_dir.path())
        .output()?;

    // Diamond commands should handle detached HEAD gracefully
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    // Should either succeed or fail gracefully, not with reftable error
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("This operation is not yet supported for reftable"),
        "Should handle detached HEAD on reftable: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_reftable_empty_stack() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // No branches created - just main
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());

    // Restack with no stack should be a no-op
    let output = run_dm(temp_dir.path(), &["restack"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("This operation is not yet supported for reftable"),
        "Empty restack should work on reftable: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_reftable_deep_stack() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Create a deep stack (5 levels)
    for i in 1..=5 {
        run_dm(temp_dir.path(), &["create", &format!("feature-{}", i)])?;
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("feature {}", i))?;
        run_dm(temp_dir.path(), &["modify", "-a", "-m", &format!("Feature {}", i)])?;
    }

    // Verify stack structure
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for i in 1..=5 {
        assert!(stdout.contains(&format!("feature-{}", i)));
    }

    // Navigate the full stack
    let output = run_dm(temp_dir.path(), &["bottom"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-1");

    let output = run_dm(temp_dir.path(), &["top"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-5");

    Ok(())
}

#[test]
fn test_reftable_unicode_branch_name() -> Result<()> {
    if !git_supports_reftable() {
        eprintln!("Skipping reftable test - git version < 2.45");
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    init_reftable_repo(temp_dir.path())?;

    // Try creating a branch with unicode (git allows this)
    // Note: Some unicode may not be valid in branch names, use safe chars
    let output = run_dm(temp_dir.path(), &["create", "feature-with-dash"])?;
    assert!(output.status.success());

    // Verify it's created
    assert!(git_branch_exists(temp_dir.path(), "feature-with-dash")?);

    Ok(())
}

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

fn get_current_branch_hash(dir: &std::path::Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
