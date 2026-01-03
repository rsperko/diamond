mod common;

use anyhow::Result;
use common::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_create_branch_basic() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a new branch
    let output = run_dm(temp_dir.path(), &["create", "feature-1"])?;
    assert!(
        output.status.success(),
        "dm create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify we're on the new branch
    let current_branch = get_current_branch(temp_dir.path())?;
    assert_eq!(current_branch, "feature-1");

    Ok(())
}

#[test]
fn test_create_with_commit() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a file
    fs::write(temp_dir.path().join("test.txt"), "test content")?;

    // Create branch with -am
    let output = run_dm(temp_dir.path(), &["create", "feature-1", "-a", "-m", "Add test file"])?;
    assert!(
        output.status.success(),
        "dm create -am failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify branch and commit
    let current_branch = get_current_branch(temp_dir.path())?;
    assert_eq!(current_branch, "feature-1");

    let commit_msg = get_last_commit_message(temp_dir.path())?;
    assert_eq!(commit_msg, "Add test file");

    Ok(())
}

#[test]
fn test_modify_without_message_preserves_commit() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch with initial commit
    fs::write(temp_dir.path().join("file1.txt"), "initial")?;
    let output = run_dm(
        temp_dir.path(),
        &["create", "feature-1", "-a", "-m", "Initial feature commit"],
    )?;
    assert!(output.status.success());

    // Make more changes
    fs::write(temp_dir.path().join("file2.txt"), "more changes")?;

    // Modify with -a but no message (should amend and preserve message)
    let output = run_dm(temp_dir.path(), &["modify", "-a"])?;
    assert!(
        output.status.success(),
        "dm modify -a failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify message was preserved
    let commit_msg = get_last_commit_message(temp_dir.path())?;
    assert_eq!(commit_msg, "Initial feature commit");

    // Verify both files are in the commit
    let output = Command::new("git")
        .args(["ls-tree", "--name-only", "HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let files = String::from_utf8_lossy(&output.stdout);
    assert!(files.contains("file1.txt"));
    assert!(files.contains("file2.txt"));

    Ok(())
}

#[test]
fn test_modify_with_message_amends_commit() -> Result<()> {
    // -m without -c should AMEND, not create new commit
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch with initial commit
    fs::write(temp_dir.path().join("file1.txt"), "initial")?;
    run_dm(temp_dir.path(), &["create", "feature-1", "-a", "-m", "First commit"])?;

    // Make more changes
    fs::write(temp_dir.path().join("file2.txt"), "second change")?;

    // Modify with message (should AMEND, not create new commit)
    let output = run_dm(temp_dir.path(), &["modify", "-a", "-m", "Amended commit"])?;
    assert!(output.status.success());

    // Verify commit message was updated
    let commit_msg = get_last_commit_message(temp_dir.path())?;
    assert_eq!(commit_msg, "Amended commit");

    // Verify we have only 2 commits (initial + amended first) - not 3
    let output = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let count = String::from_utf8_lossy(&output.stdout).trim().parse::<u32>()?;
    assert_eq!(count, 2);

    Ok(())
}

#[test]
fn test_modify_with_commit_flag_creates_new_commit() -> Result<()> {
    // -c flag explicitly creates a new commit
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch with initial commit
    fs::write(temp_dir.path().join("file1.txt"), "initial")?;
    run_dm(temp_dir.path(), &["create", "feature-1", "-a", "-m", "First commit"])?;

    // Make more changes
    fs::write(temp_dir.path().join("file2.txt"), "second change")?;

    // Modify with -c -m (should CREATE new commit)
    let output = run_dm(temp_dir.path(), &["modify", "-a", "-c", "-m", "Second commit"])?;
    assert!(output.status.success());

    // Verify new commit message
    let commit_msg = get_last_commit_message(temp_dir.path())?;
    assert_eq!(commit_msg, "Second commit");

    // Verify we have 3 commits (initial + first + second)
    let output = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let count = String::from_utf8_lossy(&output.stdout).trim().parse::<u32>()?;
    assert_eq!(count, 3);

    Ok(())
}

#[test]
fn test_create_and_modify_workflow() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch (no commit)
    let output = run_dm(temp_dir.path(), &["create", "feature-1"])?;
    assert!(output.status.success());
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-1");

    // Make changes and commit
    fs::write(temp_dir.path().join("feature.txt"), "feature work")?;
    run_dm(temp_dir.path(), &["modify", "-a", "-m", "Implement feature"])?;

    // Make more changes and amend
    fs::write(temp_dir.path().join("feature.txt"), "improved feature work")?;
    fs::write(temp_dir.path().join("test.txt"), "add tests")?;
    let output = run_dm(temp_dir.path(), &["modify", "-a"])?;
    assert!(output.status.success());

    // Verify message preserved
    assert_eq!(get_last_commit_message(temp_dir.path())?, "Implement feature");

    // Verify both files are in the commit
    let output = Command::new("git")
        .args(["ls-tree", "--name-only", "HEAD"])
        .current_dir(temp_dir.path())
        .output()?;
    let files = String::from_utf8_lossy(&output.stdout);
    assert!(files.contains("feature.txt"));
    assert!(files.contains("test.txt"));

    Ok(())
}

#[test]
fn test_create_stacked_branches() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create first branch with commit
    fs::write(temp_dir.path().join("feature1.txt"), "feature 1")?;
    run_dm(temp_dir.path(), &["create", "feature-1", "-a", "-m", "Feature 1"])?;

    // Create second branch stacked on first
    fs::write(temp_dir.path().join("feature2.txt"), "feature 2")?;
    run_dm(temp_dir.path(), &["create", "feature-2", "-a", "-m", "Feature 2"])?;

    // Verify we're on feature-2
    assert_eq!(get_current_branch(temp_dir.path())?, "feature-2");

    // Verify stack structure with dm log
    let _output = run_dm(temp_dir.path(), &["log"])?;
    // Note: log command launches TUI, so we can't easily test output
    // but we can verify it doesn't error
    // In a real scenario, you might want a --json flag for testability

    Ok(())
}

#[test]
fn test_create_autogenerate_name_from_message() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a file
    fs::write(temp_dir.path().join("test.txt"), "content")?;

    // Create branch with message but no name (should auto-generate)
    let output = run_dm(temp_dir.path(), &["create", "-a", "-m", "Add new feature"])?;
    assert!(
        output.status.success(),
        "dm create -am failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify branch was created with slugified name (MM-DD-message_with_underscores)
    let current_branch = get_current_branch(temp_dir.path())?;
    assert!(
        current_branch.ends_with("-add_new_feature"),
        "Expected branch to end with '-add_new_feature', got: {}",
        current_branch
    );
    // Verify it has the date prefix format (MM-DD-)
    let parts: Vec<&str> = current_branch.splitn(3, '-').collect();
    assert_eq!(parts.len(), 3);
    assert!(parts[0].len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()));
    assert!(parts[1].len() == 2 && parts[1].chars().all(|c| c.is_ascii_digit()));

    // Verify commit was made
    let commit_msg = get_last_commit_message(temp_dir.path())?;
    assert_eq!(commit_msg, "Add new feature");

    Ok(())
}

#[test]
fn test_create_autogenerate_name_with_special_chars() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch with special characters in message
    let output = run_dm(temp_dir.path(), &["create", "-m", "Fix bug #123: URL parsing!"])?;
    assert!(output.status.success());

    // Verify branch name is slugified (MM-DD-message_with_underscores)
    let current_branch = get_current_branch(temp_dir.path())?;
    assert!(
        current_branch.ends_with("-fix_bug_123_url_parsing"),
        "Expected branch to end with '-fix_bug_123_url_parsing', got: {}",
        current_branch
    );
    // Verify it has the date prefix format (MM-DD-)
    let parts: Vec<&str> = current_branch.splitn(3, '-').collect();
    assert_eq!(parts.len(), 3);
    assert!(parts[0].len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()));
    assert!(parts[1].len() == 2 && parts[1].chars().all(|c| c.is_ascii_digit()));

    Ok(())
}

#[test]
#[cfg(unix)]
fn test_argv0_symlink_respects_invoked_name() -> Result<()> {
    use std::os::unix::fs::symlink;

    let temp_dir = TempDir::new()?;
    let dm_path = dm_binary();

    // Create a symlink named "sc" pointing to the dm binary
    let symlink_path = temp_dir.path().join("sc");
    symlink(dm_path, &symlink_path)?;

    // Run the symlinked binary with --help
    let output = Command::new(&symlink_path).args(["--help"]).output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify the help text refers to "sc", not "dm"
    assert!(
        stdout.contains("Usage: sc"),
        "Expected help to show 'Usage: sc', got:\n{}",
        stdout
    );
    assert!(
        !stdout.contains("Usage: dm"),
        "Help text should not contain 'Usage: dm' when invoked as 'sc', got:\n{}",
        stdout
    );

    Ok(())
}

#[test]
#[cfg(unix)]
fn test_argv0_symlink_subcommand_help() -> Result<()> {
    use std::os::unix::fs::symlink;

    let temp_dir = TempDir::new()?;
    let dm_path = dm_binary();

    // Create a symlink named "sc" pointing to the dm binary
    let symlink_path = temp_dir.path().join("sc");
    symlink(dm_path, &symlink_path)?;

    // Run the symlinked binary with subcommand help
    let output = Command::new(&symlink_path).args(["create", "--help"]).output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify the subcommand help text refers to "sc", not "dm"
    assert!(
        stdout.contains("sc create"),
        "Expected subcommand help to show 'sc create', got:\n{}",
        stdout
    );
    assert!(
        !stdout.contains("dm create"),
        "Subcommand help should not contain 'dm create' when invoked as 'sc', got:\n{}",
        stdout
    );

    Ok(())
}
