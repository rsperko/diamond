#![allow(dead_code)] // Will be used by submit, sync, get commands
//! Forge abstraction layer for git hosting providers
//!
//! This module provides a trait-based abstraction over different git forges
//! (GitHub, GitLab, Bitbucket, Gitea, etc.) allowing Diamond to work with
//! any of them through a common interface.
//!
//! # Architecture
//!
//! The `Forge` trait defines the interface that all forge implementations must
//! provide. Each implementation wraps the respective CLI tool:
//! - GitHub: `gh` CLI
//! - GitLab: `glab` CLI
//! - Bitbucket: custom or API
//! - Gitea: `tea` CLI
//!
//! # Async Support
//!
//! The `AsyncForge` trait provides async versions of forge operations for
//! parallel execution. It includes batch methods that can be optimized with
//! GraphQL queries in the future.
//!
//! # Auto-detection
//!
//! The forge type is auto-detected from the git remote URL:
//! - `github.com` or `*.github.com` → GitHub
//! - `gitlab.com` or `*.gitlab.com` → GitLab
//! - `bitbucket.org` → Bitbucket
//! - Can be overridden in `.git/diamond/config.json`

pub mod ci_wait;
pub mod github;
pub mod gitlab;
pub mod mock;
pub mod types;

pub use ci_wait::{wait_for_ci, CiWaitConfig, CiWaitResult};
pub use github::GitHubForge;
pub use gitlab::GitLabForge;
pub use types::{CiStatus, ForgeConfig, ForgeType, MergeMethod, PrFullInfo, PrInfo, PrOptions, PrState, ReviewState};

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::process::Command;

/// Trait defining the interface for git forge operations
///
/// All forge implementations (GitHub, GitLab, etc.) must implement this trait.
/// This allows Diamond commands to work with any forge through a common interface.
pub trait Forge: Send + Sync {
    /// Returns the forge type
    fn forge_type(&self) -> ForgeType;

    /// Returns the CLI command name (e.g., "gh", "glab")
    fn cli_name(&self) -> &str;

    /// Check if the CLI is installed and authenticated
    fn check_auth(&self) -> Result<()>;

    /// Check if a PR exists for the given branch
    ///
    /// Returns `Some(PrInfo)` if a PR exists, `None` otherwise
    fn pr_exists(&self, branch: &str) -> Result<Option<PrInfo>>;

    /// Create a new PR
    ///
    /// # Arguments
    /// * `branch` - The head branch (branch being merged)
    /// * `base` - The base branch (branch being merged into)
    /// * `title` - PR title
    /// * `body` - PR body/description
    /// * `options` - Additional options (draft, reviewers)
    ///
    /// # Returns
    /// The URL of the created PR
    fn create_pr(&self, branch: &str, base: &str, title: &str, body: &str, options: &PrOptions) -> Result<String>;

    /// Get information about a PR by reference (URL or number)
    fn get_pr_info(&self, pr_ref: &str) -> Result<PrInfo>;

    /// Get the chain of PRs for a stacked branch
    ///
    /// Returns PRs in parent-first order (base → tip)
    fn get_pr_chain(&self, pr_ref: &str) -> Result<Vec<PrInfo>>;

    /// Check if a branch has been merged on the remote
    fn is_branch_merged(&self, branch: &str, into: &str) -> Result<bool>;

    /// Get full PR information including review and CI status
    ///
    /// This includes everything from `get_pr_info` plus:
    /// - Draft status
    /// - Review state (pending, approved, changes requested)
    /// - CI/check status (success, failure, pending)
    fn get_pr_full_info(&self, pr_ref: &str) -> Result<PrFullInfo>;

    /// Get the body/description of a PR
    fn get_pr_body(&self, pr_ref: &str) -> Result<String>;

    /// Update the body/description of a PR
    ///
    /// This replaces the entire body with the new content.
    /// To preserve user content, use `update_pr_description_with_stack`.
    fn update_pr_body(&self, pr_ref: &str, body: &str) -> Result<()>;

    /// Update the base branch of an existing PR
    ///
    /// This is called when a parent branch is merged or when a branch is
    /// moved to a new parent, to keep the PR base in sync with Diamond's
    /// parent relationship.
    fn update_pr_base(&self, branch: &str, new_base: &str) -> Result<()>;

    /// Mark a draft PR as ready for review
    ///
    /// Returns Ok(()) if successful or if the PR is already not a draft.
    fn mark_pr_ready(&self, pr_ref: &str) -> Result<()>;

    /// Enable auto-merge for a PR
    ///
    /// # Arguments
    /// * `pr_ref` - PR reference (number, URL, or branch name)
    /// * `merge_method` - The merge method to use (squash, merge, rebase)
    fn enable_auto_merge(&self, pr_ref: &str, merge_method: &str) -> Result<()>;

    /// Merge a PR/MR
    ///
    /// # Arguments
    /// * `pr_ref` - PR reference (number, URL, or branch name)
    /// * `method` - The merge method to use (squash, merge, rebase)
    /// * `auto_confirm` - If true, skip confirmation prompts (pass --yes or equivalent)
    ///
    /// # Returns
    /// Ok(()) if the merge was successful, Err with details if it failed.
    ///
    /// # Errors
    /// Returns errors for:
    /// - Authentication failures
    /// - PR not mergeable (conflicts, stale branch)
    /// - Failing CI checks
    /// - Required reviews not met
    /// - Merge queue blocking
    fn merge_pr(&self, pr_ref: &str, method: MergeMethod, auto_confirm: bool) -> Result<()>;

    /// Open a PR/MR in the default web browser
    ///
    /// # Arguments
    /// * `pr_ref` - PR reference (number, URL, or branch name)
    ///
    /// # Returns
    /// Ok(()) if the browser was opened successfully, Err if the PR doesn't exist
    /// or the CLI failed.
    fn open_pr_in_browser(&self, pr_ref: &str) -> Result<()>;

    /// Push a branch to the configured remote
    ///
    /// # Arguments
    /// * `branch` - Branch name to push
    /// * `force` - If true, use `--force`, otherwise use `--force-with-lease`
    fn push_branch(&self, branch: &str, force: bool) -> Result<()> {
        let gateway = GitGateway::new()?;
        let force_arg = if force { "--force" } else { "--force-with-lease" };

        if crate::context::ExecutionContext::is_verbose() {
            use colored::Colorize;
            eprintln!(
                "  {} git push --quiet {} {} {}",
                "[cmd]".dimmed(),
                gateway.remote(),
                branch,
                force_arg
            );
        }

        // Use --quiet to suppress remote messages, capture output to reduce noise
        let output = Command::new("git")
            .args(["push", "--quiet", gateway.remote(), branch, force_arg])
            .output()
            .context("Failed to run git push")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Detect protected branch rejection (GitLab/GitHub block force push)
            if stderr.contains("protected branch")
                || stderr.contains("not allowed to force push")
                || stderr.contains("force-pushing to protected branches")
            {
                anyhow::bail!(
                    "Push rejected: branch '{}' is protected against force push.\n\n\
                     {} requires force push for stacked workflows (rebasing changes commit history).\n\n\
                     To fix: enable 'Allow force push' in your repository's branch protection settings,\n\
                     or only protect your main branch (not feature branches).",
                    branch,
                    program_name()
                );
            }

            // Detect force-with-lease rejection (remote has new commits)
            if stderr.contains("stale info")
                || stderr.contains("[rejected]")
                || stderr.contains("fetch first")
                || stderr.contains("non-fast-forward")
            {
                anyhow::bail!(
                    "Push rejected: remote branch '{}' has new commits.\n\
                     Run '{} sync' to pull changes first, or use '--force' to overwrite.",
                    branch,
                    program_name()
                );
            }

            anyhow::bail!("Failed to push branch '{}': {}", branch, stderr.trim());
        }
        Ok(())
    }
}

/// Detect the forge type from the git remote URL
pub fn detect_forge_type() -> Result<ForgeType> {
    let gateway = GitGateway::new()?;
    let url = gateway.get_remote_url(gateway.remote())?;
    detect_forge_from_url(&url)
}

/// Detect forge type from a URL
pub fn detect_forge_from_url(url: &str) -> Result<ForgeType> {
    let url_lower = url.to_lowercase();

    if url_lower.contains("github.com") || url_lower.contains("github.") {
        return Ok(ForgeType::GitHub);
    }
    if url_lower.contains("gitlab.com") || url_lower.contains("gitlab.") {
        return Ok(ForgeType::GitLab);
    }
    if url_lower.contains("bitbucket.org") || url_lower.contains("bitbucket.") {
        return Ok(ForgeType::Bitbucket);
    }
    if url_lower.contains("gitea.") || url_lower.contains("codeberg.org") {
        return Ok(ForgeType::Gitea);
    }

    // Default to GitHub as it's most common
    Ok(ForgeType::GitHub)
}

/// Get a forge instance based on the detected or configured type
pub fn get_forge(config: Option<&ForgeConfig>) -> Result<Box<dyn Forge>> {
    let forge_type = if let Some(cfg) = config {
        if let Some(ft) = cfg.forge_type {
            ft
        } else {
            detect_forge_type()?
        }
    } else {
        detect_forge_type()?
    };

    match forge_type {
        ForgeType::GitHub => Ok(Box::new(GitHubForge::new(config))),
        ForgeType::GitLab => Ok(Box::new(GitLabForge::new(config))),
        ForgeType::Bitbucket => {
            anyhow::bail!("Bitbucket support not yet implemented. Contributions welcome!")
        }
        ForgeType::Gitea => {
            anyhow::bail!("Gitea support not yet implemented. Contributions welcome!")
        }
    }
}

/// Get an async forge instance based on the detected or configured type
pub fn get_async_forge(config: Option<&ForgeConfig>) -> Result<Box<dyn AsyncForge>> {
    let forge_type = if let Some(cfg) = config {
        if let Some(ft) = cfg.forge_type {
            ft
        } else {
            detect_forge_type()?
        }
    } else {
        detect_forge_type()?
    };

    match forge_type {
        ForgeType::GitHub => Ok(Box::new(GitHubForge::new(config))),
        ForgeType::GitLab => Ok(Box::new(GitLabForge::new(config))),
        ForgeType::Bitbucket => {
            anyhow::bail!("Bitbucket support not yet implemented. Contributions welcome!")
        }
        ForgeType::Gitea => {
            anyhow::bail!("Gitea support not yet implemented. Contributions welcome!")
        }
    }
}

/// Async trait for batch forge operations
///
/// This trait extends `Forge` with batch methods that enable parallel
/// execution of independent API calls. These batch methods can be
/// optimized with GraphQL queries in the future.
///
/// The default implementations wrap the sync `Forge` methods and run them
/// concurrently using tokio tasks.
#[async_trait]
pub trait AsyncForge: Forge + Send + Sync {
    /// Batch fetch full PR info for multiple branches
    ///
    /// This is a key optimization point - implementations can use GraphQL
    /// to fetch all PRs in a single API call instead of N calls.
    ///
    /// Default implementation calls sync `get_pr_full_info` concurrently.
    async fn get_prs_full_info(&self, branches: &[String]) -> Vec<PrFullInfo> {
        let futures: Vec<_> = branches
            .iter()
            .map(|branch| {
                let branch = branch.clone();
                // We need to clone self for spawn_blocking, but Forge is not Clone
                // Instead, just call sync method directly (blocking in async context for now)
                // This will be replaced with proper async implementation later
                let result = Forge::get_pr_full_info(self, &branch);
                async move { result }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        // Collect successful results, skip failures (branches without PRs)
        results.into_iter().filter_map(|r| r.ok()).collect()
    }

    /// Batch check PR existence for multiple branches
    ///
    /// Returns a vec of (branch_name, Option<PrInfo>) pairs.
    /// Default implementation calls sync `pr_exists` concurrently.
    async fn check_prs_exist(&self, branches: &[String]) -> Vec<(String, Option<PrInfo>)> {
        let futures: Vec<_> = branches
            .iter()
            .map(|branch| {
                let branch = branch.clone();
                let result = Forge::pr_exists(self, &branch);
                async move { (branch, result.ok().flatten()) }
            })
            .collect();

        futures::future::join_all(futures).await
    }

    /// Batch get PR bodies for multiple PRs
    ///
    /// Returns a vec of (pr_ref, body) pairs for successful fetches.
    async fn get_pr_bodies(&self, pr_refs: &[String]) -> Vec<(String, String)> {
        let futures: Vec<_> = pr_refs
            .iter()
            .map(|pr_ref| {
                let pr_ref = pr_ref.clone();
                let result = Forge::get_pr_body(self, &pr_ref);
                async move { result.ok().map(|body| (pr_ref, body)) }
            })
            .collect();

        let results = futures::future::join_all(futures).await;
        results.into_iter().flatten().collect()
    }

    /// Batch update PR bodies
    ///
    /// Updates multiple PR bodies concurrently.
    /// Returns the number of successful updates.
    async fn update_pr_bodies(&self, updates: &[(String, String)]) -> usize {
        let futures: Vec<_> = updates
            .iter()
            .map(|(pr_ref, body)| {
                let result = Forge::update_pr_body(self, pr_ref, body);
                async move { result.is_ok() }
            })
            .collect();

        let results = futures::future::join_all(futures).await;
        results.into_iter().filter(|&ok| ok).count()
    }

    /// Batch update PR base branches
    ///
    /// Updates the base branch for multiple PRs concurrently.
    /// Takes a slice of (branch_name, new_base) pairs.
    /// Returns the number of successful updates.
    async fn update_pr_bases(&self, updates: &[(String, String)]) -> usize {
        let futures: Vec<_> = updates
            .iter()
            .map(|(branch, new_base)| {
                let result = Forge::update_pr_base(self, branch, new_base);
                async move { result.is_ok() }
            })
            .collect();

        let results = futures::future::join_all(futures).await;
        results.into_iter().filter(|&ok| ok).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === GitHub URL Detection ===

    #[test]
    fn test_detect_forge_from_github_ssh_url() {
        assert_eq!(
            detect_forge_from_url("git@github.com:user/repo.git").unwrap(),
            ForgeType::GitHub
        );
    }

    #[test]
    fn test_detect_forge_from_github_https_url() {
        assert_eq!(
            detect_forge_from_url("https://github.com/user/repo.git").unwrap(),
            ForgeType::GitHub
        );
        assert_eq!(
            detect_forge_from_url("https://github.com/user/repo").unwrap(),
            ForgeType::GitHub
        );
    }

    #[test]
    fn test_detect_forge_from_github_enterprise_url() {
        assert_eq!(
            detect_forge_from_url("https://github.mycompany.com/user/repo.git").unwrap(),
            ForgeType::GitHub
        );
        assert_eq!(
            detect_forge_from_url("git@github.enterprise.corp:org/repo.git").unwrap(),
            ForgeType::GitHub
        );
    }

    #[test]
    fn test_detect_forge_from_github_mixed_case() {
        assert_eq!(
            detect_forge_from_url("https://GitHub.COM/user/repo").unwrap(),
            ForgeType::GitHub
        );
        assert_eq!(
            detect_forge_from_url("git@GITHUB.com:user/repo.git").unwrap(),
            ForgeType::GitHub
        );
    }

    // === GitLab URL Detection ===

    #[test]
    fn test_detect_forge_from_gitlab_ssh_url() {
        assert_eq!(
            detect_forge_from_url("git@gitlab.com:user/repo.git").unwrap(),
            ForgeType::GitLab
        );
    }

    #[test]
    fn test_detect_forge_from_gitlab_https_url() {
        assert_eq!(
            detect_forge_from_url("https://gitlab.com/user/repo").unwrap(),
            ForgeType::GitLab
        );
        assert_eq!(
            detect_forge_from_url("https://gitlab.com/group/subgroup/repo.git").unwrap(),
            ForgeType::GitLab
        );
    }

    #[test]
    fn test_detect_forge_from_gitlab_self_hosted_url() {
        assert_eq!(
            detect_forge_from_url("https://gitlab.mycompany.com/user/repo").unwrap(),
            ForgeType::GitLab
        );
        assert_eq!(
            detect_forge_from_url("git@gitlab.internal.corp:team/project.git").unwrap(),
            ForgeType::GitLab
        );
    }

    #[test]
    fn test_detect_forge_from_gitlab_with_port() {
        assert_eq!(
            detect_forge_from_url("ssh://git@gitlab.company.com:2222/team/repo.git").unwrap(),
            ForgeType::GitLab
        );
    }

    #[test]
    fn test_detect_forge_from_gitlab_mixed_case() {
        assert_eq!(
            detect_forge_from_url("https://GITLAB.com/user/repo").unwrap(),
            ForgeType::GitLab
        );
    }

    // === Bitbucket URL Detection ===

    #[test]
    fn test_detect_forge_from_bitbucket_ssh_url() {
        assert_eq!(
            detect_forge_from_url("git@bitbucket.org:user/repo.git").unwrap(),
            ForgeType::Bitbucket
        );
    }

    #[test]
    fn test_detect_forge_from_bitbucket_https_url() {
        assert_eq!(
            detect_forge_from_url("https://bitbucket.org/user/repo.git").unwrap(),
            ForgeType::Bitbucket
        );
    }

    #[test]
    fn test_detect_forge_from_bitbucket_self_hosted_url() {
        assert_eq!(
            detect_forge_from_url("https://bitbucket.mycompany.com/scm/proj/repo.git").unwrap(),
            ForgeType::Bitbucket
        );
    }

    // === Gitea URL Detection ===

    #[test]
    fn test_detect_forge_from_codeberg_url() {
        assert_eq!(
            detect_forge_from_url("https://codeberg.org/user/repo.git").unwrap(),
            ForgeType::Gitea
        );
        assert_eq!(
            detect_forge_from_url("git@codeberg.org:user/repo.git").unwrap(),
            ForgeType::Gitea
        );
    }

    #[test]
    fn test_detect_forge_from_gitea_self_hosted_url() {
        assert_eq!(
            detect_forge_from_url("https://gitea.mycompany.com/org/repo.git").unwrap(),
            ForgeType::Gitea
        );
    }

    // === Fallback Behavior ===

    #[test]
    fn test_detect_forge_unknown_defaults_to_github() {
        // Unknown hosting providers default to GitHub
        assert_eq!(
            detect_forge_from_url("https://git.mycompany.com/repo.git").unwrap(),
            ForgeType::GitHub
        );
        assert_eq!(
            detect_forge_from_url("https://code.internal.corp/project.git").unwrap(),
            ForgeType::GitHub
        );
        assert_eq!(
            detect_forge_from_url("git@source.company.io:team/repo.git").unwrap(),
            ForgeType::GitHub
        );
    }

    #[test]
    fn test_detect_forge_empty_url_defaults_to_github() {
        // Edge case: empty or minimal URLs
        assert_eq!(detect_forge_from_url("").unwrap(), ForgeType::GitHub);
        assert_eq!(
            detect_forge_from_url("file:///path/to/repo").unwrap(),
            ForgeType::GitHub
        );
    }

    // === Priority/Ambiguity Tests ===

    #[test]
    fn test_detect_forge_github_takes_priority_in_path() {
        // If "github" appears in the URL, it should be detected as GitHub
        // even if other forge names appear in the path
        assert_eq!(
            detect_forge_from_url("https://github.com/user/gitlab-mirror.git").unwrap(),
            ForgeType::GitHub
        );
    }

    #[test]
    fn test_detect_forge_gitlab_takes_priority_in_path() {
        // If "gitlab" appears in the host, it should be detected as GitLab
        // even if "github" appears in the repo name
        assert_eq!(
            detect_forge_from_url("https://gitlab.com/user/github-clone.git").unwrap(),
            ForgeType::GitLab
        );
    }
}
