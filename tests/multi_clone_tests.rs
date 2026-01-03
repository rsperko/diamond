mod common;

use anyhow::Result;
use common::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

// ============================================================================
// MULTI-CLONE VISIBILITY TESTS
// ============================================================================
//
// These tests verify that Diamond correctly isolates stacks between different
// local clones of the same repository. The key behaviors tested:
//
// 1. A fresh clone should only see trunk in `dm log`, not stacks created by others
// 2. Checking out a branch fetches its diamond parent ref from remote
// 3. After checkout, only the checked-out stack is visible (not all remote stacks)
// 4. Stacks from different users remain isolated until explicitly checked out
//
// This simulates two developers (Session 1 and Session 2) working on the same
// repository from different machines.
// ============================================================================

/// Helper to create a bare "remote" repository with an initial commit
fn init_bare_remote(path: &std::path::Path) -> Result<()> {
    // Create a temp dir for initial setup
    let setup_dir = TempDir::new()?;

    // Initialize a regular repo
    Command::new("git")
        .args(["init"])
        .current_dir(setup_dir.path())
        .output()?;

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(setup_dir.path())
        .output()?;

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(setup_dir.path())
        .output()?;

    // Create initial commit
    fs::write(setup_dir.path().join("README.md"), "# Test Repo")?;
    Command::new("git")
        .args(["add", "."])
        .current_dir(setup_dir.path())
        .output()?;

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(setup_dir.path())
        .output()?;

    // Clone as bare to create the "remote"
    Command::new("git")
        .args([
            "clone",
            "--bare",
            setup_dir.path().to_str().unwrap(),
            path.to_str().unwrap(),
        ])
        .output()?;

    Ok(())
}

/// Helper to clone from a bare remote and initialize Diamond
fn clone_and_init_dm(remote_path: &std::path::Path, local_path: &std::path::Path) -> Result<()> {
    // Clone the remote
    let output = Command::new("git")
        .args(["clone", remote_path.to_str().unwrap(), local_path.to_str().unwrap()])
        .output()?;

    assert!(
        output.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Configure git
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(local_path)
        .output()?;

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(local_path)
        .output()?;

    // Prevent editors from blocking
    Command::new("git")
        .args(["config", "core.editor", "true"])
        .current_dir(local_path)
        .output()?;

    Command::new("git")
        .args(["config", "sequence.editor", "true"])
        .current_dir(local_path)
        .output()?;

    // Initialize Diamond
    let output = run_dm(local_path, &["init"])?;
    assert!(
        output.status.success(),
        "dm init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

/// Helper to get branches shown in dm log short output
fn get_branches_from_log(dir: &std::path::Path) -> Result<Vec<String>> {
    let output = run_dm(dir, &["log", "short"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse branch names from dm log output
    // Format is like: "◯ branch-name" or "◉ branch-name" with optional indentation
    // MARKER_CURRENT = "◉" (U+25C9), MARKER_OTHER = "◯" (U+25EF)
    let mut branches = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }
        // Extract branch name after the marker (◯ or ◉)
        // Also handle the tree lines that have "│" or "├" or "└" prefixes
        let after_marker = trimmed
            .strip_prefix("◯")
            .or_else(|| trimmed.strip_prefix("◉"))
            .map(|s| s.trim_start());

        if let Some(name) = after_marker {
            // Remove any trailing annotations like "(needs restack)"
            let name = name.split_whitespace().next().unwrap_or(name);
            if !name.is_empty() {
                branches.push(name.to_string());
            }
        }
    }

    Ok(branches)
}

/// Helper to check if a specific branch appears in dm log
fn branch_in_log(dir: &std::path::Path, branch: &str) -> Result<bool> {
    let branches = get_branches_from_log(dir)?;
    Ok(branches.iter().any(|b| b == branch))
}

// ============================================================================
// TEST: Fresh clone shows only trunk
// ============================================================================

#[test]
fn test_fresh_clone_shows_only_trunk() -> Result<()> {
    // Create bare "remote" repository
    let remote_dir = TempDir::new()?;
    init_bare_remote(remote_dir.path())?;

    // Session 1: Clone, init, create stacks, and submit
    let session1_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session1_dir.path())?;

    // Create Stack 1 in Session 1
    fs::write(session1_dir.path().join("s1-f1.txt"), "Stack 1 File 1")?;
    let output = run_dm(
        session1_dir.path(),
        &["create", "stack1-level1", "-a", "-m", "Stack 1 Level 1"],
    )?;
    assert!(output.status.success(), "create stack1-level1 failed");

    fs::write(session1_dir.path().join("s1-f2.txt"), "Stack 1 File 2")?;
    let output = run_dm(
        session1_dir.path(),
        &["create", "stack1-level2", "-a", "-m", "Stack 1 Level 2"],
    )?;
    assert!(output.status.success(), "create stack1-level2 failed");

    // Push the stack (this pushes both branches and diamond refs)
    // Note: submit may fail without GitHub configured, but we need to push manually
    run_git(session1_dir.path(), &["push", "-u", "origin", "stack1-level1"])?;
    run_git(session1_dir.path(), &["push", "-u", "origin", "stack1-level2"])?;

    // Push diamond refs manually (since submit requires GitHub)
    run_git(
        session1_dir.path(),
        &[
            "push",
            "origin",
            "refs/diamond/parent/stack1-level1:refs/diamond/parent/stack1-level1",
        ],
    )?;
    run_git(
        session1_dir.path(),
        &[
            "push",
            "origin",
            "refs/diamond/parent/stack1-level2:refs/diamond/parent/stack1-level2",
        ],
    )?;

    // Session 2: Fresh clone
    let session2_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session2_dir.path())?;

    // Verify Session 2 only sees trunk (main), not the stacks from Session 1
    let branches = get_branches_from_log(session2_dir.path())?;

    assert!(
        branches.contains(&"main".to_string()),
        "Session 2 should see 'main' in dm log. Got: {:?}",
        branches
    );

    assert!(
        !branches.contains(&"stack1-level1".to_string()),
        "Session 2 should NOT see 'stack1-level1' without checkout. Got: {:?}",
        branches
    );

    assert!(
        !branches.contains(&"stack1-level2".to_string()),
        "Session 2 should NOT see 'stack1-level2' without checkout. Got: {:?}",
        branches
    );

    Ok(())
}

// ============================================================================
// TEST: Checkout fetches diamond ref and makes branch visible
// ============================================================================

#[test]
fn test_checkout_fetches_diamond_ref_and_shows_branch() -> Result<()> {
    // Create bare "remote" repository
    let remote_dir = TempDir::new()?;
    init_bare_remote(remote_dir.path())?;

    // Session 1: Clone, init, create stack, and push
    let session1_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session1_dir.path())?;

    // Create a stack
    fs::write(session1_dir.path().join("feature.txt"), "Feature content")?;
    run_dm(
        session1_dir.path(),
        &["create", "feature-branch", "-a", "-m", "Feature"],
    )?;

    // Push branch and diamond ref
    run_git(session1_dir.path(), &["push", "-u", "origin", "feature-branch"])?;
    run_git(
        session1_dir.path(),
        &[
            "push",
            "origin",
            "refs/diamond/parent/feature-branch:refs/diamond/parent/feature-branch",
        ],
    )?;

    // Session 2: Fresh clone
    let session2_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session2_dir.path())?;

    // Before checkout: branch should NOT be in dm log
    assert!(
        !branch_in_log(session2_dir.path(), "feature-branch")?,
        "Branch should NOT be visible before checkout"
    );

    // Checkout the branch
    let output = run_dm(session2_dir.path(), &["checkout", "feature-branch"])?;
    assert!(
        output.status.success(),
        "checkout failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // After checkout: branch SHOULD be in dm log
    assert!(
        branch_in_log(session2_dir.path(), "feature-branch")?,
        "Branch SHOULD be visible after checkout"
    );

    Ok(())
}

// ============================================================================
// TEST: Only checked-out stack is visible (isolation between stacks)
// ============================================================================

#[test]
fn test_only_checked_out_stack_visible() -> Result<()> {
    // Create bare "remote" repository
    let remote_dir = TempDir::new()?;
    init_bare_remote(remote_dir.path())?;

    // Session 1: Clone, init, create TWO separate stacks
    let session1_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session1_dir.path())?;

    // Create Stack 1 (2 levels)
    fs::write(session1_dir.path().join("s1-f1.txt"), "Stack 1 File 1")?;
    run_dm(session1_dir.path(), &["create", "stack1-level1", "-a", "-m", "S1L1"])?;

    fs::write(session1_dir.path().join("s1-f2.txt"), "Stack 1 File 2")?;
    run_dm(session1_dir.path(), &["create", "stack1-level2", "-a", "-m", "S1L2"])?;

    // Push Stack 1
    run_git(session1_dir.path(), &["push", "-u", "origin", "stack1-level1"])?;
    run_git(session1_dir.path(), &["push", "-u", "origin", "stack1-level2"])?;
    run_git(
        session1_dir.path(),
        &[
            "push",
            "origin",
            "refs/diamond/parent/stack1-level1:refs/diamond/parent/stack1-level1",
        ],
    )?;
    run_git(
        session1_dir.path(),
        &[
            "push",
            "origin",
            "refs/diamond/parent/stack1-level2:refs/diamond/parent/stack1-level2",
        ],
    )?;

    // Go back to main to create Stack 2
    run_dm(session1_dir.path(), &["checkout", "main"])?;

    // Create Stack 2 (1 level)
    fs::write(session1_dir.path().join("s2-f1.txt"), "Stack 2 File 1")?;
    run_dm(session1_dir.path(), &["create", "stack2-level1", "-a", "-m", "S2L1"])?;

    // Push Stack 2
    run_git(session1_dir.path(), &["push", "-u", "origin", "stack2-level1"])?;
    run_git(
        session1_dir.path(),
        &[
            "push",
            "origin",
            "refs/diamond/parent/stack2-level1:refs/diamond/parent/stack2-level1",
        ],
    )?;

    // Session 2: Fresh clone
    let session2_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session2_dir.path())?;

    // Checkout ONLY stack2-level1 (not stack1)
    let output = run_dm(session2_dir.path(), &["checkout", "stack2-level1"])?;
    assert!(output.status.success(), "checkout failed");

    // Verify: Session 2 should see stack2-level1 but NOT stack1-level1 or stack1-level2
    let branches = get_branches_from_log(session2_dir.path())?;

    assert!(
        branches.contains(&"stack2-level1".to_string()),
        "Should see checked-out branch 'stack2-level1'. Got: {:?}",
        branches
    );

    assert!(
        !branches.contains(&"stack1-level1".to_string()),
        "Should NOT see 'stack1-level1' (different stack). Got: {:?}",
        branches
    );

    assert!(
        !branches.contains(&"stack1-level2".to_string()),
        "Should NOT see 'stack1-level2' (different stack). Got: {:?}",
        branches
    );

    Ok(())
}

// ============================================================================
// TEST: Checkout child branch shows parent chain
// ============================================================================

#[test]
fn test_checkout_child_shows_parent_chain() -> Result<()> {
    // Create bare "remote" repository
    let remote_dir = TempDir::new()?;
    init_bare_remote(remote_dir.path())?;

    // Session 1: Clone, init, create a 3-level stack
    let session1_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session1_dir.path())?;

    // Create 3-level stack
    fs::write(session1_dir.path().join("l1.txt"), "Level 1")?;
    run_dm(session1_dir.path(), &["create", "level1", "-a", "-m", "L1"])?;

    fs::write(session1_dir.path().join("l2.txt"), "Level 2")?;
    run_dm(session1_dir.path(), &["create", "level2", "-a", "-m", "L2"])?;

    fs::write(session1_dir.path().join("l3.txt"), "Level 3")?;
    run_dm(session1_dir.path(), &["create", "level3", "-a", "-m", "L3"])?;

    // Push all levels and their diamond refs
    for level in ["level1", "level2", "level3"] {
        run_git(session1_dir.path(), &["push", "-u", "origin", level])?;
        run_git(
            session1_dir.path(),
            &[
                "push",
                "origin",
                &format!("refs/diamond/parent/{}:refs/diamond/parent/{}", level, level),
            ],
        )?;
    }

    // Session 2: Fresh clone
    let session2_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session2_dir.path())?;

    // Checkout ONLY the top-most branch (level3)
    let output = run_dm(session2_dir.path(), &["checkout", "level3"])?;
    assert!(output.status.success(), "checkout level3 failed");

    // Key question: Does checkout recursively fetch parent refs?
    // Based on the code, it only fetches the checked-out branch's ref.
    // So level3's parent (level2) should NOT be automatically visible.

    let branches = get_branches_from_log(session2_dir.path())?;

    // level3 should definitely be visible
    assert!(
        branches.contains(&"level3".to_string()),
        "Should see checked-out branch 'level3'. Got: {:?}",
        branches
    );

    // NOTE: This test documents CURRENT behavior.
    // level2 and level1 will NOT be visible because their diamond refs aren't fetched.
    // This may or may not be the desired UX - documenting for discussion.
    assert!(
        !branches.contains(&"level2".to_string()),
        "level2 is NOT automatically visible (parent ref not fetched). Got: {:?}",
        branches
    );

    Ok(())
}

// ============================================================================
// TEST: Multiple checkouts accumulate visibility
// ============================================================================

#[test]
fn test_multiple_checkouts_accumulate_visibility() -> Result<()> {
    // Create bare "remote" repository
    let remote_dir = TempDir::new()?;
    init_bare_remote(remote_dir.path())?;

    // Session 1: Clone, init, create two separate stacks
    let session1_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session1_dir.path())?;

    // Stack A
    fs::write(session1_dir.path().join("a.txt"), "A")?;
    run_dm(session1_dir.path(), &["create", "stack-a", "-a", "-m", "A"])?;
    run_git(session1_dir.path(), &["push", "-u", "origin", "stack-a"])?;
    run_git(
        session1_dir.path(),
        &[
            "push",
            "origin",
            "refs/diamond/parent/stack-a:refs/diamond/parent/stack-a",
        ],
    )?;

    // Go back to main for Stack B
    run_dm(session1_dir.path(), &["checkout", "main"])?;

    // Stack B
    fs::write(session1_dir.path().join("b.txt"), "B")?;
    run_dm(session1_dir.path(), &["create", "stack-b", "-a", "-m", "B"])?;
    run_git(session1_dir.path(), &["push", "-u", "origin", "stack-b"])?;
    run_git(
        session1_dir.path(),
        &[
            "push",
            "origin",
            "refs/diamond/parent/stack-b:refs/diamond/parent/stack-b",
        ],
    )?;

    // Session 2: Fresh clone
    let session2_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session2_dir.path())?;

    // Initially: no stacks visible
    let branches = get_branches_from_log(session2_dir.path())?;
    assert!(
        !branches.contains(&"stack-a".to_string()),
        "stack-a should not be visible initially"
    );
    assert!(
        !branches.contains(&"stack-b".to_string()),
        "stack-b should not be visible initially"
    );

    // Checkout stack-a
    run_dm(session2_dir.path(), &["checkout", "stack-a"])?;
    let branches = get_branches_from_log(session2_dir.path())?;
    assert!(
        branches.contains(&"stack-a".to_string()),
        "stack-a should be visible after checkout"
    );
    assert!(
        !branches.contains(&"stack-b".to_string()),
        "stack-b should NOT be visible yet"
    );

    // Checkout stack-b
    run_dm(session2_dir.path(), &["checkout", "stack-b"])?;
    let branches = get_branches_from_log(session2_dir.path())?;
    assert!(
        branches.contains(&"stack-a".to_string()),
        "stack-a should STILL be visible"
    );
    assert!(
        branches.contains(&"stack-b".to_string()),
        "stack-b should NOW be visible"
    );

    Ok(())
}

// ============================================================================
// TEST: Diamond refs are pushed correctly by submit (if configured)
// ============================================================================

#[test]
fn test_diamond_refs_exist_on_remote_after_push() -> Result<()> {
    // Create bare "remote" repository
    let remote_dir = TempDir::new()?;
    init_bare_remote(remote_dir.path())?;

    // Session 1: Clone, init, create branch, push
    let session1_dir = TempDir::new()?;
    clone_and_init_dm(remote_dir.path(), session1_dir.path())?;

    fs::write(session1_dir.path().join("f.txt"), "content")?;
    run_dm(session1_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Push branch
    run_git(session1_dir.path(), &["push", "-u", "origin", "feature"])?;

    // Push diamond ref
    let output = run_git(
        session1_dir.path(),
        &[
            "push",
            "origin",
            "refs/diamond/parent/feature:refs/diamond/parent/feature",
        ],
    )?;
    assert!(output.status.success(), "Failed to push diamond ref");

    // Verify the ref exists on remote
    let output = Command::new("git")
        .args(["show-ref", "refs/diamond/parent/feature"])
        .current_dir(remote_dir.path())
        .output()?;

    assert!(output.status.success(), "Diamond ref should exist on remote after push");

    Ok(())
}
