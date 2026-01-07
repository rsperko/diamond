//! Conflict message formatting for enhanced UX during rebases

use anyhow::Result;
use colored::Colorize;

use crate::git_gateway::{ConflictedFile, GitGateway};
use crate::program_name::program_name;
use crate::ref_store::RefStore;

#[cfg(test)]
use crate::git_gateway::ConflictType;

/// Generate mini stack visualization showing current position during conflict
///
/// Shows:
/// - Parent branch (base)
/// - Current branch (marked as CONFLICTED)
/// - Immediate children (marked as pending)
///
/// Example output:
/// ```text
/// Stack position (you are here):
///   feature-1 ← base
///   └─ feature-2 [CONFLICTED]
///      ├─ feature-3 (pending)
///      └─ feature-4 (pending)
/// ```
pub fn format_conflict_stack_position(current_branch: &str, ref_store: &RefStore) -> Result<String> {
    let mut lines = vec![format!("Stack position (you are here, resolving {}):", current_branch)];

    // Get parent (if exists)
    let parent = ref_store.get_parent(current_branch)?;

    // Get immediate children
    let children = ref_store.get_children(current_branch).unwrap_or_default();
    let mut sorted_children: Vec<_> = children.into_iter().collect();
    sorted_children.sort();

    // Format parent line
    if let Some(parent) = &parent {
        lines.push(format!("  {} ← base", parent));
    }

    // Format current branch line
    let current_line = if parent.is_some() {
        format!("  └─ {} {}", current_branch, "[CONFLICTED]".red().bold())
    } else {
        // No parent (root branch)
        format!("  {} {}", current_branch, "[CONFLICTED]".red().bold())
    };
    lines.push(current_line);

    // Format children lines (if any)
    if !sorted_children.is_empty() {
        let num_children = sorted_children.len();
        for (i, child) in sorted_children.iter().enumerate() {
            let is_last = i == num_children - 1;
            let connector = if is_last { "└─" } else { "├─" };
            lines.push(format!("     {} {} {}", connector, child, "(pending)".dimmed()));
        }
    }

    Ok(lines.join("\n"))
}

/// Format conflicted files list with conflict types
///
/// Example output:
/// ```text
/// Conflicted files (3):
///   • src/main.rs (both modified)
///   • src/lib.rs (deleted by them)
///   • docs/README.md (both added)
/// ```
fn format_conflicted_files(files: &[ConflictedFile]) -> String {
    if files.is_empty() {
        return String::new();
    }

    let mut lines = vec![format!("Conflicted files ({}):", files.len().to_string().red().bold())];

    for file in files {
        lines.push(format!("  • {} ({})", file.path, file.conflict_type));
    }

    lines.join("\n")
}

/// Display rich conflict message with stack context and conflicted files
///
/// Shows:
/// - Conflict header
/// - Stack visualization
/// - Conflicted files list
/// - Remaining branches to process
/// - Resolution steps
///
/// # Arguments
/// * `current_branch` - The branch currently being rebased (with conflicts)
/// * `parent_branch` - The branch being rebased onto
/// * `remaining_branches` - Branches that still need processing after this one
/// * `ref_store` - For stack relationships
/// * `gateway` - For querying conflicted files
/// * `is_continue` - true if called from `dm continue`, false if initial conflict
pub fn display_conflict_message(
    current_branch: &str,
    parent_branch: &str,
    remaining_branches: &[String],
    ref_store: &RefStore,
    gateway: &GitGateway,
    is_continue: bool,
) -> Result<()> {
    // Header
    let header = if is_continue {
        format!(
            "{} Conflicts still present in '{}'",
            "!".yellow().bold(),
            current_branch
        )
    } else {
        format!(
            "{} Conflicts detected rebasing '{}' onto '{}'",
            "!".yellow().bold(),
            current_branch,
            parent_branch
        )
    };
    println!("\n{}\n", header);

    // Stack visualization
    let stack_viz = format_conflict_stack_position(current_branch, ref_store)?;
    println!("{}\n", stack_viz);

    // Conflicted files
    let conflicts = gateway.get_conflicted_files()?;
    let files_section = format_conflicted_files(&conflicts);
    if !files_section.is_empty() {
        println!("{}\n", files_section);
    }

    // Remaining branches
    if !remaining_branches.is_empty() {
        let branch_list = if remaining_branches.len() <= 3 {
            remaining_branches.join(", ")
        } else {
            format!(
                "{}, and {} more",
                remaining_branches[..2].join(", "),
                remaining_branches.len() - 2
            )
        };
        println!(
            "Remaining after resolution: {} ({} {})\n",
            branch_list,
            remaining_branches.len().to_string().dimmed(),
            if remaining_branches.len() == 1 {
                "branch"
            } else {
                "branches"
            }
            .dimmed()
        );
    }

    // Resolution steps
    println!("To resolve:");
    println!("  (1) Fix conflicts in the files above");
    println!("  (2) Stage changes: git add <files>");
    println!("  (3) Continue: {} continue", program_name());
    println!();
    println!("Or cancel: {} abort", program_name());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_format_conflict_stack_position_with_parent_and_children() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();

        // Create stack: main -> feature-1 -> feature-2 -> feature-3, feature-4
        let gateway = GitGateway::new().unwrap();
        gateway.create_branch("feature-1").unwrap();
        gateway.create_branch("feature-2").unwrap();
        gateway.create_branch("feature-3").unwrap();
        gateway.checkout_branch("feature-2").unwrap();
        gateway.create_branch("feature-4").unwrap();

        ref_store.set_parent("feature-1", "main").unwrap();
        ref_store.set_parent("feature-2", "feature-1").unwrap();
        ref_store.set_parent("feature-3", "feature-2").unwrap();
        ref_store.set_parent("feature-4", "feature-2").unwrap();

        let result = format_conflict_stack_position("feature-2", &ref_store).unwrap();

        assert!(result.contains("Stack position"));
        assert!(result.contains("feature-1 ← base"));
        assert!(result.contains("feature-2"));
        assert!(result.contains("[CONFLICTED]"));
        assert!(result.contains("feature-3"));
        assert!(result.contains("feature-4"));
        assert!(result.contains("(pending)"));
    }

    #[test]
    fn test_format_conflict_stack_position_no_children() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();

        let gateway = GitGateway::new().unwrap();
        gateway.create_branch("feature-1").unwrap();
        gateway.create_branch("feature-2").unwrap();

        ref_store.set_parent("feature-1", "main").unwrap();
        ref_store.set_parent("feature-2", "feature-1").unwrap();

        let result = format_conflict_stack_position("feature-2", &ref_store).unwrap();

        assert!(result.contains("feature-1 ← base"));
        assert!(result.contains("feature-2"));
        assert!(result.contains("[CONFLICTED]"));
        assert!(!result.contains("(pending)"));
    }

    #[test]
    fn test_format_conflict_stack_position_no_parent() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();

        let gateway = GitGateway::new().unwrap();
        gateway.create_branch("feature-1").unwrap();
        gateway.create_branch("feature-2").unwrap();

        ref_store.set_parent("feature-2", "feature-1").unwrap();

        // feature-1 has no parent (orphaned)
        let result = format_conflict_stack_position("feature-1", &ref_store).unwrap();

        assert!(result.contains("feature-1"));
        assert!(result.contains("[CONFLICTED]"));
        assert!(result.contains("feature-2"));
    }

    #[test]
    fn test_format_conflicted_files_empty() {
        let result = format_conflicted_files(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_conflicted_files_multiple() {
        let files = vec![
            ConflictedFile {
                path: "src/main.rs".to_string(),
                conflict_type: ConflictType::BothModified,
            },
            ConflictedFile {
                path: "src/lib.rs".to_string(),
                conflict_type: ConflictType::DeletedByThem,
            },
        ];

        let result = format_conflicted_files(&files);

        assert!(result.contains("Conflicted files (2)"));
        assert!(result.contains("src/main.rs"));
        assert!(result.contains("both modified"));
        assert!(result.contains("src/lib.rs"));
        assert!(result.contains("deleted by them"));
    }
}
