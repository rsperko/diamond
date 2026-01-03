//! Long log output - shows commits for each branch.

use anyhow::Result;
use colored::Colorize;

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;

use super::find_roots;

/// Long log output - shows commits for each branch
/// Shows trunk at bottom, tips at top
pub fn run_long(ref_store: &RefStore, current_branch: &str, gateway: &GitGateway) -> Result<()> {
    let roots = find_roots(ref_store)?;

    if roots.is_empty() {
        println!(
            "No branches tracked. Use '{} track' to start tracking branches.",
            program_name()
        );
        return Ok(());
    }

    // Collect all lines, then reverse to show trunk at bottom
    let mut lines: Vec<String> = Vec::new();

    for root in roots {
        collect_long_tree(ref_store, &root, current_branch, gateway, 0, &mut lines)?;
    }

    // Reverse so trunk is at bottom
    lines.reverse();

    for line in lines {
        println!("{}", line);
    }

    Ok(())
}

fn collect_long_tree(
    ref_store: &RefStore,
    branch: &str,
    current_branch: &str,
    gateway: &GitGateway,
    depth: usize,
    lines: &mut Vec<String>,
) -> Result<()> {
    let is_current = branch == current_branch;
    let indent = "│ ".repeat(depth);
    let marker = if is_current { "◉" } else { "◯" };

    // Check if this branch needs restack (parent's tip is not an ancestor of this branch)
    let needs_restack = if let Ok(Some(parent)) = ref_store.get_parent(branch) {
        !gateway.is_ancestor(&parent, branch).unwrap_or(true)
    } else {
        false
    };

    let restack_suffix = if needs_restack {
        " (needs restack)".yellow().to_string()
    } else {
        String::new()
    };

    // Get commit info for this branch
    let commit_info = gateway.get_branch_commit_info(branch).unwrap_or_default();

    let line = if is_current {
        format!(
            "{}{}  {}{} {}",
            indent,
            marker.green().bold(),
            branch.green().bold(),
            restack_suffix,
            commit_info.dimmed()
        )
    } else {
        format!(
            "{}{}  {}{} {}",
            indent,
            marker,
            branch,
            restack_suffix,
            commit_info.dimmed()
        )
    };
    lines.push(line);

    let mut children: Vec<_> = ref_store.get_children(branch)?.into_iter().collect();
    children.sort();
    for child in children {
        collect_long_tree(ref_store, &child, current_branch, gateway, depth + 1, lines)?;
    }

    Ok(())
}
