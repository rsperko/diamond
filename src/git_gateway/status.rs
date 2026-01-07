//! Git status parsing, particularly for conflict detection.

use anyhow::{Context, Result};
use std::fmt;

use super::GitGateway;

/// A file in conflict state during a rebase/merge
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictedFile {
    pub path: String,
    pub conflict_type: ConflictType,
}

/// Type of merge conflict
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictType {
    /// Both modified (UU)
    BothModified,
    /// Both added (AA)
    BothAdded,
    /// Deleted by us (DU)
    DeletedByUs,
    /// Deleted by them (UD)
    DeletedByThem,
    /// Added by us (AU)
    AddedByUs,
    /// Added by them (UA)
    AddedByThem,
}

impl fmt::Display for ConflictType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConflictType::BothModified => write!(f, "both modified"),
            ConflictType::BothAdded => write!(f, "both added"),
            ConflictType::DeletedByUs => write!(f, "deleted by us"),
            ConflictType::DeletedByThem => write!(f, "deleted by them"),
            ConflictType::AddedByUs => write!(f, "added by us"),
            ConflictType::AddedByThem => write!(f, "added by them"),
        }
    }
}

impl GitGateway {
    /// Get list of files currently in conflict state.
    ///
    /// Runs `git status --porcelain` and parses conflict markers.
    /// Returns empty vec if no conflicts exist.
    pub fn get_conflicted_files(&self) -> Result<Vec<ConflictedFile>> {
        let output = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git status")?;

        if !output.status.success() {
            anyhow::bail!("git status failed");
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        let mut conflicts = Vec::new();
        for line in stdout.lines() {
            if line.len() < 3 {
                continue;
            }

            // Format: "XY path" where X and Y are status codes
            let x = line.chars().nth(0).unwrap();
            let y = line.chars().nth(1).unwrap();
            let path = line[3..].to_string();

            let conflict_type = match (x, y) {
                ('U', 'U') => Some(ConflictType::BothModified),
                ('A', 'A') => Some(ConflictType::BothAdded),
                ('D', 'U') => Some(ConflictType::DeletedByUs),
                ('U', 'D') => Some(ConflictType::DeletedByThem),
                ('A', 'U') => Some(ConflictType::AddedByUs),
                ('U', 'A') => Some(ConflictType::AddedByThem),
                _ => None,
            };

            if let Some(conflict_type) = conflict_type {
                conflicts.push(ConflictedFile { path, conflict_type });
            }
        }

        Ok(conflicts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_no_conflicts_returns_empty() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();
        let conflicts = gateway.get_conflicted_files().unwrap();

        assert_eq!(conflicts.len(), 0);
    }

    #[test]
    fn test_both_modified_conflict() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();

        // Create base file on main
        std::fs::write(dir.path().join("file.txt"), "base content\n").unwrap();
        gateway.stage_file("file.txt").unwrap();
        gateway.commit("Add file").unwrap();

        // Create feature branch from this point
        gateway.create_branch("feature").unwrap();

        // Back to main: modify the file
        gateway.checkout_branch("main").unwrap();
        std::fs::write(dir.path().join("file.txt"), "main content\n").unwrap();
        gateway.stage_file("file.txt").unwrap();
        gateway.commit("Modify file on main").unwrap();

        // On feature: modify the same file differently
        gateway.checkout_branch("feature").unwrap();
        std::fs::write(dir.path().join("file.txt"), "feature content\n").unwrap();
        gateway.stage_file("file.txt").unwrap();
        gateway.commit("Modify file on feature").unwrap();

        // Try to rebase feature onto main - will conflict
        let result = gateway.rebase_onto("feature", "main");
        assert!(result.is_ok());
        assert!(result.unwrap().has_conflicts());

        // Check conflicted files
        let conflicts = gateway.get_conflicted_files().unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, "file.txt");
        assert_eq!(conflicts[0].conflict_type, ConflictType::BothModified);

        // Clean up
        gateway.rebase_abort().ok();
    }

    #[test]
    fn test_conflict_type_display() {
        assert_eq!(ConflictType::BothModified.to_string(), "both modified");
        assert_eq!(ConflictType::BothAdded.to_string(), "both added");
        assert_eq!(ConflictType::DeletedByUs.to_string(), "deleted by us");
        assert_eq!(ConflictType::DeletedByThem.to_string(), "deleted by them");
        assert_eq!(ConflictType::AddedByUs.to_string(), "added by us");
        assert_eq!(ConflictType::AddedByThem.to_string(), "added by them");
    }
}
