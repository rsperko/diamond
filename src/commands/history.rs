use anyhow::Result;
use chrono::Local;
use colored::Colorize;

use crate::operation_log::{Operation, OperationLog};
use crate::program_name::program_name;

/// Show operation history
pub fn run(count: Option<usize>) -> Result<()> {
    let log = OperationLog::new()?;
    let limit = count.unwrap_or(20); // Default to last 20 entries

    let entries = if limit > 0 {
        log.read_last(limit)?
    } else {
        log.read_all()?
    };

    if entries.is_empty() {
        let prog = program_name();
        println!("{} No operations recorded yet", "ℹ".blue());
        println!("\nOperations will be logged as you use Diamond commands:");
        println!("  • {} sync, {} restack, {} move", prog, prog, prog);
        println!("  • {} create, {} delete, {} rename", prog, prog, prog);
        return Ok(());
    }

    println!(
        "{} Operation History (last {}):\n",
        "📜".blue(),
        if limit > 0 {
            format!("{} entries", entries.len())
        } else {
            "all".to_string()
        }
    );

    for entry in &entries {
        let timestamp = entry.timestamp.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S");

        let (icon, description) = match &entry.operation {
            Operation::BranchCreated { branch, parent } => (
                "✨",
                format!(
                    "Created branch {} (parent: {})",
                    branch.green(),
                    parent.as_ref().map(|p| p.as_str()).unwrap_or("none").yellow()
                ),
            ),
            Operation::BranchDeleted { branch } => ("🗑️", format!("Deleted branch {}", branch.red())),
            Operation::BranchMoved {
                branch,
                old_parent,
                new_parent,
            } => (
                "🔀",
                format!(
                    "Moved {} from {} to {}",
                    branch.green(),
                    old_parent.as_ref().map(|p| p.as_str()).unwrap_or("none").yellow(),
                    new_parent.as_ref().map(|p| p.as_str()).unwrap_or("none").yellow()
                ),
            ),
            Operation::BranchRenamed { old_name, new_name } => {
                ("✏️", format!("Renamed {} to {}", old_name.yellow(), new_name.green()))
            }
            Operation::SyncStarted { branches } => (
                "🔄",
                format!("Started sync ({} branches)", branches.len().to_string().yellow()),
            ),
            Operation::SyncCompleted { branches: _, success } => {
                if *success {
                    ("✅", "Sync completed successfully".green().to_string())
                } else {
                    ("❌", "Sync failed".red().to_string())
                }
            }
            Operation::RestackStarted { branches } => (
                "📚",
                format!("Started restack ({} branches)", branches.len().to_string().yellow()),
            ),
            Operation::RestackCompleted { branches: _, success } => {
                if *success {
                    ("✅", "Restack completed successfully".green().to_string())
                } else {
                    ("❌", "Restack failed".red().to_string())
                }
            }
            Operation::BackupCreated { branch, backup_ref } => {
                let ref_short = backup_ref.split('/').next_back().unwrap_or(backup_ref);
                ("💾", format!("Backed up {} ({})", branch.cyan(), ref_short))
            }
            Operation::BackupRestored { branch, backup_ref } => {
                let ref_short = backup_ref.split('/').next_back().unwrap_or(backup_ref);
                ("♻️", format!("Restored {} from {}", branch.green(), ref_short))
            }
        };

        println!("{} {} {}", timestamp.to_string().bright_black(), icon, description);

        if let Some(msg) = &entry.message {
            println!("    {}", msg.bright_black());
        }
    }

    println!();
    println!("{} To see all entries: dm history --all", "💡".blue());
    println!("{} To see last N entries: dm history --count N", "💡".blue());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use tempfile::tempdir;

    use crate::test_context::TestRepoContext;

    #[test]
    fn test_history_no_entries() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());
        fs::create_dir_all(dir.path().join(".git").join("diamond"))?;

        let result = run(Some(10));
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_history_with_entries() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());
        fs::create_dir_all(dir.path().join(".git").join("diamond"))?;

        // Create some log entries
        let log = OperationLog::new()?;
        log.log(crate::operation_log::LogEntry::new(Operation::BranchCreated {
            branch: "feature-1".to_string(),
            parent: Some("main".to_string()),
        }))?;

        log.log(crate::operation_log::LogEntry::new(Operation::SyncStarted {
            branches: vec!["feature-1".to_string()],
        }))?;

        // Should display without errors
        let result = run(Some(10));
        assert!(result.is_ok());

        Ok(())
    }
}
