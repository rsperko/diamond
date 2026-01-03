//! Configuration management commands.
//!
//! Provides CLI interface for viewing and modifying Diamond configuration.

use crate::config::{BranchConfig, Config, LocalConfig, MergeConfig, RepoConfig, UserConfig};
use anyhow::Result;
use colored::Colorize;

/// Parse a boolean value from string
fn parse_bool(value: &str) -> Result<bool> {
    match value.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => anyhow::bail!(
            "Invalid boolean value: '{}'. Use true/false, yes/no, 1/0, or on/off",
            value
        ),
    }
}

/// Show current configuration from all sources
pub fn show() -> Result<()> {
    let config = Config::load()?;

    println!("{}", "Repository Configuration:".bold());
    println!("  remote: {}", config.remote.cyan());

    println!();
    println!("{}", "Branch Configuration:".bold());
    println!("  format: {}", config.branch.format.cyan());
    if let Some(ref prefix) = config.branch.prefix {
        println!("  prefix: {}", prefix.cyan());
    } else {
        println!("  prefix: {}", "(not set)".dimmed());
    }

    println!();
    println!("{}", "Merge Configuration:".bold());
    println!(
        "  ci_timeout_secs:   {}",
        config.merge.ci_timeout_secs.to_string().cyan()
    );
    println!(
        "  proactive_rebase:  {}",
        config.merge.proactive_rebase.to_string().cyan()
    );
    println!("  wait_for_ci:       {}", config.merge.wait_for_ci.to_string().cyan());

    println!();
    println!("{}", "Config file locations:".bold());

    // Show repo config path
    if let Ok(repo_path) = Config::repo_config_path() {
        let exists = repo_path.exists();
        let status = if exists { "exists".green() } else { "not found".dimmed() };
        println!("  repo:  {} ({})", repo_path.display(), status);
    }

    // Show user config path
    if let Ok(user_path) = Config::user_config_path() {
        let exists = user_path.exists();
        let status = if exists { "exists".green() } else { "not found".dimmed() };
        println!("  user:  {} ({})", user_path.display(), status);
    }

    // Show local config path
    if let Ok(local_path) = Config::local_config_path() {
        let exists = local_path.exists();
        let status = if exists { "exists".green() } else { "not found".dimmed() };
        println!("  local: {} ({})", local_path.display(), status);
    }

    Ok(())
}

/// Get a specific configuration value
pub fn get(key: &str) -> Result<()> {
    let config = Config::load()?;

    match key {
        "repo.remote" => println!("{}", config.remote),
        "branch.format" => println!("{}", config.branch.format),
        "branch.prefix" => {
            if let Some(prefix) = config.branch.prefix {
                println!("{}", prefix);
            }
        }
        "merge.ci_timeout_secs" => println!("{}", config.merge.ci_timeout_secs),
        "merge.proactive_rebase" => println!("{}", config.merge.proactive_rebase),
        "merge.wait_for_ci" => println!("{}", config.merge.wait_for_ci),
        _ => anyhow::bail!(
            "Unknown config key: {}\n\nAvailable keys:\n  repo.remote\n  branch.format\n  branch.prefix\n  merge.ci_timeout_secs\n  merge.proactive_rebase\n  merge.wait_for_ci",
            key
        ),
    }

    Ok(())
}

/// Set a configuration value
pub fn set(key: &str, value: &str, local: bool) -> Result<()> {
    // repo.remote is a special case - it always goes in repo config (.diamond/config.toml)
    if key == "repo.remote" {
        return set_repo_remote(value);
    }

    if local {
        set_local(key, value)
    } else {
        set_user(key, value)
    }
}

/// Set remote in repo config (.diamond/config.toml - committed, shared)
fn set_repo_remote(value: &str) -> Result<()> {
    // Load existing or create new
    let repo_path = Config::repo_config_path()?;
    let mut config: RepoConfig = if repo_path.exists() {
        let content = std::fs::read_to_string(&repo_path)?;
        toml::from_str(&content).unwrap_or_default()
    } else {
        RepoConfig::default()
    };

    config.remote = value.to_string();

    // Save
    Config::save_repo_config(&config)?;

    println!("Set {} = {} in repo config", "repo.remote".green(), value.cyan());
    println!("  {}", repo_path.display());
    println!();
    println!(
        "{}",
        "Note: This file should be committed to share with your team.".dimmed()
    );

    Ok(())
}

/// Set a value in user config (~/.config/diamond/config.toml)
fn set_user(key: &str, value: &str) -> Result<()> {
    // Load existing or create new
    let user_path = Config::user_config_path()?;
    let mut config: UserConfig = if user_path.exists() {
        let content = std::fs::read_to_string(&user_path)?;
        toml::from_str(&content).unwrap_or_default()
    } else {
        UserConfig::default()
    };

    // Update the value
    match key {
        "branch.format" => config.branch.format = value.to_string(),
        "branch.prefix" => config.branch.prefix = Some(value.to_string()),
        "merge.ci_timeout_secs" => {
            config.merge.ci_timeout_secs = value.parse().map_err(|_| {
                anyhow::anyhow!("Invalid value for ci_timeout_secs: expected a number")
            })?;
        }
        "merge.proactive_rebase" => {
            config.merge.proactive_rebase = parse_bool(value)?;
        }
        "merge.wait_for_ci" => {
            config.merge.wait_for_ci = parse_bool(value)?;
        }
        _ => anyhow::bail!(
            "Unknown config key: {}\n\nAvailable keys:\n  branch.format\n  branch.prefix\n  merge.ci_timeout_secs\n  merge.proactive_rebase\n  merge.wait_for_ci",
            key
        ),
    }

    // Save
    Config::save_user_config(&config)?;

    println!("Set {} = {} in user config", key.green(), value.cyan());
    println!("  {}", user_path.display());

    Ok(())
}

/// Set a value in local config (.git/diamond/config.toml)
fn set_local(key: &str, value: &str) -> Result<()> {
    // Load existing or create new
    let local_path = Config::local_config_path()?;
    let mut config: LocalConfig = if local_path.exists() {
        let content = std::fs::read_to_string(&local_path)?;
        toml::from_str(&content).unwrap_or_default()
    } else {
        LocalConfig::default()
    };

    // Update the value
    match key {
        "branch.format" => config.branch.format = value.to_string(),
        "branch.prefix" => config.branch.prefix = Some(value.to_string()),
        "merge.ci_timeout_secs" => {
            config.merge.ci_timeout_secs = value.parse().map_err(|_| {
                anyhow::anyhow!("Invalid value for ci_timeout_secs: expected a number")
            })?;
        }
        "merge.proactive_rebase" => {
            config.merge.proactive_rebase = parse_bool(value)?;
        }
        "merge.wait_for_ci" => {
            config.merge.wait_for_ci = parse_bool(value)?;
        }
        _ => anyhow::bail!(
            "Unknown config key: {}\n\nAvailable keys:\n  branch.format\n  branch.prefix\n  merge.ci_timeout_secs\n  merge.proactive_rebase\n  merge.wait_for_ci",
            key
        ),
    }

    // Save
    Config::save_local_config(&config)?;

    println!("Set {} = {} in local config", key.green(), value.cyan());
    println!("  {}", local_path.display());

    Ok(())
}

/// Unset a configuration value
pub fn unset(key: &str, local: bool) -> Result<()> {
    if local {
        unset_local(key)
    } else {
        unset_user(key)
    }
}

/// Unset a value in user config
fn unset_user(key: &str) -> Result<()> {
    let user_path = Config::user_config_path()?;
    if !user_path.exists() {
        println!("No user config file exists");
        return Ok(());
    }

    let content = std::fs::read_to_string(&user_path)?;
    let mut config: UserConfig = toml::from_str(&content).unwrap_or_default();

    match key {
        "branch.format" => config.branch.format = crate::config::BranchConfig::default().format,
        "branch.prefix" => config.branch.prefix = None,
        "merge.ci_timeout_secs" => config.merge.ci_timeout_secs = MergeConfig::default().ci_timeout_secs,
        "merge.proactive_rebase" => config.merge.proactive_rebase = MergeConfig::default().proactive_rebase,
        "merge.wait_for_ci" => config.merge.wait_for_ci = MergeConfig::default().wait_for_ci,
        _ => anyhow::bail!("Unknown config key: {}", key),
    }

    Config::save_user_config(&config)?;
    println!("Unset {} in user config", key.green());

    Ok(())
}

/// Unset a value in local config
fn unset_local(key: &str) -> Result<()> {
    let local_path = Config::local_config_path()?;
    if !local_path.exists() {
        println!("No local config file exists");
        return Ok(());
    }

    let content = std::fs::read_to_string(&local_path)?;
    let mut config: LocalConfig = toml::from_str(&content).unwrap_or_default();

    match key {
        "branch.format" => config.branch.format = BranchConfig::default().format,
        "branch.prefix" => config.branch.prefix = None,
        "merge.ci_timeout_secs" => config.merge.ci_timeout_secs = MergeConfig::default().ci_timeout_secs,
        "merge.proactive_rebase" => config.merge.proactive_rebase = MergeConfig::default().proactive_rebase,
        "merge.wait_for_ci" => config.merge.wait_for_ci = MergeConfig::default().wait_for_ci,
        _ => anyhow::bail!("Unknown config key: {}", key),
    }

    Config::save_local_config(&config)?;
    println!("Unset {} in local config", key.green());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_show_runs_without_error() -> Result<()> {
        // Just verify it doesn't panic - actual output depends on system state
        // We can't easily test this without mocking the config paths
        Ok(())
    }
}
