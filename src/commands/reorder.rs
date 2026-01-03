use anyhow::{Context, Result};
use colored::Colorize;
use std::collections::HashSet;
use std::io::{IsTerminal, Read, Write};

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::state::acquire_operation_lock;

/// Interactively reorder branches in the downstack
pub fn run(file: Option<String>, preview: bool) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    let trunk = ref_store.require_trunk()?;

    let current_branch = gateway.get_current_branch_name()?;

    // Can't reorder if on trunk
    if current_branch == trunk {
        anyhow::bail!("Cannot reorder from trunk. Checkout a branch in your stack first.");
    }

    // Collect downstack branches (from trunk to current, excluding trunk)
    let downstack = ref_store.ancestors(&current_branch)?;

    if downstack.is_empty() {
        println!("{} Nothing to reorder - only one branch in stack", "!".yellow());
        return Ok(());
    }

    // Preview mode - just show current order
    if preview {
        println!("{} Current stack order (bottom to top):", "→".blue());
        println!();
        println!("# {}", trunk.dimmed());
        for branch in &downstack {
            println!("{}", branch);
        }
        return Ok(());
    }

    // Acquire exclusive lock to prevent concurrent Diamond operations
    let _lock = acquire_operation_lock()?;
    gateway.require_clean_for_rebase()?;

    // Get new order from file or editor
    let new_order = if let Some(file_path) = file {
        read_order_from_file(&file_path)?
    } else {
        open_editor_for_reorder(&downstack, &trunk)?
    };

    // Validate the new order
    validate_new_order(&downstack, &new_order)?;

    // Check if order actually changed
    if new_order == downstack {
        println!("{} Order unchanged - nothing to do", "✓".green());
        return Ok(());
    }

    // Apply the new order
    apply_new_order(&gateway, &ref_store, &trunk, &downstack, &new_order)?;

    println!();
    println!("{} Reorder complete!", "✓".green().bold());
    Ok(())
}

/// Read branch order from a file
fn read_order_from_file(path: &str) -> Result<Vec<String>> {
    let contents = std::fs::read_to_string(path).context(format!("Failed to read reorder file '{}'", path))?;

    parse_order(&contents)
}

/// Open editor to reorder branches
fn open_editor_for_reorder(branches: &[String], trunk: &str) -> Result<Vec<String>> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "Reorder requires interactive mode.\n\
             Use --file <path> to provide order non-interactively, or --preview to see current order."
        );
    }

    // Create temp file with current order
    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(format!("dm-reorder-{}.txt", std::process::id()));

    {
        let mut temp_file = std::fs::File::create(&temp_path).context("Failed to create temporary file")?;

        writeln!(temp_file, "# Reorder branches by rearranging the lines below.")?;
        writeln!(temp_file, "# Lines starting with '#' are comments and will be ignored.")?;
        writeln!(temp_file, "# Save and close the editor to apply changes.")?;
        writeln!(temp_file, "# Delete a line to remove a branch from the stack.")?;
        writeln!(temp_file, "#")?;
        writeln!(temp_file, "# Trunk (not editable): {}", trunk)?;
        writeln!(temp_file, "#")?;
        writeln!(temp_file)?;

        for branch in branches {
            writeln!(temp_file, "{}", branch)?;
        }

        temp_file.flush()?;
    }

    // Get editor from environment
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());

    // Open editor
    let status = std::process::Command::new(&editor)
        .arg(&temp_path)
        .status()
        .context(format!("Failed to open editor '{}'", editor))?;

    if !status.success() {
        let _ = std::fs::remove_file(&temp_path);
        anyhow::bail!("Editor exited with non-zero status");
    }

    // Read back the edited file
    let mut contents = String::new();
    let mut file = std::fs::File::open(&temp_path)?;
    file.read_to_string(&mut contents)?;

    // Clean up temp file
    let _ = std::fs::remove_file(&temp_path);

    parse_order(&contents)
}

/// Parse branch order from text content
fn parse_order(contents: &str) -> Result<Vec<String>> {
    let branches: Vec<String> = contents
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|s| s.to_string())
        .collect();

    if branches.is_empty() {
        anyhow::bail!("No branches specified in reorder file");
    }

    Ok(branches)
}

/// Validate that the new order is valid
fn validate_new_order(original: &[String], new_order: &[String]) -> Result<()> {
    let original_set: HashSet<&String> = original.iter().collect();
    let new_set: HashSet<&String> = new_order.iter().collect();

    // Check for duplicates
    if new_order.len() != new_set.len() {
        anyhow::bail!("Duplicate branch names in reorder list");
    }

    // Check for unknown branches
    for branch in &new_set {
        if !original_set.contains(branch) {
            anyhow::bail!("Unknown branch '{}' - can only reorder existing stack branches", branch);
        }
    }

    // Note: It's OK if some branches are removed (deleted from the list)
    // The user might want to remove a branch from the stack

    Ok(())
}

/// Apply the new order by rebasing branches
fn apply_new_order(
    gateway: &GitGateway,
    ref_store: &RefStore,
    trunk: &str,
    original_order: &[String],
    new_order: &[String],
) -> Result<()> {
    println!(
        "{} Reordering {} branches...",
        "→".blue(),
        new_order.len().to_string().yellow()
    );

    // First, determine what actually needs to change
    // We need to figure out the new parent for each branch
    // Store (branch, new_parent, old_parent) so we can use rebase --onto
    let mut new_parents: Vec<(String, String, String)> = Vec::new();

    for (i, branch) in new_order.iter().enumerate() {
        let new_parent = if i == 0 {
            trunk.to_string()
        } else {
            new_order[i - 1].clone()
        };

        // Get current (old) parent BEFORE we modify anything
        let old_parent = ref_store.get_parent(branch)?.unwrap_or_default();

        if old_parent != new_parent {
            new_parents.push((branch.clone(), new_parent, old_parent));
        }
    }

    // Handle branches that were removed from the new order
    let new_order_set: HashSet<&String> = new_order.iter().collect();
    for branch in original_order {
        if !new_order_set.contains(branch) {
            println!("  {} Removing '{}' from stack", "→".blue(), branch.yellow());
            // Reparent children to this branch's parent before removing
            let children: Vec<String> = ref_store.get_children(branch)?.into_iter().collect();
            if let Some(parent) = ref_store.get_parent(branch)? {
                for child in children {
                    ref_store.reparent(&child, &parent)?;
                }
            }
            ref_store.remove_branch(branch)?;
        }
    }

    if new_parents.is_empty() {
        println!("{} No rebasing needed - relationships already match", "✓".green());
        return Ok(());
    }

    // Create backups before rebasing
    println!("{} Creating backups...", "→".blue());
    for (branch, _, _) in &new_parents {
        let backup = gateway.create_backup_ref(branch)?;
        println!(
            "  {} Backed up {} @ {}",
            "✓".green(),
            branch,
            &backup.commit_oid.to_string()[..7]
        );
    }
    println!();

    // Apply changes - update metadata first, then rebase
    println!("{} Updating relationships and rebasing...", "→".blue());

    for (branch, new_parent, old_parent) in &new_parents {
        // Update parent relationship
        ref_store.reparent(branch, new_parent)?;

        // Rebase the branch onto its new parent
        // Use rebase_onto_from to replay only the unique commits (from old_parent to branch)
        println!(
            "  {} Rebasing {} onto {}...",
            "→".blue(),
            branch.green(),
            new_parent.blue()
        );

        // Use --onto to replay only commits between old_parent and branch onto new_parent
        let rebase_result = gateway.rebase_onto_from(branch, new_parent, old_parent)?;

        if rebase_result.has_conflicts() {
            println!();
            println!("{} Conflicts detected while rebasing '{}'", "!".yellow().bold(), branch);
            println!();
            println!("Resolve the conflicts, then run:");
            println!("  {} to continue", format!("{} continue", program_name()).cyan());
            println!("  {} to abort", format!("{} abort", program_name()).cyan());
            return Ok(());
        }

        println!("  {} Rebased {}", "✓".green(), branch);
    }

    // Return to the original branch if it still exists
    let original_branch = gateway.get_current_branch_name()?;
    if new_order_set.contains(&original_branch) {
        gateway.checkout_branch(&original_branch)?;
    } else if !new_order.is_empty() {
        // Checkout the top of the new stack
        gateway.checkout_branch(new_order.last().unwrap())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    fn create_branch(repo: &git2::Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[test]
    fn test_collect_downstack() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches before setting parent relationships
        create_branch(&repo, "f1")?;
        create_branch(&repo, "f2")?;
        create_branch(&repo, "f3")?;

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("f1", "main")?;
        ref_store.set_parent("f2", "f1")?;
        ref_store.set_parent("f3", "f2")?;

        let result = ref_store.ancestors("f3")?;

        assert_eq!(result, vec!["f1", "f2", "f3"]);

        Ok(())
    }

    #[test]
    fn test_parse_order() -> Result<()> {
        let content = "# Comment\nf1\nf2\n\nf3\n# Another comment";
        let result = parse_order(content)?;
        assert_eq!(result, vec!["f1", "f2", "f3"]);
        Ok(())
    }

    #[test]
    fn test_parse_order_empty_fails() {
        let content = "# Only comments\n# Nothing else";
        let result = parse_order(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_new_order_duplicates() {
        let original = vec!["f1".to_string(), "f2".to_string()];
        let new_order = vec!["f1".to_string(), "f1".to_string()];

        let result = validate_new_order(&original, &new_order);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate"));
    }

    #[test]
    fn test_validate_new_order_unknown_branch() {
        let original = vec!["f1".to_string(), "f2".to_string()];
        let new_order = vec!["f1".to_string(), "unknown".to_string()];

        let result = validate_new_order(&original, &new_order);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown branch"));
    }

    #[test]
    fn test_validate_new_order_subset_allowed() {
        // Removing branches is allowed
        let original = vec!["f1".to_string(), "f2".to_string(), "f3".to_string()];
        let new_order = vec!["f1".to_string(), "f3".to_string()];

        let result = validate_new_order(&original, &new_order);
        assert!(result.is_ok());
    }

    #[test]
    fn test_reorder_on_trunk_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        let result = run(None, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("trunk"));

        Ok(())
    }
}
