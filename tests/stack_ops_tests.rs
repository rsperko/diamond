mod common;

use anyhow::Result;
use common::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

// ============================================================================
// DELETE / REPARENTING TESTS
// ============================================================================

#[test]
fn test_delete_middle_of_stack_reparents_children() -> Result<()> {
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

    // Go to f1
    run_dm(temp_dir.path(), &["checkout", "f1"])?;

    // Delete f2 (should reparent f3 to f1)
    let output = run_dm(temp_dir.path(), &["delete", "f2", "--force"])?;
    assert!(output.status.success());

    // Verify f3 is now a child of f1
    let output = run_dm(temp_dir.path(), &["children"])?;
    let children = String::from_utf8_lossy(&output.stdout);
    assert!(children.contains("f3"), "f3 should be child of f1: {}", children);

    // Verify f2 is gone
    let output = Command::new("git")
        .args(["branch", "--list", "f2"])
        .current_dir(temp_dir.path())
        .output()?;
    let branches = String::from_utf8_lossy(&output.stdout);
    assert!(!branches.contains("f2"), "f2 should be deleted");

    Ok(())
}

#[test]
fn test_delete_leaf_branch() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> f1 -> f2
    for i in 1..=2 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("{}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("f{}", i), "-a", "-m", &format!("F{}", i)],
        )?;
    }

    // Delete f2 (leaf)
    let output = run_dm(temp_dir.path(), &["delete", "f2", "--force"])?;
    assert!(output.status.success());

    // Should be back on f1
    assert_eq!(get_current_branch(temp_dir.path())?, "f1");

    // f1 should have no children now
    let output = run_dm(temp_dir.path(), &["children"])?;
    let children = String::from_utf8_lossy(&output.stdout);
    assert!(
        children.contains("(none)") || children.trim().is_empty(),
        "f1 should have no children: {}",
        children
    );

    Ok(())
}

#[test]
fn test_delete_current_branch_switches_to_parent() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create parent and child
    fs::write(temp_dir.path().join("p.txt"), "p")?;
    run_dm(temp_dir.path(), &["create", "parent", "-a", "-m", "P"])?;

    fs::write(temp_dir.path().join("c.txt"), "c")?;
    run_dm(temp_dir.path(), &["create", "child", "-a", "-m", "C"])?;

    // Delete current branch (child) by specifying name
    let output = run_dm(temp_dir.path(), &["delete", "child", "--force"])?;
    assert!(output.status.success());

    // Should now be on parent
    assert_eq!(get_current_branch(temp_dir.path())?, "parent");

    Ok(())
}

#[test]
fn test_delete_with_multiple_children_restacks_all() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> [f2a, f2b, f3]
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2a.txt"), "f2a")?;
    run_dm(temp_dir.path(), &["create", "f2a", "-a", "-m", "F2A"])?;

    run_dm(temp_dir.path(), &["checkout", "f1"])?;
    fs::write(temp_dir.path().join("f2b.txt"), "f2b")?;
    run_dm(temp_dir.path(), &["create", "f2b", "-a", "-m", "F2B"])?;

    run_dm(temp_dir.path(), &["checkout", "f1"])?;
    fs::write(temp_dir.path().join("f3.txt"), "f3")?;
    run_dm(temp_dir.path(), &["create", "f3", "-a", "-m", "F3"])?;

    // Get original commit hashes for children
    let original_f2a = Command::new("git")
        .args(["rev-parse", "f2a"])
        .current_dir(temp_dir.path())
        .output()?;
    let original_f2a = String::from_utf8_lossy(&original_f2a.stdout).trim().to_string();

    let original_f2b = Command::new("git")
        .args(["rev-parse", "f2b"])
        .current_dir(temp_dir.path())
        .output()?;
    let original_f2b = String::from_utf8_lossy(&original_f2b.stdout).trim().to_string();

    let original_f3 = Command::new("git")
        .args(["rev-parse", "f3"])
        .current_dir(temp_dir.path())
        .output()?;
    let original_f3 = String::from_utf8_lossy(&original_f3.stdout).trim().to_string();

    // Delete f1 (should restack all three children onto main)
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    let output = run_dm(temp_dir.path(), &["delete", "f1", "--force"])?;
    assert!(output.status.success());

    // Verify f1 is deleted
    let output = Command::new("git")
        .args(["branch", "--list", "f1"])
        .current_dir(temp_dir.path())
        .output()?;
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    // All children should have main as parent
    for branch in &["f2a", "f2b", "f3"] {
        run_dm(temp_dir.path(), &["checkout", branch])?;
        let output = run_dm(temp_dir.path(), &["parent"])?;
        let parent = String::from_utf8_lossy(&output.stdout);
        assert!(
            parent.contains("main"),
            "{}'s parent should be main: {}",
            branch,
            parent
        );
    }

    // Verify all children were restacked (commit hashes changed)
    let new_f2a = Command::new("git")
        .args(["rev-parse", "f2a"])
        .current_dir(temp_dir.path())
        .output()?;
    let new_f2a = String::from_utf8_lossy(&new_f2a.stdout).trim().to_string();

    let new_f2b = Command::new("git")
        .args(["rev-parse", "f2b"])
        .current_dir(temp_dir.path())
        .output()?;
    let new_f2b = String::from_utf8_lossy(&new_f2b.stdout).trim().to_string();

    let new_f3 = Command::new("git")
        .args(["rev-parse", "f3"])
        .current_dir(temp_dir.path())
        .output()?;
    let new_f3 = String::from_utf8_lossy(&new_f3.stdout).trim().to_string();

    assert_ne!(original_f2a, new_f2a, "f2a should have been restacked");
    assert_ne!(original_f2b, new_f2b, "f2b should have been restacked");
    assert_ne!(original_f3, new_f3, "f3 should have been restacked");

    Ok(())
}

// ============================================================================
// POP TESTS
// ============================================================================

#[test]
fn test_pop_deletes_branch_keeps_changes() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> feature
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Create uncommitted change
    fs::write(temp_dir.path().join("uncommitted.txt"), "uncommitted")?;

    // Pop should delete feature and preserve changes
    let output = run_dm(temp_dir.path(), &["pop"])?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Popped"));

    // Should be on main now
    let output = run_dm(temp_dir.path(), &["info", "trunk"])?;
    let _trunk = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Verify feature is gone
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(!log.contains("feature"), "feature should be deleted");

    // Uncommitted changes should be preserved
    let content = fs::read_to_string(temp_dir.path().join("uncommitted.txt"))?;
    assert_eq!(content, "uncommitted", "uncommitted changes should be preserved");

    Ok(())
}

#[test]
fn test_pop_with_children_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> middle -> leaf
    fs::write(temp_dir.path().join("m.txt"), "middle")?;
    run_dm(temp_dir.path(), &["create", "middle", "-a", "-m", "Middle"])?;

    fs::write(temp_dir.path().join("l.txt"), "leaf")?;
    run_dm(temp_dir.path(), &["create", "leaf", "-a", "-m", "Leaf"])?;

    // Go back to middle
    run_dm(temp_dir.path(), &["checkout", "middle"])?;

    // Pop should fail (has children)
    let output = run_dm(temp_dir.path(), &["pop"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("children") || stderr.contains("leaf"));

    Ok(())
}

// ============================================================================
// FOLD TESTS
// ============================================================================

#[test]
fn test_fold_combines_into_parent() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Fold f2 into f1
    let output = run_dm(temp_dir.path(), &["fold"])?;
    assert!(output.status.success());

    // Should now be on f1
    assert_eq!(get_current_branch(temp_dir.path())?, "f1");

    // f2 should be deleted
    let output = Command::new("git")
        .args(["branch", "--list", "f2"])
        .current_dir(temp_dir.path())
        .output()?;
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    // f1 should have f2's commit
    let output = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(temp_dir.path())
        .output()?;
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(log.contains("F2"), "F2 commit should be in log: {}", log);

    Ok(())
}

#[test]
fn test_fold_with_keep_flag() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> parent-name -> child-name
    fs::write(temp_dir.path().join("p.txt"), "p")?;
    run_dm(temp_dir.path(), &["create", "parent-name", "-a", "-m", "Parent"])?;

    fs::write(temp_dir.path().join("c.txt"), "c")?;
    run_dm(temp_dir.path(), &["create", "child-name", "-a", "-m", "Child"])?;

    // Fold with --keep (keep child's name)
    let output = run_dm(temp_dir.path(), &["fold", "--keep"])?;
    assert!(output.status.success());

    // Branch should be named child-name (not parent-name)
    assert_eq!(get_current_branch(temp_dir.path())?, "child-name");

    Ok(())
}

#[test]
fn test_fold_when_both_have_children() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create: main -> parent -> [child1, child2] and parent has sibling
    fs::write(temp_dir.path().join("p.txt"), "p")?;
    run_dm(temp_dir.path(), &["create", "parent", "-a", "-m", "P"])?;

    fs::write(temp_dir.path().join("c1.txt"), "c1")?;
    run_dm(temp_dir.path(), &["create", "child1", "-a", "-m", "C1"])?;

    run_dm(temp_dir.path(), &["checkout", "parent"])?;
    fs::write(temp_dir.path().join("c2.txt"), "c2")?;
    run_dm(temp_dir.path(), &["create", "child2", "-a", "-m", "C2"])?;

    // child2 now has parent as parent
    // Fold child2 into parent
    let output = run_dm(temp_dir.path(), &["fold"])?;
    assert!(output.status.success());

    // Should be on parent now
    assert_eq!(get_current_branch(temp_dir.path())?, "parent");

    // child1 should still be a child of parent
    let output = run_dm(temp_dir.path(), &["children"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("child1"));

    Ok(())
}

// ============================================================================
// SQUASH TESTS
// ============================================================================

#[test]
fn test_squash_multiple_commits() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch
    run_dm(temp_dir.path(), &["create", "feature"])?;

    // Add multiple commits using -c flag
    for i in 1..=3 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("{}", i))?;
        run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", &format!("Commit {}", i)])?;
    }

    // Verify we have 3 commits on this branch
    let output = Command::new("git")
        .args(["rev-list", "--count", "main..HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let count: u32 = String::from_utf8_lossy(&output.stdout).trim().parse()?;
    assert_eq!(count, 3, "Should have 3 commits before squash");

    // Squash with message
    let output = run_dm(temp_dir.path(), &["squash", "-m", "Combined commit"])?;
    assert!(output.status.success());

    // Verify now only 1 commit
    let output = Command::new("git")
        .args(["rev-list", "--count", "main..HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let count: u32 = String::from_utf8_lossy(&output.stdout).trim().parse()?;
    assert_eq!(count, 1, "Should have 1 commit after squash");

    // Verify message
    let msg = get_last_commit_message(temp_dir.path())?;
    assert_eq!(msg, "Combined commit");

    Ok(())
}

#[test]
fn test_squash_branch_with_single_commit() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch with just one commit
    fs::write(temp_dir.path().join("f.txt"), "f")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Single commit"])?;

    // Squash should handle single commit gracefully
    let output = run_dm(temp_dir.path(), &["squash", "-m", "Squashed"])?;
    // Should either succeed (no-op) or fail gracefully
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success() || !combined.contains("panic"),
        "Squash should handle single commit"
    );

    Ok(())
}

// ============================================================================
// RENAME TESTS
// ============================================================================

#[test]
fn test_rename_updates_stack_metadata() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> old-name -> child
    fs::write(temp_dir.path().join("o.txt"), "o")?;
    run_dm(temp_dir.path(), &["create", "old-name", "-a", "-m", "Old"])?;

    fs::write(temp_dir.path().join("c.txt"), "c")?;
    run_dm(temp_dir.path(), &["create", "child", "-a", "-m", "Child"])?;

    // Go back to old-name
    run_dm(temp_dir.path(), &["checkout", "old-name"])?;

    // Rename to new-name
    let output = run_dm(temp_dir.path(), &["rename", "new-name"])?;
    assert!(output.status.success());

    // Verify current branch is new-name
    assert_eq!(get_current_branch(temp_dir.path())?, "new-name");

    // Verify old-name no longer exists
    let output = Command::new("git")
        .args(["branch", "--list", "old-name"])
        .current_dir(temp_dir.path())
        .output()?;
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    // Verify child still has correct parent (check via dm parent)
    run_dm(temp_dir.path(), &["checkout", "child"])?;
    let output = run_dm(temp_dir.path(), &["parent"])?;
    let parent = String::from_utf8_lossy(&output.stdout);
    assert!(
        parent.contains("new-name"),
        "Child's parent should be new-name: {}",
        parent
    );

    Ok(())
}

#[test]
fn test_rename_to_existing_name_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create two branches
    fs::write(temp_dir.path().join("a.txt"), "a")?;
    run_dm(temp_dir.path(), &["create", "branch-a", "-a", "-m", "A"])?;

    run_dm(temp_dir.path(), &["checkout", "main"])?;
    fs::write(temp_dir.path().join("b.txt"), "b")?;
    run_dm(temp_dir.path(), &["create", "branch-b", "-a", "-m", "B"])?;

    // Try to rename branch-b to branch-a (already exists)
    let output = run_dm(temp_dir.path(), &["rename", "branch-a"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("exists") || stderr.contains("already"),
        "Should mention branch exists: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_rename_local_flag_works() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "f")?;
    run_dm(temp_dir.path(), &["create", "old-name", "-a", "-m", "Feature"])?;

    // Rename with --local flag (should succeed even without remote)
    let output = run_dm(temp_dir.path(), &["rename", "new-name", "--local"])?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Renamed") && stdout.contains("new-name"),
        "Should show rename: {}",
        stdout
    );

    // Verify branch was renamed
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    let log_output = String::from_utf8_lossy(&output.stdout);
    assert!(
        log_output.contains("new-name"),
        "Log should show new name: {}",
        log_output
    );
    assert!(
        !log_output.contains("old-name"),
        "Log should not show old name: {}",
        log_output
    );

    Ok(())
}

// ============================================================================
// MOVE TESTS
// ============================================================================

#[test]
fn test_move_branch_to_different_parent() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Move f2 onto main (bypassing f1)
    let output = run_dm(temp_dir.path(), &["move", "--onto", "main"])?;
    assert!(output.status.success());

    // Verify f2's parent is now main
    let output = run_dm(temp_dir.path(), &["parent"])?;
    let parent = String::from_utf8_lossy(&output.stdout);
    assert!(parent.contains("main"), "f2's parent should be main: {}", parent);

    // Verify f1 has no children
    run_dm(temp_dir.path(), &["checkout", "f1"])?;
    let output = run_dm(temp_dir.path(), &["children"])?;
    let children = String::from_utf8_lossy(&output.stdout);
    assert!(
        children.contains("(none)") || children.trim().is_empty(),
        "f1 should have no children: {}",
        children
    );

    Ok(())
}

#[test]
fn test_move_creates_cycle_rejected() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create chain: main -> a -> b -> c
    fs::write(temp_dir.path().join("a.txt"), "a")?;
    run_dm(temp_dir.path(), &["create", "branch-a", "-a", "-m", "A"])?;

    fs::write(temp_dir.path().join("b.txt"), "b")?;
    run_dm(temp_dir.path(), &["create", "branch-b", "-a", "-m", "B"])?;

    fs::write(temp_dir.path().join("c.txt"), "c")?;
    run_dm(temp_dir.path(), &["create", "branch-c", "-a", "-m", "C"])?;

    // Go to branch-a and try to move it onto branch-c (its descendant)
    run_dm(temp_dir.path(), &["checkout", "branch-a"])?;
    let output = run_dm(temp_dir.path(), &["move", "--onto", "branch-c"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cycle") || stderr.contains("descendant") || stderr.contains("Circular"),
        "Should reject cycle: {}",
        stderr
    );

    Ok(())
}

// ============================================================================
// INSERT TESTS
// ============================================================================

#[test]
fn test_insert_branch_between_parent_and_child() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> f3
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f3.txt"), "f3")?;
    run_dm(temp_dir.path(), &["create", "f3", "-a", "-m", "F3"])?;

    // Go back to f1
    run_dm(temp_dir.path(), &["checkout", "f1"])?;

    // Insert f2 between f1 and f3
    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    let output = run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2", "-i", "f3"])?;
    assert!(output.status.success());

    // Verify the stack is now main -> f1 -> f2 -> f3
    run_dm(temp_dir.path(), &["checkout", "f2"])?;

    // f2's parent should be f1
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("f1"));

    // f2's child should be f3
    let output = run_dm(temp_dir.path(), &["children"])?;
    assert!(String::from_utf8_lossy(&output.stdout).contains("f3"));

    Ok(())
}
