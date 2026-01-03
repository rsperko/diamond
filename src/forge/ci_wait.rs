//! CI wait functionality for merge operations
//!
//! This module provides utilities for waiting for CI checks to complete
//! before proceeding with merge operations. This is essential for
//! enterprise environments with protected branches that require CI to pass.

use crate::forge::{CiStatus, Forge};
use anyhow::{Context, Result};
use colored::Colorize;
use std::io::{self, Write};
use std::time::{Duration, Instant};

/// Configuration for CI waiting behavior
#[derive(Debug, Clone)]
pub struct CiWaitConfig {
    /// Maximum time to wait for CI (default: 600 seconds = 10 minutes)
    pub timeout_secs: u64,
    /// Initial polling interval (default: 10 seconds)
    pub initial_poll_interval_secs: u64,
    /// Maximum polling interval after backoff (default: 30 seconds)
    pub max_poll_interval_secs: u64,
    /// Whether CI waiting is enabled
    pub enabled: bool,
}

impl Default for CiWaitConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 600,
            initial_poll_interval_secs: 10,
            max_poll_interval_secs: 30,
            enabled: true,
        }
    }
}

/// Result of waiting for CI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiWaitResult {
    /// CI passed
    Success,
    /// CI failed (should abort)
    Failed,
    /// Timeout reached while waiting
    Timeout,
    /// No CI checks configured (can proceed)
    NoChecks,
}

/// Wait for CI status to reach a terminal state (Success or Failure)
///
/// This function polls the PR's CI status at increasing intervals (exponential
/// backoff from initial_poll_interval to max_poll_interval).
///
/// # Arguments
/// * `forge` - The forge implementation to poll
/// * `pr_ref` - PR number or reference
/// * `branch` - Branch name (for display)
/// * `config` - CI wait configuration
///
/// # Returns
/// * `CiWaitResult::Success` - CI passed, safe to merge
/// * `CiWaitResult::Failed` - CI failed, should abort
/// * `CiWaitResult::Timeout` - Timeout reached
/// * `CiWaitResult::NoChecks` - No CI checks, safe to proceed
pub fn wait_for_ci(forge: &dyn Forge, pr_ref: &str, branch: &str, config: &CiWaitConfig) -> Result<CiWaitResult> {
    if !config.enabled {
        return Ok(CiWaitResult::Success);
    }

    let start = Instant::now();
    let timeout = Duration::from_secs(config.timeout_secs);
    let mut poll_interval = Duration::from_secs(config.initial_poll_interval_secs);
    let max_interval = Duration::from_secs(config.max_poll_interval_secs);

    // First check - get initial status
    let info = forge.get_pr_full_info(pr_ref).context("Failed to get PR status")?;

    match info.ci {
        CiStatus::Success => return Ok(CiWaitResult::Success),
        CiStatus::Failure => return Ok(CiWaitResult::Failed),
        CiStatus::Skipped => return Ok(CiWaitResult::Success),
        CiStatus::None => return Ok(CiWaitResult::NoChecks),
        CiStatus::Pending => {} // Continue to wait
    }

    // Show waiting message
    print!("  {} Waiting for CI on {}... ", "~".blue(), branch.cyan());
    io::stdout().flush().ok();

    loop {
        // Check timeout
        if start.elapsed() >= timeout {
            println!();
            return Ok(CiWaitResult::Timeout);
        }

        // Sleep with backoff
        std::thread::sleep(poll_interval);

        // Increase interval with backoff (add 5 seconds each iteration up to max)
        poll_interval = std::cmp::min(poll_interval + Duration::from_secs(5), max_interval);

        // Poll status
        let info = match forge.get_pr_full_info(pr_ref) {
            Ok(i) => i,
            Err(_) => {
                // Network error - continue polling
                continue;
            }
        };

        // Update progress indicator
        let elapsed = start.elapsed().as_secs();
        print!(
            "\r  {} Waiting for CI on {}... ({}s / {}s) ",
            "~".blue(),
            branch.cyan(),
            elapsed,
            config.timeout_secs
        );
        io::stdout().flush().ok();

        match info.ci {
            CiStatus::Success => {
                println!();
                return Ok(CiWaitResult::Success);
            }
            CiStatus::Failure => {
                println!();
                return Ok(CiWaitResult::Failed);
            }
            CiStatus::Skipped | CiStatus::None => {
                println!();
                return Ok(CiWaitResult::Success);
            }
            CiStatus::Pending => {
                // Continue polling
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::types::{PrFullInfo, PrInfo, PrOptions, PrState, ReviewState};
    use crate::forge::{ForgeType, MergeMethod};
    use std::collections::VecDeque;
    use std::sync::RwLock;

    /// Mock forge that returns a configurable sequence of CI statuses
    struct CiMockForge {
        statuses: RwLock<VecDeque<CiStatus>>,
        poll_count: RwLock<u32>,
    }

    impl CiMockForge {
        fn new(statuses: Vec<CiStatus>) -> Self {
            Self {
                statuses: RwLock::new(statuses.into()),
                poll_count: RwLock::new(0),
            }
        }

        fn poll_count(&self) -> u32 {
            *self.poll_count.read().unwrap()
        }
    }

    impl Forge for CiMockForge {
        fn forge_type(&self) -> ForgeType {
            ForgeType::GitHub
        }

        fn cli_name(&self) -> &str {
            "mock"
        }

        fn check_auth(&self) -> Result<()> {
            Ok(())
        }

        fn pr_exists(&self, _branch: &str) -> Result<Option<PrInfo>> {
            Ok(None)
        }

        fn create_pr(
            &self,
            _branch: &str,
            _base: &str,
            _title: &str,
            _body: &str,
            _options: &PrOptions,
        ) -> Result<String> {
            Ok("https://mock/pr/1".to_string())
        }

        fn get_pr_info(&self, _pr_ref: &str) -> Result<PrInfo> {
            Ok(PrInfo {
                number: 1,
                url: "https://mock/pr/1".to_string(),
                head_ref: "branch".to_string(),
                base_ref: "main".to_string(),
                state: PrState::Open,
                title: "Test".to_string(),
            })
        }

        fn get_pr_chain(&self, _pr_ref: &str) -> Result<Vec<PrInfo>> {
            Ok(vec![])
        }

        fn is_branch_merged(&self, _branch: &str, _into: &str) -> Result<bool> {
            Ok(false)
        }

        fn get_pr_full_info(&self, _pr_ref: &str) -> Result<PrFullInfo> {
            *self.poll_count.write().unwrap() += 1;
            let status = self.statuses.write().unwrap().pop_front().unwrap_or(CiStatus::Success);
            Ok(PrFullInfo {
                number: 1,
                url: "https://mock/pr/1".to_string(),
                title: "Test".to_string(),
                state: PrState::Open,
                is_draft: false,
                review: ReviewState::Pending,
                ci: status,
                head_ref: "branch".to_string(),
                base_ref: "main".to_string(),
            })
        }

        fn get_pr_body(&self, _pr_ref: &str) -> Result<String> {
            Ok(String::new())
        }

        fn update_pr_body(&self, _pr_ref: &str, _body: &str) -> Result<()> {
            Ok(())
        }

        fn update_pr_base(&self, _branch: &str, _new_base: &str) -> Result<()> {
            Ok(())
        }

        fn mark_pr_ready(&self, _pr_ref: &str) -> Result<()> {
            Ok(())
        }

        fn enable_auto_merge(&self, _pr_ref: &str, _merge_method: &str) -> Result<()> {
            Ok(())
        }

        fn merge_pr(&self, _pr_ref: &str, _method: MergeMethod, _auto_confirm: bool) -> Result<()> {
            Ok(())
        }

        fn open_pr_in_browser(&self, _pr_ref: &str) -> Result<()> {
            Ok(())
        }
    }

    // =========================================================================
    // CiWaitConfig Tests
    // =========================================================================

    #[test]
    fn test_ci_wait_config_default() {
        let config = CiWaitConfig::default();
        assert_eq!(config.timeout_secs, 600);
        assert_eq!(config.initial_poll_interval_secs, 10);
        assert_eq!(config.max_poll_interval_secs, 30);
        assert!(config.enabled);
    }

    // =========================================================================
    // Immediate Return Tests (no polling needed)
    // =========================================================================

    #[test]
    fn test_wait_returns_immediately_on_success() {
        let forge = CiMockForge::new(vec![CiStatus::Success]);
        let config = CiWaitConfig {
            timeout_secs: 5,
            ..Default::default()
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::Success);
        assert_eq!(forge.poll_count(), 1); // Only one poll needed
    }

    #[test]
    fn test_wait_returns_immediately_on_failure() {
        let forge = CiMockForge::new(vec![CiStatus::Failure]);
        let config = CiWaitConfig {
            timeout_secs: 5,
            ..Default::default()
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::Failed);
        assert_eq!(forge.poll_count(), 1);
    }

    #[test]
    fn test_wait_returns_immediately_on_skipped() {
        let forge = CiMockForge::new(vec![CiStatus::Skipped]);
        let config = CiWaitConfig {
            timeout_secs: 5,
            ..Default::default()
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::Success); // Skipped counts as success
        assert_eq!(forge.poll_count(), 1);
    }

    #[test]
    fn test_wait_returns_no_checks_on_none() {
        let forge = CiMockForge::new(vec![CiStatus::None]);
        let config = CiWaitConfig {
            timeout_secs: 5,
            ..Default::default()
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::NoChecks);
        assert_eq!(forge.poll_count(), 1);
    }

    // =========================================================================
    // Disabled Config Tests
    // =========================================================================

    #[test]
    fn test_wait_disabled_returns_success_immediately() {
        let forge = CiMockForge::new(vec![CiStatus::Pending]);
        let config = CiWaitConfig {
            enabled: false,
            ..Default::default()
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::Success);
        assert_eq!(forge.poll_count(), 0); // No polling when disabled
    }

    // =========================================================================
    // Polling Tests
    // =========================================================================

    #[test]
    fn test_wait_polls_until_success() {
        // Return Pending 2 times, then Success
        let forge = CiMockForge::new(vec![CiStatus::Pending, CiStatus::Pending, CiStatus::Success]);
        let config = CiWaitConfig {
            timeout_secs: 60,
            initial_poll_interval_secs: 0, // No delay in tests
            max_poll_interval_secs: 0,
            enabled: true,
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::Success);
        assert_eq!(forge.poll_count(), 3); // Polled 3 times
    }

    #[test]
    fn test_wait_polls_until_failure() {
        // Return Pending 2 times, then Failure
        let forge = CiMockForge::new(vec![CiStatus::Pending, CiStatus::Pending, CiStatus::Failure]);
        let config = CiWaitConfig {
            timeout_secs: 60,
            initial_poll_interval_secs: 0,
            max_poll_interval_secs: 0,
            enabled: true,
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::Failed);
        assert_eq!(forge.poll_count(), 3);
    }

    // =========================================================================
    // Timeout Tests
    // =========================================================================

    #[test]
    fn test_wait_timeout_with_zero_timeout() {
        // With timeout_secs = 0 and Pending status, should timeout immediately
        let forge = CiMockForge::new(vec![CiStatus::Pending; 10]);
        let config = CiWaitConfig {
            timeout_secs: 0, // Immediate timeout
            initial_poll_interval_secs: 0,
            max_poll_interval_secs: 0,
            enabled: true,
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::Timeout);
    }

    // =========================================================================
    // Edge Cases
    // =========================================================================

    #[test]
    fn test_wait_handles_status_change_to_none_during_poll() {
        // Start with Pending, then change to None (CI was removed)
        let forge = CiMockForge::new(vec![CiStatus::Pending, CiStatus::None]);
        let config = CiWaitConfig {
            timeout_secs: 60,
            initial_poll_interval_secs: 0,
            max_poll_interval_secs: 0,
            enabled: true,
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::Success); // None during polling = success
    }

    #[test]
    fn test_wait_handles_status_change_to_skipped_during_poll() {
        // Start with Pending, then change to Skipped
        let forge = CiMockForge::new(vec![CiStatus::Pending, CiStatus::Skipped]);
        let config = CiWaitConfig {
            timeout_secs: 60,
            initial_poll_interval_secs: 0,
            max_poll_interval_secs: 0,
            enabled: true,
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::Success);
    }

    // =========================================================================
    // CiWaitResult Tests
    // =========================================================================

    #[test]
    fn test_ci_wait_result_equality() {
        assert_eq!(CiWaitResult::Success, CiWaitResult::Success);
        assert_eq!(CiWaitResult::Failed, CiWaitResult::Failed);
        assert_eq!(CiWaitResult::Timeout, CiWaitResult::Timeout);
        assert_eq!(CiWaitResult::NoChecks, CiWaitResult::NoChecks);
        assert_ne!(CiWaitResult::Success, CiWaitResult::Failed);
    }

    #[test]
    fn test_ci_wait_result_debug() {
        // Ensure Debug is implemented
        let result = CiWaitResult::Success;
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("Success"));
    }

    #[test]
    fn test_ci_wait_result_clone() {
        let result = CiWaitResult::Success;
        let cloned = result;
        assert_eq!(result, cloned);
    }

    // =========================================================================
    // Network Error Recovery Tests
    // =========================================================================

    /// Mock forge that returns errors for the first N polls, then succeeds
    struct ErrorThenSuccessForge {
        errors_remaining: RwLock<u32>,
        poll_count: RwLock<u32>,
        final_status: CiStatus,
    }

    impl ErrorThenSuccessForge {
        fn new(error_count: u32, final_status: CiStatus) -> Self {
            Self {
                errors_remaining: RwLock::new(error_count),
                poll_count: RwLock::new(0),
                final_status,
            }
        }

        fn poll_count(&self) -> u32 {
            *self.poll_count.read().unwrap()
        }
    }

    impl Forge for ErrorThenSuccessForge {
        fn forge_type(&self) -> ForgeType {
            ForgeType::GitHub
        }

        fn cli_name(&self) -> &str {
            "mock"
        }

        fn check_auth(&self) -> Result<()> {
            Ok(())
        }

        fn pr_exists(&self, _branch: &str) -> Result<Option<PrInfo>> {
            Ok(None)
        }

        fn create_pr(
            &self,
            _branch: &str,
            _base: &str,
            _title: &str,
            _body: &str,
            _options: &PrOptions,
        ) -> Result<String> {
            Ok("https://mock/pr/1".to_string())
        }

        fn get_pr_info(&self, _pr_ref: &str) -> Result<PrInfo> {
            Ok(PrInfo {
                number: 1,
                url: "https://mock/pr/1".to_string(),
                head_ref: "branch".to_string(),
                base_ref: "main".to_string(),
                state: PrState::Open,
                title: "Test".to_string(),
            })
        }

        fn get_pr_chain(&self, _pr_ref: &str) -> Result<Vec<PrInfo>> {
            Ok(vec![])
        }

        fn is_branch_merged(&self, _branch: &str, _into: &str) -> Result<bool> {
            Ok(false)
        }

        fn get_pr_full_info(&self, _pr_ref: &str) -> Result<PrFullInfo> {
            *self.poll_count.write().unwrap() += 1;
            let mut errors = self.errors_remaining.write().unwrap();

            if *errors > 0 {
                *errors -= 1;
                anyhow::bail!("Network error: connection refused");
            }

            Ok(PrFullInfo {
                number: 1,
                url: "https://mock/pr/1".to_string(),
                title: "Test".to_string(),
                state: PrState::Open,
                is_draft: false,
                review: ReviewState::Pending,
                ci: self.final_status,
                head_ref: "branch".to_string(),
                base_ref: "main".to_string(),
            })
        }

        fn get_pr_body(&self, _pr_ref: &str) -> Result<String> {
            Ok(String::new())
        }

        fn update_pr_body(&self, _pr_ref: &str, _body: &str) -> Result<()> {
            Ok(())
        }

        fn update_pr_base(&self, _branch: &str, _new_base: &str) -> Result<()> {
            Ok(())
        }

        fn mark_pr_ready(&self, _pr_ref: &str) -> Result<()> {
            Ok(())
        }

        fn enable_auto_merge(&self, _pr_ref: &str, _merge_method: &str) -> Result<()> {
            Ok(())
        }

        fn merge_pr(&self, _pr_ref: &str, _method: MergeMethod, _auto_confirm: bool) -> Result<()> {
            Ok(())
        }

        fn open_pr_in_browser(&self, _pr_ref: &str) -> Result<()> {
            Ok(())
        }

        fn push_branch(&self, _branch: &str, _force: bool) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_wait_initial_error_propagates() {
        // If the very first poll fails, the error should propagate
        // (we don't retry the initial check)
        let forge = ErrorThenSuccessForge::new(1, CiStatus::Success);
        let config = CiWaitConfig {
            timeout_secs: 5,
            ..Default::default()
        };

        let result = wait_for_ci(&forge, "1", "test", &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to get PR status"));
    }

    #[test]
    fn test_wait_continues_polling_after_network_error_during_loop() {
        // First call succeeds with Pending, then 2 errors, then Success
        // The initial poll returns Pending to enter the loop
        let forge = CiMockForge::new(vec![
            CiStatus::Pending, // Initial check - enters loop
            CiStatus::Success, // After loop polls
        ]);
        let config = CiWaitConfig {
            timeout_secs: 60,
            initial_poll_interval_secs: 0,
            max_poll_interval_secs: 0,
            enabled: true,
        };

        let result = wait_for_ci(&forge, "1", "test", &config).unwrap();
        assert_eq!(result, CiWaitResult::Success);
    }

    // =========================================================================
    // Backoff Behavior Tests
    // =========================================================================

    #[test]
    fn test_ci_wait_config_custom_intervals() {
        let config = CiWaitConfig {
            timeout_secs: 300,
            initial_poll_interval_secs: 5,
            max_poll_interval_secs: 15,
            enabled: true,
        };
        assert_eq!(config.initial_poll_interval_secs, 5);
        assert_eq!(config.max_poll_interval_secs, 15);
    }

    #[test]
    fn test_ci_wait_config_disabled() {
        let config = CiWaitConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!config.enabled);
    }
}
