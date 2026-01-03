use crate::git_gateway::{BackupRef, GitGateway};
use crate::operation_log::{Operation, OperationLog, OperationRecorder};
use crate::program_name::program_name;
use anyhow::{bail, Result};
use chrono::{DateTime, Local, Utc};
use colored::Colorize;
use std::io::{IsTerminal, Write};

/// List and restore from backup refs
pub fn run(branch: Option<String>, list_all: bool, force: bool) -> Result<()> {
    let gateway = GitGateway::new()?;

    if list_all {
        list_backups(&gateway)?;
        return Ok(());
    }

    if let Some(branch_name) = branch {
        restore_branch(&gateway, &branch_name)?;
    } else {
        // Undo last operation
        undo_last_operation(&gateway, force)?;
    }

    Ok(())
}

fn list_backups(gateway: &GitGateway) -> Result<()> {
    let backups = gateway.list_backup_refs()?;

    if backups.is_empty() {
        println!("{} No backup refs found", "ℹ".blue());
        println!("\nBackups are created automatically before destructive operations.");
        println!(
            "Run '{} sync' or '{} restack' to create backups.",
            program_name(),
            program_name()
        );
        return Ok(());
    }

    println!("{} Available backups:\n", "📦".blue());

    // Group by branch
    let mut by_branch: std::collections::HashMap<String, Vec<BackupRef>> = std::collections::HashMap::new();

    for backup in backups {
        by_branch.entry(backup.branch_name.clone()).or_default().push(backup);
    }

    for (branch, mut backups) in by_branch {
        println!("{}", branch.cyan().bold());

        // Sort by timestamp (newest first)
        backups.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        for (i, backup) in backups.iter().enumerate() {
            let timestamp = DateTime::from_timestamp(backup.timestamp as i64, 0)
                .map(|dt| dt.with_timezone(&Local))
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "Unknown time".to_string());

            let commit_short = &backup.commit_oid.to_string()[..7];

            if i == 0 {
                println!(
                    "  {} {} @ {} ({})",
                    "→".green(),
                    timestamp.green(),
                    commit_short.yellow(),
                    "latest".green()
                );
            } else {
                println!("    {} @ {}", timestamp, commit_short.yellow());
            }
        }
        println!();
    }

    println!("{} To restore: dm undo <branch>", "💡".blue());

    Ok(())
}

fn restore_branch(gateway: &GitGateway, branch_name: &str) -> Result<()> {
    let backups = gateway.list_backup_refs()?;

    // Find backups for this branch
    let branch_backups: Vec<BackupRef> = backups.into_iter().filter(|b| b.branch_name == branch_name).collect();

    if branch_backups.is_empty() {
        bail!("No backups found for branch '{}'", branch_name);
    }

    // Get latest backup (safe because we checked is_empty above)
    let latest = branch_backups
        .iter()
        .max_by_key(|b| b.timestamp)
        .ok_or_else(|| anyhow::anyhow!("No backups found for branch '{}'", branch_name))?;

    let timestamp = DateTime::from_timestamp(latest.timestamp as i64, 0)
        .map(|dt| dt.with_timezone(&Local))
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "Unknown time".to_string());

    let commit_short = &latest.commit_oid.to_string()[..7];

    println!("{} Restoring '{}' from backup:", "🔄".blue(), branch_name.cyan());
    println!("  Time:   {}", timestamp.green());
    println!("  Commit: {}", commit_short.yellow());

    // Perform restore
    gateway.restore_from_backup(latest)?;

    println!(
        "\n{} Branch '{}' restored successfully!",
        "✓".green().bold(),
        branch_name.cyan()
    );
    println!("\n{} The backup ref has been kept for future use.", "ℹ".blue());
    println!("  To remove old backups: dm doctor --cleanup");

    Ok(())
}

/// Undo the last undoable operation (sync or restack)
fn undo_last_operation(gateway: &GitGateway, force: bool) -> Result<()> {
    let log = OperationLog::new()?;
    let operation = log.get_last_undoable_operation()?;

    let Some(op) = operation else {
        println!("{} No undoable operations found", "ℹ".blue());
        println!("\nOperations that support undo: sync, restack");
        println!("These create backup refs before making changes.");
        return Ok(());
    };

    // Find backups for these branches
    let backups = gateway.list_backup_refs()?;
    let op_timestamp = op.timestamp.timestamp() as u64;

    // Match backups created just before/at operation time (within 60s tolerance)
    let matching_backups: Vec<_> = op
        .branches
        .iter()
        .filter_map(|branch| {
            backups
                .iter()
                .filter(|b| b.branch_name == *branch)
                .filter(|b| b.timestamp <= op_timestamp && op_timestamp - b.timestamp < 60)
                .max_by_key(|b| b.timestamp)
                .cloned()
        })
        .collect();

    if matching_backups.is_empty() {
        bail!("No backups found for the last {} operation", op.operation_type);
    }

    // Show what we're about to do
    let time_ago = format_time_ago(op.timestamp);
    println!(
        "{} Found last operation: {} ({})",
        "🔄".blue(),
        op.operation_type.cyan(),
        time_ago
    );
    println!("   Affected branches:");
    for backup in &matching_backups {
        println!(
            "     {} @ {}",
            backup.branch_name.cyan(),
            &backup.commit_oid.to_string()[..7].yellow()
        );
    }

    // Confirm unless --force
    if !force {
        if !std::io::stdin().is_terminal() {
            bail!("This command requires confirmation. Use --force to skip.");
        }
        print!("\nRestore all branches to their pre-operation state? [y/N] ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Restore all branches and log each restoration (for chain undo support)
    let recorder = OperationRecorder::new()?;
    println!();
    for backup in &matching_backups {
        gateway.restore_from_backup(backup)?;

        // Log the restoration so chain undo knows this operation was undone
        recorder.record(Operation::BackupRestored {
            branch: backup.branch_name.clone(),
            backup_ref: backup.ref_name.clone(),
        })?;

        println!(
            "{} Restored {} @ {}",
            "✓".green(),
            backup.branch_name.cyan(),
            &backup.commit_oid.to_string()[..7].yellow()
        );
    }

    println!(
        "\n{} Done! {} branch(es) restored.",
        "✓".green().bold(),
        matching_backups.len()
    );
    Ok(())
}

/// Format a timestamp as a human-readable "time ago" string
fn format_time_ago(timestamp: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(timestamp);

    let seconds = duration.num_seconds();
    if seconds < 60 {
        return format!("{} seconds ago", seconds);
    }

    let minutes = duration.num_minutes();
    if minutes < 60 {
        return format!("{} minute{} ago", minutes, if minutes == 1 { "" } else { "s" });
    }

    let hours = duration.num_hours();
    if hours < 24 {
        return format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" });
    }

    let days = duration.num_days();
    format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::TestRepoContext;

    #[test]
    fn test_undo_no_backups() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());

        // Initialize git repo
        let repo = git2::Repository::init(dir.path())?;
        let sig = git2::Signature::now("Test", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial", &tree, &[])?;

        // Should handle no backups gracefully (list mode)
        let result = run(None, true, false);
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_undo_no_operations() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());

        // Initialize git repo
        let repo = git2::Repository::init(dir.path())?;
        let sig = git2::Signature::now("Test", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial", &tree, &[])?;

        // Should handle no operations gracefully (with --force to skip prompt)
        let result = run(None, false, true);
        assert!(result.is_ok());

        Ok(())
    }
}
