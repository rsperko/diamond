use crate::branch_tree::{build_branch_tree, find_current_branch_index, format_indent, MARKER_CURRENT, MARKER_OTHER};
use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::ui::{highlight_matches, render_search_box, SearchState, NO_MATCHES_MESSAGE};
use anyhow::Result;
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

/// Checkout a branch
///
/// Flags:
/// - `name`: Specific branch name to checkout
/// - `trunk`: Go directly to trunk branch
/// - `stack`: Show only current stack branches (TUI mode)
/// - `all`: Show all trunks in selection (TUI mode)
/// - `untracked`: Include untracked branches (TUI mode)
pub fn run(
    name: Option<String>,
    trunk: bool,
    _stack: bool,     // TODO: implement stack filter for TUI
    _all: bool,       // TODO: implement all-trunks filter for TUI
    _untracked: bool, // TODO: implement untracked filter for TUI
) -> Result<()> {
    // Silent cleanup of orphaned refs before checkout
    let gateway = GitGateway::new()?;
    if let Err(_e) = crate::validation::silent_cleanup_orphaned_refs(&gateway) {
        // Non-fatal: if cleanup fails, still proceed with checkout
    }

    let ref_store = RefStore::new()?;

    // --trunk flag: go directly to trunk
    if trunk {
        let trunk_branch = ref_store.require_trunk()?;
        gateway.checkout_branch_worktree_safe(&trunk_branch)?;
        println!("Checked out trunk '{}'", trunk_branch);
        return Ok(());
    }

    // Non-interactive mode with explicit branch name
    if let Some(target) = name {
        // Use safe checkout that respects uncommitted changes AND worktree conflicts
        gateway.checkout_branch_worktree_safe(&target)?;

        // Fetch diamond ref for this branch from remote (best effort)
        // This enables collaboration - we get the parent relationship from remote
        let _ = gateway.fetch_diamond_ref_for_branch(&target);

        println!("Checked out '{}'", target);
        return Ok(());
    }

    // Check if stdout is a TTY before launching TUI
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        anyhow::bail!(
            "checkout requires a branch name when running non-interactively. Usage: {} checkout <branch>",
            program_name()
        );
    }

    // Interactive TUI mode
    let current_branch = gateway.get_current_branch_name().unwrap_or_default();
    let selected = run_tui(&ref_store, &current_branch, &gateway)?;

    if let Some(target) = selected {
        println!("Selected: {}", target);
        // Use safe checkout that respects uncommitted changes AND worktree conflicts
        gateway.checkout_branch_worktree_safe(&target)?;

        // Fetch diamond ref for this branch from remote (best effort)
        let _ = gateway.fetch_diamond_ref_for_branch(&target);
    }

    Ok(())
}

fn run_tui(ref_store: &RefStore, current_branch: &str, gateway: &GitGateway) -> Result<Option<String>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, ref_store, current_branch, gateway);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ref_store: &RefStore,
    current_branch: &str,
    gateway: &GitGateway,
) -> Result<Option<String>> {
    // Build tree view using shared branch_tree module (stack order: trunk at bottom)
    let rows = build_branch_tree(ref_store, current_branch, gateway)?;

    // Handle empty list
    if rows.is_empty() {
        anyhow::bail!(
            "No branches tracked. Use '{} track' to start tracking branches.",
            program_name()
        );
    }

    let mut state = ListState::default();
    // Start with current branch selected (consistent with dm log)
    let current_idx = find_current_branch_index(&rows);
    state.select(Some(current_idx));

    // Initialize fuzzy search state
    let mut search_state = SearchState::new(rows.len());

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(3), // Search box
                        Constraint::Min(0),    // Branch list
                        Constraint::Length(3), // Help text
                    ]
                    .as_ref(),
                )
                .split(f.area());

            // Render search input box
            let search_widget = render_search_box(search_state.query());
            f.render_widget(search_widget, chunks[0]);

            // Build items from FILTERED indices with match highlighting
            // Clone filtered indices to avoid borrow conflicts with mutable get_match_indices
            let filtered: Vec<usize> = search_state.filtered_indices().to_vec();

            // Pre-compute match indices for all filtered branches
            let match_indices_map: Vec<(usize, Vec<usize>)> = filtered
                .iter()
                .map(|&idx| {
                    let branch_name = &rows[idx].name;
                    (idx, search_state.get_match_indices(branch_name))
                })
                .collect();

            let items: Vec<ListItem> = if filtered.is_empty() {
                // No matches - show helpful message
                vec![ListItem::new(Line::from(vec![Span::styled(
                    NO_MATCHES_MESSAGE,
                    Style::default().fg(Color::Yellow),
                )]))]
            } else {
                match_indices_map
                    .iter()
                    .map(|(idx, match_indices)| {
                        let branch = &rows[*idx];

                        // Build styled spans with highlighting
                        let indent = format_indent(branch.depth);
                        let marker = if branch.is_current {
                            MARKER_CURRENT
                        } else {
                            MARKER_OTHER
                        };
                        let restack_indicator = if branch.needs_restack { " (needs restack)" } else { "" };

                        // Base style
                        let base_style = if branch.is_current {
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                        } else if branch.needs_restack {
                            Style::default().fg(Color::Yellow)
                        } else {
                            Style::default()
                        };

                        // Build spans with match highlighting
                        let mut spans = vec![Span::styled(format!("{}{} ", indent, marker), base_style)];

                        // Use shared highlighting logic
                        spans.extend(highlight_matches(&branch.name, match_indices, base_style));
                        spans.push(Span::styled(restack_indicator, base_style));

                        ListItem::new(Line::from(spans))
                    })
                    .collect()
            };

            // Add match count to title
            let total = rows.len();
            let shown = filtered.len();
            let title = if search_state.is_empty() {
                format!(" Select Branch ({} branches) ", total)
            } else {
                format!(" Select Branch ({} of {} branches) ", shown, total)
            };

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(title)
                        .title_style(Style::default().add_modifier(Modifier::BOLD)),
                )
                .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
                .highlight_symbol("▶ ");

            f.render_stateful_widget(list, chunks[1], &mut state);
            let help =
                Paragraph::new("Type to filter | Enter: Select | Esc: Clear/Quit | ↑↓/jk: Navigate | g/G: Top/Bottom")
                    .block(Block::default().borders(Borders::ALL));
            f.render_widget(help, chunks[2]);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    // Character input - add to search query
                    KeyCode::Char(c) if c != 'q' && c != 'j' && c != 'k' && c != 'g' && c != 'G' => {
                        search_state.push_char(c);
                        search_state.filter(&rows, |branch| &branch.name);

                        // Reset selection to first match
                        state.select(if search_state.filtered_indices().is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                    }

                    // Backspace - remove last character
                    KeyCode::Backspace => {
                        search_state.pop_char();
                        search_state.filter(&rows, |branch| &branch.name);

                        // Reset selection to first match
                        state.select(if search_state.filtered_indices().is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                    }

                    // Escape - clear search or exit
                    KeyCode::Esc => {
                        if search_state.is_empty() {
                            return Ok(None); // Exit
                        } else {
                            search_state.clear();
                            search_state.filter(&rows, |branch| &branch.name);

                            // Restore selection to current branch
                            let current_idx = find_current_branch_index(&rows);
                            state.select(Some(current_idx));
                        }
                    }

                    // Q - always quit
                    KeyCode::Char('q') => return Ok(None),

                    // Enter - select current branch
                    KeyCode::Enter => {
                        if let Some(i) = state.selected() {
                            let filtered = search_state.filtered_indices();
                            if i < filtered.len() {
                                let original_idx = filtered[i];
                                return Ok(Some(rows[original_idx].name.clone()));
                            }
                        }
                    }

                    // Navigation through FILTERED list
                    KeyCode::Down | KeyCode::Char('j') => {
                        let max = search_state.filtered_indices().len();
                        if max == 0 {
                            state.select(None);
                        } else {
                            let i = match state.selected() {
                                Some(i) => {
                                    if i >= max - 1 {
                                        0
                                    } else {
                                        i + 1
                                    }
                                }
                                None => 0,
                            };
                            state.select(Some(i));
                        }
                    }

                    KeyCode::Up | KeyCode::Char('k') => {
                        let max = search_state.filtered_indices().len();
                        if max == 0 {
                            state.select(None);
                        } else {
                            let i = match state.selected() {
                                Some(i) => {
                                    if i == 0 {
                                        max - 1
                                    } else {
                                        i - 1
                                    }
                                }
                                None => 0,
                            };
                            state.select(Some(i));
                        }
                    }

                    // Jump to top
                    KeyCode::Char('g') | KeyCode::Home => {
                        state.select(if search_state.filtered_indices().is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                    }

                    // Jump to bottom
                    KeyCode::Char('G') | KeyCode::End => {
                        state.select(if search_state.filtered_indices().is_empty() {
                            None
                        } else {
                            Some(search_state.filtered_indices().len().saturating_sub(1))
                        });
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
    use crate::git_gateway::GitGateway;
    use crate::platform::DisplayPath;
    use anyhow::Result;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_checkout_existing_branch() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create branches using git directly
        let head = repo.head()?;
        let commit = head.peel_to_commit()?;
        repo.branch("feature-1", &commit, false)?;
        repo.branch("feature-2", &commit, false)?;

        // Checkout feature-1
        run(Some("feature-1".to_string()), false, false, false, false)?;

        // Verify we're on feature-1
        assert_eq!(gateway.get_current_branch_name()?, "feature-1");

        // Checkout feature-2
        run(Some("feature-2".to_string()), false, false, false, false)?;

        // Verify we're on feature-2
        assert_eq!(gateway.get_current_branch_name()?, "feature-2");

        Ok(())
    }

    #[test]
    fn test_checkout_nonexistent_branch_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Try to checkout branch that doesn't exist
        let result = run(Some("does-not-exist".to_string()), false, false, false, false);
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_checkout_same_branch_twice() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create branch
        let head = repo.head()?;
        let commit = head.peel_to_commit()?;
        repo.branch("feature", &commit, false)?;

        // Checkout once
        run(Some("feature".to_string()), false, false, false, false)?;
        assert_eq!(gateway.get_current_branch_name()?, "feature");

        // Checkout again - should work (idempotent)
        run(Some("feature".to_string()), false, false, false, false)?;
        assert_eq!(gateway.get_current_branch_name()?, "feature");

        Ok(())
    }

    #[test]
    fn test_checkout_with_empty_name() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Empty string should fail
        let result = run(Some("".to_string()), false, false, false, false);
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_checkout_trunk_flag() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Initialize with trunk
        ref_store.set_trunk("main")?;

        // Create and checkout a feature branch
        let head = repo.head()?;
        let commit = head.peel_to_commit()?;
        repo.branch("feature", &commit, false)?;
        gateway.checkout_branch_worktree_safe("feature")?;

        // Use --trunk flag to go back to trunk
        run(None, true, false, false, false)?;

        // Verify we're on trunk
        assert_eq!(gateway.get_current_branch_name()?, "main");

        Ok(())
    }

    #[test]
    fn test_checkout_trunk_flag_fails_if_not_initialized() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Don't initialize Diamond (no trunk set)

        // Try --trunk - should fail
        let result = run(None, true, false, false, false);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("init") || err_msg.contains("trunk"),
            "Expected error about initialization, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_checkout_with_dirty_tree_fails() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create a file and commit it
        let file_path = dir.path().join("tracked.txt");
        std::fs::write(&file_path, "original content")?;
        let mut index = repo.index()?;
        index.add_path(std::path::Path::new("tracked.txt"))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let parent = repo.head()?.peel_to_commit()?;
        let sig = git2::Signature::now("Test", "test@test.com")?;
        repo.commit(Some("HEAD"), &sig, &sig, "Add file", &tree, &[&parent])?;

        // Create a branch with different content
        let head = repo.head()?.peel_to_commit()?;
        repo.branch("feature", &head, false)?;

        // Modify file content on feature branch
        let obj = repo.revparse_single("feature")?;
        repo.checkout_tree(&obj, None)?;
        repo.set_head("refs/heads/feature")?;
        std::fs::write(&file_path, "feature content")?;
        let mut index = repo.index()?;
        index.add_path(std::path::Path::new("tracked.txt"))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let parent = repo.head()?.peel_to_commit()?;
        repo.commit(Some("HEAD"), &sig, &sig, "Change file", &tree, &[&parent])?;

        // Go back to main
        let obj = repo.revparse_single("main")?;
        repo.checkout_tree(&obj, None)?;
        repo.set_head("refs/heads/main")?;

        // Create dirty working tree (modify the file without committing)
        std::fs::write(&file_path, "dirty content")?;

        // Try to checkout feature - should fail due to dirty tree
        let result = run(Some("feature".to_string()), false, false, false, false);
        assert!(result.is_err(), "Checkout should fail with dirty tree");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("uncommitted changes"),
            "Error should mention uncommitted changes: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_checkout_preserves_untracked_files() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create and commit a file on main
        let tracked_file = dir.path().join("a.txt");
        std::fs::write(&tracked_file, "tracked content")?;
        let mut index = repo.index()?;
        index.add_path(std::path::Path::new("a.txt"))?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let parent = repo.head()?.peel_to_commit()?;
        let sig = git2::Signature::now("Test", "test@test.com")?;
        repo.commit(Some("HEAD"), &sig, &sig, "Add a.txt", &tree, &[&parent])?;

        // Create feature branch
        gateway.create_branch("feature")?;

        // Create an untracked file on feature
        let untracked_file = dir.path().join("b.txt");
        std::fs::write(&untracked_file, "untracked content")?;

        // Verify untracked file exists
        assert!(untracked_file.exists(), "Untracked file should exist before checkout");

        // Checkout main - this should NOT delete the untracked file
        run(Some("main".to_string()), false, false, false, false)?;

        // Verify we're on main
        assert_eq!(gateway.get_current_branch_name()?, "main");

        // CRITICAL: Verify untracked file still exists
        assert!(
            untracked_file.exists(),
            "Untracked file 'b.txt' was deleted during checkout! This is a CATASTROPHIC bug."
        );

        // Verify content is preserved
        let content = std::fs::read_to_string(&untracked_file)?;
        assert_eq!(content, "untracked content", "Untracked file content was modified");

        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn test_checkout_branch_in_worktree_fails_with_helpful_message() -> Result<()> {
        use std::process::Command;

        let dir = tempdir()?;
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        std::fs::create_dir_all(&main_path)?;
        let _repo = init_test_repo(&main_path)?;

        // Create a worktree with a branch
        Command::new("git")
            .args(["worktree", "add", wt_path.to_str().unwrap(), "-b", "locked-branch"])
            .current_dir(&main_path)
            .output()
            .expect("git worktree add failed");

        // Change to main worktree directory (required for git worktree commands)
        std::env::set_current_dir(&main_path)?;

        // Create test context after changing directory
        let _ctx = TestRepoContext::new(&main_path);

        let result = run(Some("locked-branch".to_string()), false, false, false, false);

        // Should fail with a clear, informative error
        assert!(
            result.is_err(),
            "Checkout should fail when branch is in another worktree"
        );
        let err_msg = result.unwrap_err().to_string();

        // Verify error message shows the problem clearly
        assert!(
            err_msg.contains("already checked out at"),
            "Error should explain the problem: {}",
            err_msg
        );

        // Verify it shows the actual path (not a placeholder, no extra command needed)
        // On Windows, paths may differ after canonicalization, so we compare canonicalized+displayed paths
        let expected_path = format!("{}", DisplayPath(&wt_path.canonicalize()?));
        assert!(
            err_msg.contains(&expected_path),
            "Error should show the actual worktree path.\nExpected to contain: {}\nActual error: {}",
            expected_path,
            err_msg
        );

        // Verify it doesn't assume user wants to delete (most users have persistent worktrees)
        assert!(
            !err_msg.contains("remove"),
            "Error should not assume user wants to delete the worktree: {}",
            err_msg
        );

        // Verify it's informative without making assumptions about intent
        assert!(
            !err_msg.contains("git worktree list"),
            "Error should not make user run another command for info we already have: {}",
            err_msg
        );

        Ok(())
    }
}
