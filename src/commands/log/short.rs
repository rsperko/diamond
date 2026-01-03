//! Short log output - simple text tree.

use anyhow::Result;
use colored::Colorize;

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;

use super::find_roots;

/// Short log output - simple text tree
/// Shows trunk at bottom, tips at top
pub fn run_short(ref_store: &RefStore, current_branch: &str) -> Result<()> {
    let roots = find_roots(ref_store)?;

    if roots.is_empty() {
        println!(
            "No branches tracked. Use '{} track' to start tracking branches.",
            program_name()
        );
        return Ok(());
    }

    let gateway = GitGateway::new()?;

    // Collect all lines, then reverse to show trunk at bottom
    // (is_current, marker, branch_name, needs_restack)
    let mut lines: Vec<(bool, String, String, bool)> = Vec::new();

    for root in roots {
        collect_short_tree(ref_store, &root, current_branch, &mut lines, &gateway)?;
    }

    // Reverse so trunk is at bottom
    lines.reverse();

    for (is_current, marker, branch, needs_restack) in lines {
        let suffix = if needs_restack {
            " (needs restack)".yellow().to_string()
        } else {
            String::new()
        };
        if is_current {
            println!("{}  {}{}", marker.green().bold(), branch.green().bold(), suffix);
        } else {
            println!("{}  {}{}", marker, branch, suffix);
        }
    }

    Ok(())
}

fn collect_short_tree(
    ref_store: &RefStore,
    branch: &str,
    current_branch: &str,
    lines: &mut Vec<(bool, String, String, bool)>,
    gateway: &GitGateway,
) -> Result<()> {
    let is_current = branch == current_branch;
    let marker = if is_current { "◉" } else { "◯" };

    // Check if this branch needs restack (parent's tip is not an ancestor of this branch)
    let needs_restack = if let Ok(Some(parent)) = ref_store.get_parent(branch) {
        !gateway.is_ancestor(&parent, branch).unwrap_or(true)
    } else {
        false
    };

    lines.push((is_current, marker.to_string(), branch.to_string(), needs_restack));

    let mut children: Vec<_> = ref_store.get_children(branch)?.into_iter().collect();
    children.sort();
    for child in children {
        collect_short_tree(ref_store, &child, current_branch, lines, gateway)?;
    }
    Ok(())
}
