mod common;

use anyhow::Result;
use common::*;
use tempfile::TempDir;

#[test]
fn test_passthrough_status_command() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["status"])?;

    // Should succeed
    assert!(
        output.status.success(),
        "dm status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should show passthrough message
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Passing command through to git"),
        "Expected passthrough message, got: {}",
        stderr
    );

    // Should show git status output
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("On branch") || stdout.contains("nothing to commit"),
        "Expected git status output, got: {}",
        stdout
    );

    Ok(())
}

#[test]
fn test_passthrough_diff_command() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["diff"])?;

    // Should succeed (even with no diff)
    assert!(
        output.status.success(),
        "dm diff failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should show passthrough message
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Passing command through to git"));

    Ok(())
}

#[test]
fn test_passthrough_with_arguments() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Test git show with arguments
    let output = run_dm(temp_dir.path(), &["show", "--oneline", "-s", "HEAD"])?;

    assert!(
        output.status.success(),
        "dm show --oneline -s HEAD failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Running: \"git show --oneline -s HEAD\""));

    Ok(())
}

#[test]
fn test_passthrough_unknown_command_shows_help() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["notarealcommand"])?;

    // Should fail
    assert!(!output.status.success(), "dm notarealcommand should have failed");

    // Should show Diamond help (either in stdout or stderr depending on clap version)
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("Diamond") || combined.contains("dm") || combined.contains("Usage"),
        "Expected help output, got stdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    Ok(())
}

#[test]
fn test_passthrough_git_failure_propagates() -> Result<()> {
    // Test outside a git repo - git status should fail
    let temp_dir = TempDir::new()?;
    // Don't init git repo

    let output = run_dm(temp_dir.path(), &["status"])?;

    // Should fail because not a git repo
    assert!(!output.status.success(), "dm status outside git repo should fail");

    Ok(())
}

#[test]
fn test_diamond_commands_not_passed_through() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // dm log should NOT pass through to git log
    // It should use Diamond's log command
    let output = run_dm(temp_dir.path(), &["log", "short"])?;

    // Should succeed
    assert!(
        output.status.success(),
        "dm log short failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should NOT contain passthrough message
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Passing command through to git"),
        "dm log should not pass through to git, got: {}",
        stderr
    );

    Ok(())
}

#[test]
fn test_passthrough_version() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["version"])?;

    // Should succeed
    assert!(
        output.status.success(),
        "dm version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should show git version output
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("git version"),
        "Expected git version output, got: {}",
        stdout
    );

    Ok(())
}

#[test]
fn test_passthrough_remote_command() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["remote", "-v"])?;

    // Should succeed (even with no remotes)
    assert!(
        output.status.success(),
        "dm remote -v failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should show passthrough message
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Passing command through to git"));

    Ok(())
}

#[test]
fn test_passthrough_rev_parse_command() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Use rev-parse which is a git command Diamond doesn't have
    let output = run_dm(temp_dir.path(), &["rev-parse", "--show-toplevel"])?;

    // Should succeed
    assert!(
        output.status.success(),
        "dm rev-parse --show-toplevel failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should show passthrough message
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Passing command through to git"));

    // Should show the repo path
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(temp_dir.path().to_str().unwrap())
            || stdout
                .trim()
                .ends_with(temp_dir.path().file_name().unwrap().to_str().unwrap()),
        "Expected repo path, got: {}",
        stdout
    );

    Ok(())
}
