//! Garbage collection for Diamond refs.
//!
//! Cleans up old backup refs to prevent repository bloat.

use crate::git_gateway::backup::DEFAULT_BACKUP_TTL_DAYS;
use crate::git_gateway::GitGateway;
use anyhow::Result;
use colored::Colorize;

/// Default number of backups to keep per branch
const DEFAULT_KEEP_PER_BRANCH: usize = 10;

/// Run garbage collection on Diamond refs
pub fn run(max_age_days: Option<u64>, keep_per_branch: Option<usize>, dry_run: bool) -> Result<()> {
    let gateway = GitGateway::new()?;

    let max_age = max_age_days.unwrap_or(DEFAULT_BACKUP_TTL_DAYS);
    let keep = keep_per_branch.unwrap_or(DEFAULT_KEEP_PER_BRANCH);

    if dry_run {
        println!("{} Dry run - showing what would be deleted:\n", "🔍".blue());
        run_dry(gateway, max_age, keep)
    } else {
        run_gc(gateway, max_age, keep)
    }
}

fn run_gc(gateway: GitGateway, max_age_days: u64, keep_per_branch: usize) -> Result<()> {
    println!("{} Running garbage collection...\n", "🗑".blue());

    let (deleted_by_age, deleted_by_count) = gateway.gc(max_age_days, keep_per_branch)?;

    let total = deleted_by_age + deleted_by_count;

    if total == 0 {
        println!("{} No backup refs to clean up.", "✓".green().bold());
    } else {
        println!(
            "{} Cleaned up {} backup ref{}:",
            "✓".green().bold(),
            total,
            if total == 1 { "" } else { "s" }
        );
        if deleted_by_age > 0 {
            println!("  • {} older than {} days", deleted_by_age, max_age_days);
        }
        if deleted_by_count > 0 {
            println!(
                "  • {} exceeding {} per branch limit",
                deleted_by_count, keep_per_branch
            );
        }
    }

    Ok(())
}

fn run_dry(gateway: GitGateway, max_age_days: u64, keep_per_branch: usize) -> Result<()> {
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    let backups = gateway.list_backup_refs()?;

    if backups.is_empty() {
        println!("{} No backup refs found.", "ℹ".blue());
        return Ok(());
    }

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let max_age_secs = max_age_days * 24 * 60 * 60;
    let cutoff = now.saturating_sub(max_age_secs);

    // Count what would be deleted by age
    let mut would_delete_by_age = 0;
    let mut remaining_after_age = Vec::new();

    for backup in &backups {
        if backup.timestamp < cutoff {
            would_delete_by_age += 1;
            let age_days = (now - backup.timestamp) / (24 * 60 * 60);
            println!("  {} {} ({} days old)", "×".red(), backup.ref_name.dimmed(), age_days);
        } else {
            remaining_after_age.push(backup);
        }
    }

    // Count what would be deleted by count (from remaining)
    let mut by_branch: HashMap<String, Vec<_>> = HashMap::new();
    for backup in remaining_after_age {
        by_branch.entry(backup.branch_name.clone()).or_default().push(backup);
    }

    let mut would_delete_by_count = 0;
    for (_branch, branch_backups) in by_branch {
        if branch_backups.len() > keep_per_branch {
            let excess = branch_backups.len() - keep_per_branch;
            would_delete_by_count += excess;
            // Show excess refs (they're already sorted newest first)
            for backup in branch_backups.iter().skip(keep_per_branch) {
                println!(
                    "  {} {} (excess for {})",
                    "×".red(),
                    backup.ref_name.dimmed(),
                    backup.branch_name
                );
            }
        }
    }

    let total = would_delete_by_age + would_delete_by_count;

    println!();
    if total == 0 {
        println!("{} Nothing to clean up.", "✓".green().bold());
    } else {
        println!(
            "{} Would delete {} backup ref{}",
            "ℹ".blue(),
            total,
            if total == 1 { "" } else { "s" }
        );
        println!("\nRun without --dry-run to actually delete.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_gc_no_backups() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());
        init_test_repo(dir.path())?;

        // Should succeed with no backups
        let result = run(None, None, false);
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_gc_dry_run() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());
        init_test_repo(dir.path())?;

        // Create a feature branch and backup ref
        let gateway = GitGateway::new()?;
        gateway.create_branch("feature")?;
        gateway.create_backup_ref("feature")?;

        // Dry run should succeed and not delete anything
        let result = run(None, None, true);
        assert!(result.is_ok());

        // Backup should still exist
        let backups = gateway.list_backup_refs()?;
        assert_eq!(backups.len(), 1);

        Ok(())
    }

    #[test]
    fn test_gc_deletes_old_backups() -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());
        init_test_repo(dir.path())?;

        let gateway = GitGateway::new()?;

        // Create a feature branch
        gateway.create_branch("feature")?;

        // Get commit OID for creating refs
        let commit_oid = gateway.resolve_to_oid("HEAD")?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        // Create an old backup (60 days ago)
        let sixty_days_ago = now - (60 * 24 * 60 * 60);
        gateway.create_reference(
            &format!("refs/diamond/backup/feature-{}", sixty_days_ago),
            &commit_oid,
            false,
            "Old backup",
        )?;

        // Create a recent backup
        gateway.create_backup_ref("feature")?;

        // Verify 2 backups exist
        assert_eq!(gateway.list_backup_refs()?.len(), 2);

        // Run gc with default settings (30 day max age)
        let result = run(Some(30), None, false);
        assert!(result.is_ok());

        // Only 1 backup should remain (the recent one)
        let backups = gateway.list_backup_refs()?;
        assert_eq!(backups.len(), 1);
        assert!(backups[0].timestamp >= now - 60); // Recent backup

        Ok(())
    }

    #[test]
    fn test_gc_respects_keep_count() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());
        init_test_repo(dir.path())?;

        let gateway = GitGateway::new()?;

        // Create a feature branch
        gateway.create_branch("feature")?;
        gateway.checkout_branch_worktree_safe("feature")?;

        // Create 5 recent backups
        for i in 1..=5 {
            std::fs::write(dir.path().join(format!("test{}.txt", i)), "data")?;
            gateway.stage_all()?;
            gateway.commit(&format!("Commit {}", i))?;
            gateway.create_backup_ref("feature")?;
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        // Verify 5 backups exist
        assert_eq!(gateway.list_backup_refs()?.len(), 5);

        // Run gc keeping only 2 per branch
        let result = run(Some(365), Some(2), false); // Long max age so only count matters
        assert!(result.is_ok());

        // Only 2 backups should remain
        assert_eq!(gateway.list_backup_refs()?.len(), 2);

        Ok(())
    }
}
