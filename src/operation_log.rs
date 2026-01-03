use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

/// Maximum number of log entries to keep before rotation
const MAX_LOG_ENTRIES: usize = 1000;

/// Types of operations that can be logged
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Operation {
    /// Branch was created
    BranchCreated { branch: String, parent: Option<String> },
    /// Branch was deleted
    BranchDeleted { branch: String },
    /// Branch was moved to new parent
    BranchMoved {
        branch: String,
        old_parent: Option<String>,
        new_parent: Option<String>,
    },
    /// Branch was renamed
    BranchRenamed { old_name: String, new_name: String },
    /// Sync operation started
    SyncStarted { branches: Vec<String> },
    /// Sync operation completed
    SyncCompleted { branches: Vec<String>, success: bool },
    /// Restack operation started
    RestackStarted { branches: Vec<String> },
    /// Restack operation completed
    RestackCompleted { branches: Vec<String>, success: bool },
    /// Backup ref created
    BackupCreated { branch: String, backup_ref: String },
    /// Backup ref restored
    BackupRestored { branch: String, backup_ref: String },
}

/// A log entry with timestamp and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Timestamp when operation occurred
    pub timestamp: DateTime<Utc>,
    /// The operation that was performed
    pub operation: Operation,
    /// Optional user message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl LogEntry {
    /// Create a new log entry with current timestamp
    pub fn new(operation: Operation) -> Self {
        Self {
            timestamp: Utc::now(),
            operation,
            message: None,
        }
    }

    /// Create a new log entry with a message
    pub fn with_message(operation: Operation, message: String) -> Self {
        Self {
            timestamp: Utc::now(),
            operation,
            message: Some(message),
        }
    }
}

/// Manages the operation log stored in .git/diamond/operations.jsonl
pub struct OperationLog {
    log_path: PathBuf,
}

impl OperationLog {
    /// Create a new operation log
    pub fn new() -> Result<Self> {
        let repo_root = crate::state::find_git_root()?;
        let diamond_dir = repo_root.join(".git").join("diamond");

        if !diamond_dir.exists() {
            fs::create_dir_all(&diamond_dir)?;
        }

        let log_path = diamond_dir.join("operations.jsonl");

        Ok(Self { log_path })
    }

    /// Create operation log from a specific path (for testing)
    #[cfg(test)]
    pub fn from_path(path: PathBuf) -> Self {
        Self { log_path: path }
    }

    /// Append a log entry to the operation log
    pub fn log(&self, entry: LogEntry) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .context("Failed to open operation log")?;

        let json = serde_json::to_string(&entry).context("Failed to serialize log entry")?;

        writeln!(file, "{}", json).context("Failed to write to operation log")?;

        // Periodically trim log to prevent unbounded growth
        // We check every ~100 entries to avoid overhead on every write
        self.maybe_trim_log()?;

        Ok(())
    }

    /// Trim the log if it exceeds MAX_LOG_ENTRIES
    fn maybe_trim_log(&self) -> Result<()> {
        if !self.log_path.exists() {
            return Ok(());
        }

        // Quick check: count lines to see if trimming is needed
        let file = File::open(&self.log_path).context("Failed to open log for trimming check")?;
        let reader = BufReader::new(file);
        let line_count = reader.lines().count();

        // Only trim if we exceed the max by a margin (to avoid trimming on every write)
        if line_count <= MAX_LOG_ENTRIES + 100 {
            return Ok(());
        }

        // Read all entries
        let entries = self.read_all()?;
        if entries.len() <= MAX_LOG_ENTRIES {
            return Ok(());
        }

        // Keep only the last MAX_LOG_ENTRIES
        let to_keep = &entries[entries.len() - MAX_LOG_ENTRIES..];

        // Write back (atomic: write to temp file, then rename)
        let temp_path = self.log_path.with_extension("jsonl.tmp");
        {
            let mut file = File::create(&temp_path).context("Failed to create temp log file")?;
            for entry in to_keep {
                let json = serde_json::to_string(entry)?;
                writeln!(file, "{}", json)?;
            }
        }

        fs::rename(&temp_path, &self.log_path).context("Failed to rotate log file")?;

        Ok(())
    }

    /// Read all log entries
    #[allow(dead_code)] // Will be used in dm doctor command
    pub fn read_all(&self) -> Result<Vec<LogEntry>> {
        if !self.log_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.log_path).context("Failed to open operation log")?;
        let reader = BufReader::new(file);

        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line.context("Failed to read line from log")?;
            if line.trim().is_empty() {
                continue;
            }

            let entry: LogEntry = serde_json::from_str(&line).context("Failed to parse log entry")?;
            entries.push(entry);
        }

        Ok(entries)
    }

    /// Read the last N log entries
    ///
    /// # Performance Note
    ///
    /// TODO(perf): Currently loads entire log into memory before taking last N.
    /// For very large logs (100K+ entries), this could use ~20MB memory.
    /// Consider streaming from end of file if this becomes an issue.
    /// See: agent_notes/code_review_20260102/code_quality.md Issue #4
    #[allow(dead_code)] // Will be used in dm doctor command
    pub fn read_last(&self, n: usize) -> Result<Vec<LogEntry>> {
        let all_entries = self.read_all()?;
        let start = all_entries.len().saturating_sub(n);
        Ok(all_entries[start..].to_vec())
    }

    /// Clear the operation log (use with caution!)
    #[allow(dead_code)]
    pub fn clear(&self) -> Result<()> {
        if self.log_path.exists() {
            fs::remove_file(&self.log_path).context("Failed to clear operation log")?;
        }
        Ok(())
    }

    /// Find the most recent undoable operation that hasn't been undone yet.
    ///
    /// We look for SyncStarted/RestackStarted rather than Completed because
    /// the Completed events may have empty branch lists (branches are removed
    /// from remaining_branches as they're processed).
    ///
    /// Chain undo support: Skips operations where all branches have been
    /// restored (via BackupRestored) since the operation occurred.
    pub fn get_last_undoable_operation(&self) -> Result<Option<UndoableOperation>> {
        let entries = self.read_all()?;

        // Scan from newest to oldest
        for (i, entry) in entries.iter().enumerate().rev() {
            let (op_type, branches) = match &entry.operation {
                Operation::SyncStarted { branches } if !branches.is_empty() => ("sync", branches),
                Operation::RestackStarted { branches } if !branches.is_empty() => ("restack", branches),
                _ => continue,
            };

            // Check if there's a corresponding Completed event after this Started event
            // If there's no Completed, the operation might still be in progress
            let has_completed = entries[i + 1..].iter().any(|later| {
                matches!(
                    &later.operation,
                    Operation::SyncCompleted { .. } | Operation::RestackCompleted { .. }
                )
            });

            if !has_completed {
                // Operation might still be in progress, skip it
                continue;
            }

            // Check if this operation has been undone
            // (all branches restored after this operation's timestamp)
            let op_timestamp = entry.timestamp;
            let all_restored = branches.iter().all(|branch| {
                // Look for BackupRestored for this branch AFTER this operation
                entries[i + 1..].iter().any(|later| {
                    matches!(
                        &later.operation,
                        Operation::BackupRestored { branch: b, .. }
                            if b == branch && later.timestamp > op_timestamp
                    )
                })
            });

            if !all_restored {
                return Ok(Some(UndoableOperation {
                    operation_type: op_type.to_string(),
                    branches: branches.clone(),
                    timestamp: entry.timestamp,
                }));
            }
        }
        Ok(None)
    }
}

/// Information about an undoable operation
#[derive(Debug, Clone)]
pub struct UndoableOperation {
    /// Type of operation ("sync" or "restack")
    pub operation_type: String,
    /// Branches affected by this operation
    pub branches: Vec<String>,
    /// When the operation occurred
    pub timestamp: DateTime<Utc>,
}

/// Helper for recording operations
pub struct OperationRecorder {
    log: OperationLog,
}

impl OperationRecorder {
    /// Create a new operation recorder
    #[allow(dead_code)] // Will be used when integrating with commands
    pub fn new() -> Result<Self> {
        Ok(Self {
            log: OperationLog::new()?,
        })
    }

    /// Record an operation
    #[allow(dead_code)] // Will be used when integrating with commands
    pub fn record(&self, operation: Operation) -> Result<()> {
        self.log.log(LogEntry::new(operation))
    }

    /// Record an operation with a message
    #[allow(dead_code)] // Will be used when integrating with commands
    pub fn record_with_message(&self, operation: Operation, message: String) -> Result<()> {
        self.log.log(LogEntry::with_message(operation, message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_log_entry_creation() {
        let entry = LogEntry::new(Operation::BranchCreated {
            branch: "test".to_string(),
            parent: Some("main".to_string()),
        });

        assert!(matches!(entry.operation, Operation::BranchCreated { .. }));
        assert!(entry.message.is_none());
    }

    #[test]
    fn test_log_entry_with_message() {
        let entry = LogEntry::with_message(
            Operation::BranchDeleted {
                branch: "test".to_string(),
            },
            "Deleted test branch".to_string(),
        );

        assert!(matches!(entry.operation, Operation::BranchDeleted { .. }));
        assert_eq!(entry.message, Some("Deleted test branch".to_string()));
    }

    #[test]
    fn test_operation_log_append() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path);

        let entry1 = LogEntry::new(Operation::BranchCreated {
            branch: "feature-1".to_string(),
            parent: Some("main".to_string()),
        });

        let entry2 = LogEntry::new(Operation::BranchCreated {
            branch: "feature-2".to_string(),
            parent: Some("main".to_string()),
        });

        log.log(entry1)?;
        log.log(entry2)?;

        let entries = log.read_all()?;
        assert_eq!(entries.len(), 2);

        Ok(())
    }

    #[test]
    fn test_operation_log_read_last() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path);

        // Create 5 entries
        for i in 1..=5 {
            log.log(LogEntry::new(Operation::BranchCreated {
                branch: format!("feature-{}", i),
                parent: Some("main".to_string()),
            }))?;
        }

        // Read last 3
        let last_3 = log.read_last(3)?;
        assert_eq!(last_3.len(), 3);

        // Should be feature-3, feature-4, feature-5
        if let Operation::BranchCreated { branch, .. } = &last_3[0].operation {
            assert_eq!(branch, "feature-3");
        } else {
            panic!("Expected BranchCreated operation");
        }

        Ok(())
    }

    #[test]
    fn test_operation_serialization() -> Result<()> {
        let operation = Operation::BranchMoved {
            branch: "feature".to_string(),
            old_parent: Some("main".to_string()),
            new_parent: Some("develop".to_string()),
        };

        let entry = LogEntry::new(operation);
        let json = serde_json::to_string(&entry)?;

        // Should be able to deserialize back
        let deserialized: LogEntry = serde_json::from_str(&json)?;
        assert!(matches!(deserialized.operation, Operation::BranchMoved { .. }));

        Ok(())
    }

    #[test]
    fn test_sync_operations() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path);

        let branches = vec!["b1".to_string(), "b2".to_string(), "b3".to_string()];

        log.log(LogEntry::new(Operation::SyncStarted {
            branches: branches.clone(),
        }))?;

        log.log(LogEntry::new(Operation::SyncCompleted {
            branches: branches.clone(),
            success: true,
        }))?;

        let entries = log.read_all()?;
        assert_eq!(entries.len(), 2);

        assert!(matches!(entries[0].operation, Operation::SyncStarted { .. }));
        assert!(matches!(entries[1].operation, Operation::SyncCompleted { .. }));

        Ok(())
    }

    #[test]
    fn test_backup_operations() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path);

        log.log(LogEntry::new(Operation::BackupCreated {
            branch: "feature".to_string(),
            backup_ref: "refs/diamond/backup/feature-12345".to_string(),
        }))?;

        log.log(LogEntry::new(Operation::BackupRestored {
            branch: "feature".to_string(),
            backup_ref: "refs/diamond/backup/feature-12345".to_string(),
        }))?;

        let entries = log.read_all()?;
        assert_eq!(entries.len(), 2);

        Ok(())
    }

    #[test]
    fn test_empty_log() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path);

        let entries = log.read_all()?;
        assert_eq!(entries.len(), 0);

        let last_10 = log.read_last(10)?;
        assert_eq!(last_10.len(), 0);

        Ok(())
    }

    #[test]
    fn test_operation_recorder() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path.clone());

        let recorder = OperationRecorder { log };

        recorder.record(Operation::BranchCreated {
            branch: "test".to_string(),
            parent: None,
        })?;

        recorder.record_with_message(
            Operation::BranchDeleted {
                branch: "test".to_string(),
            },
            "Cleanup".to_string(),
        )?;

        let log = OperationLog::from_path(log_path);
        let entries = log.read_all()?;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].message, Some("Cleanup".to_string()));

        Ok(())
    }

    #[test]
    fn test_log_rotation() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path);

        // Create more entries than MAX_LOG_ENTRIES + 100 (the trim threshold)
        // We add 250 extra to ensure we definitely trigger trimming
        let num_entries = super::MAX_LOG_ENTRIES + 250;
        for i in 0..num_entries {
            log.log(LogEntry::new(Operation::BranchCreated {
                branch: format!("branch-{}", i),
                parent: Some("main".to_string()),
            }))?;
        }

        // After trimming, should have at most MAX_LOG_ENTRIES + 100
        // (trimming happens when we exceed MAX_LOG_ENTRIES + 100, leaving MAX_LOG_ENTRIES)
        // After trim, we continue adding until we hit threshold again
        let entries = log.read_all()?;
        assert!(
            entries.len() <= super::MAX_LOG_ENTRIES + 100,
            "Expected at most {} entries, got {}",
            super::MAX_LOG_ENTRIES + 100,
            entries.len()
        );

        // Verify that old entries were removed - the first entry should NOT be branch-0
        if let Operation::BranchCreated { branch, .. } = &entries[0].operation {
            let branch_num: usize = branch.strip_prefix("branch-").unwrap().parse().unwrap();
            assert!(
                branch_num > 0,
                "Expected oldest entries to be trimmed, but branch-0 is still present"
            );
        }

        Ok(())
    }

    #[test]
    fn test_get_last_undoable_operation_sync() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path);

        // Log a sync operation (Started + Completed)
        log.log(LogEntry::new(Operation::SyncStarted {
            branches: vec!["feature-1".to_string(), "feature-2".to_string()],
        }))?;
        log.log(LogEntry::new(Operation::SyncCompleted {
            branches: vec![], // Empty after completion
            success: true,
        }))?;

        // Should find this operation using the Started event
        let result = log.get_last_undoable_operation()?;
        assert!(result.is_some());

        let op = result.unwrap();
        assert_eq!(op.operation_type, "sync");
        assert_eq!(op.branches, vec!["feature-1", "feature-2"]);

        Ok(())
    }

    #[test]
    fn test_get_last_undoable_operation_empty() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path);

        // Empty log should return None
        let result = log.get_last_undoable_operation()?;
        assert!(result.is_none());

        Ok(())
    }

    #[test]
    fn test_get_last_undoable_operation_skips_in_progress() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path);

        // Log only SyncStarted without Completed - operation in progress
        log.log(LogEntry::new(Operation::SyncStarted {
            branches: vec!["feature-1".to_string()],
        }))?;

        // Should return None since operation is not complete
        let result = log.get_last_undoable_operation()?;
        assert!(result.is_none());

        Ok(())
    }

    #[test]
    fn test_get_last_undoable_operation_chain_undo() -> Result<()> {
        let dir = tempdir()?;
        let log_path = dir.path().join("operations.jsonl");
        let log = OperationLog::from_path(log_path);

        // 1. Log Sync for branches [a, b]
        log.log(LogEntry::new(Operation::SyncStarted {
            branches: vec!["a".to_string(), "b".to_string()],
        }))?;
        log.log(LogEntry::new(Operation::SyncCompleted {
            branches: vec![],
            success: true,
        }))?;

        // 2. Log Restack for branch [c]
        log.log(LogEntry::new(Operation::RestackStarted {
            branches: vec!["c".to_string()],
        }))?;
        log.log(LogEntry::new(Operation::RestackCompleted {
            branches: vec![],
            success: true,
        }))?;

        // 3. First call returns restack (most recent)
        let result = log.get_last_undoable_operation()?;
        assert!(result.is_some());
        let op = result.unwrap();
        assert_eq!(op.operation_type, "restack");
        assert_eq!(op.branches, vec!["c"]);

        // 4. Log BackupRestored for branch c (simulating undo)
        log.log(LogEntry::new(Operation::BackupRestored {
            branch: "c".to_string(),
            backup_ref: "refs/diamond/backup/c-12345".to_string(),
        }))?;

        // 5. Second call returns sync (restack was undone)
        let result = log.get_last_undoable_operation()?;
        assert!(result.is_some());
        let op = result.unwrap();
        assert_eq!(op.operation_type, "sync");
        assert_eq!(op.branches, vec!["a", "b"]);

        // 6. Log BackupRestored for branches a, b (simulating undo)
        log.log(LogEntry::new(Operation::BackupRestored {
            branch: "a".to_string(),
            backup_ref: "refs/diamond/backup/a-12345".to_string(),
        }))?;
        log.log(LogEntry::new(Operation::BackupRestored {
            branch: "b".to_string(),
            backup_ref: "refs/diamond/backup/b-12345".to_string(),
        }))?;

        // 7. Third call returns None (all undone)
        let result = log.get_last_undoable_operation()?;
        assert!(result.is_none());

        Ok(())
    }
}
