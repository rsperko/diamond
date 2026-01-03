//! Common types for forge abstraction
//!
//! These types are used across all forge implementations (GitHub, GitLab, etc.)

use serde::{Deserialize, Serialize};
use std::fmt;

/// Merge method for PRs/MRs
///
/// Different forges support different merge methods:
/// - GitHub: squash, merge, rebase
/// - GitLab: squash (merge --squash), merge, rebase (merge --rebase)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MergeMethod {
    /// Squash all commits into one before merging
    #[default]
    Squash,
    /// Create a merge commit
    Merge,
    /// Rebase commits onto base branch
    Rebase,
}

impl MergeMethod {
    /// Get the string representation used by CLIs
    pub fn as_str(&self) -> &'static str {
        match self {
            MergeMethod::Squash => "squash",
            MergeMethod::Merge => "merge",
            MergeMethod::Rebase => "rebase",
        }
    }
}

impl fmt::Display for MergeMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Options for creating a Pull Request
#[derive(Debug, Clone, Default)]
pub struct PrOptions {
    /// Create as draft PR
    pub draft: bool,
    /// Publish (mark as ready for review) - for existing draft PRs
    pub publish: bool,
    /// Enable auto-merge after CI passes
    pub merge_when_ready: bool,
    /// Reviewer usernames to assign
    pub reviewers: Vec<String>,
}

/// Information about a Pull/Merge Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrInfo {
    /// PR/MR number (e.g., 123)
    pub number: u64,
    /// Full URL to the PR/MR
    pub url: String,
    /// Head branch name (the branch being merged)
    pub head_ref: String,
    /// Base branch name (the branch being merged into)
    pub base_ref: String,
    /// Current state of the PR/MR
    pub state: PrState,
    /// Title of the PR/MR
    pub title: String,
}

/// State of a Pull/Merge Request
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl std::fmt::Display for PrState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrState::Open => write!(f, "open"),
            PrState::Closed => write!(f, "closed"),
            PrState::Merged => write!(f, "merged"),
        }
    }
}

/// Review state for a PR
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReviewState {
    /// No reviews yet
    #[default]
    Pending,
    /// Approved by reviewer(s)
    Approved,
    /// Changes requested by reviewer(s)
    ChangesRequested,
    /// Reviewed with comments only (no approval/rejection)
    Commented,
}

impl fmt::Display for ReviewState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewState::Pending => write!(f, "pending"),
            ReviewState::Approved => write!(f, "approved"),
            ReviewState::ChangesRequested => write!(f, "changes_requested"),
            ReviewState::Commented => write!(f, "commented"),
        }
    }
}

impl ReviewState {
    /// Get the emoji representation for the review state
    pub fn emoji(&self) -> &'static str {
        match self {
            ReviewState::Pending => "👀",
            ReviewState::Approved => "✅",
            ReviewState::ChangesRequested => "🔶",
            ReviewState::Commented => "💬",
        }
    }
}

/// CI/Check status for a PR
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CiStatus {
    /// No checks or unknown status
    #[default]
    None,
    /// Checks are pending/running
    Pending,
    /// All checks passed
    Success,
    /// Some checks failed
    Failure,
    /// Checks were skipped
    Skipped,
}

impl fmt::Display for CiStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CiStatus::None => write!(f, "none"),
            CiStatus::Pending => write!(f, "pending"),
            CiStatus::Success => write!(f, "success"),
            CiStatus::Failure => write!(f, "failure"),
            CiStatus::Skipped => write!(f, "skipped"),
        }
    }
}

impl CiStatus {
    /// Get the emoji representation for the CI status
    pub fn emoji(&self) -> &'static str {
        match self {
            CiStatus::None => "—",
            CiStatus::Pending => "🔄",
            CiStatus::Success => "✅",
            CiStatus::Failure => "❌",
            CiStatus::Skipped => "⏭️",
        }
    }
}

/// Extended PR information including review and CI status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrFullInfo {
    /// PR/MR number (e.g., 123)
    pub number: u64,
    /// Full URL to the PR/MR
    pub url: String,
    /// Title of the PR/MR
    pub title: String,
    /// Current state of the PR/MR
    pub state: PrState,
    /// Whether the PR is a draft
    pub is_draft: bool,
    /// Review status
    pub review: ReviewState,
    /// CI/Checks status
    pub ci: CiStatus,
    /// Head branch name (the branch being merged)
    pub head_ref: String,
    /// Base branch name (the branch being merged into)
    pub base_ref: String,
}

impl PrFullInfo {
    /// Get the emoji representation for the PR state
    pub fn state_emoji(&self) -> &'static str {
        if self.is_draft {
            "📝"
        } else {
            match self.state {
                PrState::Open => "🔄",
                PrState::Merged => "✅",
                PrState::Closed => "❌",
            }
        }
    }

    /// Get the display state (considering draft status)
    pub fn state_display(&self) -> &'static str {
        if self.is_draft {
            "Draft"
        } else {
            match self.state {
                PrState::Open => "Open",
                PrState::Merged => "Merged",
                PrState::Closed => "Closed",
            }
        }
    }
}

/// Supported forge types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForgeType {
    GitHub,
    GitLab,
    Bitbucket,
    Gitea,
}

impl std::fmt::Display for ForgeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForgeType::GitHub => write!(f, "github"),
            ForgeType::GitLab => write!(f, "gitlab"),
            ForgeType::Bitbucket => write!(f, "bitbucket"),
            ForgeType::Gitea => write!(f, "gitea"),
        }
    }
}

impl std::str::FromStr for ForgeType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "github" => Ok(ForgeType::GitHub),
            "gitlab" => Ok(ForgeType::GitLab),
            "bitbucket" => Ok(ForgeType::Bitbucket),
            "gitea" => Ok(ForgeType::Gitea),
            _ => Err(format!("Unknown forge type: {}", s)),
        }
    }
}

/// Configuration for a forge instance
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ForgeConfig {
    /// Override the forge type (auto-detected if not set)
    pub forge_type: Option<ForgeType>,
    /// Custom host for enterprise instances (e.g., "github.mycompany.com")
    pub host: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forge_type_display() {
        assert_eq!(ForgeType::GitHub.to_string(), "github");
        assert_eq!(ForgeType::GitLab.to_string(), "gitlab");
    }

    #[test]
    fn test_forge_type_from_str() {
        assert_eq!("github".parse::<ForgeType>().unwrap(), ForgeType::GitHub);
        assert_eq!("GITLAB".parse::<ForgeType>().unwrap(), ForgeType::GitLab);
        assert!("unknown".parse::<ForgeType>().is_err());
    }

    #[test]
    fn test_pr_state_display() {
        assert_eq!(PrState::Open.to_string(), "open");
        assert_eq!(PrState::Merged.to_string(), "merged");
    }

    #[test]
    fn test_review_state_display() {
        assert_eq!(ReviewState::Pending.to_string(), "pending");
        assert_eq!(ReviewState::Approved.to_string(), "approved");
        assert_eq!(ReviewState::ChangesRequested.to_string(), "changes_requested");
        assert_eq!(ReviewState::Commented.to_string(), "commented");
    }

    #[test]
    fn test_review_state_emoji() {
        assert_eq!(ReviewState::Pending.emoji(), "👀");
        assert_eq!(ReviewState::Approved.emoji(), "✅");
        assert_eq!(ReviewState::ChangesRequested.emoji(), "🔶");
        assert_eq!(ReviewState::Commented.emoji(), "💬");
    }

    #[test]
    fn test_ci_status_display() {
        assert_eq!(CiStatus::None.to_string(), "none");
        assert_eq!(CiStatus::Pending.to_string(), "pending");
        assert_eq!(CiStatus::Success.to_string(), "success");
        assert_eq!(CiStatus::Failure.to_string(), "failure");
        assert_eq!(CiStatus::Skipped.to_string(), "skipped");
    }

    #[test]
    fn test_ci_status_emoji() {
        assert_eq!(CiStatus::None.emoji(), "—");
        assert_eq!(CiStatus::Pending.emoji(), "🔄");
        assert_eq!(CiStatus::Success.emoji(), "✅");
        assert_eq!(CiStatus::Failure.emoji(), "❌");
        assert_eq!(CiStatus::Skipped.emoji(), "⏭️");
    }

    #[test]
    fn test_pr_full_info_state_emoji_and_display() {
        let pr = PrFullInfo {
            number: 123,
            url: "https://github.com/user/repo/pull/123".to_string(),
            title: "Test PR".to_string(),
            state: PrState::Open,
            is_draft: false,
            review: ReviewState::Pending,
            ci: CiStatus::Success,
            head_ref: "feature".to_string(),
            base_ref: "main".to_string(),
        };
        assert_eq!(pr.state_emoji(), "🔄");
        assert_eq!(pr.state_display(), "Open");

        let draft_pr = PrFullInfo {
            is_draft: true,
            ..pr.clone()
        };
        assert_eq!(draft_pr.state_emoji(), "📝");
        assert_eq!(draft_pr.state_display(), "Draft");

        let merged_pr = PrFullInfo {
            state: PrState::Merged,
            is_draft: false,
            ..pr.clone()
        };
        assert_eq!(merged_pr.state_emoji(), "✅");
        assert_eq!(merged_pr.state_display(), "Merged");

        let closed_pr = PrFullInfo {
            state: PrState::Closed,
            is_draft: false,
            ..pr
        };
        assert_eq!(closed_pr.state_emoji(), "❌");
        assert_eq!(closed_pr.state_display(), "Closed");
    }

    #[test]
    fn test_review_state_default() {
        let state: ReviewState = Default::default();
        assert_eq!(state, ReviewState::Pending);
    }

    #[test]
    fn test_ci_status_default() {
        let status: CiStatus = Default::default();
        assert_eq!(status, CiStatus::None);
    }

    // === MergeMethod Tests ===

    #[test]
    fn test_merge_method_as_str() {
        assert_eq!(MergeMethod::Squash.as_str(), "squash");
        assert_eq!(MergeMethod::Merge.as_str(), "merge");
        assert_eq!(MergeMethod::Rebase.as_str(), "rebase");
    }

    #[test]
    fn test_merge_method_display() {
        assert_eq!(MergeMethod::Squash.to_string(), "squash");
        assert_eq!(MergeMethod::Merge.to_string(), "merge");
        assert_eq!(MergeMethod::Rebase.to_string(), "rebase");
    }

    #[test]
    fn test_merge_method_default() {
        let method: MergeMethod = Default::default();
        assert_eq!(method, MergeMethod::Squash);
    }
}
