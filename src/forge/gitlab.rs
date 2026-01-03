//! GitLab forge implementation using the `glab` CLI
//!
//! This implementation wraps the GitLab CLI (`glab`) to provide
//! MR (Merge Request) operations for GitLab repositories.

use super::{
    AsyncForge, CiStatus, Forge, ForgeConfig, ForgeType, MergeMethod, PrFullInfo, PrInfo, PrOptions, PrState,
    ReviewState,
};
use crate::git_gateway::GitGateway;
use anyhow::{Context, Result};
use std::process::Command;

/// GitLab forge implementation
pub struct GitLabForge {
    /// Custom host for self-hosted GitLab (auto-detected by glab from remote URL)
    #[allow(dead_code)]
    host: Option<String>,
}

impl GitLabForge {
    /// Create a new GitLab forge instance
    pub fn new(config: Option<&ForgeConfig>) -> Self {
        let host = config.and_then(|c| c.host.clone());
        Self { host }
    }

    /// Run a glab command
    fn run_glab(&self, args: &[&str]) -> Result<std::process::Output> {
        let mut cmd = Command::new("glab");

        cmd.args(args)
            .output()
            .context("Failed to run glab CLI. Is it installed? Install with: brew install glab")
    }

    /// Parse MR state from glab CLI output
    fn parse_mr_state(state: &str) -> PrState {
        match state.to_lowercase().as_str() {
            "opened" | "open" => PrState::Open,
            "closed" => PrState::Closed,
            "merged" => PrState::Merged,
            _ => PrState::Open, // Default to open
        }
    }

    /// Parse review/approval state from glab CLI approvals data
    ///
    /// GitLab uses an approval system. We map it to review states:
    /// 1. If approved_by is not empty, return Approved
    /// 2. If there are discussions/comments, return Commented
    /// 3. Otherwise return Pending
    fn parse_review_state(json: &serde_json::Value) -> ReviewState {
        // Check if MR is approved
        if let Some(approved_by) = json.get("approved_by").and_then(|v| v.as_array()) {
            if !approved_by.is_empty() {
                return ReviewState::Approved;
            }
        }

        // Check approvals_left field (if 0 and there are required approvers, it's approved)
        if let Some(approvals_left) = json.get("approvals_left").and_then(|v| v.as_i64()) {
            if approvals_left == 0 {
                if let Some(approvals_required) = json.get("approvals_required").and_then(|v| v.as_i64()) {
                    if approvals_required > 0 {
                        return ReviewState::Approved;
                    }
                }
            }
        }

        // Check for user_notes_count (comments/discussions)
        if let Some(notes_count) = json.get("user_notes_count").and_then(|v| v.as_i64()) {
            if notes_count > 0 {
                return ReviewState::Commented;
            }
        }

        ReviewState::Pending
    }

    /// Parse CI/pipeline status from glab CLI data
    ///
    /// GitLab embeds pipeline status in the MR data under head_pipeline.
    fn parse_ci_status(json: &serde_json::Value) -> CiStatus {
        let pipeline = json.get("head_pipeline").or_else(|| json.get("pipeline"));

        if let Some(pipeline) = pipeline {
            if pipeline.is_null() {
                return CiStatus::None;
            }

            let status = pipeline.get("status").and_then(|v| v.as_str()).unwrap_or("");

            match status.to_lowercase().as_str() {
                "success" | "passed" => CiStatus::Success,
                "failed" | "failure" => CiStatus::Failure,
                "running" | "pending" | "created" | "waiting_for_resource" | "preparing" => CiStatus::Pending,
                "canceled" | "skipped" | "manual" => CiStatus::Skipped,
                _ => CiStatus::None,
            }
        } else {
            CiStatus::None
        }
    }
}

impl Forge for GitLabForge {
    fn forge_type(&self) -> ForgeType {
        ForgeType::GitLab
    }

    fn cli_name(&self) -> &str {
        "glab"
    }

    fn check_auth(&self) -> Result<()> {
        let output = self.run_glab(&["auth", "status"])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not logged") || stderr.contains("no token") {
                anyhow::bail!("Not authenticated with GitLab CLI. Run 'glab auth login' to authenticate.");
            }
            anyhow::bail!("GitLab CLI auth check failed: {}", stderr);
        }
        Ok(())
    }

    fn pr_exists(&self, branch: &str) -> Result<Option<PrInfo>> {
        // glab mr list with source branch filter
        let output = self.run_glab(&["mr", "list", "--source-branch", branch, "--output", "json"])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // No MR found is not an error
            if stderr.contains("no merge requests") || stderr.contains("404") {
                return Ok(None);
            }
            anyhow::bail!("Failed to check MR: {}", stderr);
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("Failed to parse glab mr list output")?;

        // glab mr list returns an array
        if let Some(arr) = json.as_array() {
            if let Some(mr) = arr.first() {
                return Ok(Some(PrInfo {
                    number: mr["iid"].as_u64().unwrap_or(0),
                    url: mr["web_url"].as_str().unwrap_or("").to_string(),
                    head_ref: mr["source_branch"].as_str().unwrap_or("").to_string(),
                    base_ref: mr["target_branch"].as_str().unwrap_or("").to_string(),
                    state: Self::parse_mr_state(mr["state"].as_str().unwrap_or("opened")),
                    title: mr["title"].as_str().unwrap_or("").to_string(),
                }));
            }
        }

        Ok(None)
    }

    fn create_pr(&self, branch: &str, base: &str, title: &str, body: &str, options: &PrOptions) -> Result<String> {
        let mut args = vec![
            "mr".to_string(),
            "create".to_string(),
            "--source-branch".to_string(),
            branch.to_string(),
            "--target-branch".to_string(),
            base.to_string(),
            "--title".to_string(),
            title.to_string(),
            "--description".to_string(),
            body.to_string(),
            "--yes".to_string(), // Non-interactive mode
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
        let output = self.run_glab(&args_refs)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to create MR: {}", stderr);
        }

        // glab mr create outputs the MR URL
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Extract URL from output (glab prints "Merge request created: <url>")
        let url = stdout
            .lines()
            .find(|line| line.contains("http"))
            .map(|line| {
                line.split_whitespace()
                    .find(|word| word.starts_with("http"))
                    .unwrap_or(line)
            })
            .unwrap_or(&stdout)
            .trim()
            .to_string();

        Ok(url)
    }

    fn get_pr_info(&self, pr_ref: &str) -> Result<PrInfo> {
        let output = self.run_glab(&["mr", "view", pr_ref, "--output", "json"])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to get MR info: {}", stderr);
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("Failed to parse glab mr view output")?;

        Ok(PrInfo {
            number: json["iid"].as_u64().unwrap_or(0),
            url: json["web_url"].as_str().unwrap_or("").to_string(),
            head_ref: json["source_branch"].as_str().unwrap_or("").to_string(),
            base_ref: json["target_branch"].as_str().unwrap_or("").to_string(),
            state: Self::parse_mr_state(json["state"].as_str().unwrap_or("opened")),
            title: json["title"].as_str().unwrap_or("").to_string(),
        })
    }

    fn get_pr_chain(&self, pr_ref: &str) -> Result<Vec<PrInfo>> {
        let mut chain = Vec::new();
        let mut current_ref = pr_ref.to_string();

        // Walk up the MR chain until we hit a non-MR base (like main)
        loop {
            let info = self.get_pr_info(&current_ref)?;
            let base = info.base_ref.clone();
            chain.push(info);

            // Check if the base branch has an MR
            match self.pr_exists(&base)? {
                Some(_) => {
                    current_ref = base;
                }
                None => break, // Base is not an MR (probably trunk)
            }
        }

        // Reverse to get parent-first order
        chain.reverse();
        Ok(chain)
    }

    fn is_branch_merged(&self, branch: &str, into: &str) -> Result<bool> {
        // Check if the branch's MR is merged
        if let Some(mr) = self.pr_exists(branch)? {
            if mr.state == PrState::Merged {
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
        let output = self.run_glab(&["mr", "view", pr_ref, "--output", "json"])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to get MR info: {}", stderr);
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("Failed to parse glab mr view output")?;

        // GitLab uses "draft" field or "work_in_progress" for draft status
        let is_draft = json["draft"].as_bool().unwrap_or(false) || json["work_in_progress"].as_bool().unwrap_or(false);

        Ok(PrFullInfo {
            number: json["iid"].as_u64().unwrap_or(0),
            url: json["web_url"].as_str().unwrap_or("").to_string(),
            title: json["title"].as_str().unwrap_or("").to_string(),
            state: Self::parse_mr_state(json["state"].as_str().unwrap_or("opened")),
            is_draft,
            review: Self::parse_review_state(&json),
            ci: Self::parse_ci_status(&json),
            head_ref: json["source_branch"].as_str().unwrap_or("").to_string(),
            base_ref: json["target_branch"].as_str().unwrap_or("").to_string(),
        })
    }

    fn get_pr_body(&self, pr_ref: &str) -> Result<String> {
        let output = self.run_glab(&["mr", "view", pr_ref, "--output", "json"])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to get MR body: {}", stderr);
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("Failed to parse glab mr view output")?;

        Ok(json["description"].as_str().unwrap_or("").to_string())
    }

    fn update_pr_body(&self, pr_ref: &str, body: &str) -> Result<()> {
        let output = self.run_glab(&["mr", "update", pr_ref, "--description", body])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to update MR body: {}", stderr);
        }

        Ok(())
    }

    fn mark_pr_ready(&self, pr_ref: &str) -> Result<()> {
        let output = self.run_glab(&["mr", "update", pr_ref, "--ready"])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "already not a draft" type errors
            if stderr.contains("not a draft") || stderr.contains("already") {
                return Ok(());
            }
            anyhow::bail!("Failed to mark MR as ready: {}", stderr);
        }

        Ok(())
    }

    fn update_pr_base(&self, branch: &str, new_base: &str) -> Result<()> {
        let output = self.run_glab(&["mr", "update", branch, "--target-branch", new_base])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to update MR base: {}", stderr);
        }

        Ok(())
    }

    fn enable_auto_merge(&self, pr_ref: &str, merge_method: &str) -> Result<()> {
        // GitLab's auto-merge is "merge when pipeline succeeds"
        // The merge method (squash, merge) is set via --squash flag
        let mut args = vec!["mr", "merge", pr_ref, "--when-pipeline-succeeds"];

        if merge_method == "squash" {
            args.push("--squash");
        } else if merge_method == "rebase" {
            args.push("--rebase");
        }

        let output = self.run_glab(&args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Pipeline has not succeeded")
                || stderr.contains("not allowed")
                || stderr.contains("cannot be merged")
            {
                anyhow::bail!(
                    "Cannot enable auto-merge: pipeline must be configured and passing.\n\
                     Ensure the project has CI/CD configured and the pipeline is running."
                );
            }
            anyhow::bail!("Failed to enable auto-merge: {}", stderr);
        }

        Ok(())
    }

    fn merge_pr(&self, pr_ref: &str, method: MergeMethod, auto_confirm: bool) -> Result<()> {
        let mut args = vec!["mr", "merge", pr_ref];

        // Add merge method flags
        match method {
            MergeMethod::Squash => args.push("--squash"),
            MergeMethod::Rebase => args.push("--rebase"),
            MergeMethod::Merge => {} // Default behavior, no flag needed
        }

        // Add --yes if auto-confirm is enabled
        if auto_confirm {
            args.push("--yes");
        }

        let output = self.run_glab(&args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Check for common errors and provide helpful messages
            if stderr.contains("not logged") || stderr.contains("no token") {
                anyhow::bail!("GitLab CLI not authenticated. Run 'glab auth login' first.");
            }
            if stderr.contains("cannot be merged") || stderr.contains("has conflicts") {
                anyhow::bail!("MR {} cannot be merged: conflicts or merge blocked.", pr_ref);
            }
            if stderr.contains("Pipeline has not succeeded") || stderr.contains("pipeline") {
                anyhow::bail!("MR {} has failing or pending pipeline checks.", pr_ref);
            }
            if stderr.contains("approval") || stderr.contains("approvals") {
                anyhow::bail!("MR {} requires approvals that haven't been granted.", pr_ref);
            }

            anyhow::bail!("glab mr merge failed: {}", stderr.trim());
        }

        Ok(())
    }

    fn open_pr_in_browser(&self, pr_ref: &str) -> Result<()> {
        let output = self.run_glab(&["mr", "view", pr_ref, "--web"])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);

            if stderr.contains("no merge requests found") || stderr.contains("404") {
                anyhow::bail!("No MR found for '{}'. Does it exist?", pr_ref);
            }

            anyhow::bail!("Failed to open MR: {}", stderr.trim());
        }

        Ok(())
    }
}

/// AsyncForge implementation uses default methods that wrap sync Forge calls
impl AsyncForge for GitLabForge {
    // All batch methods use default implementations from the trait
    // These will be replaced with GraphQL batch queries in the future
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mr_state() {
        assert_eq!(GitLabForge::parse_mr_state("opened"), PrState::Open);
        assert_eq!(GitLabForge::parse_mr_state("open"), PrState::Open);
        assert_eq!(GitLabForge::parse_mr_state("closed"), PrState::Closed);
        assert_eq!(GitLabForge::parse_mr_state("merged"), PrState::Merged);
        assert_eq!(GitLabForge::parse_mr_state("OPENED"), PrState::Open);
        assert_eq!(GitLabForge::parse_mr_state("unknown"), PrState::Open);
    }

    #[test]
    fn test_gitlab_forge_new() {
        let forge = GitLabForge::new(None);
        assert_eq!(forge.forge_type(), ForgeType::GitLab);
        assert_eq!(forge.cli_name(), "glab");
        assert!(forge.host.is_none());
    }

    #[test]
    fn test_gitlab_forge_with_host() {
        let config = ForgeConfig {
            forge_type: Some(ForgeType::GitLab),
            host: Some("gitlab.mycompany.com".to_string()),
        };
        let forge = GitLabForge::new(Some(&config));
        assert_eq!(forge.host, Some("gitlab.mycompany.com".to_string()));
    }

    #[test]
    fn test_parse_review_state_pending() {
        let json = serde_json::json!({});
        assert_eq!(GitLabForge::parse_review_state(&json), ReviewState::Pending);

        let json = serde_json::json!({"approved_by": []});
        assert_eq!(GitLabForge::parse_review_state(&json), ReviewState::Pending);
    }

    #[test]
    fn test_parse_review_state_approved() {
        let json = serde_json::json!({
            "approved_by": [{"username": "reviewer1"}]
        });
        assert_eq!(GitLabForge::parse_review_state(&json), ReviewState::Approved);
    }

    #[test]
    fn test_parse_review_state_approved_via_approvals_left() {
        let json = serde_json::json!({
            "approvals_left": 0,
            "approvals_required": 1
        });
        assert_eq!(GitLabForge::parse_review_state(&json), ReviewState::Approved);
    }

    #[test]
    fn test_parse_review_state_commented() {
        let json = serde_json::json!({
            "user_notes_count": 3
        });
        assert_eq!(GitLabForge::parse_review_state(&json), ReviewState::Commented);
    }

    #[test]
    fn test_parse_ci_status_none() {
        let json = serde_json::json!({});
        assert_eq!(GitLabForge::parse_ci_status(&json), CiStatus::None);

        let json = serde_json::json!({"head_pipeline": null});
        assert_eq!(GitLabForge::parse_ci_status(&json), CiStatus::None);
    }

    #[test]
    fn test_parse_ci_status_success() {
        let json = serde_json::json!({
            "head_pipeline": {"status": "success"}
        });
        assert_eq!(GitLabForge::parse_ci_status(&json), CiStatus::Success);

        let json = serde_json::json!({
            "head_pipeline": {"status": "passed"}
        });
        assert_eq!(GitLabForge::parse_ci_status(&json), CiStatus::Success);
    }

    #[test]
    fn test_parse_ci_status_failure() {
        let json = serde_json::json!({
            "head_pipeline": {"status": "failed"}
        });
        assert_eq!(GitLabForge::parse_ci_status(&json), CiStatus::Failure);
    }

    #[test]
    fn test_parse_ci_status_pending() {
        let json = serde_json::json!({
            "head_pipeline": {"status": "running"}
        });
        assert_eq!(GitLabForge::parse_ci_status(&json), CiStatus::Pending);

        let json = serde_json::json!({
            "head_pipeline": {"status": "pending"}
        });
        assert_eq!(GitLabForge::parse_ci_status(&json), CiStatus::Pending);
    }

    #[test]
    fn test_parse_ci_status_skipped() {
        let json = serde_json::json!({
            "head_pipeline": {"status": "canceled"}
        });
        assert_eq!(GitLabForge::parse_ci_status(&json), CiStatus::Skipped);

        let json = serde_json::json!({
            "head_pipeline": {"status": "skipped"}
        });
        assert_eq!(GitLabForge::parse_ci_status(&json), CiStatus::Skipped);
    }

    #[test]
    fn test_parse_ci_status_with_pipeline_key() {
        // Some responses use "pipeline" instead of "head_pipeline"
        let json = serde_json::json!({
            "pipeline": {"status": "success"}
        });
        assert_eq!(GitLabForge::parse_ci_status(&json), CiStatus::Success);
    }
}
