//! Beautiful progress display for PR updates.
//!
//! Shows real-time progress when updating multiple PRs in parallel.
//!
//! TTY mode (interactive):
//! ```text
//! Updating stack visualization...
//!   ⡇ #42 feat-auth          Fetching...
//!   ✓ #43 feat-api           Updated
//!   ⡇ #44 feat-ui            Updating...
//!
//! ✓ Updated 3 PRs
//! ```
//!
//! Non-TTY mode (CI/logs):
//! ```text
//! → Updating stack visualization...
//!   → #42 feat-auth
//!   → #43 feat-api
//!   → #44 feat-ui
//! ✓ Updated 3 PRs
//! ```

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::io::IsTerminal;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::style::*;

/// Status of a single PR update operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrStatus {
    /// Waiting to start
    Pending,
    /// Fetching PR info from remote
    Fetching,
    /// Generating stack markdown
    Generating,
    /// Updating PR description
    Updating,
    /// Successfully updated
    Done,
    /// Skipped (not open, or no changes needed)
    Skipped,
    /// Failed to update
    Failed,
}

impl PrStatus {
    fn label(&self) -> &'static str {
        match self {
            PrStatus::Pending => "Waiting...",
            PrStatus::Fetching => "Fetching...",
            PrStatus::Generating => "Generating...",
            PrStatus::Updating => "Updating...",
            PrStatus::Done => "Updated",
            PrStatus::Skipped => "Skipped",
            PrStatus::Failed => "Failed",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, PrStatus::Done | PrStatus::Skipped | PrStatus::Failed)
    }
}

/// Information about a PR being updated.
#[derive(Clone, Debug)]
pub struct PrUpdateInfo {
    /// PR number (e.g., 42)
    pub number: u64,
    /// Branch name (e.g., "feat-auth")
    pub branch: String,
    /// Current status
    pub status: PrStatus,
}

/// Progress tracker for parallel PR updates.
///
/// In TTY mode, shows animated spinners for each PR.
/// In non-TTY mode, prints simple progress messages.
pub struct PrProgressTracker {
    /// Whether we're in TTY mode
    is_tty: bool,
    /// Multi-progress container (TTY mode only)
    mp: Option<MultiProgress>,
    /// Individual progress bars keyed by PR number
    bars: Arc<Mutex<HashMap<u64, ProgressBar>>>,
    /// PR info keyed by PR number
    infos: Arc<Mutex<HashMap<u64, PrUpdateInfo>>>,
}

impl PrProgressTracker {
    /// Create a new progress tracker.
    pub fn new(header: &str) -> Self {
        let is_tty = std::io::stdout().is_terminal();

        // Print header
        println!("{} {}", MARK_STEP.blue(), header);

        let mp = if is_tty { Some(MultiProgress::new()) } else { None };

        Self {
            is_tty,
            mp,
            bars: Arc::new(Mutex::new(HashMap::new())),
            infos: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Add a PR to track.
    pub fn add_pr(&self, number: u64, branch: &str) {
        let info = PrUpdateInfo {
            number,
            branch: branch.to_string(),
            status: PrStatus::Pending,
        };

        self.infos.lock().unwrap().insert(number, info.clone());

        if let Some(mp) = &self.mp {
            let pb = mp.add(ProgressBar::new_spinner());
            pb.set_style(
                ProgressStyle::default_spinner()
                    .tick_chars(SPINNER_FRAMES)
                    .template("  {spinner:.blue} {msg}")
                    .expect("Invalid spinner template"),
            );
            pb.set_message(self.format_pr_line(&info));
            pb.enable_steady_tick(Duration::from_millis(80));
            self.bars.lock().unwrap().insert(number, pb);
        }
    }

    /// Update the status of a PR.
    pub fn update_status(&self, number: u64, status: PrStatus) {
        let mut infos = self.infos.lock().unwrap();
        if let Some(info) = infos.get_mut(&number) {
            info.status = status;
            let line = self.format_pr_line(info);

            if self.is_tty {
                let bars = self.bars.lock().unwrap();
                if let Some(pb) = bars.get(&number) {
                    if status.is_terminal() {
                        let marker = match status {
                            PrStatus::Done => MARK_SUCCESS.green().to_string(),
                            PrStatus::Skipped => MARK_SKIP.bright_black().to_string(),
                            PrStatus::Failed => MARK_ERROR.red().to_string(),
                            _ => unreachable!(),
                        };
                        pb.finish_with_message(format!("{} {}", marker, line));
                    } else {
                        pb.set_message(line);
                    }
                }
            } else if status.is_terminal() {
                // Non-TTY: print completion message
                let marker = match status {
                    PrStatus::Done => MARK_SUCCESS.green().to_string(),
                    PrStatus::Skipped => MARK_SKIP.bright_black().to_string(),
                    PrStatus::Failed => MARK_ERROR.red().to_string(),
                    _ => unreachable!(),
                };
                println!("  {} {}", marker, line);
            }
        }
    }

    /// Batch add multiple PRs.
    pub fn add_prs(&self, prs: &[(u64, &str)]) {
        for (number, branch) in prs {
            self.add_pr(*number, branch);
        }
    }

    /// Mark all pending PRs as fetching.
    pub fn start_fetching(&self) {
        let numbers: Vec<u64> = self.infos.lock().unwrap().keys().cloned().collect();
        for number in numbers {
            self.update_status(number, PrStatus::Fetching);
        }
    }

    /// Finish the progress display with a summary.
    pub fn finish(&self, updated: usize, skipped: usize, failed: usize) {
        // Ensure all progress bars are finished
        if self.is_tty {
            let bars = self.bars.lock().unwrap();
            for pb in bars.values() {
                if !pb.is_finished() {
                    pb.finish_and_clear();
                }
            }
        }

        // Print summary
        println!();
        if failed > 0 {
            println!(
                "{} Updated {} PR{}, {} failed",
                MARK_WARNING.yellow(),
                updated.to_string().green(),
                if updated == 1 { "" } else { "s" },
                failed.to_string().red()
            );
        } else if updated > 0 {
            println!(
                "{} Updated {} PR{}",
                MARK_SUCCESS.green(),
                updated.to_string().green().bold(),
                if updated == 1 { "" } else { "s" }
            );
        } else if skipped > 0 {
            println!(
                "{} {} PR{} already up to date",
                MARK_INFO.blue(),
                skipped,
                if skipped == 1 { "" } else { "s" }
            );
        } else {
            println!("{} No PRs to update", MARK_INFO.blue());
        }
    }

    /// Format a PR line for display.
    fn format_pr_line(&self, info: &PrUpdateInfo) -> String {
        // Truncate branch name if too long
        let max_branch_len = 20;
        let branch_display = if info.branch.len() > max_branch_len {
            format!("{}...", &info.branch[..max_branch_len - 3])
        } else {
            info.branch.clone()
        };

        // Pad for alignment
        let padded_branch = format!("{:<width$}", branch_display, width = max_branch_len);

        format!(
            "{} {}  {}",
            format!("#{}", info.number).cyan(),
            padded_branch.white(),
            info.status.label().bright_black()
        )
    }
}

/// Simplified progress for when we just want to show a spinner with count.
pub struct SimplePrProgress {
    spinner: Option<ProgressBar>,
    total: usize,
}

impl SimplePrProgress {
    /// Create a new simple progress indicator.
    pub fn new(message: &str, total: usize) -> Self {
        if !std::io::stdout().is_terminal() {
            println!("{} {} ({} PRs)", MARK_STEP.blue(), message, total);
            return Self { spinner: None, total };
        }

        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_chars(SPINNER_FRAMES)
                .template("{spinner:.blue} {msg}")
                .expect("Invalid spinner template"),
        );
        pb.set_message(format!("{} ({} PRs)", message, total));
        pb.enable_steady_tick(Duration::from_millis(80));

        Self {
            spinner: Some(pb),
            total,
        }
    }

    /// Update the message.
    pub fn set_message(&self, message: &str) {
        if let Some(pb) = &self.spinner {
            pb.set_message(format!("{} ({} PRs)", message, self.total));
        }
    }

    /// Finish with success.
    pub fn finish_success(&self, updated: usize) {
        match &self.spinner {
            Some(pb) => {
                pb.finish_with_message(format!(
                    "{} Updated {} PR{}",
                    MARK_SUCCESS.green(),
                    updated.to_string().green().bold(),
                    if updated == 1 { "" } else { "s" }
                ));
            }
            None => {
                println!(
                    "{} Updated {} PR{}",
                    MARK_SUCCESS.green(),
                    updated,
                    if updated == 1 { "" } else { "s" }
                );
            }
        }
    }

    /// Finish with a custom message.
    pub fn finish(&self, message: &str) {
        match &self.spinner {
            Some(pb) => {
                pb.finish_with_message(message.to_string());
            }
            None => {
                println!("{}", message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pr_status_labels() {
        assert_eq!(PrStatus::Pending.label(), "Waiting...");
        assert_eq!(PrStatus::Fetching.label(), "Fetching...");
        assert_eq!(PrStatus::Generating.label(), "Generating...");
        assert_eq!(PrStatus::Updating.label(), "Updating...");
        assert_eq!(PrStatus::Done.label(), "Updated");
        assert_eq!(PrStatus::Skipped.label(), "Skipped");
        assert_eq!(PrStatus::Failed.label(), "Failed");
    }

    #[test]
    fn test_pr_status_terminal() {
        assert!(!PrStatus::Pending.is_terminal());
        assert!(!PrStatus::Fetching.is_terminal());
        assert!(!PrStatus::Generating.is_terminal());
        assert!(!PrStatus::Updating.is_terminal());
        assert!(PrStatus::Done.is_terminal());
        assert!(PrStatus::Skipped.is_terminal());
        assert!(PrStatus::Failed.is_terminal());
    }

    #[test]
    fn test_tracker_creation() {
        // Should not panic in non-TTY test environment
        let tracker = PrProgressTracker::new("Testing...");
        tracker.add_pr(42, "feature-branch");
        tracker.update_status(42, PrStatus::Fetching);
        tracker.update_status(42, PrStatus::Done);
        tracker.finish(1, 0, 0);
    }

    #[test]
    fn test_simple_progress() {
        let progress = SimplePrProgress::new("Testing...", 3);
        progress.set_message("Still testing...");
        progress.finish_success(3);
    }

    #[test]
    fn test_tracker_batch_add() {
        let tracker = PrProgressTracker::new("Batch test");
        tracker.add_prs(&[(1, "branch-a"), (2, "branch-b"), (3, "branch-c")]);
        tracker.start_fetching();
        tracker.finish(3, 0, 0);
    }

    #[test]
    fn test_tracker_mixed_status() {
        // Test a realistic scenario with mixed outcomes
        let tracker = PrProgressTracker::new("Mixed status test");
        tracker.add_prs(&[(1, "branch-updated"), (2, "branch-skipped"), (3, "branch-failed")]);

        tracker.update_status(1, PrStatus::Fetching);
        tracker.update_status(2, PrStatus::Fetching);
        tracker.update_status(3, PrStatus::Fetching);

        tracker.update_status(1, PrStatus::Done);
        tracker.update_status(2, PrStatus::Skipped);
        tracker.update_status(3, PrStatus::Failed);

        tracker.finish(1, 1, 1);
    }

    #[test]
    fn test_tracker_all_skipped() {
        let tracker = PrProgressTracker::new("All skipped test");
        tracker.add_prs(&[(1, "merged-pr"), (2, "closed-pr")]);
        tracker.update_status(1, PrStatus::Skipped);
        tracker.update_status(2, PrStatus::Skipped);
        tracker.finish(0, 2, 0);
    }

    #[test]
    fn test_tracker_empty() {
        // Edge case: no PRs to track
        let tracker = PrProgressTracker::new("Empty test");
        tracker.finish(0, 0, 0);
    }

    #[test]
    fn test_tracker_update_nonexistent_pr() {
        // Should not panic when updating status of PR that wasn't added
        let tracker = PrProgressTracker::new("Nonexistent PR test");
        tracker.add_pr(1, "branch-a");
        tracker.update_status(999, PrStatus::Done); // PR 999 doesn't exist
        tracker.finish(0, 0, 0);
    }

    #[test]
    fn test_tracker_long_branch_name() {
        // Test truncation of long branch names
        let tracker = PrProgressTracker::new("Long name test");
        tracker.add_pr(1, "this-is-a-very-long-branch-name-that-should-be-truncated");
        tracker.update_status(1, PrStatus::Done);
        tracker.finish(1, 0, 0);
    }

    #[test]
    fn test_pr_update_info_clone() {
        let info = PrUpdateInfo {
            number: 42,
            branch: "feature".to_string(),
            status: PrStatus::Pending,
        };
        let cloned = info.clone();
        assert_eq!(cloned.number, 42);
        assert_eq!(cloned.branch, "feature");
        assert_eq!(cloned.status, PrStatus::Pending);
    }

    #[test]
    fn test_simple_progress_finish_custom() {
        let progress = SimplePrProgress::new("Custom finish test", 2);
        progress.finish("Custom message here");
    }

    #[test]
    fn test_status_transitions() {
        // Test the full lifecycle of a PR update
        let tracker = PrProgressTracker::new("Lifecycle test");
        tracker.add_pr(1, "feature");

        // Walk through all non-terminal states
        tracker.update_status(1, PrStatus::Pending);
        tracker.update_status(1, PrStatus::Fetching);
        tracker.update_status(1, PrStatus::Generating);
        tracker.update_status(1, PrStatus::Updating);
        tracker.update_status(1, PrStatus::Done);

        tracker.finish(1, 0, 0);
    }
}
