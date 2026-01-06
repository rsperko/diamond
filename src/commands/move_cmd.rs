use anyhow::{Context, Result};
use colored::Colorize;
use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use std::io;

use crate::branch_tree::{build_branch_tree, format_indent, MARKER_CURRENT, MARKER_OTHER};
use crate::forge::get_forge;
use crate::git_gateway::GitGateway;
use crate::operation_log::{Operation, OperationRecorder};
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::state::{acquire_operation_lock, OperationState};
use crate::ui;
use crate::worktree;

/// Move a branch to a new parent
/// If source is None, moves the current branch.
/// If source is Some(branch), moves that branch instead.
pub fn run(onto: Option<String>, source: Option<String>) -> Result<()> {
    let gateway = GitGateway::new()?;
    gateway.require_clean_for_rebase()?;

    let current = gateway.get_current_branch_name()?;
    let ref_store = RefStore::new()?;

    // Determine which branch to move
    let branch_to_move = match source {
        Some(ref s) => {
            // Verify source branch exists and is tracked
            if !gateway.branch_exists(s)? {
                anyhow::bail!("Branch '{}' does not exist", s);
            }
            if !ref_store.is_tracked(s)? {
                anyhow::bail!(
                    "Branch '{}' is not tracked by Diamond. Run '{} track' first.",
                    s,
                    program_name()
                );
            }
            s.clone()
        }
        None => current.clone(),
    };

    // Verify the branch to move is tracked (only check if no explicit source was given)
    if source.is_none() && !ref_store.is_tracked(&branch_to_move)? {
        anyhow::bail!(
            "Branch '{}' is not tracked by Diamond. Run '{} track' first.",
            branch_to_move,
            program_name()
        );
    }

    // Validate current parent exists before moving
    if let Some(current_parent) = ref_store.get_parent(&branch_to_move)? {
        // Skip validation if parent is trunk (trunk always exists)
        let trunk = ref_store.get_trunk()?;
        if Some(&current_parent) != trunk.as_ref() {
            gateway
                .validate_parent_exists(&current_parent)
                .context("Cannot move branch - current parent has been deleted. Run 'dm sync' first.")?;
        }
    }

    // Acquire exclusive lock to prevent concurrent Diamond operations
    let _lock = acquire_operation_lock()?;

    // Determine target parent
    let target_parent = match onto {
        Some(parent) => parent,
        None => {
            // Check if stdout is a TTY before launching TUI
            if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                anyhow::bail!(
                    "move requires --onto when running non-interactively.\n\
                     Usage: {} move --onto <parent-branch>",
                    program_name()
                );
            }

            // Interactive TUI mode - select target parent
            let selected = run_move_tui(&ref_store, &branch_to_move, &gateway)?;
            match selected {
                Some(target) => target,
                None => {
                    println!("Move cancelled.");
                    return Ok(());
                }
            }
        }
    };

    // Verify target exists
    if !gateway.branch_exists(&target_parent)? {
        anyhow::bail!("Target branch '{}' does not exist", target_parent);
    }

    // Verify target is tracked or is trunk
    let is_trunk = ref_store.get_trunk()? == Some(target_parent.clone());
    if !ref_store.is_tracked(&target_parent)? && !is_trunk {
        anyhow::bail!(
            "Target branch '{}' is not tracked by Diamond. Run '{} track {}' first.",
            target_parent,
            program_name(),
            target_parent
        );
    }

    // Verify we're not trying to move onto a descendant (would create cycle)
    if is_descendant(&ref_store, &target_parent, &branch_to_move)? {
        anyhow::bail!(
            "Cannot move '{}' onto '{}': target is a descendant (would create a cycle)",
            branch_to_move,
            target_parent
        );
    }

    // Collect all branches to rebase (branch_to_move + all descendants in DFS order)
    let mut branches_to_rebase = vec![branch_to_move.clone()];
    branches_to_rebase.extend(ref_store.descendants(&branch_to_move)?);

    // Validate that all branches in the subtree actually exist in git
    for branch in &branches_to_rebase {
        if !gateway.branch_exists(branch)? {
            anyhow::bail!(
                "Cannot move: branch '{}' is tracked but doesn't exist in git.\n\
                 Run '{} doctor --fix' to clean up metadata.",
                branch,
                program_name()
            );
        }
    }

    // Check for worktree conflicts before starting any rebase operations
    worktree::check_branches_for_worktree_conflicts(&branches_to_rebase)?;

    // Get old parent for rollback
    let old_parent = ref_store.get_parent(&branch_to_move)?;

    println!(
        "{} Moving {} (with {} descendants) onto {}...",
        "→".blue(),
        branch_to_move.green(),
        (branches_to_rebase.len() - 1).to_string().yellow(),
        target_parent.green()
    );

    // Create backup refs for all branches BEFORE starting
    println!("{} Creating backups...", "→".blue());
    let recorder = OperationRecorder::new()?;
    for branch in &branches_to_rebase {
        let backup = gateway.create_backup_ref(branch)?;
        println!(
            "  {} Backed up {} @ {}",
            "✓".green(),
            branch,
            &backup.commit_oid.to_string()[..7]
        );

        // Log backup creation
        recorder.record(Operation::BackupCreated {
            branch: branch.clone(),
            backup_ref: backup.ref_name.clone(),
        })?;
    }
    println!();

    // Log branch move
    recorder.record(Operation::BranchMoved {
        branch: branch_to_move.clone(),
        old_parent: old_parent.clone(),
        new_parent: Some(target_parent.clone()),
    })?;

    // STATE-FIRST: Save operation state BEFORE modifying anything
    // This ensures we can always recover with `dm abort` even if crash happens
    // after metadata is updated but before rebase completes
    // Note: original_branch is where we return after the move - this should be `current`
    // (the branch we were on when starting the move), not `branch_to_move`
    let state = OperationState::new_move(current.clone(), branches_to_rebase, target_parent.clone(), old_parent);
    state.save()?;

    // METADATA-SECOND: Update metadata BEFORE rebasing (commit intent)
    // If we crash after this, we have state saved so `dm abort` can restore
    println!("{} Updating metadata...", "→".blue());
    update_metadata_after_move(&ref_store, &branch_to_move, &target_parent)?;
    println!("  {} Metadata updated", "✓".green());

    // Update PR base on GitHub if forge is available and PR exists
    if let Ok(forge) = get_forge(None) {
        if let Ok(Some(_)) = forge.pr_exists(&branch_to_move) {
            if let Err(e) = forge.update_pr_base(&branch_to_move, &target_parent) {
                ui::warning(&format!("Could not update PR base for {}: {}", branch_to_move, e));
            } else {
                println!("  {} PR base updated to {}", "✓".green(), target_parent);
            }
        }
    }

    // Start rebasing to match git state to metadata
    let mut state = state;
    continue_move_from_state(&mut state, &ref_store)
}

/// Check if `branch` is a descendant of `ancestor`
/// Includes cycle detection to prevent infinite loops on corrupted metadata
fn is_descendant(ref_store: &RefStore, branch: &str, ancestor: &str) -> Result<bool> {
    let mut current = branch.to_string();
    let mut visited = std::collections::HashSet::new();

    loop {
        if current == ancestor {
            return Ok(true);
        }

        // Cycle detection: if we've seen this branch before, we have a cycle
        if !visited.insert(current.clone()) {
            // Log warning about cycle but don't panic - just return false
            eprintln!(
                "Warning: cycle detected in branch metadata at '{}'. Run '{} doctor' to fix.",
                current,
                program_name()
            );
            return Ok(false);
        }

        if let Some(parent) = ref_store.get_parent(&current)? {
            current = parent;
        } else {
            return Ok(false);
        }
    }
}

/// Continue moving from saved state
/// This is public so it can be called from the standalone continue command
pub fn continue_move_from_state(state: &mut OperationState, _ref_store: &RefStore) -> Result<()> {
    let gateway = GitGateway::new()?;

    // Reload ref_store to get current metadata (which was updated at start of move)
    let ref_store = RefStore::new()?;

    let target_parent = state
        .move_target_parent
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Move operation missing target parent"))?
        .clone();

    // Get the old parent (saved before metadata was updated)
    let old_parent = state
        .old_parent
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Move operation missing old parent"))?
        .clone();

    let first_branch = state.remaining_branches.first().cloned();

    while !state.remaining_branches.is_empty() {
        let branch = state.remaining_branches.remove(0);
        state.current_branch = Some(branch.clone());

        // Determine what to rebase onto and what the old base was
        let (onto, from) = if Some(&branch) == first_branch.as_ref() {
            // For the first branch (the one being moved):
            // - onto = target_parent (new parent)
            // - from = old_parent (original parent before move)
            // This uses `git rebase --onto target_parent old_parent branch`
            // which replays only the commits unique to this branch
            (target_parent.clone(), old_parent.clone())
        } else {
            // For descendants, their parent in metadata points to another branch
            // that was already rebased. Use the previous branch as both onto and from
            // since --fork-point should work correctly here.
            let parent = ref_store.get_parent(&branch)?.unwrap_or_else(|| target_parent.clone());
            (parent.clone(), parent)
        };

        // Check if branch is already rebased onto target (crash recovery)
        if gateway.is_branch_based_on(&branch, &onto)? {
            println!("  {} {} already rebased onto {}", "✓".green(), branch, onto);
            continue;
        }

        println!("{} Rebasing {} onto {}...", "→".blue(), branch.green(), onto.blue());

        // CHECKPOINT: Save state BEFORE rebase (crash recovery)
        state.save()?;

        // For the first branch, use rebase_onto_from to replay only its unique commits
        // For descendants, use regular rebase since their parent was just rebased
        let rebase_result = if Some(&branch) == first_branch.as_ref() {
            gateway.rebase_onto_from(&branch, &onto, &from)?
        } else {
            // Use --fork-point for descendants since their parent was just modified
            gateway.rebase_fork_point(&branch, &onto)?
        };

        if rebase_result.has_conflicts() {
            // State already saved above, show rich conflict message
            println!();

            ui::display_conflict_message(
                &branch,
                &onto,
                &state.remaining_branches,
                &ref_store,
                &gateway,
                false, // initial conflict
            )?;

            return Ok(());
        }

        println!("  {} Rebased {}", "✓".green(), branch);
    }

    // All done - metadata was already updated at start, just clean up
    state.current_branch = None;
    state.in_progress = false;
    OperationState::clear()?;

    // Return to original branch
    gateway.checkout_branch_worktree_safe(&state.original_branch)?;

    println!();
    println!("{} Move complete!", "✓".green().bold());
    Ok(())
}

fn update_metadata_after_move(ref_store: &RefStore, branch: &str, new_parent: &str) -> Result<()> {
    ref_store.reparent(branch, new_parent)?;
    Ok(())
}

/// Run the interactive TUI for selecting a move target
fn run_move_tui(ref_store: &RefStore, branch_to_move: &str, gateway: &GitGateway) -> Result<Option<String>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_move_app(&mut terminal, ref_store, branch_to_move, gateway);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn run_move_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ref_store: &RefStore,
    branch_to_move: &str,
    gateway: &GitGateway,
) -> Result<Option<String>> {
    let current_branch = gateway.get_current_branch_name().unwrap_or_default();

    // Build tree view using shared branch_tree module
    let all_rows = build_branch_tree(ref_store, &current_branch, gateway)?;

    // Get current parent of branch being moved (to mark as "(current)")
    let current_parent = ref_store.get_parent(branch_to_move)?;

    // Get all descendants of the branch being moved (invalid targets - would create cycle)
    let descendant_vec = ref_store.descendants(branch_to_move)?;
    let descendants: std::collections::HashSet<_> = descendant_vec.into_iter().collect();

    // Filter out invalid move targets:
    // - The branch being moved itself
    // - All descendants of the branch (would create a cycle)
    let valid_rows: Vec<_> = all_rows
        .iter()
        .filter(|b| {
            // Can't move onto self
            if b.name == branch_to_move {
                return false;
            }
            // Can't move onto a descendant (would create cycle)
            if descendants.contains(&b.name) {
                return false;
            }
            true
        })
        .cloned()
        .collect();

    // Handle empty list
    if valid_rows.is_empty() {
        anyhow::bail!(
            "No valid target branches found. Cannot move '{}' - all branches are descendants.",
            branch_to_move
        );
    }

    let mut state = ListState::default();
    state.select(Some(0));

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
                .split(f.area());

            let items: Vec<ListItem> = valid_rows
                .iter()
                .map(|branch| {
                    // Build display line with tree indentation
                    let indent = format_indent(branch.depth);
                    let marker = if branch.is_current {
                        MARKER_CURRENT
                    } else {
                        MARKER_OTHER
                    };

                    // Mark current parent for clarity
                    let current_indicator = if current_parent.as_ref() == Some(&branch.name) {
                        " (current parent)"
                    } else {
                        ""
                    };

                    let display = format!("{}{} {}{}", indent, marker, branch.name, current_indicator);

                    // Style: current branch in green, current parent dimmed
                    let style = if branch.is_current {
                        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                    } else if current_parent.as_ref() == Some(&branch.name) {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default()
                    };

                    ListItem::new(Line::from(vec![Span::styled(display, style)]))
                })
                .collect();

            let title = format!(" Move '{}' onto: ", branch_to_move);
            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(title)
                        .title_style(Style::default().add_modifier(Modifier::BOLD)),
                )
                .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
                .highlight_symbol("▶ ");

            f.render_stateful_widget(list, chunks[0], &mut state);
            let help = Paragraph::new("Enter: Select | q: Cancel | j/k: Navigate | g/G: Top/Bottom")
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(help, chunks[1]);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                    KeyCode::Enter => {
                        if let Some(i) = state.selected() {
                            if i < valid_rows.len() {
                                return Ok(Some(valid_rows[i].name.clone()));
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let i = match state.selected() {
                            Some(i) => {
                                if i >= valid_rows.len() - 1 {
                                    0
                                } else {
                                    i + 1
                                }
                            }
                            None => 0,
                        };
                        state.select(Some(i));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let i = match state.selected() {
                            Some(i) => {
                                if i == 0 {
                                    valid_rows.len() - 1
                                } else {
                                    i - 1
                                }
                            }
                            None => 0,
                        };
                        state.select(Some(i));
                    }
                    // Jump to top
                    KeyCode::Char('g') | KeyCode::Home => {
                        state.select(Some(0));
                    }
                    // Jump to bottom
                    KeyCode::Char('G') | KeyCode::End => {
                        state.select(Some(valid_rows.len().saturating_sub(1)));
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    fn create_branch(repo: &git2::Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[test]
    fn test_move_untracked_branch_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // No refs set - branch not tracked
        let result = run(Some("develop".to_string()), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));
    }

    #[test]
    fn test_is_descendant() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches before setting parent relationships
        create_branch(&repo, "feature-1")?;
        create_branch(&repo, "feature-2")?;

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature-1", "main")?;
        ref_store.set_parent("feature-2", "feature-1")?;

        assert!(is_descendant(&ref_store, "feature-2", "feature-1")?);
        assert!(is_descendant(&ref_store, "feature-2", "main")?);
        assert!(!is_descendant(&ref_store, "feature-1", "feature-2")?);
        assert!(!is_descendant(&ref_store, "main", "feature-1")?);

        Ok(())
    }

    #[test]
    fn test_descendants_from_trunk() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches before setting parent relationships
        create_branch(&repo, "child-1")?;
        create_branch(&repo, "child-2")?;
        create_branch(&repo, "grandchild")?;

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("child-1", "main")?;
        ref_store.set_parent("child-2", "main")?;
        ref_store.set_parent("grandchild", "child-1")?;

        let result = ref_store.descendants("main")?;

        // Should get all descendants in DFS order
        assert!(result.contains(&"child-1".to_string()));
        assert!(result.contains(&"child-2".to_string()));
        assert!(result.contains(&"grandchild".to_string()));

        Ok(())
    }

    #[test]
    fn test_move_requires_onto() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Set up trunk and a tracked branch via refs
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Create and checkout a tracked branch
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &main_commit, false).unwrap();
        ref_store.set_parent("feature", "main").unwrap();

        // Checkout feature
        std::process::Command::new("git")
            .args(["checkout", "feature"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Without --onto should fail with helpful message about --onto
        let result = run(None, None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("--onto"),
            "Expected error about --onto being required, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_move_fails_if_descendant_missing() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create feature branch
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &main_commit, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();

        // Set up refs with feature having a non-existent child
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature", "main").unwrap();
        ref_store.set_parent("non-existent-child", "feature").unwrap();

        // Checkout feature
        std::process::Command::new("git")
            .args(["checkout", "feature"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Try to move feature - should fail because child doesn't exist
        let result = run(Some("main".to_string()), None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("doesn't exist in git") || err_msg.contains("not found"),
            "Error message should mention missing branch: {}",
            err_msg
        );
    }

    #[test]
    fn test_move_with_source_flag() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new().unwrap();
        let ref_store = RefStore::new().unwrap();

        // Create branches: main -> feature1, main -> feature2
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature1", &main_commit, false).unwrap();
        repo.branch("feature2", &main_commit, false).unwrap();
        repo.branch("develop", &main_commit, false).unwrap();

        // Set up refs
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature1", "main").unwrap();
        ref_store.set_parent("feature2", "main").unwrap();
        ref_store.set_parent("develop", "main").unwrap();

        // Stay on main, but move feature2 to be child of develop using --source
        // Move feature2 onto develop
        let result = run(Some("develop".to_string()), Some("feature2".to_string()));
        assert!(result.is_ok(), "Move with --source should succeed");

        // Verify feature2 is now child of develop
        assert_eq!(ref_store.get_parent("feature2").unwrap(), Some("develop".to_string()));
        // Verify we're still on main
        assert_eq!(gateway.get_current_branch_name().unwrap(), "main");
    }

    #[test]
    fn test_move_source_nonexistent_branch_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Try to move non-existent source
        let result = run(Some("main".to_string()), Some("nonexistent".to_string()));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("does not exist") || err_msg.contains("not tracked"),
            "Expected error about nonexistent branch, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_move_without_onto_in_non_tty_shows_helpful_error() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Create and checkout a tracked branch
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &main_commit, false).unwrap();
        ref_store.set_parent("feature", "main").unwrap();

        std::process::Command::new("git")
            .args(["checkout", "feature"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Without --onto, should fail with helpful error message
        let result = run(None, None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();

        // Should mention --onto flag
        assert!(
            err_msg.contains("--onto"),
            "Error should mention --onto flag: {}",
            err_msg
        );

        // Should mention non-interactive usage
        assert!(
            err_msg.contains("non-interactively") || err_msg.contains("Usage"),
            "Error should provide usage hint: {}",
            err_msg
        );
    }

    #[test]
    fn test_descendants_leaf_branch() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature", "main")?;

        // feature is a leaf - no descendants
        let result = ref_store.descendants("feature")?;
        assert!(result.is_empty(), "Leaf branch should have no descendants");

        Ok(())
    }

    #[test]
    fn test_move_onto_descendant_fails() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Create stack: main -> parent -> child
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("parent", &main_commit, false).unwrap();
        repo.branch("child", &main_commit, false).unwrap();

        ref_store.set_parent("parent", "main").unwrap();
        ref_store.set_parent("child", "parent").unwrap();

        std::process::Command::new("git")
            .args(["checkout", "parent"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Try to move parent onto child (its descendant) - should fail
        let result = run(Some("child".to_string()), None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("cycle") || err_msg.contains("descendant"),
            "Expected error about cycle/descendant, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_move_onto_self_fails() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &main_commit, false).unwrap();
        ref_store.set_parent("feature", "main").unwrap();

        std::process::Command::new("git")
            .args(["checkout", "feature"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Try to move feature onto itself - should fail
        let result = run(Some("feature".to_string()), None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // This should fail at the is_descendant check (a branch is technically a descendant of itself)
        // or with a different validation error
        assert!(
            err_msg.contains("cycle") || err_msg.contains("descendant") || err_msg.contains("same"),
            "Expected error about moving onto self, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_move_fails_when_current_parent_deleted() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Create stack: main -> A -> B
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("A", &main_commit, false).unwrap();
        ref_store.set_parent("A", "main").unwrap();

        repo.branch("B", &main_commit, false).unwrap();
        ref_store.set_parent("B", "A").unwrap();

        // Delete A (B's parent) directly with git
        repo.find_branch("A", git2::BranchType::Local)
            .unwrap()
            .delete()
            .unwrap();

        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["checkout", "B"])
            .output()
            .unwrap();

        // Try to move B (which has deleted parent A)
        // This should fail at parent validation (B's parent A doesn't exist)
        let result = run(Some("main".to_string()), None);
        assert!(result.is_err());
        // Error may be about parent validation or about target not being tracked
        // Either way, the move fails as expected
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("does not exist")
                || err_msg.contains("deleted")
                || err_msg.contains("parent")
                || err_msg.contains("tracked"),
            "Expected error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_move_succeeds_when_parent_is_trunk() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Create branch A from main (trunk)
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("A", &main_commit, false).unwrap();
        ref_store.set_parent("A", "main").unwrap();

        // Create another branch B to move to
        repo.branch("B", &main_commit, false).unwrap();
        ref_store.set_parent("B", "main").unwrap();

        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["checkout", "A"])
            .output()
            .unwrap();

        // Move A onto B should succeed (A's parent is trunk, which always exists)
        let result = run(Some("B".to_string()), None);
        if let Err(ref e) = result {
            panic!("Expected move to succeed, but got error: {}", e);
        }
        assert!(result.is_ok());
    }
}
