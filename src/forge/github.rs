//! GitHub forge implementation using the `gh` CLI
//!
//! This implementation wraps the GitHub CLI (`gh`) to provide
//! PR operations for GitHub repositories.

use super::{AsyncForge, CiStatus, Forge, ForgeConfig, ForgeType, MergeMethod, PrFullInfo, PrInfo, PrState, ReviewState};
use crate::git_gateway::GitGateway;
use anyhow::{Context, Result};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Maximum number of retries for rate-limited requests
const MAX_RATE_LIMIT_RETRIES: u32 = 3;

/// Base delay for exponential backoff (seconds)
const RATE_LIMIT_BASE_DELAY_SECS: u64 = 5;

/// GitHub forge implementation
pub struct GitHubForge {
    /// Custom host for GitHub Enterprise
    host: Option<String>,
}

impl GitHubForge {
    /// Create a new GitHub forge instance
    pub fn new(config: Option<&ForgeConfig>) -> Self {
        let host = config.and_then(|c| c.host.clone());
        Self { host }
    }

    /// Run a gh command with optional host override
    ///
    /// This method includes automatic retry with exponential backoff for
    /// rate-limited requests. GitHub API rate limits return specific error
    /// messages that we detect and handle.
    fn run_gh(&self, args: &[&str]) -> Result<std::process::Output> {
        self.run_gh_with_retry(args, MAX_RATE_LIMIT_RETRIES)
    }

    /// Internal implementation with retry support
    fn run_gh_with_retry(&self, args: &[&str], max_retries: u32) -> Result<std::process::Output> {
        let mut retries = 0;

        loop {
            let mut cmd = Command::new("gh");

            // Add host if configured (for GitHub Enterprise)
            if let Some(ref host) = self.host {
                cmd.env("GH_HOST", host);
            }

            let output = cmd
                .args(args)
                .output()
                .with_context(|| format!("Failed to run 'gh {}'. Is gh CLI installed?", args.join(" ")))?;

            // Check for rate limiting
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);

                if Self::is_rate_limited(&stderr) && retries < max_retries {
                    retries += 1;
                    let delay_secs = RATE_LIMIT_BASE_DELAY_SECS * (1 << retries); // Exponential backoff
                    eprintln!(
                        "GitHub API rate limited. Retrying in {} seconds ({}/{})",
                        delay_secs, retries, max_retries
                    );
                    thread::sleep(Duration::from_secs(delay_secs));
                    continue;
                }
            }

            return Ok(output);
        }
    }

    /// Check if the error indicates rate limiting
    fn is_rate_limited(stderr: &str) -> bool {
        let stderr_lower = stderr.to_lowercase();
        stderr_lower.contains("rate limit")
            || stderr_lower.contains("api rate")
            || stderr_lower.contains("secondary rate")
            || stderr_lower.contains("abuse detection")
            || stderr_lower.contains("try again later")
            || stderr_lower.contains("too many requests")
    }

    /// Format a gh command error with helpful context
    fn format_gh_error(args: &[&str], output: &std::process::Output) -> String {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        let mut msg = format!("gh {} failed", args.join(" "));

        if !stderr.is_empty() {
            msg.push_str(&format!("\n  Error: {}", stderr.trim()));
        }
        if !stdout.is_empty() && stderr.is_empty() {
            msg.push_str(&format!("\n  Output: {}", stdout.trim()));
        }

        // Add hints for common issues
        if stderr.contains("not logged") || stderr.contains("authentication") {
            msg.push_str("\n  Hint: Run 'gh auth login' to authenticate.");
        } else if stderr.contains("Could not resolve") {
            msg.push_str("\n  Hint: Ensure you're in a git repository with a GitHub remote.");
        }

        msg
    }

    /// Parse PR state from gh CLI output
    fn parse_pr_state(state: &str) -> PrState {
        match state.to_uppercase().as_str() {
            "OPEN" => PrState::Open,
            "CLOSED" => PrState::Closed,
            "MERGED" => PrState::Merged,
            _ => PrState::Open, // Default to open
        }
    }

    /// Parse review state from gh CLI reviews array
    ///
    /// GitHub returns an array of reviews. We determine the overall state by:
    /// 1. If any review is CHANGES_REQUESTED, return ChangesRequested
    /// 2. If any review is APPROVED, return Approved
    /// 3. If any review is COMMENTED, return Commented
    /// 4. Otherwise return Pending
    fn parse_review_state(reviews: &serde_json::Value) -> ReviewState {
        if let Some(arr) = reviews.as_array() {
            let mut has_approved = false;
            let mut has_commented = false;

            for review in arr {
                let state = review["state"].as_str().unwrap_or("");
                match state.to_uppercase().as_str() {
                    "CHANGES_REQUESTED" => return ReviewState::ChangesRequested,
                    "APPROVED" => has_approved = true,
                    "COMMENTED" => has_commented = true,
                    _ => {}
                }
            }

            if has_approved {
                return ReviewState::Approved;
            }
            if has_commented {
                return ReviewState::Commented;
            }
        }
        ReviewState::Pending
    }

    /// Parse CI status from gh CLI statusCheckRollup
    ///
    /// statusCheckRollup contains an array of check results.
    /// We determine the overall status by:
    /// 1. If any check is FAILURE or ERROR, return Failure
    /// 2. If any check is PENDING or IN_PROGRESS, return Pending
    /// 3. If all checks are SUCCESS, return Success
    /// 4. If no checks, return None
    fn parse_ci_status(status_check_rollup: &serde_json::Value) -> CiStatus {
        if let Some(arr) = status_check_rollup.as_array() {
            if arr.is_empty() {
                return CiStatus::None;
            }

            let mut all_success = true;
            let mut has_pending = false;

            for check in arr {
                // Status checks use "state", check runs use "conclusion"
                let state = check["state"]
                    .as_str()
                    .or_else(|| check["status"].as_str())
                    .unwrap_or("");
                let conclusion = check["conclusion"].as_str().unwrap_or("");

                match (state.to_uppercase().as_str(), conclusion.to_uppercase().as_str()) {
                    ("FAILURE", _) | ("ERROR", _) | (_, "FAILURE") | (_, "ERROR") => return CiStatus::Failure,
                    ("PENDING", _) | ("IN_PROGRESS", _) | ("QUEUED", _) => {
                        has_pending = true;
                        all_success = false;
                    }
                    ("SUCCESS", _) | (_, "SUCCESS") => {}
                    ("SKIPPED", _) | (_, "SKIPPED") | (_, "NEUTRAL") => {}
                    _ => {
                        all_success = false;
                    }
                }
            }

            if has_pending {
                return CiStatus::Pending;
            }
            if all_success {
                return CiStatus::Success;
            }
        }
        CiStatus::None
    }
}

impl Forge for GitHubForge {
    fn forge_type(&self) -> ForgeType {
        ForgeType::GitHub
    }

    fn cli_name(&self) -> &str {
        "gh"
    }

    fn check_auth(&self) -> Result<()> {
        let output = self.run_gh(&["auth", "status"])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not logged") {
                anyhow::bail!("Not authenticated with GitHub CLI. Run 'gh auth login' to authenticate.");
            }
            anyhow::bail!("GitHub CLI auth check failed: {}", stderr);
        }
        Ok(())
    }

    fn pr_exists(&self, branch: &str) -> Result<Option<PrInfo>> {
        let args = [
            "pr",
            "view",
            branch,
            "--json",
            "number,url,headRefName,baseRefName,state,title",
        ];
        let output = self.run_gh(&args)?;

        if !output.status.success() {
            // gh returns error if no PR exists - this is not an error for us
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no pull requests found")
                || stderr.contains("Could not resolve")
                || stderr.contains("no open pull requests")
            {
                return Ok(None);
            }
            anyhow::bail!("{}", Self::format_gh_error(&args, &output));
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("Failed to parse gh pr view output")?;

        Ok(Some(PrInfo {
            number: json["number"].as_u64().unwrap_or(0),
            url: json["url"].as_str().unwrap_or("").to_string(),
            head_ref: json["headRefName"].as_str().unwrap_or("").to_string(),
            base_ref: json["baseRefName"].as_str().unwrap_or("").to_string(),
            state: Self::parse_pr_state(json["state"].as_str().unwrap_or("OPEN")),
            title: json["title"].as_str().unwrap_or("").to_string(),
        }))
    }

    fn create_pr(
        &self,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
        options: &super::PrOptions,
    ) -> Result<String> {
        let mut args = vec![
            "pr".to_string(),
            "create".to_string(),
            "--head".to_string(),
            branch.to_string(),
            "--base".to_string(),
            base.to_string(),
            "--title".to_string(),
            title.to_string(),
            "--body".to_string(),
            body.to_string(),
        ];

        // Add --draft if requested
        if options.draft {
            args.push("--draft".to_string());
        }

        // Add reviewers if specified
        for reviewer in &options.reviewers {
            args.push("--reviewer".to_string());
            args.push(reviewer.clone());
        }

        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let output = self.run_gh(&args_refs)?;

        if !output.status.success() {
            anyhow::bail!("{}", Self::format_gh_error(&args_refs, &output));
        }

        // gh pr create outputs the PR URL
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(url)
    }

    fn get_pr_info(&self, pr_ref: &str) -> Result<PrInfo> {
        let output = self.run_gh(&[
            "pr",
            "view",
            pr_ref,
            "--json",
            "number,url,headRefName,baseRefName,state,title",
        ])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to get PR info: {}", stderr);
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("Failed to parse gh pr view output")?;

        Ok(PrInfo {
            number: json["number"].as_u64().unwrap_or(0),
            url: json["url"].as_str().unwrap_or("").to_string(),
            head_ref: json["headRefName"].as_str().unwrap_or("").to_string(),
            base_ref: json["baseRefName"].as_str().unwrap_or("").to_string(),
            state: Self::parse_pr_state(json["state"].as_str().unwrap_or("OPEN")),
            title: json["title"].as_str().unwrap_or("").to_string(),
        })
    }

    fn get_pr_chain(&self, pr_ref: &str) -> Result<Vec<PrInfo>> {
        let mut chain = Vec::new();
        let mut current_ref = pr_ref.to_string();

        // Walk up the PR chain until we hit a non-PR base (like main)
        loop {
            let info = self.get_pr_info(&current_ref)?;
            let base = info.base_ref.clone();
            chain.push(info);

            // Check if the base branch has a PR
            match self.pr_exists(&base)? {
                Some(_) => {
                    current_ref = base;
                }
                None => break, // Base is not a PR (probably trunk)
            }
        }

        // Reverse to get parent-first order
        chain.reverse();
        Ok(chain)
    }

    fn is_branch_merged(&self, branch: &str, into: &str) -> Result<bool> {
        // Check if the branch's PR is merged
        if let Some(pr) = self.pr_exists(branch)? {
            if pr.state == PrState::Merged {
                return Ok(true);
            }
        }

        // Also check if the branch exists on remote
        let gateway = GitGateway::new()?;
        let output = Command::new("git")
            .args(["ls-remote", "--heads", gateway.remote(), branch])
            .output()
            .context("Failed to check remote branch")?;

        if output.stdout.is_empty() {
            // Branch doesn't exist on remote - might have been deleted after merge
            // Check if the local branch's tip is reachable from the target

            match gateway.get_merge_base(branch, &format!("{}/{}", gateway.remote(), into)) {
                Ok(merge_base) => {
                    let branch_oid = gateway.resolve_ref(branch)?;
                    let branch_tip = branch_oid.to_string();

                    // If merge-base equals branch tip (first 7 chars), it's been merged
                    return Ok(merge_base.starts_with(&branch_tip[..7]));
                }
                Err(_) => {
                    // No common ancestor, branch not merged
                }
            }
        }

        Ok(false)
    }

    fn get_pr_full_info(&self, pr_ref: &str) -> Result<PrFullInfo> {
        let output = self.run_gh(&[
            "pr",
            "view",
            pr_ref,
            "--json",
            "number,url,title,state,isDraft,headRefName,baseRefName,reviews,statusCheckRollup",
        ])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to get PR info: {}", stderr);
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("Failed to parse gh pr view output")?;

        Ok(PrFullInfo {
            number: json["number"].as_u64().unwrap_or(0),
            url: json["url"].as_str().unwrap_or("").to_string(),
            title: json["title"].as_str().unwrap_or("").to_string(),
            state: Self::parse_pr_state(json["state"].as_str().unwrap_or("OPEN")),
            is_draft: json["isDraft"].as_bool().unwrap_or(false),
            review: Self::parse_review_state(&json["reviews"]),
            ci: Self::parse_ci_status(&json["statusCheckRollup"]),
            head_ref: json["headRefName"].as_str().unwrap_or("").to_string(),
            base_ref: json["baseRefName"].as_str().unwrap_or("").to_string(),
        })
    }

    fn get_pr_body(&self, pr_ref: &str) -> Result<String> {
        let output = self.run_gh(&["pr", "view", pr_ref, "--json", "body"])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to get PR body: {}", stderr);
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("Failed to parse gh pr view output")?;

        Ok(json["body"].as_str().unwrap_or("").to_string())
    }

    fn update_pr_body(&self, pr_ref: &str, body: &str) -> Result<()> {
        let output = self.run_gh(&["pr", "edit", pr_ref, "--body", body])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to update PR body: {}", stderr);
        }

        Ok(())
    }

    fn update_pr_base(&self, branch: &str, new_base: &str) -> Result<()> {
        let output = self.run_gh(&["pr", "edit", branch, "--base", new_base])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to update PR base: {}", stderr);
        }

        Ok(())
    }

    fn mark_pr_ready(&self, pr_ref: &str) -> Result<()> {
        let output = self.run_gh(&["pr", "ready", pr_ref])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "already not a draft" type errors
            if stderr.contains("not a draft") || stderr.contains("already") {
                return Ok(());
            }
            anyhow::bail!("Failed to mark PR as ready: {}", stderr);
        }

        Ok(())
    }

    fn enable_auto_merge(&self, pr_ref: &str, merge_method: &str) -> Result<()> {
        let output = self.run_gh(&["pr", "merge", pr_ref, "--auto", &format!("--{}", merge_method)])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Return a helpful error message
            if stderr.contains("auto-merge is not allowed") {
                anyhow::bail!(
                    "Auto-merge is not enabled for this repository.\n\
                     Enable it in repository settings under 'General > Pull Requests > Allow auto-merge'."
                );
            }
            anyhow::bail!("Failed to enable auto-merge: {}", stderr);
        }

        Ok(())
    }

    fn merge_pr(&self, pr_ref: &str, method: MergeMethod, auto_confirm: bool) -> Result<()> {
        let method_flag = match method {
            MergeMethod::Squash => "--squash",
            MergeMethod::Merge => "--merge",
            MergeMethod::Rebase => "--rebase",
        };

        let mut args = vec!["pr", "merge", pr_ref, method_flag];

        // Add --yes if auto-confirm is enabled
        if auto_confirm {
            args.push("--yes");
        }

        let output = self.run_gh(&args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Check for common errors and provide helpful messages
            if stderr.contains("not authenticated") || stderr.contains("gh auth login") {
                anyhow::bail!("GitHub CLI not authenticated. Run 'gh auth login' first.");
            }
            if stderr.contains("merge queue") {
                anyhow::bail!("PR {} is in a merge queue. Visit the PR page to check status.", pr_ref);
            }
            if stderr.contains("required status") || stderr.contains("checks") {
                anyhow::bail!("PR {} has failing checks or required status checks not met.", pr_ref);
            }
            if stderr.contains("review") && stderr.contains("required") {
                anyhow::bail!("PR {} requires reviews that haven't been approved.", pr_ref);
            }

            anyhow::bail!("gh pr merge failed: {}", stderr.trim());
        }

        Ok(())
    }

    fn open_pr_in_browser(&self, pr_ref: &str) -> Result<()> {
        let output = self.run_gh(&["pr", "view", pr_ref, "--web"])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            if stderr.contains("no pull requests found") || stderr.contains("Could not resolve") {
                anyhow::bail!("No PR found for '{}'. Does it exist?", pr_ref);
            }

            anyhow::bail!("Failed to open PR: {}", stderr.trim());
        }

        Ok(())
    }
}

/// AsyncForge implementation uses default methods that wrap sync Forge calls
impl AsyncForge for GitHubForge {
    // All batch methods use default implementations from the trait
    // These will be replaced with GraphQL batch queries in the future
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pr_state() {
        assert_eq!(GitHubForge::parse_pr_state("OPEN"), PrState::Open);
        assert_eq!(GitHubForge::parse_pr_state("CLOSED"), PrState::Closed);
        assert_eq!(GitHubForge::parse_pr_state("MERGED"), PrState::Merged);
        assert_eq!(GitHubForge::parse_pr_state("open"), PrState::Open);
        assert_eq!(GitHubForge::parse_pr_state("unknown"), PrState::Open);
    }

    #[test]
    fn test_github_forge_new() {
        let forge = GitHubForge::new(None);
        assert_eq!(forge.forge_type(), ForgeType::GitHub);
        assert_eq!(forge.cli_name(), "gh");
        assert!(forge.host.is_none());
    }

    #[test]
    fn test_github_forge_with_enterprise_host() {
        let config = ForgeConfig {
            forge_type: Some(ForgeType::GitHub),
            host: Some("github.mycompany.com".to_string()),
        };
        let forge = GitHubForge::new(Some(&config));
        assert_eq!(forge.host, Some("github.mycompany.com".to_string()));
    }

    #[test]
    fn test_parse_review_state_pending() {
        let reviews = serde_json::json!([]);
        assert_eq!(GitHubForge::parse_review_state(&reviews), ReviewState::Pending);

        let reviews = serde_json::json!(null);
        assert_eq!(GitHubForge::parse_review_state(&reviews), ReviewState::Pending);
    }

    #[test]
    fn test_parse_review_state_approved() {
        let reviews = serde_json::json!([
            {"state": "APPROVED", "author": {"login": "user1"}}
        ]);
        assert_eq!(GitHubForge::parse_review_state(&reviews), ReviewState::Approved);
    }

    #[test]
    fn test_parse_review_state_changes_requested() {
        let reviews = serde_json::json!([
            {"state": "APPROVED", "author": {"login": "user1"}},
            {"state": "CHANGES_REQUESTED", "author": {"login": "user2"}}
        ]);
        assert_eq!(GitHubForge::parse_review_state(&reviews), ReviewState::ChangesRequested);
    }

    #[test]
    fn test_parse_review_state_commented() {
        let reviews = serde_json::json!([
            {"state": "COMMENTED", "author": {"login": "user1"}}
        ]);
        assert_eq!(GitHubForge::parse_review_state(&reviews), ReviewState::Commented);
    }

    #[test]
    fn test_parse_review_state_approved_beats_commented() {
        let reviews = serde_json::json!([
            {"state": "COMMENTED", "author": {"login": "user1"}},
            {"state": "APPROVED", "author": {"login": "user2"}}
        ]);
        assert_eq!(GitHubForge::parse_review_state(&reviews), ReviewState::Approved);
    }

    #[test]
    fn test_parse_ci_status_none() {
        let checks = serde_json::json!([]);
        assert_eq!(GitHubForge::parse_ci_status(&checks), CiStatus::None);

        let checks = serde_json::json!(null);
        assert_eq!(GitHubForge::parse_ci_status(&checks), CiStatus::None);
    }

    #[test]
    fn test_parse_ci_status_success() {
        let checks = serde_json::json!([
            {"state": "SUCCESS", "context": "ci/test"},
            {"conclusion": "SUCCESS", "name": "build"}
        ]);
        assert_eq!(GitHubForge::parse_ci_status(&checks), CiStatus::Success);
    }

    #[test]
    fn test_parse_ci_status_failure() {
        let checks = serde_json::json!([
            {"state": "SUCCESS", "context": "ci/test"},
            {"state": "FAILURE", "context": "ci/lint"}
        ]);
        assert_eq!(GitHubForge::parse_ci_status(&checks), CiStatus::Failure);
    }

    #[test]
    fn test_parse_ci_status_pending() {
        let checks = serde_json::json!([
            {"state": "SUCCESS", "context": "ci/test"},
            {"state": "PENDING", "context": "ci/lint"}
        ]);
        assert_eq!(GitHubForge::parse_ci_status(&checks), CiStatus::Pending);
    }

    #[test]
    fn test_parse_ci_status_in_progress() {
        let checks = serde_json::json!([
            {"status": "IN_PROGRESS", "name": "build"}
        ]);
        assert_eq!(GitHubForge::parse_ci_status(&checks), CiStatus::Pending);
    }

    #[test]
    fn test_parse_ci_status_failure_beats_pending() {
        let checks = serde_json::json!([
            {"state": "PENDING", "context": "ci/test"},
            {"state": "FAILURE", "context": "ci/lint"}
        ]);
        assert_eq!(GitHubForge::parse_ci_status(&checks), CiStatus::Failure);
    }
}
