//! Section headers and formatting for multi-step operations.

use colored::Colorize;
use std::io::IsTerminal;

use super::style::*;

// ──────────────────────────────────────────────────────────────
// Section headers
// ──────────────────────────────────────────────────────────────

/// Print a section header with horizontal line.
///
/// In TTY mode: `── Title ──────────────────────`
/// In non-TTY mode: `=== Title ===`
///
/// # Example
/// ```ignore
/// ui::header("Syncing branches");
/// // ... do sync work ...
/// ui::header("Restacking");
/// // ... do restack work ...
/// ```
pub fn header(title: &str) {
    if std::io::stdout().is_terminal() {
        // Calculate padding for consistent width (50 chars total)
        let title_len = title.chars().count();
        let total_width: usize = 50;
        let side_len: usize = 2; // "── "
        let remaining = total_width.saturating_sub(title_len + side_len + 1);

        println!();
        println!(
            "{} {} {}",
            "──".bright_black(),
            title.bold(),
            "─".repeat(remaining).bright_black()
        );
        println!();
    } else {
        println!();
        println!("=== {} ===", title);
        println!();
    }
}

/// Print a sub-section header (smaller, no extra newlines).
///
/// In TTY mode: `→ Title`
/// In non-TTY mode: `--- Title ---`
pub fn subheader(title: &str) {
    if std::io::stdout().is_terminal() {
        println!("{} {}", MARK_STEP.blue(), title.bold());
    } else {
        println!("--- {} ---", title);
    }
}

/// Print a decorative section separator (opt-in via --decorators flag).
///
/// When enabled, prints: `━━━ Title ━━━`
/// When disabled, just prints a blank line for minimal separation.
///
/// This is for users who want more visual structure in their output.
/// Most users should leave this disabled (default) for cleaner output.
///
/// # Example
/// ```ignore
/// ui::decorator("Fetch", decorators_enabled);
/// // ... fetch operations ...
/// ui::decorator("Sync", decorators_enabled);
/// // ... sync operations ...
/// ```
pub fn decorator(title: &str, enabled: bool) {
    if !enabled {
        // Minimal separation - just a blank line
        println!();
        return;
    }

    if std::io::stdout().is_terminal() {
        // Pretty decorated separator
        println!("{} {} {}", "━━━".bright_black(), title.bold(), "━━━".bright_black());
    } else {
        // Non-TTY: simple ASCII separator
        println!("=== {} ===", title);
    }
}

// ──────────────────────────────────────────────────────────────
// Summary boxes
// ──────────────────────────────────────────────────────────────

/// Print a summary box with key-value pairs.
///
/// # Example
/// ```ignore
/// ui::summary("Sync Results", &[
///     ("Branches synced", "5"),
///     ("Conflicts", "0"),
///     ("PRs updated", "3"),
/// ]);
/// ```
pub fn summary(title: &str, items: &[(&str, &str)]) {
    if std::io::stdout().is_terminal() {
        println!();
        println!("{}", title.bold());
        for (key, value) in items {
            println!("  {}: {}", key.bright_black(), value);
        }
    } else {
        println!();
        println!("{}:", title);
        for (key, value) in items {
            println!("  {}: {}", key, value);
        }
    }
}

// ──────────────────────────────────────────────────────────────
// Completion messages
// ──────────────────────────────────────────────────────────────

/// Print operation complete message with optional stats.
///
/// # Example
/// ```ignore
/// ui::complete("Sync complete!", Some(&[
///     ("branches", 5),
///     ("PRs updated", 3),
/// ]));
/// ```
pub fn complete(message: &str, stats: Option<&[(&str, usize)]>) {
    println!();
    println!("{} {}", MARK_SUCCESS.green().bold(), message.green().bold());

    if let Some(stats) = stats {
        for (label, count) in stats {
            if *count > 0 {
                println!(
                    "  {} {} {}",
                    MARK_BULLET.bright_black(),
                    count.to_string().yellow(),
                    label
                );
            }
        }
    }
}

/// Print dry-run notice.
pub fn dry_run_notice() {
    println!(
        "{} {}",
        "[preview]".yellow().bold(),
        "Dry run - no changes made".yellow()
    );
}

/// Print preview header for dry-run mode.
pub fn preview_header(message: &str) {
    println!("{} {}", "[preview]".yellow().bold(), message);
}

// ──────────────────────────────────────────────────────────────
// Error boxes
// ──────────────────────────────────────────────────────────────

/// Print a styled error box (for main.rs error handler).
///
/// In TTY mode, shows a bordered box.
/// In non-TTY mode, shows plain "Error: message".
pub fn error_box(message: &str) {
    if std::io::stdout().is_terminal() {
        let lines: Vec<&str> = message.lines().collect();
        let max_width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let box_width = max_width.max(40) + 4;

        eprintln!();
        eprintln!(
            "{}{}{}",
            "┌─ ".red(),
            "Error".red().bold(),
            format!(" {}", "─".repeat(box_width - 10)).red()
        );
        for line in &lines {
            let padding = box_width - 4 - line.chars().count();
            eprintln!("{} {}{}", "│".red(), line, " ".repeat(padding));
        }
        eprintln!("{}", format!("└{}┘", "─".repeat(box_width - 2)).red());
    } else {
        eprintln!("Error: {}", message);
    }
}

/// Print a styled warning box.
pub fn warning_box(message: &str) {
    if std::io::stdout().is_terminal() {
        let lines: Vec<&str> = message.lines().collect();

        eprintln!();
        eprintln!("{} {}", MARK_WARNING.yellow().bold(), "Warning".yellow().bold());
        for line in &lines {
            eprintln!("  {}", line.yellow());
        }
    } else {
        eprintln!("Warning: {}", message);
    }
}

// ──────────────────────────────────────────────────────────────
// Progress counter
// ──────────────────────────────────────────────────────────────

/// Print a step counter: "[1/5] message"
pub fn step_counter(current: usize, total: usize, message: &str) {
    println!("  {} {}", format!("[{}/{}]", current, total).bright_black(), message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_doesnt_panic() {
        header("Test Section");
        subheader("Sub Section");
    }

    #[test]
    fn test_summary_doesnt_panic() {
        summary("Results", &[("Branches", "5"), ("Conflicts", "0"), ("Updated", "3")]);
    }

    #[test]
    fn test_complete_doesnt_panic() {
        complete("Operation complete!", None);
        complete("With stats", Some(&[("branches", 5), ("conflicts", 0), ("updated", 3)]));
    }

    #[test]
    fn test_dry_run_notice() {
        dry_run_notice();
        preview_header("Would do something");
    }

    #[test]
    fn test_error_box() {
        error_box("Something went wrong");
        error_box("Multi-line\nerror\nmessage");
    }

    #[test]
    fn test_warning_box() {
        warning_box("Something might be wrong");
    }

    #[test]
    fn test_step_counter() {
        step_counter(1, 5, "Processing item");
        step_counter(5, 5, "Last item");
    }

    #[test]
    fn test_decorator() {
        // Test both enabled and disabled modes
        decorator("Test Section", true);
        decorator("Test Section", false);
    }
}
