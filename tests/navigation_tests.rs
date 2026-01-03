mod common;

use anyhow::Result;
use common::*;
use std::fs;
use tempfile::TempDir;

// ============================================================================
// DEEPLY NESTED STACK TESTS
// ============================================================================

#[test]
fn test_deeply_nested_stack_10_levels() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a 10-level deep stack
    for i in 1..=10 {
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

    // Verify we're on level-10
    assert_eq!(get_current_branch(temp_dir.path())?, "level-10");

    // Verify log shows all levels
    let output = run_dm(temp_dir.path(), &["log", "short"])?;
    assert!(output.status.success());
    let log_output = String::from_utf8_lossy(&output.stdout);
    for i in 1..=10 {
        assert!(
            log_output.contains(&format!("level-{}", i)),
            "Log missing level-{}: {}",
            i,
            log_output
        );
    }

    Ok(())
}

#[test]
fn test_navigate_deep_stack_up_down() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create 5-level stack
    for i in 1..=5 {
        fs::write(temp_dir.path().join(format!("f{}.txt", i)), format!("{}", i))?;
        run_dm(
            temp_dir.path(),
            &["create", &format!("feature-{}", i), "-a", "-m", &format!("F{}", i)],
        )?;
    }

    // At feature-5, navigate down 3 levels
    let output = run_dm(temp_dir.path(), &["down", "3"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-2");

    // Navigate up 2 levels
    let output = run_dm(temp_dir.path(), &["up", "2"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-4");

    // Navigate to bottom
    let output = run_dm(temp_dir.path(), &["bottom"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-1");

    // Navigate to top
    let output = run_dm(temp_dir.path(), &["top"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-5");

    Ok(())
}

#[test]
fn test_navigate_down_at_trunk_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create one feature branch
    fs::write(temp_dir.path().join("f.txt"), "f")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "F"])?;

    // Go back to main
    run_dm(temp_dir.path(), &["checkout", "main"])?;

    // Try to go down from main (should fail)
    let output = run_dm(temp_dir.path(), &["down"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("root") || stderr.contains("parent"),
        "Expected 'root' or 'parent' error, got: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_navigate_up_at_leaf_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a single feature branch (no children)
    fs::write(temp_dir.path().join("f.txt"), "f")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "F"])?;

    // Try to go up from feature (should fail - no children)
    let output = run_dm(temp_dir.path(), &["up"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("child") || stderr.contains("No child"),
        "Expected 'child' error, got: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_parent_children_trunk_commands() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Test trunk command
    let output = run_dm(temp_dir.path(), &["trunk"])?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("main"));

    // Test parent command (from f2)
    let output = run_dm(temp_dir.path(), &["parent"])?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("f1"));

    // Test children command (from f1)
    run_dm(temp_dir.path(), &["checkout", "f1"])?;
    let output = run_dm(temp_dir.path(), &["children"])?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("f2"));

    Ok(())
}

#[test]
fn test_multiple_children_navigation() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create main with two children: main -> f1, main -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    run_dm(temp_dir.path(), &["checkout", "main"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Go back to main
    run_dm(temp_dir.path(), &["checkout", "main"])?;

    // Verify main has two children
    let output = run_dm(temp_dir.path(), &["children"])?;
    let children = String::from_utf8_lossy(&output.stdout);
    assert!(children.contains("f1"), "Should have f1 child: {}", children);
    assert!(children.contains("f2"), "Should have f2 child: {}", children);

    // up should go to first child alphabetically (f1)
    let output = run_dm(temp_dir.path(), &["up"])?;
    assert!(output.status.success());
    assert_eq!(
        get_current_branch(temp_dir.path())?,
        "f1",
        "Should navigate to f1 (first alphabetically)"
    );

    Ok(())
}

// ============================================================================
// CHECKOUT TESTS
// ============================================================================

#[test]
fn test_checkout_nonexistent_branch_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["checkout", "does-not-exist"])?;
    assert!(!output.status.success());

    Ok(())
}

#[test]
fn test_checkout_without_arg_lists_branches() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create some branches
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // checkout without arg - should either show interactive picker or list branches
    // Since we can't test interactive mode, just verify it doesn't crash
    let output = run_dm(temp_dir.path(), &["checkout"])?;
    // Either succeeds with a list or fails gracefully
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success() || !combined.contains("panic"),
        "checkout without arg should not panic: {}",
        combined
    );

    Ok(())
}
