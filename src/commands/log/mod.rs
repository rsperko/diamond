//! Log command - display stack visualization.

mod long;
mod short;
mod tui;

#[cfg(test)]
mod tests;

use anyhow::Result;

use crate::git_gateway::GitGateway;
use crate::ref_store::RefStore;

/// Action that can be performed from the TUI
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum TuiAction {
    None,
    Checkout,
    Delete,
    NavigateUp,
    NavigateDown,
    NavigateTop,
    NavigateBottom,
}

pub fn run(mode: Option<String>) -> Result<()> {
    // Silent cleanup of orphaned refs before displaying log
    let gateway = GitGateway::new()?;
    gateway.cleanup_orphaned_refs_silently();

    // Load state
    let ref_store = RefStore::new()?;

    // Get current branch
    let current_branch = gateway.get_current_branch_name()?;

    match mode.as_deref() {
        Some("short") | Some("s") => short::run_short(&ref_store, &current_branch),
        Some("long") | Some("l") => long::run_long(&ref_store, &current_branch, &gateway),
        Some(other) => {
            anyhow::bail!("Unknown log mode '{}'. Use 'short' or 'long', or omit for TUI.", other)
        }
        None => {
            // Check if stdout is a TTY - if not, fall back to short mode
            if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                tui::run_tui(&ref_store, &current_branch, &gateway)
            } else {
                // Running in non-interactive environment (tests, pipes, etc.)
                short::run_short(&ref_store, &current_branch)
            }
        }
    }
}

pub(crate) fn find_roots(ref_store: &RefStore) -> Result<Vec<String>> {
    // In RefStore, the root is the trunk
    let trunk = ref_store.get_trunk()?;
    match trunk {
        Some(t) => Ok(vec![t]),
        None => Ok(vec![]),
    }
}
