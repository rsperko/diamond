mod common;

use anyhow::Result;
use common::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_config_show_works() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["config", "show"])?;
    assert!(
        output.status.success(),
        "dm config show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should show branch configuration section
    assert!(stdout.contains("Branch Configuration"));
    // Should show format (default)
    assert!(stdout.contains("{date}-{name}"));

    Ok(())
}

#[test]
fn test_config_get_branch_format() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["config", "get", "branch.format"])?;
    assert!(
        output.status.success(),
        "dm config get failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("{date}-{name}"));

    Ok(())
}

#[test]
fn test_config_set_local_prefix() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Set a local prefix
    let output = run_dm(temp_dir.path(), &["config", "set", "branch.prefix", "test/", "--local"])?;
    assert!(
        output.status.success(),
        "dm config set failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the config file was created
    let config_path = temp_dir.path().join(".git/diamond/config.toml");
    assert!(config_path.exists(), "Local config file should exist");

    let config_content = fs::read_to_string(&config_path)?;
    assert!(config_content.contains("prefix"), "Config should contain prefix");
    assert!(
        config_content.contains("test/"),
        "Config should contain the prefix value"
    );

    Ok(())
}

#[test]
fn test_create_with_format_from_message() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Create branch using -m (auto-generated name should use format)
    let output = run_dm(temp_dir.path(), &["create", "-m", "Add feature"])?;
    assert!(
        output.status.success(),
        "dm create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // With default format "{date}-{name}", branch should be MM-DD-add_feature
    let current_branch = get_current_branch(temp_dir.path())?;
    assert!(
        current_branch.ends_with("-add_feature"),
        "Expected branch to end with '-add_feature', got: {}",
        current_branch
    );

    Ok(())
}

#[test]
fn test_create_explicit_name_not_formatted() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Set a prefix in local config
    run_dm(temp_dir.path(), &["config", "set", "branch.prefix", "test/", "--local"])?;
    run_dm(
        temp_dir.path(),
        &["config", "set", "branch.format", "{prefix}{name}", "--local"],
    )?;

    // Create branch with explicit name - should NOT apply prefix
    let output = run_dm(temp_dir.path(), &["create", "my-branch"])?;
    assert!(
        output.status.success(),
        "dm create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Branch should be exactly "my-branch", not "test/my-branch"
    let current_branch = get_current_branch(temp_dir.path())?;
    assert_eq!(
        current_branch, "my-branch",
        "Explicit branch name should not be formatted"
    );

    Ok(())
}

#[test]
fn test_create_with_prefix_config() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Set prefix and format in local config
    run_dm(
        temp_dir.path(),
        &["config", "set", "branch.prefix", "alice/", "--local"],
    )?;
    run_dm(
        temp_dir.path(),
        &["config", "set", "branch.format", "{prefix}{name}", "--local"],
    )?;

    // Create branch using -m (should apply prefix)
    let output = run_dm(temp_dir.path(), &["create", "-m", "Add feature"])?;
    assert!(
        output.status.success(),
        "dm create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Branch should be "alice/add_feature"
    let current_branch = get_current_branch(temp_dir.path())?;
    assert_eq!(current_branch, "alice/add_feature", "Branch should have prefix applied");

    Ok(())
}

#[test]
fn test_create_with_prefix_and_date() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Set prefix and format with date
    run_dm(
        temp_dir.path(),
        &["config", "set", "branch.prefix", "alice/", "--local"],
    )?;
    run_dm(
        temp_dir.path(),
        &["config", "set", "branch.format", "{prefix}{date}-{name}", "--local"],
    )?;

    // Create branch using -m (should apply prefix and date)
    let output = run_dm(temp_dir.path(), &["create", "-m", "Add feature"])?;
    assert!(
        output.status.success(),
        "dm create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Branch should be "alice/MM-DD-add_feature"
    let current_branch = get_current_branch(temp_dir.path())?;
    assert!(
        current_branch.starts_with("alice/"),
        "Branch should start with prefix 'alice/', got: {}",
        current_branch
    );
    assert!(
        current_branch.ends_with("-add_feature"),
        "Branch should end with '-add_feature', got: {}",
        current_branch
    );
    // Verify date format in the middle
    let without_prefix = current_branch.strip_prefix("alice/").unwrap();
    let parts: Vec<&str> = without_prefix.splitn(3, '-').collect();
    assert_eq!(parts.len(), 3, "Should have date and name parts");
    assert!(parts[0].len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()));
    assert!(parts[1].len() == 2 && parts[1].chars().all(|c| c.is_ascii_digit()));

    Ok(())
}

#[test]
fn test_config_unset() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Set a prefix
    run_dm(temp_dir.path(), &["config", "set", "branch.prefix", "test/", "--local"])?;

    // Verify it was set
    let config_path = temp_dir.path().join(".git/diamond/config.toml");
    let content = fs::read_to_string(&config_path)?;
    assert!(content.contains("test/"));

    // Unset it
    let output = run_dm(temp_dir.path(), &["config", "unset", "branch.prefix", "--local"])?;
    assert!(
        output.status.success(),
        "dm config unset failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify prefix is no longer set (file may still exist but prefix should be None)
    let output = run_dm(temp_dir.path(), &["config", "get", "branch.prefix"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Empty output means prefix is not set
    assert!(stdout.trim().is_empty(), "Prefix should be unset, got: {}", stdout);

    Ok(())
}

#[test]
fn test_config_get_repo_remote_default() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Get default remote (should be "origin")
    let output = run_dm(temp_dir.path(), &["config", "get", "repo.remote"])?;
    assert!(
        output.status.success(),
        "dm config get repo.remote failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "origin", "Default remote should be 'origin'");

    Ok(())
}

#[test]
fn test_config_set_repo_remote() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    // Set repo.remote to "upstream"
    let output = run_dm(temp_dir.path(), &["config", "set", "repo.remote", "upstream"])?;
    assert!(
        output.status.success(),
        "dm config set repo.remote failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the config file was created in .diamond/
    let config_path = temp_dir.path().join(".diamond/config.toml");
    assert!(
        config_path.exists(),
        "Repo config file should exist at .diamond/config.toml"
    );

    let config_content = fs::read_to_string(&config_path)?;
    assert!(config_content.contains("remote"), "Config should contain remote");
    assert!(
        config_content.contains("upstream"),
        "Config should contain the remote value"
    );

    // Verify it reads back correctly
    let output = run_dm(temp_dir.path(), &["config", "get", "repo.remote"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "upstream", "Remote should be 'upstream'");

    Ok(())
}

#[test]
fn test_config_show_displays_remote() -> Result<()> {
    let temp_dir = TempDir::new()?;
    init_test_repo(temp_dir.path())?;

    let output = run_dm(temp_dir.path(), &["config", "show"])?;
    assert!(
        output.status.success(),
        "dm config show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should show repository configuration section with remote
    assert!(
        stdout.contains("Repository Configuration"),
        "Should show Repository Configuration section"
    );
    assert!(stdout.contains("remote:"), "Should show remote setting");
    assert!(stdout.contains("origin"), "Should show default remote value");

    Ok(())
}
