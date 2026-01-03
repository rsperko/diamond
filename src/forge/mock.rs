//! Mock forge implementation for testing
//!
//! This module provides a mock forge that can simulate various scenarios
//! including API failures, rate limits, and network issues.

use super::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Response configuration for mock forge operations
#[derive(Debug, Clone)]
pub enum MockResponse {
    /// Operation succeeds with the given value
    Success(String),
    /// Operation fails with the given error message
    Error(String),
    /// Operation is rate limited
    RateLimit,
    /// Operation times out
    Timeout,
    /// Auth token is invalid/expired
    AuthError,
}

/// Mock forge for testing
///
/// This forge can be configured to return specific responses for operations,
/// allowing tests to simulate various failure scenarios.
pub struct MockForge {
    /// Configured responses for operations
    responses: Arc<Mutex<HashMap<String, MockResponse>>>,
    /// Call counter for operations
    call_count: Arc<Mutex<HashMap<String, usize>>>,
    /// Forge type to simulate
    forge_type: ForgeType,
}

impl MockForge {
    /// Create a new mock forge
    pub fn new(forge_type: ForgeType) -> Self {
        Self {
            responses: Arc::new(Mutex::new(HashMap::new())),
            call_count: Arc::new(Mutex::new(HashMap::new())),
            forge_type,
        }
    }

    /// Configure a response for a specific operation
    ///
    /// # Example
    /// ```ignore
    /// mock.set_response("pr_exists:my-branch", MockResponse::Success("123".to_string()));
    /// mock.set_response("create_pr", MockResponse::RateLimit);
    /// ```
    pub fn set_response(&self, operation: &str, response: MockResponse) {
        self.responses.lock().unwrap().insert(operation.to_string(), response);
    }

    /// Get the number of times an operation was called
    pub fn get_call_count(&self, operation: &str) -> usize {
        *self.call_count.lock().unwrap().get(operation).unwrap_or(&0)
    }

    /// Record a call and return the configured response
    fn handle_call(&self, operation: &str) -> Result<String> {
        // Increment call count
        let mut counts = self.call_count.lock().unwrap();
        *counts.entry(operation.to_string()).or_insert(0) += 1;
        drop(counts);

        // Get configured response
        let responses = self.responses.lock().unwrap();
        let response = responses
            .get(operation)
            .cloned()
            .unwrap_or(MockResponse::Success("".to_string()));
        drop(responses);

        // Return based on response type
        match response {
            MockResponse::Success(value) => Ok(value),
            MockResponse::Error(msg) => anyhow::bail!("{}", msg),
            MockResponse::RateLimit => {
                anyhow::bail!("API rate limit exceeded. Please wait and try again.")
            }
            MockResponse::Timeout => {
                anyhow::bail!("Request timed out: connection timeout after 30s")
            }
            MockResponse::AuthError => {
                anyhow::bail!("Authentication failed: token expired or invalid")
            }
        }
    }
}

impl Forge for MockForge {
    fn forge_type(&self) -> ForgeType {
        self.forge_type
    }

    fn cli_name(&self) -> &str {
        match self.forge_type {
            ForgeType::GitHub => "gh",
            ForgeType::GitLab => "glab",
            ForgeType::Bitbucket => "bb",
            ForgeType::Gitea => "tea",
        }
    }

    fn check_auth(&self) -> Result<()> {
        self.handle_call("check_auth")?;
        Ok(())
    }

    fn pr_exists(&self, branch: &str) -> Result<Option<PrInfo>> {
        let key = format!("pr_exists:{}", branch);
        match self.handle_call(&key) {
            Ok(pr_number) if !pr_number.is_empty() => Ok(Some(PrInfo {
                number: pr_number.parse().unwrap_or(123),
                url: format!("https://github.com/test/repo/pull/{}", pr_number),
                title: "Test PR".to_string(),
                state: PrState::Open,
                head_ref: branch.to_string(),
                base_ref: "main".to_string(),
            })),
            Ok(_) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn create_pr(&self, branch: &str, _base: &str, _title: &str, _body: &str, _options: &PrOptions) -> Result<String> {
        let key = format!("create_pr:{}", branch);
        self.handle_call(&key)
    }

    fn get_pr_info(&self, pr_ref: &str) -> Result<PrInfo> {
        let key = format!("get_pr_info:{}", pr_ref);
        let _ = self.handle_call(&key)?;

        Ok(PrInfo {
            number: pr_ref.parse().unwrap_or(123),
            url: format!("https://github.com/test/repo/pull/{}", pr_ref),
            title: "Test PR".to_string(),
            state: PrState::Open,
            head_ref: "feature".to_string(),
            base_ref: "main".to_string(),
        })
    }

    fn get_pr_chain(&self, pr_ref: &str) -> Result<Vec<PrInfo>> {
        let key = format!("get_pr_chain:{}", pr_ref);
        let _ = self.handle_call(&key)?;
        Ok(vec![self.get_pr_info(pr_ref)?])
    }

    fn is_branch_merged(&self, branch: &str, _into: &str) -> Result<bool> {
        let key = format!("is_branch_merged:{}", branch);
        let result = self.handle_call(&key)?;
        Ok(result == "true")
    }

    fn get_pr_full_info(&self, pr_ref: &str) -> Result<PrFullInfo> {
        let key = format!("get_pr_full_info:{}", pr_ref);
        let _ = self.handle_call(&key)?;

        Ok(PrFullInfo {
            number: pr_ref.parse().unwrap_or(123),
            url: format!("https://github.com/test/repo/pull/{}", pr_ref),
            title: "Test PR".to_string(),
            state: PrState::Open,
            head_ref: "feature".to_string(),
            base_ref: "main".to_string(),
            is_draft: false,
            review: ReviewState::Pending,
            ci: CiStatus::Success,
        })
    }

    fn get_pr_body(&self, pr_ref: &str) -> Result<String> {
        let key = format!("get_pr_body:{}", pr_ref);
        self.handle_call(&key)
    }

    fn update_pr_body(&self, pr_ref: &str, _body: &str) -> Result<()> {
        let key = format!("update_pr_body:{}", pr_ref);
        self.handle_call(&key)?;
        Ok(())
    }

    fn update_pr_base(&self, branch: &str, _new_base: &str) -> Result<()> {
        let key = format!("update_pr_base:{}", branch);
        self.handle_call(&key)?;
        Ok(())
    }

    fn mark_pr_ready(&self, pr_ref: &str) -> Result<()> {
        let key = format!("mark_pr_ready:{}", pr_ref);
        self.handle_call(&key)?;
        Ok(())
    }

    fn enable_auto_merge(&self, pr_ref: &str, _merge_method: &str) -> Result<()> {
        let key = format!("enable_auto_merge:{}", pr_ref);
        self.handle_call(&key)?;
        Ok(())
    }

    fn merge_pr(&self, pr_ref: &str, _method: MergeMethod, _auto_confirm: bool) -> Result<()> {
        let key = format!("merge_pr:{}", pr_ref);
        self.handle_call(&key)?;
        Ok(())
    }

    fn open_pr_in_browser(&self, pr_ref: &str) -> Result<()> {
        let key = format!("open_pr_in_browser:{}", pr_ref);
        self.handle_call(&key)?;
        Ok(())
    }

    fn push_branch(&self, branch: &str, _force: bool) -> Result<()> {
        let key = format!("push_branch:{}", branch);
        self.handle_call(&key)?;
        Ok(())
    }
}

#[async_trait]
impl AsyncForge for MockForge {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_forge_success() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response("check_auth", MockResponse::Success("ok".to_string()));

        assert!(mock.check_auth().is_ok());
        assert_eq!(mock.get_call_count("check_auth"), 1);
    }

    #[test]
    fn test_mock_forge_error() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response("check_auth", MockResponse::Error("custom error".to_string()));

        let result = mock.check_auth();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("custom error"));
    }

    #[test]
    fn test_mock_forge_rate_limit() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response("check_auth", MockResponse::RateLimit);

        let result = mock.check_auth();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("rate limit"));
    }

    #[test]
    fn test_mock_forge_timeout() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response("check_auth", MockResponse::Timeout);

        let result = mock.check_auth();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));
    }

    #[test]
    fn test_mock_forge_auth_error() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response("check_auth", MockResponse::AuthError);

        let result = mock.check_auth();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Authentication failed"));
    }

    #[test]
    fn test_mock_forge_call_counting() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response("check_auth", MockResponse::Success("ok".to_string()));

        assert_eq!(mock.get_call_count("check_auth"), 0);
        mock.check_auth().ok();
        assert_eq!(mock.get_call_count("check_auth"), 1);
        mock.check_auth().ok();
        assert_eq!(mock.get_call_count("check_auth"), 2);
    }

    #[test]
    fn test_mock_forge_pr_exists() {
        let mock = MockForge::new(ForgeType::GitHub);

        // Configure to return PR #123
        mock.set_response("pr_exists:my-branch", MockResponse::Success("123".to_string()));

        let result = mock.pr_exists("my-branch");
        assert!(result.is_ok());
        let pr = result.unwrap();
        assert!(pr.is_some());
        assert_eq!(pr.unwrap().number, 123);
    }

    #[test]
    fn test_mock_forge_pr_not_exists() {
        let mock = MockForge::new(ForgeType::GitHub);

        // Configure to return empty (no PR)
        mock.set_response("pr_exists:my-branch", MockResponse::Success("".to_string()));

        let result = mock.pr_exists("my-branch");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    // === Network Failure Scenario Tests ===

    #[test]
    fn test_create_pr_handles_rate_limit() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response("create_pr:my-feature", MockResponse::RateLimit);

        let result = mock.create_pr("my-feature", "main", "My Feature", "Description", &PrOptions::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("rate limit"));
        assert_eq!(mock.get_call_count("create_pr:my-feature"), 1);
    }

    #[test]
    fn test_create_pr_handles_network_timeout() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response("create_pr:my-feature", MockResponse::Timeout);

        let result = mock.create_pr("my-feature", "main", "My Feature", "Description", &PrOptions::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));
    }

    #[test]
    fn test_create_pr_handles_auth_failure() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response("create_pr:my-feature", MockResponse::AuthError);

        let result = mock.create_pr("my-feature", "main", "My Feature", "Description", &PrOptions::default());
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("Authentication") || error.contains("token"));
    }

    #[test]
    fn test_update_pr_body_partial_failure() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response("update_pr_body:123", MockResponse::Success("ok".to_string()));
        mock.set_response("update_pr_body:456", MockResponse::Error("PR not found".to_string()));

        // First update succeeds
        let result1 = mock.update_pr_body("123", "Updated body");
        assert!(result1.is_ok());

        // Second update fails
        let result2 = mock.update_pr_body("456", "Updated body");
        assert!(result2.is_err());
        assert!(result2.unwrap_err().to_string().contains("not found"));

        // Both calls were made
        assert_eq!(mock.get_call_count("update_pr_body:123"), 1);
        assert_eq!(mock.get_call_count("update_pr_body:456"), 1);
    }

    #[test]
    fn test_update_pr_base_handles_failures() {
        let mock = MockForge::new(ForgeType::GitHub);

        // Rate limit
        mock.set_response("update_pr_base:feature-1", MockResponse::RateLimit);
        let result = mock.update_pr_base("feature-1", "new-parent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("rate limit"));

        // Timeout
        mock.set_response("update_pr_base:feature-2", MockResponse::Timeout);
        let result = mock.update_pr_base("feature-2", "new-parent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));

        // Auth error
        mock.set_response("update_pr_base:feature-3", MockResponse::AuthError);
        let result = mock.update_pr_base("feature-3", "new-parent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Authentication"));

        // Custom error
        mock.set_response(
            "update_pr_base:feature-4",
            MockResponse::Error("PR #999 not found".to_string()),
        );
        let result = mock.update_pr_base("feature-4", "new-parent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_push_branch_handles_failures() {
        let mock = MockForge::new(ForgeType::GitHub);

        // Auth failure
        mock.set_response("push_branch:feature", MockResponse::AuthError);
        let result = mock.push_branch("feature", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Authentication"));

        // Network timeout
        mock.set_response("push_branch:feature-2", MockResponse::Timeout);
        let result = mock.push_branch("feature-2", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));

        // Rejected
        mock.set_response("push_branch:feature-3", MockResponse::Error("rejected".to_string()));
        let result = mock.push_branch("feature-3", true);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("rejected"));
    }

    #[test]
    fn test_merge_pr_handles_checks_failing() {
        let mock = MockForge::new(ForgeType::GitHub);
        mock.set_response(
            "merge_pr:123",
            MockResponse::Error("Cannot merge: required status checks have not passed".to_string()),
        );

        let result = mock.merge_pr("123", MergeMethod::Squash, false);
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("status checks") || error.contains("checks"));
    }

    #[test]
    fn test_batch_operations_mixed_results() {
        let mock = MockForge::new(ForgeType::GitHub);

        // Configure mixed results
        mock.set_response("pr_exists:branch-1", MockResponse::Success("101".to_string()));
        mock.set_response("pr_exists:branch-2", MockResponse::Timeout);
        mock.set_response("pr_exists:branch-3", MockResponse::Success("103".to_string()));
        mock.set_response("pr_exists:branch-4", MockResponse::RateLimit);

        // Check each branch
        let result1 = mock.pr_exists("branch-1");
        let result2 = mock.pr_exists("branch-2");
        let result3 = mock.pr_exists("branch-3");
        let result4 = mock.pr_exists("branch-4");

        // Verify results
        assert!(result1.is_ok() && result1.unwrap().is_some());
        assert!(result2.is_err());
        assert!(result3.is_ok() && result3.unwrap().is_some());
        assert!(result4.is_err());

        // All calls were made
        assert_eq!(mock.get_call_count("pr_exists:branch-1"), 1);
        assert_eq!(mock.get_call_count("pr_exists:branch-2"), 1);
        assert_eq!(mock.get_call_count("pr_exists:branch-3"), 1);
        assert_eq!(mock.get_call_count("pr_exists:branch-4"), 1);
    }

    #[test]
    fn test_get_pr_full_info_network_failures() {
        let mock = MockForge::new(ForgeType::GitHub);

        // Timeout
        mock.set_response("get_pr_full_info:123", MockResponse::Timeout);
        let result = mock.get_pr_full_info("123");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));

        // Rate limit
        mock.set_response("get_pr_full_info:456", MockResponse::RateLimit);
        let result = mock.get_pr_full_info("456");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("rate limit"));
    }

    #[test]
    fn test_get_pr_body_handles_errors() {
        let mock = MockForge::new(ForgeType::GitHub);

        // Success
        mock.set_response("get_pr_body:123", MockResponse::Success("PR body content".to_string()));
        let result = mock.get_pr_body("123");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "PR body content");

        // Not found
        mock.set_response("get_pr_body:999", MockResponse::Error("PR not found".to_string()));
        let result = mock.get_pr_body("999");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
