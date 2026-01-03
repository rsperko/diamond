//! Backup ref operations for GitGateway.

use anyhow::{bail, Context, Result};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::GitGateway;

/// Atomic counter to ensure unique backup refs even within the same nanosecond
static BACKUP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Default backup retention period (30 days)
pub const DEFAULT_BACKUP_TTL_DAYS: u64 = 30;

/// Represents a backup reference for undo functionality.
///
/// Backup refs are stored under `refs/diamond/backup/<branch>-<timestamp>`
/// and can be used to restore branches after operations like restack or sync.
#[derive(Debug, Clone)]
pub struct BackupRef {
    /// Full ref name (e.g., "refs/diamond/backup/feature-1234567890")
    pub ref_name: String,
    /// The original branch name that was backed up
    pub branch_name: String,
    /// Unix timestamp when the backup was created
    pub timestamp: u64,
    /// The commit OID the branch pointed to at backup time (40-char hex string)
    pub commit_oid: String,
}

impl GitGateway {
    /// Create a backup ref before destructive operations
    ///
    /// Backup refs use nanosecond timestamps plus an atomic counter to ensure
    /// uniqueness even when multiple backups are created in rapid succession.
    /// This prevents data loss in fast CI environments or when operations
    /// complete faster than the clock resolution.
    pub fn create_backup_ref(&self, branch: &str) -> Result<BackupRef> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
        // Use nanoseconds for higher precision
        let nanos = now.as_nanos() as u64;
        // Add atomic counter to guarantee uniqueness even within same nanosecond
        let counter = BACKUP_COUNTER.fetch_add(1, Ordering::SeqCst);

        // Format: refs/diamond/backup/<branch>-<nanos>-<counter>
        let backup_ref_name = format!("refs/diamond/backup/{}-{}-{}", branch, nanos, counter);

        // Store seconds-precision timestamp in struct for display/age calculations
        let timestamp = now.as_secs();

        // Get commit SHA for the branch
        let commit_sha = self
            .backend
            .get_ref_sha(branch)
            .context(format!("Branch '{}' not found", branch))?
            .to_string();

        // Create the backup ref using update-ref
        let output = std::process::Command::new("git")
            .args(["update-ref", &backup_ref_name, &commit_sha])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to create backup ref")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to create backup ref: {}", stderr.trim());
        }

        Ok(BackupRef {
            ref_name: backup_ref_name,
            branch_name: branch.to_string(),
            timestamp,
            commit_oid: commit_sha,
        })
    }

    /// List all backup refs
    ///
    /// Supports both old format (branch-timestamp) and new format (branch-nanos-counter).
    pub fn list_backup_refs(&self) -> Result<Vec<BackupRef>> {
        let mut backups = Vec::new();

        let output = std::process::Command::new("git")
            .args([
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/diamond/backup/",
            ])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to list backup refs")?;

        if !output.status.success() {
            // No backup refs is not an error
            return Ok(backups);
        }

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0];
                let oid_str = parts[1];

                if let Some(suffix) = name.strip_prefix("refs/diamond/backup/") {
                    if let Some(parsed) = Self::parse_backup_ref_suffix(suffix) {
                        backups.push(BackupRef {
                            ref_name: name.to_string(),
                            branch_name: parsed.0,
                            timestamp: parsed.1,
                            commit_oid: oid_str.to_string(),
                        });
                    }
                }
            }
        }

        // Sort by timestamp (newest first)
        backups.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(backups)
    }

    /// Parse backup ref suffix to extract branch name and timestamp
    ///
    /// Handles two formats:
    /// - Old: "branch-name-1234567890" (seconds)
    /// - New: "branch-name-1234567890123456789-0" (nanos-counter)
    fn parse_backup_ref_suffix(suffix: &str) -> Option<(String, u64)> {
        // Find the last two dashes to check for new format
        let parts: Vec<&str> = suffix.rsplitn(3, '-').collect();

        match parts.len() {
            // New format: [counter, nanos, branch-name-parts...]
            3 if parts[0].parse::<u32>().is_ok() && parts[1].len() > 15 => {
                // New format with nanos (>15 digits) and counter
                let counter_ok = parts[0].parse::<u32>().is_ok();
                let nanos = parts[1].parse::<u64>().ok()?;
                if counter_ok {
                    let branch_name = parts[2].to_string();
                    // Convert nanos to seconds for display
                    let timestamp = nanos / 1_000_000_000;
                    Some((branch_name, timestamp))
                } else {
                    None
                }
            }
            // Old format or branch name with dashes: [timestamp, rest...]
            _ => {
                // Fall back to last-dash parsing for old format
                let last_dash_idx = suffix.rfind('-')?;
                let branch_name = &suffix[..last_dash_idx];
                let timestamp_str = &suffix[last_dash_idx + 1..];
                let timestamp = timestamp_str.parse::<u64>().ok()?;
                Some((branch_name.to_string(), timestamp))
            }
        }
    }

    /// Restore a branch from a backup ref
    pub fn restore_from_backup(&self, backup_ref: &BackupRef) -> Result<()> {
        let branch_name = &backup_ref.branch_name;
        let commit_sha = &backup_ref.commit_oid;

        let branch_ref = format!("refs/heads/{}", branch_name);

        // Check if branch exists
        let exists = self.backend.branch_exists(branch_name)?;

        if exists {
            // Update existing branch
            let output = std::process::Command::new("git")
                .args(["update-ref", &branch_ref, commit_sha])
                .current_dir(&self.workdir)
                .output()
                .context("Failed to restore branch")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("Failed to restore branch '{}': {}", branch_name, stderr.trim());
            }
        } else {
            // Create new branch at the commit
            let output = std::process::Command::new("git")
                .args(["branch", branch_name, commit_sha])
                .current_dir(&self.workdir)
                .output()
                .context(format!("Failed to restore branch '{}'", branch_name))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("Failed to restore branch '{}': {}", branch_name, stderr.trim());
            }
        }

        Ok(())
    }

    /// Delete a backup ref
    pub fn delete_backup_ref(&self, backup_ref: &BackupRef) -> Result<()> {
        let output = std::process::Command::new("git")
            .args(["update-ref", "-d", &backup_ref.ref_name])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to delete backup ref")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "not found" errors for idempotent behavior
            if !stderr.contains("not exist") && !stderr.contains("not found") && !stderr.contains("No such ref") {
                bail!(
                    "Failed to delete backup ref '{}': {}",
                    backup_ref.ref_name,
                    stderr.trim()
                );
            }
        }
        Ok(())
    }

    /// Clean up old backup refs (keep last N per branch)
    ///
    /// # Safety
    /// This method checks for in-progress operations before cleaning up.
    /// If an operation is in progress, cleanup is skipped to prevent
    /// accidentally deleting backups that may be needed for abort/recovery.
    pub fn cleanup_old_backups(&self, keep_per_branch: usize) -> Result<usize> {
        // Safety: Don't cleanup during operations - backups may be needed for abort
        if Self::is_operation_in_progress()? {
            return Ok(0);
        }

        let backups = self.list_backup_refs()?;
        let mut deleted_count = 0;

        // Group by branch name
        let mut by_branch: std::collections::HashMap<String, Vec<BackupRef>> = std::collections::HashMap::new();

        for backup in backups {
            by_branch.entry(backup.branch_name.clone()).or_default().push(backup);
        }

        // Delete old backups for each branch
        for (_branch, branch_backups) in by_branch {
            // Already sorted by timestamp (newest first) from list_backup_refs
            if branch_backups.len() > keep_per_branch {
                let mut branch_backups_mut = branch_backups;
                let to_delete = branch_backups_mut.split_off(keep_per_branch);
                for backup in to_delete {
                    self.delete_backup_ref(&backup)?;
                    deleted_count += 1;
                }
            }
        }

        Ok(deleted_count)
    }

    /// Clean up backup refs older than specified days
    ///
    /// This provides TTL-based cleanup to prevent unbounded repository growth.
    ///
    /// # Safety
    /// This method checks for in-progress operations before cleaning up.
    /// If an operation is in progress, cleanup is skipped to prevent
    /// accidentally deleting backups that may be needed for abort/recovery.
    pub fn cleanup_backups_by_age(&self, max_age_days: u64) -> Result<usize> {
        // Safety: Don't cleanup during operations - backups may be needed for abort
        if Self::is_operation_in_progress()? {
            return Ok(0);
        }

        let backups = self.list_backup_refs()?;
        let mut deleted_count = 0;

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let max_age_secs = Duration::from_secs(max_age_days * 24 * 60 * 60).as_secs();
        let cutoff = now.saturating_sub(max_age_secs);

        for backup in backups {
            if backup.timestamp < cutoff {
                self.delete_backup_ref(&backup)?;
                deleted_count += 1;
            }
        }

        Ok(deleted_count)
    }

    /// Check if there's an operation in progress that might need backup refs
    fn is_operation_in_progress() -> Result<bool> {
        use crate::state::OperationState;
        Ok(OperationState::load()?.is_some())
    }

    /// Comprehensive garbage collection for Diamond refs
    ///
    /// Cleans up:
    /// - Backup refs older than `max_age_days`
    /// - Excess backup refs beyond `keep_per_branch` per branch
    ///
    /// Returns (deleted_by_age, deleted_by_count)
    pub fn gc(&self, max_age_days: u64, keep_per_branch: usize) -> Result<(usize, usize)> {
        // First, clean by age (removes really old backups)
        let deleted_by_age = self.cleanup_backups_by_age(max_age_days)?;

        // Then, clean by count (keeps storage bounded per branch)
        let deleted_by_count = self.cleanup_old_backups(keep_per_branch)?;

        Ok((deleted_by_age, deleted_by_count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_backup_refs_have_unique_names() -> Result<()> {
        // CRITICAL TEST: Two backups created in rapid succession MUST have different names
        // This prevents data loss when operations run faster than 1-second resolution
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create a feature branch
        gateway.create_branch("feature-1")?;

        // Create two backups in rapid succession (same second)
        let backup1 = gateway.create_backup_ref("feature-1")?;
        let backup2 = gateway.create_backup_ref("feature-1")?;

        // CRITICAL: The ref names MUST be different
        assert_ne!(
            backup1.ref_name, backup2.ref_name,
            "Backup refs created in same second MUST have unique names to prevent data loss"
        );

        // Both backups should exist
        let backups = gateway.list_backup_refs()?;
        let feature_backups: Vec<_> = backups.iter().filter(|b| b.branch_name == "feature-1").collect();
        assert_eq!(feature_backups.len(), 2, "Both backup refs should exist");

        Ok(())
    }

    #[test]
    fn test_backup_ref_parsing_with_suffix() -> Result<()> {
        // Verify that backup refs with suffixes can be parsed correctly
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        gateway.create_branch("feature-1")?;

        // Create backup
        let backup = gateway.create_backup_ref("feature-1")?;

        // List and verify parsing
        let backups = gateway.list_backup_refs()?;
        let found = backups.iter().find(|b| b.ref_name == backup.ref_name);

        assert!(found.is_some(), "Should find the created backup");
        let found = found.unwrap();
        assert_eq!(found.branch_name, "feature-1");
        assert_eq!(found.commit_oid, backup.commit_oid);

        Ok(())
    }

    #[test]
    fn test_parse_backup_ref_suffix_old_format() {
        // Old format: branch-timestamp (seconds)
        let result = GitGateway::parse_backup_ref_suffix("feature-1-1735555200");
        assert!(result.is_some());
        let (branch, timestamp) = result.unwrap();
        assert_eq!(branch, "feature-1");
        assert_eq!(timestamp, 1735555200);
    }

    #[test]
    fn test_parse_backup_ref_suffix_old_format_with_dashes() {
        // Old format with branch name containing dashes
        let result = GitGateway::parse_backup_ref_suffix("my-feature-branch-1735555200");
        assert!(result.is_some());
        let (branch, timestamp) = result.unwrap();
        assert_eq!(branch, "my-feature-branch");
        assert_eq!(timestamp, 1735555200);
    }

    #[test]
    fn test_parse_backup_ref_suffix_new_format() {
        // New format: branch-nanos-counter
        let result = GitGateway::parse_backup_ref_suffix("feature-1-1735555200123456789-0");
        assert!(result.is_some());
        let (branch, timestamp) = result.unwrap();
        assert_eq!(branch, "feature-1");
        // Nanos converted to seconds
        assert_eq!(timestamp, 1735555200);
    }

    #[test]
    fn test_parse_backup_ref_suffix_new_format_with_dashes() {
        // New format with branch name containing dashes
        let result = GitGateway::parse_backup_ref_suffix("my-feature-branch-1735555200123456789-42");
        assert!(result.is_some());
        let (branch, timestamp) = result.unwrap();
        assert_eq!(branch, "my-feature-branch");
        assert_eq!(timestamp, 1735555200);
    }

    #[test]
    fn test_backup_cleanup_works_with_new_format() -> Result<()> {
        // Verify that cleanup/gc works with new format
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        gateway.create_branch("feature-1")?;

        // Create multiple backups
        gateway.create_backup_ref("feature-1")?;
        gateway.create_backup_ref("feature-1")?;
        gateway.create_backup_ref("feature-1")?;

        // Should have 3 backups
        let backups = gateway.list_backup_refs()?;
        assert_eq!(backups.len(), 3);

        // Cleanup to keep only 1
        let deleted = gateway.cleanup_old_backups(1)?;
        assert_eq!(deleted, 2);

        // Should have 1 backup left
        let backups = gateway.list_backup_refs()?;
        assert_eq!(backups.len(), 1);

        Ok(())
    }
}
