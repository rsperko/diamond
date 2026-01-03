//! Progress indicators: spinners and progress bars.
//!
//! All functions gracefully degrade when not in a TTY.

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::io::IsTerminal;
use std::time::Duration;

use super::style::*;

// ──────────────────────────────────────────────────────────────
// Spinners
// ──────────────────────────────────────────────────────────────

/// Create a spinner for indeterminate operations.
///
/// Returns `Some(ProgressBar)` in TTY mode, `None` otherwise.
/// When not in TTY, prints a plain step message instead.
///
/// # Example
/// ```ignore
/// let spin = ui::spinner("Fetching from origin...");
/// do_fetch()?;
/// ui::spinner_success(spin, "Fetched from origin");
/// ```
pub fn spinner(message: &str) -> Option<ProgressBar> {
    if !std::io::stdout().is_terminal() {
        // Non-TTY: print plain message
        println!("{} {}", MARK_STEP.blue(), message);
        return None;
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars(SPINNER_FRAMES)
            .template("{spinner:.blue} {msg}")
            .expect("Invalid spinner template"),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    Some(pb)
}

/// Create an indented spinner (for sub-steps).
pub fn spinner_indented(message: &str) -> Option<ProgressBar> {
    if !std::io::stdout().is_terminal() {
        println!("  {} {}", MARK_STEP.blue(), message);
        return None;
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars(SPINNER_FRAMES)
            .template("  {spinner:.blue} {msg}")
            .expect("Invalid spinner template"),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    Some(pb)
}

/// Finish spinner with success message.
pub fn spinner_success(spinner: Option<ProgressBar>, message: &str) {
    match spinner {
        Some(pb) => {
            pb.finish_with_message(format!("{} {}", MARK_SUCCESS.green(), message));
        }
        None => {
            // Non-TTY: print success message
            println!("  {} {}", MARK_SUCCESS.green(), message);
        }
    }
}

/// Finish spinner with error message.
pub fn spinner_error(spinner: Option<ProgressBar>, message: &str) {
    match spinner {
        Some(pb) => {
            pb.finish_with_message(format!("{} {}", MARK_ERROR.red(), message));
        }
        None => {
            // Non-TTY: print error message
            println!("  {} {}", MARK_ERROR.red(), message);
        }
    }
}

/// Finish spinner with warning message.
pub fn spinner_warning(spinner: Option<ProgressBar>, message: &str) {
    match spinner {
        Some(pb) => {
            pb.finish_with_message(format!("{} {}", MARK_WARNING.yellow(), message));
        }
        None => {
            println!("  {} {}", MARK_WARNING.yellow(), message);
        }
    }
}

/// Finish spinner with info message (clears spinner, shows info).
pub fn spinner_info(spinner: Option<ProgressBar>, message: &str) {
    match spinner {
        Some(pb) => {
            pb.finish_with_message(format!("{} {}", MARK_INFO.blue(), message));
        }
        None => {
            println!("  {} {}", MARK_INFO.blue(), message);
        }
    }
}

/// Clear spinner without leaving a message.
pub fn spinner_clear(spinner: Option<ProgressBar>) {
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
}

// ──────────────────────────────────────────────────────────────
// Progress bars
// ──────────────────────────────────────────────────────────────

/// Create a progress bar for counted operations.
///
/// Returns `Some(ProgressBar)` in TTY mode, `None` otherwise.
///
/// # Example
/// ```ignore
/// let pb = ui::progress_bar(branches.len(), "Rebasing branches");
/// for branch in branches {
///     rebase(branch)?;
///     ui::progress_inc(&pb);
/// }
/// ui::progress_finish(pb, "Rebased all branches");
/// ```
pub fn progress_bar(total: u64, message: &str) -> Option<ProgressBar> {
    if !std::io::stdout().is_terminal() {
        println!("{} {} ({})", MARK_STEP.blue(), message, total);
        return None;
    }

    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.blue} {msg} [{bar:30.cyan/dim}] {pos}/{len}")
            .expect("Invalid progress bar template")
            .tick_chars(SPINNER_FRAMES)
            .progress_chars("━━╺"),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    Some(pb)
}

/// Increment progress bar.
pub fn progress_inc(pb: &Option<ProgressBar>) {
    if let Some(pb) = pb {
        pb.inc(1);
    }
}

/// Update progress bar message.
pub fn progress_message(pb: &Option<ProgressBar>, message: &str) {
    if let Some(pb) = pb {
        pb.set_message(message.to_string());
    }
}

/// Finish progress bar with success.
pub fn progress_finish(pb: Option<ProgressBar>, message: &str) {
    match pb {
        Some(pb) => {
            pb.finish_with_message(format!("{} {}", MARK_SUCCESS.green(), message));
        }
        None => {
            println!("{} {}", MARK_SUCCESS.green(), message);
        }
    }
}

/// Finish progress bar with error.
pub fn progress_error(pb: Option<ProgressBar>, message: &str) {
    match pb {
        Some(pb) => {
            pb.finish_with_message(format!("{} {}", MARK_ERROR.red(), message));
        }
        None => {
            println!("{} {}", MARK_ERROR.red(), message);
        }
    }
}

// ──────────────────────────────────────────────────────────────
// Multi-progress (for parallel operations)
// ──────────────────────────────────────────────────────────────

/// Create a multi-progress container for parallel spinners.
pub fn multi_progress() -> MultiProgress {
    MultiProgress::new()
}

/// Add a spinner to a multi-progress container.
pub fn multi_spinner(mp: &MultiProgress, message: &str) -> ProgressBar {
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars(SPINNER_FRAMES)
            .template("  {spinner:.blue} {msg}")
            .expect("Invalid spinner template"),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_returns_none_in_non_tty() {
        // In test environment, stdout is typically not a TTY
        // This test verifies the function doesn't panic
        let spin = spinner("Testing...");
        spinner_success(spin, "Done");
    }

    #[test]
    fn test_spinner_finish_variants() {
        // Test all finish variants don't panic
        spinner_success(None, "success");
        spinner_error(None, "error");
        spinner_warning(None, "warning");
        spinner_info(None, "info");
        spinner_clear(None);
    }

    #[test]
    fn test_progress_bar_returns_none_in_non_tty() {
        let pb = progress_bar(10, "Testing...");
        progress_inc(&pb);
        progress_message(&pb, "Updated");
        progress_finish(pb, "Done");
    }

    #[test]
    fn test_progress_error() {
        let pb = progress_bar(10, "Testing...");
        progress_error(pb, "Failed");
    }

    #[test]
    fn test_multi_progress() {
        let mp = multi_progress();
        let pb = multi_spinner(&mp, "Task 1");
        pb.finish_with_message("Done");
    }
}
