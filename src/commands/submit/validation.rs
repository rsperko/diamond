//! Validation utilities for submit command.

use anyhow::Result;
use colored::Colorize;

use crate::forge::AsyncForge;
use crate::git_gateway::{BranchSyncState, GitGateway};
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Validate that the stack is properly structured before submitting.
///
/// Checks that each branch in the ancestry chain is actually rebased onto its parent.
/// This prevents submitting PRs with broken diffs.
pub(super) fn validate_stack_integrity(
    branch: &str,
    ref_store: &RefStore,
    gateway: &GitGateway,
    trunk: Option<&str>,
) -> Result<()> {
    let mut current = branch.to_string();
    let mut seen = std::collections::HashSet::new();
    seen.insert(current.clone());

    // Walk up the ancestry chain and verify each relationship
    while let Some(parent) = ref_store.get_parent(&current)? {
        // Skip trunk - it's the root
        if trunk == Some(parent.as_str()) {
            break;
        }

        // Cycle detection
        if !seen.insert(parent.clone()) {
            anyhow::bail!(
                "Circular parent reference detected at '{}'. Run 'dm cleanup' to repair metadata.",
                parent
            );
        }

        // Check if current branch is actually based on its parent
        match gateway.is_branch_based_on(&current, &parent) {
            Ok(true) => {
                // Good - continue checking up the chain
            }
            Ok(false) => {
                anyhow::bail!(
                    "Branch '{}' is not rebased onto '{}'. Run '{} restack' first.",
                    current,
                    parent,
                    program_name()
                );
            }
            Err(e) => {
                // If we can't check (branch doesn't exist), skip validation for this branch
                // This can happen with mocks or if a branch was deleted
                eprintln!(
                    "{} Could not validate stack integrity for '{}': {}",
                    "⚠".yellow(),
                    current,
                    e
                );
            }
        }

        current = parent;
    }

    Ok(())
}

/// Check if trunk is behind remote and warn (but don't block).
pub(super) fn check_trunk_sync(gateway: &GitGateway, trunk: Option<&str>) -> Result<()> {
    if let Some(trunk_name) = trunk {
        match gateway.check_remote_sync(trunk_name) {
            Ok(BranchSyncState::Behind(n)) => {
                eprintln!(
                    "{} Trunk '{}' is {} commit{} behind remote. Consider running '{} sync'.",
                    "⚠".yellow(),
                    trunk_name,
                    n,
                    if n == 1 { "" } else { "s" },
                    program_name()
                );
            }
            Ok(BranchSyncState::Diverged { remote_ahead, .. }) => {
                eprintln!(
                    "{} Trunk '{}' has diverged from remote ({} remote commits). Consider running '{} sync'.",
                    "⚠".yellow(),
                    trunk_name,
                    remote_ahead,
                    program_name()
                );
            }
            _ => {
                // InSync, Ahead, NoRemote, or error - all fine
            }
        }
    }
    Ok(())
}

/// Show a preview of what will be submitted (async batch version)
///
/// This version uses batch API calls for better performance with many branches.
pub(super) async fn show_submit_preview_async(
    branches: &[String],
    ref_store: &RefStore,
    forge: &dyn AsyncForge,
    update_only: bool,
) -> Result<()> {
    println!("{} Will submit:", "→".blue());

    // Batch check all PR existence in parallel
    let pr_results = forge.check_prs_exist(branches).await;

    // Build a map of branch -> PR info for quick lookup
    let pr_map: std::collections::HashMap<String, _> = pr_results.into_iter().collect();

    for branch in branches {
        let parent = ref_store.get_parent(branch)?;
        let trunk = ref_store.get_trunk()?;
        let base = parent.as_ref().or(trunk.as_ref()).map(|s| s.as_str());

        // Look up PR info from batch results
        let pr_exists = pr_map.get(branch).and_then(|opt| opt.as_ref());

        let action = if let Some(pr_info) = pr_exists {
            format!("update PR #{}", pr_info.number)
        } else if update_only {
            "skip (no PR)".dimmed().to_string()
        } else {
            "new PR".to_string()
        };

        let base_str = base.unwrap_or("?");
        println!("  • {} → {} ({})", branch.green(), base_str.blue(), action);
    }

    if update_only {
        println!("\n{} --update-only: branches without PRs will be skipped", "ℹ".blue());
    }

    Ok(())
}
