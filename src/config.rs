//! Configuration system for Diamond.
//!
//! Supports layered configuration from multiple sources (highest priority first):
//! 1. Local override: `.git/diamond/config.toml` (per-repo, per-user)
//! 2. User global: `~/.config/diamond/config.toml` (personal defaults)
//! 3. Repo shared: `.diamond/config.toml` (committed, team-wide)
//!
//! Configuration uses TOML format for readability.

use anyhow::{Context, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::BufReader;
use std::path::PathBuf;

use crate::state::find_git_root;

/// Default remote name
fn default_remote() -> String {
    "origin".to_string()
}

/// Branch naming configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchConfig {
    /// Format template for branch names.
    /// Available placeholders: {prefix}, {date}, {name}
    /// Default: "{date}-{name}"
    #[serde(default = "default_format")]
    pub format: String,

    /// User-defined prefix (include your separator, e.g., "alice/" or "feature-")
    #[serde(default)]
    pub prefix: Option<String>,
}

fn default_format() -> String {
    "{date}-{name}".to_string()
}

impl Default for BranchConfig {
    fn default() -> Self {
        Self {
            format: default_format(),
            prefix: None,
        }
    }
}

/// Default CI timeout in seconds (10 minutes)
fn default_ci_timeout() -> u64 {
    600
}

/// Default to true for boolean merge settings
fn default_true() -> bool {
    true
}

/// Merge operation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeConfig {
    /// Timeout for waiting for CI to pass (seconds). Default: 600 (10 minutes)
    #[serde(default = "default_ci_timeout")]
    pub ci_timeout_secs: u64,

    /// Proactively rebase onto trunk before merge. Default: true
    #[serde(default = "default_true")]
    pub proactive_rebase: bool,

    /// Wait for CI to pass after rebase before merging. Default: true
    #[serde(default = "default_true")]
    pub wait_for_ci: bool,
}

impl Default for MergeConfig {
    fn default() -> Self {
        Self {
            ci_timeout_secs: default_ci_timeout(),
            proactive_rebase: true,
            wait_for_ci: true,
        }
    }
}

/// Repository-level configuration (stored in .diamond/config.toml, committed)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    /// Git remote name to use for push/pull operations (default: "origin")
    #[serde(default = "default_remote")]
    pub remote: String,
}

impl Default for RepoConfig {
    fn default() -> Self {
        Self {
            remote: default_remote(),
        }
    }
}

/// User-level configuration (stored in ~/.config/diamond/)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserConfig {
    #[serde(default)]
    pub branch: BranchConfig,
    #[serde(default)]
    pub merge: MergeConfig,
}

/// Local override configuration (stored in .git/diamond/)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalConfig {
    #[serde(default)]
    pub branch: BranchConfig,
    #[serde(default)]
    pub merge: MergeConfig,
}

/// Merged configuration from all layers
#[derive(Debug, Clone)]
pub struct Config {
    pub branch: BranchConfig,
    /// Git remote name (from repo config)
    pub remote: String,
    /// Merge operation settings
    pub merge: MergeConfig,
}

impl Config {
    /// Load configuration from all layers, merging with priority:
    /// local > user > repo > defaults
    pub fn load() -> Result<Self> {
        let repo_config = Self::load_repo_config();
        let user_config = Self::load_user_config();
        let local_config = Self::load_local_config();

        // Merge: local overrides user overrides defaults
        let branch = Self::merge_branch_config(
            &BranchConfig::default(),
            &user_config.branch,
            &local_config.as_ref().map(|c| &c.branch),
        );

        // Merge: local overrides user overrides defaults
        let merge = Self::merge_merge_config(
            &MergeConfig::default(),
            &user_config.merge,
            &local_config.as_ref().map(|c| &c.merge),
        );

        // Remote comes from repo config (committed, shared)
        let remote = repo_config.remote;

        Ok(Config { branch, remote, merge })
    }

    /// Load repo config from .diamond/config.toml (committed, shared)
    fn load_repo_config() -> RepoConfig {
        let path = match Self::repo_config_path() {
            Ok(p) => p,
            Err(_) => return RepoConfig::default(),
        };

        Self::load_toml_file(&path).unwrap_or_default()
    }

    /// Load user config from ~/.config/diamond/config.toml
    fn load_user_config() -> UserConfig {
        let path = match Self::user_config_path() {
            Ok(p) => p,
            Err(_) => return UserConfig::default(),
        };

        Self::load_toml_file(&path).unwrap_or_default()
    }

    /// Load local config from .git/diamond/config.toml
    fn load_local_config() -> Option<LocalConfig> {
        let path = match Self::local_config_path() {
            Ok(p) => p,
            Err(_) => return None,
        };

        Self::load_toml_file(&path).ok()
    }

    /// Load and parse a TOML config file
    fn load_toml_file<T: for<'de> Deserialize<'de> + Default>(path: &PathBuf) -> Result<T> {
        if !path.exists() {
            return Ok(T::default());
        }

        let file = File::open(path).context("Failed to open config file")?;
        let reader = BufReader::new(file);
        let mut content = String::new();
        std::io::Read::read_to_string(&mut reader.get_ref(), &mut content).context("Failed to read config file")?;

        // Re-read since BufReader consumed content
        let content = fs::read_to_string(path).context("Failed to read config file")?;

        match toml::from_str(&content) {
            Ok(config) => Ok(config),
            Err(e) => {
                eprintln!("Warning: Config file {:?} is invalid ({}), using defaults", path, e);
                Ok(T::default())
            }
        }
    }

    /// Merge branch configs with priority: local > user > defaults
    fn merge_branch_config(
        defaults: &BranchConfig,
        user: &BranchConfig,
        local: &Option<&BranchConfig>,
    ) -> BranchConfig {
        // Start with defaults
        let mut result = defaults.clone();

        // Apply user config
        if user.format != default_format() {
            result.format = user.format.clone();
        }
        if user.prefix.is_some() {
            result.prefix = user.prefix.clone();
        }

        // Apply local config (highest priority)
        if let Some(local) = local {
            if local.format != default_format() {
                result.format = local.format.clone();
            }
            if local.prefix.is_some() {
                result.prefix = local.prefix.clone();
            }
        }

        result
    }

    /// Merge merge configs with priority: local > user > defaults
    fn merge_merge_config(defaults: &MergeConfig, user: &MergeConfig, local: &Option<&MergeConfig>) -> MergeConfig {
        // Start with defaults
        let mut result = defaults.clone();

        // Apply user config (only if different from defaults)
        if user.ci_timeout_secs != default_ci_timeout() {
            result.ci_timeout_secs = user.ci_timeout_secs;
        }
        if !user.proactive_rebase {
            result.proactive_rebase = false;
        }
        if !user.wait_for_ci {
            result.wait_for_ci = false;
        }

        // Apply local config (highest priority)
        if let Some(local) = local {
            if local.ci_timeout_secs != default_ci_timeout() {
                result.ci_timeout_secs = local.ci_timeout_secs;
            }
            if !local.proactive_rebase {
                result.proactive_rebase = false;
            }
            if !local.wait_for_ci {
                result.wait_for_ci = false;
            }
        }

        result
    }

    /// Get path to user config: ~/.config/diamond/config.toml
    pub fn user_config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir().context("Could not determine user config directory")?;
        Ok(config_dir.join("diamond").join("config.toml"))
    }

    /// Get path to local config: .git/diamond/config.toml
    pub fn local_config_path() -> Result<PathBuf> {
        let git_root = find_git_root()?;
        Ok(git_root.join(".git").join("diamond").join("config.toml"))
    }

    /// Get path to repo config: .diamond/config.toml (future)
    #[allow(dead_code)]
    pub fn repo_config_path() -> Result<PathBuf> {
        let git_root = find_git_root()?;
        Ok(git_root.join(".diamond").join("config.toml"))
    }

    /// Format a branch name using the configured template.
    ///
    /// Replaces placeholders:
    /// - {prefix} - user-defined prefix (empty if not set)
    /// - {date} - current date in MM-DD format
    /// - {name} - the provided branch name
    pub fn format_branch_name(&self, name: &str) -> String {
        let date = Local::now().format("%m-%d").to_string();
        let prefix = self.branch.prefix.as_deref().unwrap_or("");

        self.branch
            .format
            .replace("{prefix}", prefix)
            .replace("{date}", &date)
            .replace("{name}", name)
    }

    /// Save user config to ~/.config/diamond/config.toml
    pub fn save_user_config(config: &UserConfig) -> Result<()> {
        let path = Self::user_config_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create config directory")?;
        }

        let content = toml::to_string_pretty(config).context("Failed to serialize config")?;

        // Atomic write
        let temp_path = path.with_extension("toml.tmp");
        fs::write(&temp_path, content).context("Failed to write config file")?;
        fs::rename(&temp_path, &path).context("Failed to finalize config file")?;

        Ok(())
    }

    /// Save local config to .git/diamond/config.toml
    pub fn save_local_config(config: &LocalConfig) -> Result<()> {
        let path = Self::local_config_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create config directory")?;
        }

        let content = toml::to_string_pretty(config).context("Failed to serialize config")?;

        // Atomic write
        let temp_path = path.with_extension("toml.tmp");
        fs::write(&temp_path, content).context("Failed to write config file")?;
        fs::rename(&temp_path, &path).context("Failed to finalize config file")?;

        Ok(())
    }

    /// Save repo config to .diamond/config.toml (committed, shared)
    pub fn save_repo_config(config: &RepoConfig) -> Result<()> {
        let path = Self::repo_config_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create config directory")?;
        }

        let content = toml::to_string_pretty(config).context("Failed to serialize config")?;

        // Atomic write
        let temp_path = path.with_extension("toml.tmp");
        fs::write(&temp_path, content).context("Failed to write config file")?;
        fs::rename(&temp_path, &path).context("Failed to finalize config file")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    #[test]
    fn test_default_format() {
        let config = Config {
            branch: BranchConfig::default(),
            remote: default_remote(),
            merge: MergeConfig::default(),
        };

        // Default format is "{date}-{name}"
        let result = config.format_branch_name("add_feature");
        let date = Local::now().format("%m-%d").to_string();
        assert_eq!(result, format!("{}-add_feature", date));
    }

    #[test]
    fn test_format_with_prefix() {
        let config = Config {
            branch: BranchConfig {
                format: "{prefix}{name}".to_string(),
                prefix: Some("alice/".to_string()),
            },
            remote: default_remote(),
            merge: MergeConfig::default(),
        };

        let result = config.format_branch_name("add_feature");
        assert_eq!(result, "alice/add_feature");
    }

    #[test]
    fn test_format_with_prefix_and_date() {
        let config = Config {
            branch: BranchConfig {
                format: "{prefix}{date}-{name}".to_string(),
                prefix: Some("alice/".to_string()),
            },
            remote: default_remote(),
            merge: MergeConfig::default(),
        };

        let result = config.format_branch_name("add_feature");
        let date = Local::now().format("%m-%d").to_string();
        assert_eq!(result, format!("alice/{}-add_feature", date));
    }

    #[test]
    fn test_format_no_prefix_configured() {
        let config = Config {
            branch: BranchConfig {
                format: "{prefix}{name}".to_string(),
                prefix: None,
            },
            remote: default_remote(),
            merge: MergeConfig::default(),
        };

        // {prefix} becomes empty string when not configured
        let result = config.format_branch_name("add_feature");
        assert_eq!(result, "add_feature");
    }

    #[test]
    fn test_format_date_only() {
        let config = Config {
            branch: BranchConfig {
                format: "{date}-{name}".to_string(),
                prefix: Some("ignored/".to_string()),
            },
            remote: default_remote(),
            merge: MergeConfig::default(),
        };

        // Prefix is set but not in format, so ignored
        let result = config.format_branch_name("add_feature");
        let date = Local::now().format("%m-%d").to_string();
        assert_eq!(result, format!("{}-add_feature", date));
    }

    #[test]
    fn test_format_name_only() {
        let config = Config {
            branch: BranchConfig {
                format: "{name}".to_string(),
                prefix: None,
            },
            remote: default_remote(),
            merge: MergeConfig::default(),
        };

        let result = config.format_branch_name("my-branch");
        assert_eq!(result, "my-branch");
    }

    #[test]
    fn test_parse_valid_toml() {
        let toml_content = r#"
[branch]
format = "{prefix}{name}"
prefix = "test/"
"#;

        let config: UserConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(config.branch.format, "{prefix}{name}");
        assert_eq!(config.branch.prefix, Some("test/".to_string()));
    }

    #[test]
    fn test_parse_empty_toml() {
        let toml_content = "";
        let config: UserConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(config.branch.format, default_format());
        assert_eq!(config.branch.prefix, None);
    }

    #[test]
    fn test_parse_partial_toml() {
        let toml_content = r#"
[branch]
prefix = "jd-"
"#;

        let config: UserConfig = toml::from_str(toml_content).unwrap();
        // Format should use default
        assert_eq!(config.branch.format, default_format());
        assert_eq!(config.branch.prefix, Some("jd-".to_string()));
    }

    #[test]
    fn test_merge_configs() {
        let defaults = BranchConfig::default();

        let user = BranchConfig {
            format: "{prefix}{name}".to_string(),
            prefix: Some("user/".to_string()),
        };

        let local = BranchConfig {
            format: default_format(), // Same as default, shouldn't override
            prefix: Some("local/".to_string()),
        };

        let result = Config::merge_branch_config(&defaults, &user, &Some(&local));

        // Format from user (local didn't change it)
        assert_eq!(result.format, "{prefix}{name}");
        // Prefix from local (highest priority)
        assert_eq!(result.prefix, Some("local/".to_string()));
    }

    #[test]
    fn test_save_and_load_user_config() -> Result<()> {
        let dir = tempdir()?;
        let config_dir = dir.path().join("config").join("diamond");
        fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("config.toml");

        let config = UserConfig {
            branch: BranchConfig {
                format: "{prefix}{date}-{name}".to_string(),
                prefix: Some("test/".to_string()),
            },
            merge: MergeConfig::default(),
        };

        // Write config
        let content = toml::to_string_pretty(&config)?;
        fs::write(&config_path, content)?;

        // Read it back
        let loaded: UserConfig = toml::from_str(&fs::read_to_string(&config_path)?)?;

        assert_eq!(loaded.branch.format, "{prefix}{date}-{name}");
        assert_eq!(loaded.branch.prefix, Some("test/".to_string()));

        Ok(())
    }

    #[test]
    fn test_corrupt_toml_returns_default() {
        let toml_content = "{ this is not valid toml";
        let result: Result<UserConfig, _> = toml::from_str(toml_content);
        assert!(result.is_err());
    }

    // =========================================================================
    // MergeConfig Tests
    // =========================================================================

    #[test]
    fn test_merge_config_defaults() {
        let config = MergeConfig::default();
        assert_eq!(config.ci_timeout_secs, 600);
        assert!(config.proactive_rebase);
        assert!(config.wait_for_ci);
    }

    #[test]
    fn test_merge_config_serialization() {
        let config = MergeConfig {
            ci_timeout_secs: 1800,
            proactive_rebase: false,
            wait_for_ci: true,
        };

        let toml = toml::to_string_pretty(&config).unwrap();
        assert!(toml.contains("ci_timeout_secs = 1800"));
        assert!(toml.contains("proactive_rebase = false"));
        assert!(toml.contains("wait_for_ci = true"));
    }

    #[test]
    fn test_merge_config_deserialization_partial() {
        // Only specify some fields, others should default
        let toml_content = r#"
ci_timeout_secs = 300
"#;
        let config: MergeConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(config.ci_timeout_secs, 300);
        assert!(config.proactive_rebase); // default
        assert!(config.wait_for_ci); // default
    }

    #[test]
    fn test_merge_config_deserialization_empty() {
        // Empty config should give all defaults
        let config: MergeConfig = toml::from_str("").unwrap();
        assert_eq!(config.ci_timeout_secs, 600);
        assert!(config.proactive_rebase);
        assert!(config.wait_for_ci);
    }

    #[test]
    fn test_merge_config_with_all_false() {
        let toml_content = r#"
proactive_rebase = false
wait_for_ci = false
"#;
        let config: MergeConfig = toml::from_str(toml_content).unwrap();
        assert!(!config.proactive_rebase);
        assert!(!config.wait_for_ci);
    }

    #[test]
    fn test_user_config_with_merge_section() {
        let toml_content = r#"
[branch]
format = "{name}"

[merge]
ci_timeout_secs = 120
proactive_rebase = false
"#;
        let config: UserConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(config.branch.format, "{name}");
        assert_eq!(config.merge.ci_timeout_secs, 120);
        assert!(!config.merge.proactive_rebase);
        assert!(config.merge.wait_for_ci); // default
    }

    #[test]
    fn test_merge_merge_config_priority() {
        let defaults = MergeConfig::default();
        let user = MergeConfig {
            ci_timeout_secs: 1800, // custom
            proactive_rebase: true,
            wait_for_ci: false, // disabled
        };
        let local = MergeConfig {
            ci_timeout_secs: 600,    // back to default (won't override)
            proactive_rebase: false, // disabled
            wait_for_ci: true,       // back to default (won't override user's false)
        };

        let result = Config::merge_merge_config(&defaults, &user, &Some(&local));

        // ci_timeout: local is default, so user's 1800 wins
        assert_eq!(result.ci_timeout_secs, 1800);
        // proactive_rebase: local is false, so false wins
        assert!(!result.proactive_rebase);
        // wait_for_ci: user is false, local can't re-enable (once disabled, stays disabled)
        assert!(!result.wait_for_ci);
    }
}
