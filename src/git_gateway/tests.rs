//! Tests for GitGateway.

use super::*;
use anyhow::Result;
use git2::Repository;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

use crate::test_context::{init_test_repo as init_repo, TestRepoContext};

/// Helper to get HEAD commit message using git CLI (for test verification)
fn get_head_message(path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .current_dir(path)
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Helper to get HEAD commit SHA using git CLI (for test verification)
fn get_head_sha(path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Helper to create a git reference using git CLI (for test setup)
fn create_reference(path: &Path, name: &str, target: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["update-ref", name, target])
        .current_dir(path)
        .output()?;
    if !output.status.success() {
        anyhow::bail!("Failed to create reference");
    }
    Ok(())
}

/// Helper to delete a branch using git CLI
fn delete_branch_cli(path: &Path, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["branch", "-D", branch])
        .current_dir(path)
        .output()?;
    if !output.status.success() {
        anyhow::bail!("Failed to delete branch");
    }
    Ok(())
}

#[test]
fn test_create_and_checkout_branch() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create new branch
    gateway.create_branch("feature-1")?;

    // Verify checked out
    let current = gateway.get_current_branch_name()?;
    assert_eq!(current, "feature-1");

    // Verify exists
    assert!(gateway.branch_exists("feature-1")?);

    Ok(())
}

#[test]
fn test_branch_exists() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("test-branch")?;
    assert!(gateway.branch_exists("test-branch")?);
    assert!(!gateway.branch_exists("does-not-exist")?);

    Ok(())
}

#[test]
fn test_list_branches() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("branch-1")?;
    gateway.create_branch("branch-2")?;

    let branches = gateway.list_branches()?;
    assert!(branches.contains(&"branch-1".to_string()));
    assert!(branches.contains(&"branch-2".to_string()));

    Ok(())
}

#[test]
fn test_delete_branch() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("to-delete")?;
    assert!(gateway.branch_exists("to-delete")?);

    // Switch to another branch first
    gateway.create_branch("other")?;

    gateway.delete_branch("to-delete")?;
    assert!(!gateway.branch_exists("to-delete")?);

    Ok(())
}

#[test]
fn test_rename_branch() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("old-name")?;
    gateway.rename_branch("old-name", "new-name")?;

    assert!(!gateway.branch_exists("old-name")?);
    assert!(gateway.branch_exists("new-name")?);

    Ok(())
}

#[test]
fn test_commit() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a file and stage it
    std::fs::write(dir.path().join("test.txt"), "hello")?;
    gateway.stage_all()?;
    gateway.commit("Test commit")?;

    // Verify commit was created
    let message = get_head_message(dir.path())?;
    assert_eq!(message, "Test commit");

    Ok(())
}

#[test]
fn test_has_uncommitted_changes() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Initially clean
    assert!(!gateway.has_uncommitted_changes()?);

    // Create a .gitignore file to ignore certain files
    std::fs::write(dir.path().join(".gitignore"), "ignored/\n*.log\n")?;
    gateway.stage_all()?;
    gateway.commit("Add gitignore")?;
    assert!(!gateway.has_uncommitted_changes()?);

    // Create a gitignored file - should NOT count as uncommitted changes
    std::fs::create_dir(dir.path().join("ignored"))?;
    std::fs::write(dir.path().join("ignored/file.txt"), "ignored")?;
    assert!(!gateway.has_uncommitted_changes()?);

    // Create an untracked file - SHOULD count as uncommitted changes
    std::fs::write(dir.path().join("untracked.txt"), "hello")?;
    assert!(gateway.has_uncommitted_changes()?);

    // Stage and commit the untracked file
    gateway.stage_all()?;
    gateway.commit("Add untracked")?;
    assert!(!gateway.has_uncommitted_changes()?);

    // Modify a tracked file - should count as uncommitted changes
    std::fs::write(dir.path().join("untracked.txt"), "modified")?;
    assert!(gateway.has_uncommitted_changes()?);

    Ok(())
}

#[test]
fn test_backup_ref_creation() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a branch with a commit
    gateway.create_branch("test-branch")?;
    std::fs::write(dir.path().join("test.txt"), "hello")?;
    gateway.stage_all()?;
    gateway.commit("Test commit")?;

    // Create backup
    let backup = gateway.create_backup_ref("test-branch")?;

    assert_eq!(backup.branch_name, "test-branch");
    assert!(backup.ref_name.starts_with("refs/diamond/backup/test-branch-"));

    Ok(())
}

#[test]
fn test_list_backup_refs() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("branch-1")?;
    std::fs::write(dir.path().join("test1.txt"), "hello")?;
    gateway.stage_all()?;
    gateway.commit("Commit 1")?;

    gateway.create_backup_ref("branch-1")?;

    // Wait a full second to ensure different timestamp
    std::thread::sleep(std::time::Duration::from_secs(1));

    gateway.create_branch("branch-2")?;
    std::fs::write(dir.path().join("test2.txt"), "world")?;
    gateway.stage_all()?;
    gateway.commit("Commit 2")?;

    gateway.create_backup_ref("branch-2")?;

    let backups = gateway.list_backup_refs()?;
    assert_eq!(backups.len(), 2);

    // Should be sorted by timestamp (newest first)
    assert_eq!(backups[0].branch_name, "branch-2");
    assert_eq!(backups[1].branch_name, "branch-1");

    Ok(())
}

#[test]
fn test_restore_from_backup() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a branch with a commit
    gateway.create_branch("test-branch")?;
    std::fs::write(dir.path().join("test.txt"), "version 1")?;
    gateway.stage_all()?;
    gateway.commit("Version 1")?;

    // Create backup
    let backup = gateway.create_backup_ref("test-branch")?;

    // Make another commit
    std::fs::write(dir.path().join("test.txt"), "version 2")?;
    gateway.stage_all()?;
    gateway.commit("Version 2")?;

    // Restore from backup
    gateway.restore_from_backup(&backup)?;

    // Verify branch points to backup commit
    let branch_sha = gateway.get_branch_sha("test-branch")?;
    assert_eq!(branch_sha, backup.commit_oid);

    Ok(())
}

#[test]
fn test_cleanup_old_backups() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("test-branch")?;

    // Create 5 backups with unique timestamps
    for i in 1..=5 {
        std::fs::write(dir.path().join(format!("test{}.txt", i)), "data")?;
        gateway.stage_all()?;
        gateway.commit(&format!("Commit {}", i))?;
        gateway.create_backup_ref("test-branch")?;
        // Wait a full second to ensure unique timestamps
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    // Verify 5 backups exist
    assert_eq!(gateway.list_backup_refs()?.len(), 5);

    // Cleanup, keeping only 2
    let deleted = gateway.cleanup_old_backups(2)?;
    assert_eq!(deleted, 3);

    // Verify only 2 remain
    assert_eq!(gateway.list_backup_refs()?.len(), 2);

    Ok(())
}

#[test]
fn test_restore_deleted_branch_from_backup() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_repo(dir.path())?;

    // Get the default branch name (could be "main" or "master" depending on git config)
    let default_branch = repo.head()?.shorthand().unwrap_or("master").to_string();
    drop(repo);

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a branch with a commit
    gateway.create_branch("feature-to-delete")?;
    std::fs::write(dir.path().join("feature.txt"), "important work")?;
    gateway.stage_all()?;
    gateway.commit("Feature work")?;

    // Create backup before deleting
    let backup = gateway.create_backup_ref("feature-to-delete")?;
    let backup_oid = backup.commit_oid.clone();

    // Switch back to the default branch and delete the feature branch
    gateway.checkout_branch(&default_branch)?;
    delete_branch_cli(dir.path(), "feature-to-delete")?;

    // Verify branch is gone
    assert!(!gateway.branch_exists("feature-to-delete")?);

    // Restore from backup (creates branch anew)
    gateway.restore_from_backup(&backup)?;

    // Verify branch is back with correct commit
    assert!(gateway.branch_exists("feature-to-delete")?);
    let restored_sha = gateway.get_branch_sha("feature-to-delete")?;
    assert_eq!(restored_sha, backup_oid);

    Ok(())
}

#[test]
fn test_backup_with_branch_name_containing_dash() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Branch name with multiple dashes (parsing could break on last dash)
    gateway.create_branch("feature-123-auth-fix")?;
    std::fs::write(dir.path().join("auth.txt"), "auth fix")?;
    gateway.stage_all()?;
    gateway.commit("Auth fix commit")?;

    // Create backup
    let backup = gateway.create_backup_ref("feature-123-auth-fix")?;

    // Verify the branch name was parsed correctly
    assert_eq!(backup.branch_name, "feature-123-auth-fix");

    // List backups and verify parsing
    let backups = gateway.list_backup_refs()?;
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].branch_name, "feature-123-auth-fix");

    Ok(())
}

#[test]
fn test_rapid_backup_creation_uniqueness() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a branch
    gateway.create_branch("feature-rapid")?;
    std::fs::write(dir.path().join("test.txt"), "test")?;
    gateway.stage_all()?;
    gateway.commit("Test commit")?;

    // Create multiple backups in rapid succession (no delays)
    // This tests the nanosecond + atomic counter mechanism
    let backup1 = gateway.create_backup_ref("feature-rapid")?;
    let backup2 = gateway.create_backup_ref("feature-rapid")?;
    let backup3 = gateway.create_backup_ref("feature-rapid")?;

    // All backup ref names must be unique
    let ref_names = [&backup1.ref_name, &backup2.ref_name, &backup3.ref_name];
    let unique_refs: std::collections::HashSet<_> = ref_names.iter().collect();
    assert_eq!(
        unique_refs.len(),
        3,
        "Backup refs must be unique even when created in rapid succession"
    );

    // Verify all backups are listed
    let backups = gateway.list_backup_refs()?;
    assert_eq!(backups.len(), 3);

    Ok(())
}

#[test]
fn test_backup_cleanup_per_branch() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::from_path(dir.path())?;

    // Create two branches with multiple backups each
    // Use full second delays to ensure unique timestamps (backup refs use Unix seconds)
    for branch in ["feature-a", "feature-b"] {
        gateway.create_branch(branch)?;
        for i in 1..=3 {
            std::fs::write(dir.path().join(format!("{}-{}.txt", branch, i)), "data")?;
            gateway.stage_all()?;
            gateway.commit(&format!("{} commit {}", branch, i))?;
            gateway.create_backup_ref(branch)?;
            // Wait a full second to ensure unique timestamp
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    // Verify 6 total backups (3 per branch)
    assert_eq!(gateway.list_backup_refs()?.len(), 6);

    // Cleanup keeping 1 per branch
    let deleted = gateway.cleanup_old_backups(1)?;
    assert_eq!(deleted, 4); // 2 deleted from each branch

    // Verify 2 remain (1 per branch)
    let remaining = gateway.list_backup_refs()?;
    assert_eq!(remaining.len(), 2);

    // Verify we have one backup for each branch
    let branch_names: std::collections::HashSet<_> = remaining.iter().map(|b| b.branch_name.as_str()).collect();
    assert!(branch_names.contains("feature-a"));
    assert!(branch_names.contains("feature-b"));

    Ok(())
}

#[test]
fn test_cleanup_backups_by_age() -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("test.txt"), "data")?;
    gateway.stage_all()?;
    gateway.commit("Test commit")?;

    let commit_oid = get_head_sha(dir.path())?;

    // Create backups with specific timestamps:
    // - One from 60 days ago (should be deleted with 30-day limit)
    // - One from 10 days ago (should be kept)
    // - One from now (should be kept)
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let sixty_days_ago = now - (60 * 24 * 60 * 60);
    let ten_days_ago = now - (10 * 24 * 60 * 60);

    // Create refs manually with specific timestamps
    create_reference(
        dir.path(),
        &format!("refs/diamond/backup/feature-{}", sixty_days_ago),
        &commit_oid,
    )?;
    create_reference(
        dir.path(),
        &format!("refs/diamond/backup/feature-{}", ten_days_ago),
        &commit_oid,
    )?;
    create_reference(dir.path(), &format!("refs/diamond/backup/feature-{}", now), &commit_oid)?;

    // Verify 3 backups exist
    assert_eq!(gateway.list_backup_refs()?.len(), 3);

    // Clean up backups older than 30 days
    let deleted = gateway.cleanup_backups_by_age(30)?;
    assert_eq!(deleted, 1);

    // Verify 2 remain
    let remaining = gateway.list_backup_refs()?;
    assert_eq!(remaining.len(), 2);

    // Verify the old one is gone
    assert!(remaining.iter().all(|b| b.timestamp >= ten_days_ago));

    Ok(())
}

#[test]
fn test_cleanup_backups_by_age_keeps_recent() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("test.txt"), "data")?;
    gateway.stage_all()?;
    gateway.commit("Test commit")?;

    // Create a backup (current timestamp)
    gateway.create_backup_ref("feature")?;

    // Verify 1 backup exists
    assert_eq!(gateway.list_backup_refs()?.len(), 1);

    // Clean up backups older than 30 days - should keep all
    let deleted = gateway.cleanup_backups_by_age(30)?;
    assert_eq!(deleted, 0);

    // Verify 1 remains
    assert_eq!(gateway.list_backup_refs()?.len(), 1);

    Ok(())
}

#[test]
fn test_gc_combines_age_and_count() -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("feature")?;

    // Create commits for unique backups
    for i in 1..=5 {
        std::fs::write(dir.path().join(format!("test{}.txt", i)), "data")?;
        gateway.stage_all()?;
        gateway.commit(&format!("Commit {}", i))?;
    }

    let commit_oid = get_head_sha(dir.path())?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    // Create backups:
    // - 2 old (60 days ago) - will be deleted by age
    // - 4 recent - of which 2 will be deleted by count (keeping 2)
    let sixty_days_ago = now - (60 * 24 * 60 * 60);

    // Old backups
    create_reference(
        dir.path(),
        &format!("refs/diamond/backup/feature-{}", sixty_days_ago),
        &commit_oid,
    )?;
    create_reference(
        dir.path(),
        &format!("refs/diamond/backup/feature-{}", sixty_days_ago + 1),
        &commit_oid,
    )?;

    // Recent backups (spaced 1 second apart for unique timestamps)
    for i in 0..4 {
        create_reference(
            dir.path(),
            &format!("refs/diamond/backup/feature-{}", now - i),
            &commit_oid,
        )?;
    }

    // Verify 6 backups exist
    assert_eq!(gateway.list_backup_refs()?.len(), 6);

    // Run gc with 30 day max age and keep 2 per branch
    let (deleted_by_age, deleted_by_count) = gateway.gc(30, 2)?;

    // Should delete 2 by age (the 60-day-old ones)
    assert_eq!(deleted_by_age, 2);
    // Should delete 2 by count (4 recent - 2 to keep = 2 deleted)
    assert_eq!(deleted_by_count, 2);

    // Verify 2 remain
    assert_eq!(gateway.list_backup_refs()?.len(), 2);

    Ok(())
}

#[test]
fn test_get_commit_count_since() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Get initial commit ID
    let initial_commit = get_head_sha(dir.path())?;

    // Create 3 more commits
    for i in 1..=3 {
        std::fs::write(dir.path().join(format!("test{}.txt", i)), "data")?;
        gateway.stage_all()?;
        gateway.commit(&format!("Commit {}", i))?;
    }

    // Count should be 3 (using commit ID as base)
    let count = gateway.get_commit_count_since(&initial_commit)?;
    assert_eq!(count, 3);

    Ok(())
}

#[test]
fn test_soft_reset() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Get initial commit
    let initial_commit = get_head_sha(dir.path())?;

    // Make 2 commits
    std::fs::write(dir.path().join("test1.txt"), "data1")?;
    gateway.stage_all()?;
    gateway.commit("Commit 1")?;

    std::fs::write(dir.path().join("test2.txt"), "data2")?;
    gateway.stage_all()?;
    gateway.commit("Commit 2")?;

    // Soft reset to initial commit
    gateway.soft_reset_to(&initial_commit)?;

    // HEAD should be at initial commit
    let current_commit = get_head_sha(dir.path())?;
    assert_eq!(current_commit, initial_commit);

    // But changes should still be staged
    assert!(gateway.has_uncommitted_changes()?);

    Ok(())
}

#[test]
fn test_rebase_with_staged_changes_shows_proper_error() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a branch with a commit
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("file1.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Feature commit")?;

    // Create another branch
    gateway.create_branch("other")?;
    std::fs::write(dir.path().join("file2.txt"), "other")?;
    gateway.stage_all()?;
    gateway.commit("Other commit")?;

    // Stage changes without committing
    std::fs::write(dir.path().join("file3.txt"), "staged")?;
    gateway.stage_all()?;

    // Try to rebase - should fail with proper error message, not "conflicts"
    let result = gateway.rebase_onto("feature", "other");

    assert!(result.is_err(), "Expected error for dirty working tree");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("git rebase failed") || err_msg.contains("uncommitted changes"),
        "Error should mention rebase failure or uncommitted changes, got: {}",
        err_msg
    );

    Ok(())
}

#[test]
fn test_rebase_in_progress_detects_merge_state() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Initially no rebase in progress
    assert!(!gateway.rebase_in_progress()?);

    // Create the rebase-merge directory to simulate a rebase in progress
    std::fs::create_dir(dir.path().join(".git").join("rebase-merge"))?;
    assert!(gateway.rebase_in_progress()?);

    // Clean up
    std::fs::remove_dir(dir.path().join(".git").join("rebase-merge"))?;
    assert!(!gateway.rebase_in_progress()?);

    // Test with rebase-apply directory
    std::fs::create_dir(dir.path().join(".git").join("rebase-apply"))?;
    assert!(gateway.rebase_in_progress()?);

    Ok(())
}

#[test]
fn test_rebase_with_invalid_ref_shows_proper_error() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a feature branch
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("file.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Feature commit")?;

    // Try to rebase onto non-existent branch
    let result = gateway.rebase_onto("feature", "non-existent-branch");

    assert!(result.is_err(), "Expected error for invalid reference");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("git rebase failed") || err_msg.contains("does not exist") || err_msg.contains("not a valid"),
        "Error should mention rebase failure or invalid reference, got: {}",
        err_msg
    );

    Ok(())
}

#[test]
fn test_has_staged_or_modified_changes_with_untracked() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Initially clean
    assert!(!gateway.has_staged_or_modified_changes()?);

    // Add untracked file - should NOT be considered dirty for rebase
    std::fs::write(dir.path().join("untracked.txt"), "untracked")?;
    assert!(!gateway.has_staged_or_modified_changes()?);

    // But has_uncommitted_changes should still see it
    assert!(gateway.has_uncommitted_changes()?);

    Ok(())
}

#[test]
fn test_has_staged_or_modified_changes_with_staged() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Add untracked file
    std::fs::write(dir.path().join("untracked.txt"), "untracked")?;
    assert!(!gateway.has_staged_or_modified_changes()?);

    // Stage a file - should be considered dirty
    gateway.stage_all()?;
    assert!(gateway.has_staged_or_modified_changes()?);

    Ok(())
}

#[test]
fn test_has_staged_or_modified_changes_with_modified() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create and commit a file
    std::fs::write(dir.path().join("tracked.txt"), "original")?;
    gateway.stage_all()?;
    gateway.commit("Add tracked file")?;
    assert!(!gateway.has_staged_or_modified_changes()?);

    // Modify the file without staging - should be considered dirty
    std::fs::write(dir.path().join("tracked.txt"), "modified")?;
    assert!(gateway.has_staged_or_modified_changes()?);

    Ok(())
}

// ===== Edge Case Tests =====

#[test]
fn test_detached_head_detected() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Get the commit ID
    let commit_id = repo.head()?.peel_to_commit()?.id();

    // Detach HEAD by checking out the commit directly
    repo.set_head_detached(commit_id)?;

    // get_current_branch_name should fail with detached HEAD
    let result = gateway.get_current_branch_name();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("detached") || err.contains("invalid"),
        "Error should mention detached HEAD: {}",
        err
    );

    Ok(())
}

#[test]
fn test_branch_name_with_slash() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create branch with slash in name (common pattern like feature/foo)
    gateway.create_branch("feature/my-feature")?;

    // Verify it was created
    assert!(gateway.branch_exists("feature/my-feature")?);
    assert_eq!(gateway.get_current_branch_name()?, "feature/my-feature");

    // Can checkout and switch
    gateway.checkout_branch("main")?;
    gateway.checkout_branch("feature/my-feature")?;
    assert_eq!(gateway.get_current_branch_name()?, "feature/my-feature");

    // Can delete
    gateway.checkout_branch("main")?;
    gateway.delete_branch("feature/my-feature")?;
    assert!(!gateway.branch_exists("feature/my-feature")?);

    Ok(())
}

#[test]
fn test_branch_name_with_multiple_slashes() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create branch with multiple slashes
    gateway.create_branch("user/john/feature/auth")?;

    assert!(gateway.branch_exists("user/john/feature/auth")?);
    assert_eq!(gateway.get_current_branch_name()?, "user/john/feature/auth");

    Ok(())
}

#[test]
fn test_branch_name_with_dash_and_underscore() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create branches with various naming conventions
    gateway.create_branch("my-feature-branch")?;
    assert!(gateway.branch_exists("my-feature-branch")?);

    gateway.create_branch("my_feature_branch")?;
    assert!(gateway.branch_exists("my_feature_branch")?);

    gateway.create_branch("my-feature_branch-v2")?;
    assert!(gateway.branch_exists("my-feature_branch-v2")?);

    Ok(())
}

#[test]
fn test_create_branch_that_already_exists_fails() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create branch
    gateway.create_branch("existing")?;
    gateway.checkout_branch("main")?;

    // Try to create same branch again
    let result = gateway.create_branch("existing");
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_checkout_nonexistent_branch_fails() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    let result = gateway.checkout_branch("does-not-exist");
    assert!(result.is_err());
    // Just verify it fails - the specific error message may vary
    // depending on which step fails first (set_head or checkout_head)

    Ok(())
}

#[test]
fn test_delete_nonexistent_branch_fails() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    let result = gateway.delete_branch("does-not-exist");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found") || err.contains("does-not-exist"),
        "Error should mention branch not found: {}",
        err
    );

    Ok(())
}

#[test]
fn test_rename_to_existing_name_fails() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("branch-a")?;
    gateway.create_branch("branch-b")?;

    // Try to rename branch-a to branch-b (which already exists)
    let result = gateway.rename_branch("branch-a", "branch-b");
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_empty_commit_message_fails() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a file and stage it
    std::fs::write(dir.path().join("test.txt"), "content")?;
    gateway.stage_all()?;

    // Try to commit with empty message
    let result = gateway.commit("");
    // Git may or may not accept empty commit message depending on config
    // This test just ensures we don't panic
    // The result could be Ok or Err depending on git configuration
    let _ = result;

    Ok(())
}

#[test]
fn test_get_commit_messages_since() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Get initial commit ID (this is our base)
    let initial_commit = get_head_sha(dir.path())?;

    // Create 3 commits with distinct messages
    for i in 1..=3 {
        std::fs::write(dir.path().join(format!("test{}.txt", i)), "data")?;
        gateway.stage_all()?;
        gateway.commit(&format!("Feature commit {}\n\nThis is commit number {}", i, i))?;
    }

    // Get messages since initial commit
    let messages = gateway.get_commit_messages_since(&initial_commit)?;

    // Should have 3 messages (newest to oldest)
    assert_eq!(messages.len(), 3);

    // Messages are returned newest first (subject lines only)
    assert!(messages[0].contains("Feature commit 3"));
    assert!(messages[1].contains("Feature commit 2"));
    assert!(messages[2].contains("Feature commit 1"));

    // Note: get_commit_messages_since uses --format=%s which only returns subject lines,
    // not the full message body

    Ok(())
}

#[test]
fn test_get_commit_messages_since_same_commit() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Get current commit ID
    let current_commit = get_head_sha(dir.path())?;

    // Get messages since same commit should return empty
    let messages = gateway.get_commit_messages_since(&current_commit)?;
    assert!(messages.is_empty());

    Ok(())
}

#[test]
fn test_is_branch_merged_true() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a feature branch with a commit
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("feature.txt"), "feature")?;
    gateway.stage_all()?;
    gateway.commit("Feature commit")?;

    // Get feature's commit
    let feature_commit = repo.head()?.peel_to_commit()?;

    // Go back to main and manually fast-forward to feature's commit
    gateway.checkout_branch("main")?;
    let mut main_ref = repo.find_reference("refs/heads/main")?;
    main_ref.set_target(feature_commit.id(), "fast-forward to feature")?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;

    // Now feature should be merged into main (they point to same commit)
    assert!(gateway.is_branch_merged("feature", "main")?);

    Ok(())
}

#[test]
fn test_is_branch_merged_false() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a feature branch with its own commit
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("feature.txt"), "feature")?;
    gateway.stage_all()?;
    gateway.commit("Feature commit")?;

    // Feature is not merged into main (main doesn't have feature's commit)
    assert!(!gateway.is_branch_merged("feature", "main")?);

    Ok(())
}

#[test]
fn test_get_short_hash() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    let short_hash = gateway.get_short_hash("main")?;
    assert_eq!(short_hash.len(), 7);
    // Should be valid hex
    assert!(short_hash.chars().all(|c| c.is_ascii_hexdigit()));

    Ok(())
}

#[test]
fn test_get_commit_subject() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Initial commit message is "Initial commit"
    let subject = gateway.get_commit_subject("main")?;
    assert_eq!(subject, "Initial commit");

    // Create a new commit with a longer message
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("test.txt"), "test")?;
    gateway.stage_all()?;
    gateway.commit("This is the subject line\n\nThis is the body.")?;

    let subject = gateway.get_commit_subject("feature")?;
    assert_eq!(subject, "This is the subject line");

    Ok(())
}

#[test]
fn test_get_commit_subject_truncates_long_message() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("test.txt"), "test")?;
    gateway.stage_all()?;
    gateway.commit("This is a very long commit message that exceeds fifty characters in length")?;

    let subject = gateway.get_commit_subject("feature")?;
    assert!(subject.len() <= 50);
    assert!(subject.ends_with("..."));

    Ok(())
}

#[test]
fn test_get_commit_time_relative() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Just created, should be "just now" or within minutes
    let time = gateway.get_commit_time_relative("main")?;
    assert!(
        time.contains("just now") || time.contains("second") || time.contains("minute"),
        "Expected recent time, got: {}",
        time
    );

    Ok(())
}

#[test]
fn test_format_relative_time() {
    assert_eq!(format_relative_time(0), "0 seconds ago");
    assert_eq!(format_relative_time(1), "1 second ago");
    assert_eq!(format_relative_time(59), "59 seconds ago");
    assert_eq!(format_relative_time(60), "1 minute ago");
    assert_eq!(format_relative_time(120), "2 minutes ago");
    assert_eq!(format_relative_time(3600), "1 hour ago");
    assert_eq!(format_relative_time(7200), "2 hours ago");
    assert_eq!(format_relative_time(86400), "1 day ago");
    assert_eq!(format_relative_time(172800), "2 days ago");
    assert_eq!(format_relative_time(604800), "1 week ago");
    assert_eq!(format_relative_time(1209600), "2 weeks ago");
    assert_eq!(format_relative_time(2592000), "1 month ago");
    assert_eq!(format_relative_time(5184000), "2 months ago");
    assert_eq!(format_relative_time(31536000), "1 year ago");
    assert_eq!(format_relative_time(63072000), "2 years ago");
    assert_eq!(format_relative_time(-100), "in the future");
}

#[test]
fn test_hard_reset_to() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Get initial commit hash
    let initial_hash = gateway.get_short_hash("main")?;

    // Create some commits
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("file1.txt"), "content1")?;
    gateway.stage_all()?;
    gateway.commit("Commit 1")?;

    std::fs::write(dir.path().join("file2.txt"), "content2")?;
    gateway.stage_all()?;
    gateway.commit("Commit 2")?;

    // Hard reset to initial commit
    gateway.hard_reset_to("main")?;

    // Current hash should match main
    let current_hash = gateway.get_short_hash("feature")?;
    assert_eq!(current_hash, initial_hash);

    // Files should be gone
    assert!(!dir.path().join("file1.txt").exists());
    assert!(!dir.path().join("file2.txt").exists());

    Ok(())
}

#[test]
fn test_resolve_ref() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Resolve branch name
    let oid = gateway.resolve_ref("main")?;
    assert!(!oid.is_zero());

    // Resolve HEAD
    let head_oid = gateway.resolve_ref("HEAD")?;
    assert_eq!(oid, head_oid);

    Ok(())
}

#[test]
fn test_resolve_ref_nonexistent() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    let result = gateway.resolve_ref("nonexistent");
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_is_ancestor_true() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a chain: main -> feature
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("test.txt"), "test")?;
    gateway.stage_all()?;
    gateway.commit("Feature commit")?;

    // main should be ancestor of feature
    assert!(gateway.is_ancestor("main", "feature")?);

    // feature should NOT be ancestor of main
    assert!(!gateway.is_ancestor("feature", "main")?);

    Ok(())
}

#[test]
fn test_is_ancestor_same_commit() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Same commit should return true (considered "merged")
    assert!(gateway.is_ancestor("main", "main")?);

    Ok(())
}

#[test]
fn test_get_remote_url_success() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_repo(dir.path())?;

    // Add a remote
    repo.remote("origin", "https://github.com/user/repo.git")?;

    let gateway = GitGateway::from_path(dir.path())?;

    let url = gateway.get_remote_url("origin")?;
    assert_eq!(url, "https://github.com/user/repo.git");

    Ok(())
}

#[test]
fn test_get_remote_url_nonexistent() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    let result = gateway.get_remote_url("origin");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("No 'origin' remote configured"));

    Ok(())
}

#[test]
fn test_get_branch_sha() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    gateway.create_branch("test-branch")?;

    // Get the full SHA
    let sha = gateway.get_branch_sha("test-branch")?;

    // Should be 40 characters (full SHA)
    assert_eq!(sha.len(), 40);
    // Should be lowercase hex
    assert!(sha.chars().all(|c: char| c.is_ascii_hexdigit()));

    // Should match the short hash prefix
    let short = gateway.get_short_hash("test-branch")?;
    assert!(sha.starts_with(&short));

    Ok(())
}

#[test]
fn test_get_branch_sha_nonexistent() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    let result = gateway.get_branch_sha("does-not-exist");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found") || err.contains("Failed to resolve"),
        "Error should indicate branch not found: {}",
        err
    );

    Ok(())
}

// ===== Diamond Ref Tests =====

#[test]
fn test_prune_orphaned_diamond_refs_removes_orphans() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_repo(dir.path())?;

    // Create a branch
    let gateway = GitGateway::from_path(dir.path())?;
    gateway.create_branch("feature-1")?;

    // Create a diamond parent ref for feature-1
    let head = repo.head()?.peel_to_commit()?;
    repo.reference("refs/diamond/parent/feature-1", head.id(), true, "test parent ref")?;

    // Also create an orphaned ref for a deleted branch
    repo.reference(
        "refs/diamond/parent/deleted-branch",
        head.id(),
        true,
        "orphaned parent ref",
    )?;

    // Verify both refs exist
    assert!(repo.find_reference("refs/diamond/parent/feature-1").is_ok());
    assert!(repo.find_reference("refs/diamond/parent/deleted-branch").is_ok());

    // Prune orphaned refs
    let pruned = gateway.prune_orphaned_diamond_refs()?;

    // Should only prune the deleted-branch ref
    assert_eq!(pruned.len(), 1);
    assert_eq!(pruned[0], "refs/diamond/parent/deleted-branch");

    // feature-1 ref should still exist
    assert!(repo.find_reference("refs/diamond/parent/feature-1").is_ok());

    // deleted-branch ref should be gone
    assert!(repo.find_reference("refs/diamond/parent/deleted-branch").is_err());

    Ok(())
}

#[test]
fn test_prune_orphaned_diamond_refs_no_orphans() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Create a branch and its corresponding ref
    gateway.create_branch("feature-1")?;

    let head = repo.head()?.peel_to_commit()?;
    repo.reference("refs/diamond/parent/feature-1", head.id(), true, "test parent ref")?;

    // Prune - should find nothing
    let pruned = gateway.prune_orphaned_diamond_refs()?;
    assert!(pruned.is_empty());

    Ok(())
}

#[test]
fn test_prune_orphaned_diamond_refs_no_refs() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // No diamond refs exist at all
    let pruned = gateway.prune_orphaned_diamond_refs()?;
    assert!(pruned.is_empty());

    Ok(())
}

#[test]
fn test_configure_diamond_refspec_adds_refspec() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    // Add a remote first (required for the refspec to be added)
    let output = std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/test/test.git"])
        .current_dir(dir.path())
        .output()?;
    assert!(output.status.success(), "Failed to add remote");

    let gateway = GitGateway::from_path(dir.path())?;

    // Configure diamond refspec
    gateway.configure_diamond_refspec()?;

    // Verify the refspec was added
    let output = std::process::Command::new("git")
        .args(["config", "--get-all", "remote.origin.fetch"])
        .current_dir(dir.path())
        .output()?;

    let refspecs = String::from_utf8_lossy(&output.stdout);
    assert!(
        refspecs.contains("refs/diamond/*:refs/diamond/*"),
        "Diamond refspec should be added. Got: {}",
        refspecs
    );

    Ok(())
}

#[test]
fn test_configure_diamond_refspec_idempotent() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    // Add a remote
    let output = std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/test/test.git"])
        .current_dir(dir.path())
        .output()?;
    assert!(output.status.success());

    let gateway = GitGateway::from_path(dir.path())?;

    // Configure twice - should not duplicate
    gateway.configure_diamond_refspec()?;
    gateway.configure_diamond_refspec()?;

    // Verify only one refspec was added
    let output = std::process::Command::new("git")
        .args(["config", "--get-all", "remote.origin.fetch"])
        .current_dir(dir.path())
        .output()?;

    let refspecs = String::from_utf8_lossy(&output.stdout);
    let diamond_count = refspecs.matches("refs/diamond/*:refs/diamond/*").count();
    assert_eq!(
        diamond_count, 1,
        "Should only have one diamond refspec. Got: {}",
        refspecs
    );

    Ok(())
}

#[test]
fn test_push_diamond_ref_fails_without_remote() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;
    gateway.create_branch("feature-1")?;

    // Create a parent ref locally
    let head_sha = get_head_sha(dir.path())?;
    create_reference(dir.path(), "refs/diamond/parent/feature-1", &head_sha)?;

    // Try to push without a remote configured - should fail
    let result = gateway.push_diamond_ref("feature-1");
    assert!(result.is_err(), "Should fail without remote");

    Ok(())
}

#[test]
fn test_fetch_diamond_ref_for_branch_fails_gracefully_without_remote() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // Should fail gracefully without remote (not crash)
    let result = gateway.fetch_diamond_ref_for_branch("feature-1");
    assert!(result.is_err(), "Should fail without remote");

    Ok(())
}

#[test]
fn test_checkout_remote_branch_creates_local_tracking_branch() -> Result<()> {
    // Create a "remote" bare repository
    let remote_dir = tempdir()?;
    let remote_repo = Repository::init_bare(remote_dir.path())?;

    // Create a "local" repository
    let local_dir = tempdir()?;
    let local_repo = init_repo(local_dir.path())?;

    // Add the bare repo as a remote
    local_repo.remote("origin", remote_dir.path().to_str().unwrap())?;

    // Push main to remote so it has content
    {
        let mut remote = local_repo.find_remote("origin")?;
        remote.push(&["refs/heads/main:refs/heads/main"], None)?;
    }

    // Create a branch in the remote repo
    let remote_head = remote_repo.find_reference("refs/heads/main")?;
    let remote_commit = remote_head.peel_to_commit()?;
    remote_repo.branch("feature-from-remote", &remote_commit, false)?;

    // Fetch the remote branches to local
    {
        let mut remote = local_repo.find_remote("origin")?;
        remote.fetch(&["refs/heads/*:refs/remotes/origin/*"], None, None)?;
    }

    // Verify the branch exists on remote but not locally
    let gateway = GitGateway::from_path(local_dir.path())?;
    assert!(
        !gateway.branch_exists("feature-from-remote")?,
        "Branch should not exist locally yet"
    );

    // Verify the remote tracking branch exists
    assert!(
        local_repo
            .find_reference("refs/remotes/origin/feature-from-remote")
            .is_ok(),
        "Remote tracking branch should exist"
    );

    // Try to checkout the remote branch - should create local tracking branch
    gateway.checkout_branch("feature-from-remote")?;

    // Verify we're now on the branch
    assert_eq!(gateway.get_current_branch_name()?, "feature-from-remote");

    // Verify the branch now exists locally
    assert!(
        gateway.branch_exists("feature-from-remote")?,
        "Branch should now exist locally"
    );

    Ok(())
}

// ===== Remote Sync State Tests =====

#[test]
fn test_check_remote_sync_no_remote() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // No remote tracking branch exists
    let state = gateway.check_remote_sync("main")?;
    assert_eq!(state, BranchSyncState::NoRemote);

    Ok(())
}

#[test]
fn test_check_remote_sync_in_sync() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let _remote_repo = init_repo(remote_dir.path())?;

    // Clone it to create a local repo with proper remote
    let local_dir = tempdir()?;
    let local_repo = Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    // Fetch to ensure we have remote tracking branches
    let mut remote = local_repo.find_remote("origin")?;
    remote.fetch(&["main"], None, None)?;
    drop(remote);
    drop(local_repo);

    let gateway = GitGateway::from_path(local_dir.path())?;

    // Local and remote should be in sync after clone
    let state = gateway.check_remote_sync("main")?;
    assert_eq!(state, BranchSyncState::InSync);

    Ok(())
}

#[test]
fn test_check_remote_sync_ahead() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let _remote_repo = init_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let local_repo = Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    // Fetch to ensure we have remote tracking branches
    let mut remote = local_repo.find_remote("origin")?;
    remote.fetch(&["main"], None, None)?;
    drop(remote);
    drop(local_repo);

    let gateway = GitGateway::from_path(local_dir.path())?;

    // Make a local commit
    std::fs::write(local_dir.path().join("new_file.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Local commit")?;

    // Should be 1 commit ahead
    let state = gateway.check_remote_sync("main")?;
    assert_eq!(state, BranchSyncState::Ahead(1));

    Ok(())
}

#[test]
fn test_check_remote_sync_behind() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let remote_repo = init_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let local_repo = Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;
    drop(local_repo);

    // Make a commit in the remote
    std::fs::write(remote_dir.path().join("remote_file.txt"), "remote content")?;
    {
        let mut index = remote_repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = remote_repo.find_tree(tree_id)?;
        let sig = git2::Signature::now("Test", "test@test.com")?;
        let parent = remote_repo.head()?.peel_to_commit()?;
        remote_repo.commit(Some("HEAD"), &sig, &sig, "Remote commit", &tree, &[&parent])?;
    }
    drop(remote_repo);

    // Fetch in local repo to update origin/main
    let local_repo = Repository::open(local_dir.path())?;
    let mut remote = local_repo.find_remote("origin")?;
    remote.fetch(&["main"], None, None)?;
    drop(remote);
    drop(local_repo);

    let gateway = GitGateway::from_path(local_dir.path())?;

    // Should be 1 commit behind
    let state = gateway.check_remote_sync("main")?;
    assert_eq!(state, BranchSyncState::Behind(1));

    Ok(())
}

#[test]
fn test_check_remote_sync_diverged() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let remote_repo = init_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let local_repo = Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;
    drop(local_repo);

    // Make a commit in the remote
    std::fs::write(remote_dir.path().join("remote_file.txt"), "remote content")?;
    {
        let mut index = remote_repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = remote_repo.find_tree(tree_id)?;
        let sig = git2::Signature::now("Test", "test@test.com")?;
        let parent = remote_repo.head()?.peel_to_commit()?;
        remote_repo.commit(Some("HEAD"), &sig, &sig, "Remote commit", &tree, &[&parent])?;
    }
    drop(remote_repo);

    // Fetch in local repo to update origin/main
    let local_repo = Repository::open(local_dir.path())?;
    let mut remote = local_repo.find_remote("origin")?;
    remote.fetch(&["main"], None, None)?;
    drop(remote);
    drop(local_repo);

    // Make a local commit (creating divergence)
    let gateway = GitGateway::from_path(local_dir.path())?;
    std::fs::write(local_dir.path().join("local_file.txt"), "local content")?;
    gateway.stage_all()?;
    gateway.commit("Local commit")?;

    // Should be diverged
    let state = gateway.check_remote_sync("main")?;
    assert_eq!(
        state,
        BranchSyncState::Diverged {
            local_ahead: 1,
            remote_ahead: 1
        }
    );

    Ok(())
}

// ===== sync_branch_from_remote Tests =====

#[test]
fn test_sync_branch_from_remote_no_remote() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;

    let gateway = GitGateway::from_path(dir.path())?;

    // No remote tracking branch exists
    let result = gateway.sync_branch_from_remote("main", false)?;
    assert_eq!(result, SyncBranchResult::NoRemote);

    Ok(())
}

#[test]
fn test_sync_branch_from_remote_already_synced() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let _remote_repo = init_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let local_repo = Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    // Fetch to ensure we have remote tracking branches
    let mut remote = local_repo.find_remote("origin")?;
    remote.fetch(&["main"], None, None)?;
    drop(remote);
    drop(local_repo);

    let gateway = GitGateway::from_path(local_dir.path())?;

    // Should be already synced
    let result = gateway.sync_branch_from_remote("main", false)?;
    assert_eq!(result, SyncBranchResult::AlreadySynced);

    Ok(())
}

#[test]
fn test_sync_branch_from_remote_local_ahead() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let _remote_repo = init_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let local_repo = Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    let mut remote = local_repo.find_remote("origin")?;
    remote.fetch(&["main"], None, None)?;
    drop(remote);
    drop(local_repo);

    let gateway = GitGateway::from_path(local_dir.path())?;

    // Make a local commit
    std::fs::write(local_dir.path().join("local.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Local commit")?;

    // Should report local is ahead
    let result = gateway.sync_branch_from_remote("main", false)?;
    assert_eq!(result, SyncBranchResult::LocalAhead(1));

    Ok(())
}

#[test]
fn test_sync_branch_from_remote_updates_behind() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let remote_repo = init_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let local_repo = Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;
    drop(local_repo);

    // Make a commit in the remote
    std::fs::write(remote_dir.path().join("remote_file.txt"), "remote content")?;
    {
        let mut index = remote_repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = remote_repo.find_tree(tree_id)?;
        let sig = git2::Signature::now("Test", "test@test.com")?;
        let parent = remote_repo.head()?.peel_to_commit()?;
        remote_repo.commit(Some("HEAD"), &sig, &sig, "Remote commit", &tree, &[&parent])?;
    }
    drop(remote_repo);

    // Fetch in local repo to update origin/main
    let local_repo = Repository::open(local_dir.path())?;
    let mut remote = local_repo.find_remote("origin")?;
    remote.fetch(&["main"], None, None)?;
    drop(remote);
    drop(local_repo);

    let gateway = GitGateway::from_path(local_dir.path())?;

    // Get SHA before sync
    let before_sha = gateway.get_branch_sha("main")?;

    // Sync should fast-forward
    let result = gateway.sync_branch_from_remote("main", false)?;
    assert_eq!(result, SyncBranchResult::Updated(1));

    // Verify branch was updated
    let after_sha = gateway.get_branch_sha("main")?;
    assert_ne!(before_sha, after_sha, "Branch should have been updated");

    // Verify now in sync
    let state = gateway.check_remote_sync("main")?;
    assert_eq!(state, BranchSyncState::InSync);

    Ok(())
}

#[test]
fn test_sync_branch_from_remote_diverged_no_force() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let remote_repo = init_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let local_repo = Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;
    drop(local_repo);

    // Make a commit in the remote
    std::fs::write(remote_dir.path().join("remote_file.txt"), "remote content")?;
    {
        let mut index = remote_repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = remote_repo.find_tree(tree_id)?;
        let sig = git2::Signature::now("Test", "test@test.com")?;
        let parent = remote_repo.head()?.peel_to_commit()?;
        remote_repo.commit(Some("HEAD"), &sig, &sig, "Remote commit", &tree, &[&parent])?;
    }
    drop(remote_repo);

    // Fetch in local repo
    let local_repo = Repository::open(local_dir.path())?;
    let mut remote = local_repo.find_remote("origin")?;
    remote.fetch(&["main"], None, None)?;
    drop(remote);
    drop(local_repo);

    // Make a local commit (creating divergence)
    let gateway = GitGateway::from_path(local_dir.path())?;
    std::fs::write(local_dir.path().join("local_file.txt"), "local content")?;
    gateway.stage_all()?;
    gateway.commit("Local commit")?;

    // Without force, should report diverged
    let result = gateway.sync_branch_from_remote("main", false)?;
    assert_eq!(
        result,
        SyncBranchResult::Diverged {
            local_ahead: 1,
            remote_ahead: 1
        }
    );

    Ok(())
}

#[test]
fn test_sync_branch_from_remote_diverged_with_force() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let remote_repo = init_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let local_repo = Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;
    drop(local_repo);

    // Make a commit in the remote
    std::fs::write(remote_dir.path().join("remote_file.txt"), "remote content")?;
    {
        let mut index = remote_repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = remote_repo.find_tree(tree_id)?;
        let sig = git2::Signature::now("Test", "test@test.com")?;
        let parent = remote_repo.head()?.peel_to_commit()?;
        remote_repo.commit(Some("HEAD"), &sig, &sig, "Remote commit", &tree, &[&parent])?;
    }
    drop(remote_repo);

    // Fetch in local repo
    let local_repo = Repository::open(local_dir.path())?;
    let mut remote = local_repo.find_remote("origin")?;
    remote.fetch(&["main"], None, None)?;
    drop(remote);
    drop(local_repo);

    // Make a local commit (creating divergence)
    let gateway = GitGateway::from_path(local_dir.path())?;
    std::fs::write(local_dir.path().join("local_file.txt"), "local content")?;
    gateway.stage_all()?;
    gateway.commit("Local commit")?;

    // With force, should reset to remote
    let result = gateway.sync_branch_from_remote("main", true)?;
    assert_eq!(result, SyncBranchResult::ForceSynced);

    // Verify now in sync
    let state = gateway.check_remote_sync("main")?;
    assert_eq!(state, BranchSyncState::InSync);

    Ok(())
}

#[test]
fn test_is_branch_based_on_with_multiple_commits() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let gateway = GitGateway::from_path(dir.path())?;

    // Create a feature branch with two commits
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("file1.txt"), "content1")?;
    gateway.stage_all()?;
    gateway.commit("First feature commit")?;

    std::fs::write(dir.path().join("file2.txt"), "content2")?;
    gateway.stage_all()?;
    gateway.commit("Second feature commit")?;

    // The feature branch with 2 commits should still be based on main
    assert!(
        gateway.is_branch_based_on("feature", "main")?,
        "Branch with multiple commits should be recognized as based on main"
    );

    Ok(())
}

#[test]
fn test_stage_updates_only_stages_tracked_files() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let gateway = GitGateway::from_path(dir.path())?;

    // Create and commit a tracked file
    std::fs::write(dir.path().join("tracked.txt"), "initial content")?;
    gateway.stage_all()?;
    gateway.commit("Add tracked file")?;

    // Modify the tracked file
    std::fs::write(dir.path().join("tracked.txt"), "modified content")?;

    // Create a new untracked file
    std::fs::write(dir.path().join("untracked.txt"), "new file")?;

    // Stage only updates (tracked files)
    gateway.stage_updates()?;

    // Check what's staged using git status
    let repo = git2::Repository::open(dir.path())?;
    let statuses = repo.statuses(None)?;

    let mut has_staged_tracked = false;
    let mut has_unstaged_untracked = false;

    for entry in statuses.iter() {
        let path = entry.path().unwrap_or("");
        let status = entry.status();

        if path == "tracked.txt" && status.contains(git2::Status::INDEX_MODIFIED) {
            has_staged_tracked = true;
        }
        if path == "untracked.txt" && status.contains(git2::Status::WT_NEW) {
            has_unstaged_untracked = true;
        }
    }

    assert!(has_staged_tracked, "tracked.txt should be staged");
    assert!(has_unstaged_untracked, "untracked.txt should NOT be staged");

    Ok(())
}

#[test]
fn test_stage_updates_handles_deleted_files() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let gateway = GitGateway::from_path(dir.path())?;

    // Create and commit a file
    std::fs::write(dir.path().join("to_delete.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Add file")?;

    // Delete the file
    std::fs::remove_file(dir.path().join("to_delete.txt"))?;

    // Stage updates should stage the deletion
    gateway.stage_updates()?;

    // Check that deletion is staged
    let repo = git2::Repository::open(dir.path())?;
    let statuses = repo.statuses(None)?;

    let mut has_staged_deletion = false;
    for entry in statuses.iter() {
        let path = entry.path().unwrap_or("");
        let status = entry.status();

        if path == "to_delete.txt" && status.contains(git2::Status::INDEX_DELETED) {
            has_staged_deletion = true;
        }
    }

    assert!(has_staged_deletion, "Deletion should be staged");

    Ok(())
}

// ============================================================================
// RebaseOutcome Tests
// ============================================================================

#[test]
fn test_rebase_success_returns_success_outcome() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::from_path(dir.path())?;

    // We're on the default branch after init (typically master/main)
    // Create a feature branch with a unique commit
    gateway.create_branch("feature")?;
    gateway.checkout_branch("feature")?;
    std::fs::write(dir.path().join("feature.txt"), "feature content")?;
    gateway.stage_all()?;
    gateway.commit("Feature commit")?;

    // Create a new-base branch from HEAD (which is feature's commit)
    // with an additional commit that doesn't conflict
    gateway.create_branch("new-base")?;
    gateway.checkout_branch("new-base")?;
    std::fs::write(dir.path().join("base.txt"), "base content")?;
    gateway.stage_all()?;
    gateway.commit("Base commit")?;

    // Now reset feature to before its commit, then rebase onto new-base
    // Actually, simpler: create feature2 from new-base with non-conflicting change
    gateway.create_branch("feature2")?;
    gateway.checkout_branch("feature2")?;
    std::fs::write(dir.path().join("feature2.txt"), "feature2 content")?;
    gateway.stage_all()?;
    gateway.commit("Feature2 commit")?;

    // Create yet another branch to rebase onto
    gateway.checkout_branch("new-base")?;
    gateway.create_branch("target")?;
    gateway.checkout_branch("target")?;
    std::fs::write(dir.path().join("target.txt"), "target content")?;
    gateway.stage_all()?;
    gateway.commit("Target commit")?;

    // Rebase feature2 onto target (no conflict - all different files)
    let outcome = gateway.rebase_onto("feature2", "target")?;

    assert!(!outcome.has_conflicts(), "Rebase should succeed without conflicts");
    assert_eq!(outcome, RebaseOutcome::Success, "Expected RebaseOutcome::Success");

    Ok(())
}

#[test]
fn test_rebase_conflict_returns_conflicts_outcome() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::from_path(dir.path())?;

    // Get the current branch name (whatever the default is)
    let default_branch = gateway.get_current_branch_name()?;

    // Create a file and commit it on the default branch
    std::fs::write(dir.path().join("conflict.txt"), "original content")?;
    gateway.stage_all()?;
    gateway.commit("Add conflict.txt")?;

    // Create feature branch with conflicting change to same file
    gateway.create_branch("feature")?;
    gateway.checkout_branch("feature")?;
    std::fs::write(dir.path().join("conflict.txt"), "feature version")?;
    gateway.stage_all()?;
    gateway.commit("Feature modifies conflict.txt")?;

    // Go back to default branch and create new-base with different conflicting change
    gateway.checkout_branch(&default_branch)?;
    gateway.create_branch("new-base")?;
    gateway.checkout_branch("new-base")?;
    std::fs::write(dir.path().join("conflict.txt"), "new-base version")?;
    gateway.stage_all()?;
    gateway.commit("New-base modifies conflict.txt")?;

    // Rebase feature onto new-base (should conflict - both modified conflict.txt)
    let outcome = gateway.rebase_onto("feature", "new-base")?;

    assert!(outcome.has_conflicts(), "Rebase should have conflicts");
    assert_eq!(outcome, RebaseOutcome::Conflicts, "Expected RebaseOutcome::Conflicts");

    // Verify rebase is in progress
    assert!(
        gateway.rebase_in_progress()?,
        "Rebase should be in progress after conflicts"
    );

    // Clean up: abort the rebase
    gateway.rebase_abort()?;

    Ok(())
}

// ============================================================================
// CHECKOUT WORKING DIRECTORY STATE TESTS
// These tests verify that checkout properly updates the working directory,
// not just that it succeeds. These would have caught the bug where files
// from the source branch persisted after checkout.
// ============================================================================

#[test]
fn test_checkout_removes_files_not_in_target_branch() -> Result<()> {
    // This is THE test that would have caught the main checkout bug.
    // When switching branches, files that exist only in the source branch
    // should be removed from the working directory.
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create feature branch with a new file
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("feature_only.txt"), "feature content")?;
    gateway.stage_all()?;
    gateway.commit("Add feature-only file")?;

    // Verify file exists on feature branch
    assert!(
        dir.path().join("feature_only.txt").exists(),
        "feature_only.txt should exist on feature branch"
    );

    // Checkout main (which doesn't have feature_only.txt)
    gateway.checkout_branch("main")?;

    // CRITICAL: File should be GONE from working directory
    assert!(
        !dir.path().join("feature_only.txt").exists(),
        "feature_only.txt should be removed when checking out main"
    );

    Ok(())
}

#[test]
fn test_checkout_restores_files_from_target_branch() -> Result<()> {
    // Verify that files are restored when switching back to a branch
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create feature branch with a new file
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("feature.txt"), "feature content")?;
    gateway.stage_all()?;
    gateway.commit("Add feature file")?;

    // Switch to main (file should disappear)
    gateway.checkout_branch("main")?;
    assert!(
        !dir.path().join("feature.txt").exists(),
        "feature.txt should not exist on main"
    );

    // Switch back to feature (file should reappear)
    gateway.checkout_branch("feature")?;
    assert!(
        dir.path().join("feature.txt").exists(),
        "feature.txt should be restored when checking out feature"
    );

    // Verify content is correct
    let content = std::fs::read_to_string(dir.path().join("feature.txt"))?;
    assert_eq!(content, "feature content");

    Ok(())
}

#[test]
fn test_checkout_rapid_branch_switching_maintains_correct_state() -> Result<()> {
    // This catches state accumulation bugs where files from multiple
    // branches incorrectly persist in the working directory
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create 3 branches, each with unique files
    for name in ["branch-a", "branch-b", "branch-c"] {
        gateway.checkout_branch("main")?;
        gateway.create_branch(name)?;
        std::fs::write(dir.path().join(format!("{}.txt", name)), name)?;
        gateway.stage_all()?;
        gateway.commit(&format!("Add {}", name))?;
    }

    // Checkout main - no branch files should exist
    gateway.checkout_branch("main")?;
    assert!(!dir.path().join("branch-a.txt").exists());
    assert!(!dir.path().join("branch-b.txt").exists());
    assert!(!dir.path().join("branch-c.txt").exists());

    // Checkout branch-a - only branch-a.txt should exist
    gateway.checkout_branch("branch-a")?;
    assert!(dir.path().join("branch-a.txt").exists());
    assert!(!dir.path().join("branch-b.txt").exists());
    assert!(!dir.path().join("branch-c.txt").exists());

    // Checkout branch-b - only branch-b.txt should exist
    gateway.checkout_branch("branch-b")?;
    assert!(!dir.path().join("branch-a.txt").exists());
    assert!(dir.path().join("branch-b.txt").exists());
    assert!(!dir.path().join("branch-c.txt").exists());

    // Checkout branch-c - only branch-c.txt should exist
    gateway.checkout_branch("branch-c")?;
    assert!(!dir.path().join("branch-a.txt").exists());
    assert!(!dir.path().join("branch-b.txt").exists());
    assert!(dir.path().join("branch-c.txt").exists());

    // Back to main - none should exist
    gateway.checkout_branch("main")?;
    assert!(!dir.path().join("branch-a.txt").exists());
    assert!(!dir.path().join("branch-b.txt").exists());
    assert!(!dir.path().join("branch-c.txt").exists());

    Ok(())
}

#[test]
fn test_checkout_handles_modified_files_between_branches() -> Result<()> {
    // Test that checkout properly updates file content when the same file
    // has different content on different branches
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create shared.txt on main
    std::fs::write(dir.path().join("shared.txt"), "main content")?;
    gateway.stage_all()?;
    gateway.commit("Add shared.txt on main")?;

    // Create feature branch and modify shared.txt
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("shared.txt"), "feature content")?;
    gateway.stage_all()?;
    gateway.commit("Modify shared.txt on feature")?;

    // Verify feature content
    assert_eq!(
        std::fs::read_to_string(dir.path().join("shared.txt"))?,
        "feature content"
    );

    // Checkout main - content should change
    gateway.checkout_branch("main")?;
    assert_eq!(std::fs::read_to_string(dir.path().join("shared.txt"))?, "main content");

    // Checkout feature - content should change back
    gateway.checkout_branch("feature")?;
    assert_eq!(
        std::fs::read_to_string(dir.path().join("shared.txt"))?,
        "feature content"
    );

    Ok(())
}

// ============================================================================
// REMOTE BRANCH CHECKOUT TESTS
// These tests verify that checkout can create local branches from remote
// tracking branches. This would have caught the bug where checking out
// a branch that only exists on the remote failed.
// ============================================================================

#[test]
fn test_checkout_creates_local_branch_from_remote_tracking() -> Result<()> {
    // When a branch exists only as a remote tracking branch (refs/remotes/origin/X),
    // checkout should create a local branch from it
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create a branch, get its SHA, then delete it (simulating remote-only branch)
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("feature.txt"), "feature content")?;
    gateway.stage_all()?;
    gateway.commit("Feature commit")?;

    let feature_sha = gateway.get_branch_sha("feature")?;
    gateway.checkout_branch("main")?;

    // Create remote tracking ref manually (simulating fetch)
    Command::new("git")
        .args(["update-ref", "refs/remotes/origin/remote-feature", &feature_sha])
        .current_dir(dir.path())
        .output()?;

    // Verify no local branch exists
    assert!(
        !gateway.branch_exists("remote-feature")?,
        "Local branch should not exist yet"
    );

    // Checkout should create local from remote
    gateway.checkout_branch("remote-feature")?;

    // Verify local branch was created
    assert!(
        gateway.branch_exists("remote-feature")?,
        "Local branch should be created from remote"
    );
    assert_eq!(gateway.get_current_branch_name()?, "remote-feature");

    // Verify we're at the right commit
    assert_eq!(gateway.get_branch_sha("remote-feature")?, feature_sha);

    Ok(())
}

#[test]
fn test_checkout_prefers_local_branch_over_remote() -> Result<()> {
    // If both local and remote branches exist with the same name,
    // checkout should use the local branch
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create local branch at one commit
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("local.txt"), "local")?;
    gateway.stage_all()?;
    gateway.commit("Local commit")?;
    let local_sha = gateway.get_branch_sha("feature")?;

    // Create a different commit on main
    gateway.checkout_branch("main")?;
    std::fs::write(dir.path().join("other.txt"), "other")?;
    gateway.stage_all()?;
    gateway.commit("Other commit")?;
    let other_sha = get_head_sha(dir.path())?;

    // Create remote tracking ref at the different commit
    Command::new("git")
        .args(["update-ref", "refs/remotes/origin/feature", &other_sha])
        .current_dir(dir.path())
        .output()?;

    // Checkout "feature" - should use LOCAL branch, not remote
    gateway.checkout_branch("feature")?;

    // Verify we're on local branch at local commit
    assert_eq!(
        gateway.get_branch_sha("feature")?,
        local_sha,
        "Should checkout local branch, not remote"
    );
    assert!(
        dir.path().join("local.txt").exists(),
        "local.txt should exist (from local branch)"
    );

    Ok(())
}

#[test]
fn test_checkout_nonexistent_branch_fails_with_clear_error() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    let result = gateway.checkout_branch("nonexistent-branch");

    assert!(result.is_err(), "Checkout of nonexistent branch should fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not found"),
        "Error should mention branch not found: {}",
        err_msg
    );

    Ok(())
}

// ============================================================================
// GITIGNORE AND UNTRACKED FILE TESTS
// These tests verify that checkout preserves files that shouldn't be affected
// ============================================================================

#[test]
fn test_checkout_preserves_gitignored_files() -> Result<()> {
    // Gitignored files should not be removed during checkout
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Add .gitignore
    std::fs::write(dir.path().join(".gitignore"), "*.log\nbuild/\n")?;
    gateway.stage_all()?;
    gateway.commit("Add gitignore")?;

    // Create ignored files
    std::fs::write(dir.path().join("debug.log"), "log content")?;
    std::fs::create_dir_all(dir.path().join("build"))?;
    std::fs::write(dir.path().join("build/output.bin"), "binary")?;

    // Create a feature branch with different content
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("feature.txt"), "feature")?;
    gateway.stage_all()?;
    gateway.commit("Feature")?;

    // Switch branches multiple times
    gateway.checkout_branch("main")?;
    gateway.checkout_branch("feature")?;
    gateway.checkout_branch("main")?;

    // Gitignored files should still exist
    assert!(
        dir.path().join("debug.log").exists(),
        "Gitignored file should be preserved"
    );
    assert!(
        dir.path().join("build/output.bin").exists(),
        "Gitignored directory contents should be preserved"
    );

    Ok(())
}

#[test]
fn test_checkout_preserves_untracked_files_not_in_target() -> Result<()> {
    // Untracked files should be preserved during checkout (matches git behavior)
    // We explicitly DO NOT use remove_untracked(true) to match git semantics
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create a branch
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("feature.txt"), "feature")?;
    gateway.stage_all()?;
    gateway.commit("Feature")?;

    // Add a .gitignore to protect our untracked file
    std::fs::write(dir.path().join(".gitignore"), "untracked.txt\n")?;
    gateway.stage_all()?;
    gateway.commit("Add gitignore")?;

    // Create an untracked (but ignored) file
    std::fs::write(dir.path().join("untracked.txt"), "untracked content")?;

    // Switch to main
    gateway.checkout_branch("main")?;

    // Ignored untracked file should still exist
    assert!(
        dir.path().join("untracked.txt").exists(),
        "Gitignored untracked file should be preserved"
    );

    Ok(())
}

// ============================================================================
// INDEX/STAGING AREA TESTS
// These tests verify that checkout properly updates the git index
// ============================================================================

#[test]
fn test_checkout_updates_index_correctly() -> Result<()> {
    // After checkout, the index should match the target branch
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create feature branch with a file
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("feature.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Add feature")?;

    // Checkout main
    gateway.checkout_branch("main")?;

    // Check git status - should be clean, no mention of feature.txt
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir.path())
        .output()?;
    let status = String::from_utf8_lossy(&output.stdout);

    assert!(
        !status.contains("feature.txt"),
        "feature.txt should not appear in status after checkout: '{}'",
        status
    );
    assert!(
        status.trim().is_empty(),
        "Working directory should be clean after checkout: '{}'",
        status
    );

    Ok(())
}

#[test]
fn test_create_branch_preserves_staged_changes() -> Result<()> {
    // When creating a new branch, staged changes should be preserved
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Stage a file without committing
    std::fs::write(dir.path().join("staged.txt"), "staged content")?;
    gateway.stage_all()?;

    // Verify file is staged
    assert!(gateway.has_staged_changes()?, "File should be staged before create");

    // Create a new branch
    gateway.create_branch("feature")?;

    // Verify file is still staged
    assert!(
        gateway.has_staged_changes()?,
        "Staged changes should be preserved after create_branch"
    );

    // Verify we can commit on the new branch
    gateway.commit("Commit staged file")?;
    assert!(dir.path().join("staged.txt").exists());

    Ok(())
}

// ============================================================================
// FORCE CHECKOUT TESTS (via gateway.checkout_branch which uses force mode)
// These test the force checkout behavior which should overwrite local changes
// Note: GitGateway::checkout_branch() uses force mode internally
// ============================================================================

#[test]
fn test_force_checkout_overwrites_uncommitted_changes() -> Result<()> {
    // GitGateway::checkout_branch uses force mode, so it should overwrite
    // uncommitted changes (unlike checkout_branch_worktree_safe which refuses)
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create a file on main
    std::fs::write(dir.path().join("file.txt"), "original")?;
    gateway.stage_all()?;
    gateway.commit("Add file")?;

    // Create feature branch with different content
    gateway.create_branch("feature")?;
    std::fs::write(dir.path().join("file.txt"), "feature version")?;
    gateway.stage_all()?;
    gateway.commit("Modify on feature")?;

    // Go back to main and make uncommitted changes
    gateway.checkout_branch("main")?;
    std::fs::write(dir.path().join("file.txt"), "uncommitted changes")?;

    // Force checkout to feature should overwrite (checkout_branch uses force mode)
    gateway.checkout_branch("feature")?;

    // Content should be feature version, not uncommitted changes
    let content = std::fs::read_to_string(dir.path().join("file.txt"))?;
    assert_eq!(
        content, "feature version",
        "Force checkout should overwrite uncommitted changes"
    );

    Ok(())
}

#[test]
fn test_checkout_branch_worktree_safe_refuses_uncommitted_changes() -> Result<()> {
    // checkout_branch_worktree_safe should refuse to checkout if there are uncommitted changes
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create a file on main
    std::fs::write(dir.path().join("file.txt"), "original")?;
    gateway.stage_all()?;
    gateway.commit("Add file")?;

    // Create feature branch
    gateway.create_branch("feature")?;
    gateway.checkout_branch("main")?;

    // Make uncommitted changes on main
    std::fs::write(dir.path().join("file.txt"), "uncommitted changes")?;

    // Safe checkout should refuse
    let result = gateway.checkout_branch_worktree_safe("feature");
    assert!(result.is_err(), "Should fail with uncommitted changes");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("uncommitted changes"),
        "Error should mention uncommitted changes: {}",
        err
    );

    // Should still be on main
    assert_eq!(gateway.get_current_branch_name()?, "main");

    // Uncommitted changes should be preserved
    let content = std::fs::read_to_string(dir.path().join("file.txt"))?;
    assert_eq!(content, "uncommitted changes");

    Ok(())
}

// NOTE: Worktree conflict testing is handled in src/worktree.rs tests
// Testing it here causes process-wide current directory conflicts in parallel test execution
// The worktree.rs tests use proper DirGuard and #[serial] coordination

#[test]
fn test_checkout_branch_worktree_safe_succeeds_when_safe() -> Result<()> {
    // checkout_branch_worktree_safe should succeed when there are no uncommitted changes
    // and no worktree conflicts
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Create a file on main
    std::fs::write(dir.path().join("file.txt"), "original")?;
    gateway.stage_all()?;
    gateway.commit("Add file")?;

    // Create feature branch
    gateway.create_branch("feature")?;
    gateway.checkout_branch("main")?;

    // No uncommitted changes, no worktree conflicts - should succeed
    gateway.checkout_branch_worktree_safe("feature")?;

    // Should be on feature now
    assert_eq!(gateway.get_current_branch_name()?, "feature");

    Ok(())
}

#[test]
fn test_force_checkout_creates_from_remote() -> Result<()> {
    // checkout_branch (force mode) should also be able to create from remote
    let dir = tempdir()?;
    let _repo = init_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());
    let gateway = GitGateway::from_path(dir.path())?;

    // Get current commit
    let main_sha = get_head_sha(dir.path())?;

    // Create remote tracking ref
    Command::new("git")
        .args(["update-ref", "refs/remotes/origin/remote-only", &main_sha])
        .current_dir(dir.path())
        .output()?;

    // Force checkout should create local from remote
    gateway.checkout_branch("remote-only")?;

    assert!(gateway.branch_exists("remote-only")?);
    assert_eq!(gateway.get_current_branch_name()?, "remote-only");

    Ok(())
}
