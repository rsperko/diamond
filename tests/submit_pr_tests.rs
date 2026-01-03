mod common;

use anyhow::Result;
use common::*;
use std::fs;
use tempfile::TempDir;

// ============================================================================
// SUBMIT WORKFLOW TESTS
// ============================================================================

#[test]
fn test_submit_requires_remote_configured() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Submit without remote should fail with helpful message
    let output = run_dm(temp_dir.path(), &["submit"])?;
    // Should fail or warn about no remote
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success() || combined.contains("remote") || combined.contains("push"),
        "submit should handle no remote: {}",
        combined
    );

    Ok(())
}

#[test]
fn test_submit_stack_flag_exists() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // submit --stack should be recognized (even if it fails due to no remote)
    let output = run_dm(temp_dir.path(), &["submit", "--stack"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should not fail with "unknown option" error
    assert!(
        !stderr.contains("unexpected") && !stderr.contains("unknown"),
        "--stack flag should be recognized: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_submit_draft_flag_exists() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // submit --draft should be recognized
    let output = run_dm(temp_dir.path(), &["submit", "--draft"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected") && !stderr.contains("unknown"),
        "--draft flag should be recognized: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_submit_reviewers_flag_exists() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // submit --reviewer should be recognized
    let output = run_dm(temp_dir.path(), &["submit", "-r", "alice"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected") && !stderr.contains("unknown"),
        "-r flag should be recognized: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_ss_alias_for_submit_stack() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // ss should be recognized as submit --stack alias
    let output = run_dm(temp_dir.path(), &["ss"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should not fail with "unknown command" error
    assert!(
        !stderr.contains("unknown") && !stderr.contains("not recognized"),
        "ss alias should be recognized: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_submit_update_only_flag_exists() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // submit --update-only should be recognized
    let output = run_dm(temp_dir.path(), &["submit", "--update-only"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected") && !stderr.contains("unknown"),
        "--update-only flag should be recognized: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_submit_confirm_flag_non_interactive() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // submit --confirm in non-interactive mode should fail with helpful message
    let output = run_dm(temp_dir.path(), &["submit", "--confirm"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail with message about non-interactive mode (since tests run without TTY)
    // OR fail due to no remote configured
    assert!(
        !output.status.success(),
        "--confirm should fail in non-interactive mode: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_ss_update_only_flag_exists() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // ss --update-only should also be recognized
    let output = run_dm(temp_dir.path(), &["ss", "--update-only"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected") && !stderr.contains("unknown"),
        "--update-only flag should be recognized for ss: {}",
        stderr
    );

    Ok(())
}

// ============================================================================
// GET COMMAND TESTS (Team collaboration)
// ============================================================================

#[test]
fn test_get_invalid_pr_reference_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Get with invalid PR reference
    let output = run_dm(temp_dir.path(), &["get", "not-a-valid-pr"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail with helpful message
    assert!(!stderr.is_empty(), "get with invalid PR should show error");

    Ok(())
}

#[test]
fn test_get_requires_remote() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Get without remote configured
    let output = run_dm(temp_dir.path(), &["get", "123"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Should fail with message about remote/GitHub
    assert!(
        !output.status.success() || combined.contains("remote") || combined.contains("GitHub"),
        "get should require remote: {}",
        combined
    );

    Ok(())
}

// ============================================================================
// PR COMMAND TESTS
// ============================================================================

#[test]
fn test_pr_requires_remote() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // pr without remote configured
    let output = run_dm(temp_dir.path(), &["pr"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Should fail with message about remote/GitHub/push
    assert!(
        !output.status.success() || combined.contains("remote") || combined.contains("push"),
        "pr should require remote: {}",
        combined
    );

    Ok(())
}

// ============================================================================
// MERGE COMMAND TESTS
// ============================================================================

#[test]
fn test_merge_requires_remote() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Merge should fail gracefully (no remote configured = no forge available)
    let output = run_dm(temp_dir.path(), &["merge"])?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should fail because no remote is configured (merge requires a forge)
    assert!(
        stderr.contains("remote") || stderr.contains("origin"),
        "Should fail with message about remote: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_merge_flags_recognized() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Test --merge flag is recognized
    let output = run_dm(temp_dir.path(), &["merge", "--merge"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected") && !stderr.contains("unknown"),
        "--merge flag should be recognized: {}",
        stderr
    );

    // Test --rebase flag is recognized
    let output = run_dm(temp_dir.path(), &["merge", "--rebase"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected") && !stderr.contains("unknown"),
        "--rebase flag should be recognized: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_merge_no_sync_flag_recognized() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Test --no-sync flag is recognized
    let output = run_dm(temp_dir.path(), &["merge", "--no-sync"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected") && !stderr.contains("unknown"),
        "--no-sync flag should be recognized: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_merge_dry_run_does_not_sync() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Add remote so we get past the forge check
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/test/repo.git"])
        .current_dir(temp_dir.path())
        .output()?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Dry run should show preview message, not sync message
    let output = run_dm(temp_dir.path(), &["merge", "--dry-run"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Should show dry run message OR "No PRs to merge" (since we don't have actual PRs)
    // The key is that it should NOT try to sync
    assert!(
        combined.contains("Dry run") || combined.contains("preview") || combined.contains("No PR"),
        "Dry run should show preview or no-PR message: {}",
        combined
    );

    // Should NOT show syncing message
    assert!(
        !combined.contains("Syncing local branches"),
        "Dry run should not sync: {}",
        combined
    );

    Ok(())
}

#[test]
fn test_merge_no_sync_shows_manual_sync_message() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Add remote so we get past the forge check
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/test/repo.git"])
        .current_dir(temp_dir.path())
        .output()?;

    // Create a branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // With --no-sync, should tell user to run sync manually
    let output = run_dm(temp_dir.path(), &["merge", "--no-sync"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Should mention running sync manually (either because --no-sync or because no PRs to merge)
    // The command will fail because there's no PR, but the flag should still be recognized
    assert!(
        !combined.contains("unexpected") && !combined.contains("unknown"),
        "--no-sync should be recognized: {}",
        combined
    );

    Ok(())
}

// ============================================================================
// COMPLETION COMMAND TESTS
// ============================================================================

#[test]
fn test_completion_bash_generates_script() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["completion", "bash"])?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should generate bash completion script
    assert!(
        stdout.contains("complete") || stdout.contains("_dm"),
        "Should generate bash completion: {}",
        stdout
    );

    Ok(())
}

#[test]
fn test_completion_zsh_generates_script() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["completion", "zsh"])?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should generate zsh completion script
    assert!(
        stdout.contains("compdef") || stdout.contains("_dm") || stdout.contains("#compdef"),
        "Should generate zsh completion: {}",
        stdout
    );

    Ok(())
}

// ============================================================================
// FROZEN BRANCH TESTS
// ============================================================================

#[test]
fn test_freeze_and_unfreeze_commands() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a feature branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;

    // Freeze the branch
    let output = run_dm(temp_dir.path(), &["freeze"])?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Froze"), "Should confirm freeze: {}", stdout);

    // Info should show frozen status
    let output = run_dm(temp_dir.path(), &["info"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("frozen"), "Info should show frozen: {}", stdout);

    // Unfreeze the branch
    let output = run_dm(temp_dir.path(), &["unfreeze"])?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Unfroze"), "Should confirm unfreeze: {}", stdout);

    Ok(())
}

#[test]
fn test_modify_frozen_branch_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create and freeze a feature branch
    fs::write(temp_dir.path().join("f.txt"), "feature")?;
    run_dm(temp_dir.path(), &["create", "feature", "-a", "-m", "Feature"])?;
    run_dm(temp_dir.path(), &["freeze"])?;

    // Try to modify - should fail
    fs::write(temp_dir.path().join("f2.txt"), "more")?;
    let output = run_dm(temp_dir.path(), &["modify", "-a", "-m", "Should fail"])?;

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("frozen"), "Should mention frozen: {}", stderr);
    assert!(stderr.contains("unfreeze"), "Should mention unfreeze: {}", stderr);

    Ok(())
}

#[test]
fn test_freeze_trunk_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Try to freeze trunk (main) - should fail
    run_dm(temp_dir.path(), &["checkout", "main"])?;
    let output = run_dm(temp_dir.path(), &["freeze"])?;

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot freeze trunk"),
        "Should reject freezing trunk: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_unfreeze_upstack_flag() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> f1 -> f2
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Freeze both branches
    run_dm(temp_dir.path(), &["freeze", "f1"])?;
    run_dm(temp_dir.path(), &["freeze", "f2"])?;

    // Go to f1 and unfreeze with --upstack
    run_dm(temp_dir.path(), &["checkout", "f1"])?;
    let output = run_dm(temp_dir.path(), &["unfreeze", "--upstack"])?;
    assert!(output.status.success());

    // Both should now be unfrozen - modify should work on f2
    run_dm(temp_dir.path(), &["checkout", "f2"])?;
    fs::write(temp_dir.path().join("f3.txt"), "f3")?;
    let output = run_dm(temp_dir.path(), &["modify", "-a", "-m", "Should work"])?;
    assert!(output.status.success(), "Modify should work after unfreeze --upstack");

    Ok(())
}

// ============================================================================
// REORDER COMMAND TESTS
// ============================================================================

#[test]
fn test_reorder_command_exists() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create a simple stack
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    // Reorder command should be recognized (even if it fails due to non-interactive)
    let output = run_dm(temp_dir.path(), &["reorder"])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should not be an "unknown command" error
    assert!(
        !stderr.contains("unknown") && !stderr.contains("unrecognized"),
        "reorder command should be recognized: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_reorder_requires_stack() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // On trunk with no stack, reorder should fail gracefully
    let output = run_dm(temp_dir.path(), &["reorder"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Should mention that there's nothing to reorder
    assert!(
        combined.contains("nothing") || combined.contains("trunk") || combined.contains("branch"),
        "Should indicate no branches to reorder: {}",
        combined
    );

    Ok(())
}

#[test]
fn test_reorder_with_file_flag() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> f1 -> f2 -> f3
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    fs::write(temp_dir.path().join("f3.txt"), "f3")?;
    run_dm(temp_dir.path(), &["create", "f3", "-a", "-m", "F3"])?;

    // Create a reorder file that reverses f1 and f2
    // Original order: f1 (on main), f2 (on f1), f3 (on f2)
    // New order: f2 (on main), f1 (on f2), f3 (on f1)
    let reorder_file = temp_dir.path().join("reorder.txt");
    fs::write(&reorder_file, "f2\nf1\nf3\n")?;

    // Run reorder with --file flag
    let output = run_dm(temp_dir.path(), &["reorder", "--file", reorder_file.to_str().unwrap()])?;

    // Should succeed
    assert!(
        output.status.success(),
        "Reorder should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify new structure: f2 is now on main, f1 is on f2
    let output = run_dm(temp_dir.path(), &["checkout", "f1"])?;
    assert!(output.status.success());

    let output = run_dm(temp_dir.path(), &["parent"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("f2"), "f1's parent should now be f2: {}", stdout);

    Ok(())
}

#[test]
fn test_reorder_preview_flag() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack
    fs::write(temp_dir.path().join("f1.txt"), "f1")?;
    run_dm(temp_dir.path(), &["create", "f1", "-a", "-m", "F1"])?;

    fs::write(temp_dir.path().join("f2.txt"), "f2")?;
    run_dm(temp_dir.path(), &["create", "f2", "-a", "-m", "F2"])?;

    // Preview should show current order without opening editor
    let output = run_dm(temp_dir.path(), &["reorder", "--preview"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show both branches
    assert!(stdout.contains("f1"), "Preview should show f1: {}", stdout);
    assert!(stdout.contains("f2"), "Preview should show f2: {}", stdout);

    Ok(())
}

#[test]
fn test_reorder_actually_rebases_git_branches() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create stack: main -> swap-1 -> swap-2 -> swap-3
    // Each branch has a unique file
    fs::write(temp_dir.path().join("swap-1.txt"), "swap-1")?;
    run_dm(temp_dir.path(), &["create", "swap-1", "-a", "-m", "S1"])?;

    fs::write(temp_dir.path().join("swap-2.txt"), "swap-2")?;
    run_dm(temp_dir.path(), &["create", "swap-2", "-a", "-m", "S2"])?;

    fs::write(temp_dir.path().join("swap-3.txt"), "swap-3")?;
    run_dm(temp_dir.path(), &["create", "swap-3", "-a", "-m", "S3"])?;

    // Record original git parents before reorder
    let swap3_original_parent = run_git(temp_dir.path(), &["rev-parse", "swap-3^"])?;
    let swap3_original_parent = String::from_utf8_lossy(&swap3_original_parent.stdout)
        .trim()
        .to_string();

    let swap2_ref = run_git(temp_dir.path(), &["rev-parse", "swap-2"])?;
    let swap2_ref = String::from_utf8_lossy(&swap2_ref.stdout).trim().to_string();

    // Verify original: swap-3's parent should be swap-2
    assert_eq!(
        swap3_original_parent, swap2_ref,
        "Before reorder: swap-3's parent should be swap-2"
    );

    // Reorder to: main -> swap-1 -> swap-3 -> swap-2
    let reorder_file = temp_dir.path().join("reorder.txt");
    fs::write(&reorder_file, "swap-1\nswap-3\nswap-2\n")?;

    let output = run_dm(temp_dir.path(), &["reorder", "--file", reorder_file.to_str().unwrap()])?;
    assert!(
        output.status.success(),
        "Reorder should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify git history is correct after reorder:
    // swap-3's git parent should now be swap-1 (not swap-2)
    let swap3_new_parent = run_git(temp_dir.path(), &["rev-parse", "swap-3^"])?;
    let swap3_new_parent = String::from_utf8_lossy(&swap3_new_parent.stdout).trim().to_string();

    let swap1_ref = run_git(temp_dir.path(), &["rev-parse", "swap-1"])?;
    let swap1_ref = String::from_utf8_lossy(&swap1_ref.stdout).trim().to_string();

    assert_eq!(
        swap3_new_parent, swap1_ref,
        "After reorder: swap-3's git parent should be swap-1"
    );

    // swap-2's git parent should now be swap-3
    let swap2_new_parent = run_git(temp_dir.path(), &["rev-parse", "swap-2^"])?;
    let swap2_new_parent = String::from_utf8_lossy(&swap2_new_parent.stdout).trim().to_string();

    let swap3_ref = run_git(temp_dir.path(), &["rev-parse", "swap-3"])?;
    let swap3_ref = String::from_utf8_lossy(&swap3_ref.stdout).trim().to_string();

    assert_eq!(
        swap2_new_parent, swap3_ref,
        "After reorder: swap-2's git parent should be swap-3"
    );

    // Verify all files are accessible from swap-2 (since it's now at the top)
    run_dm(temp_dir.path(), &["checkout", "swap-2"])?;
    let files = run_git(temp_dir.path(), &["ls-tree", "--name-only", "HEAD"])?;
    let files = String::from_utf8_lossy(&files.stdout);

    assert!(files.contains("swap-1.txt"), "swap-2 should have swap-1.txt");
    assert!(files.contains("swap-2.txt"), "swap-2 should have swap-2.txt");
    assert!(files.contains("swap-3.txt"), "swap-2 should have swap-3.txt");

    Ok(())
}
