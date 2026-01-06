//! TUI log output - interactive tree view with rich features.

use anyhow::Result;
use crate::ui::{highlight_matches, render_search_box, SearchState, NO_MATCHES_MESSAGE};
use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use std::io;
use std::process::Command;

use crate::branch_tree::{
    self, find_current_branch_index, format_indent, get_commit_info, BranchDisplay, MARKER_CURRENT, MARKER_OTHER,
};
use crate::cache::Cache;
use crate::git_gateway::{BranchSyncState, GitGateway};
use crate::program_name::program_name;
use crate::ref_store::RefStore;

use super::TuiAction;

/// TUI log output - interactive tree view with rich features
pub fn run_tui(ref_store: &RefStore, current_branch: &str, gateway: &GitGateway) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run app
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
) -> Result<()> {
    // Build tree view using shared branch_tree module
    let rows = branch_tree::build_branch_tree(ref_store, current_branch, gateway)?;

    // Handle empty list
    if rows.is_empty() {
        anyhow::bail!(
            "No branches tracked. Use '{} track' to start tracking branches.",
            program_name()
        );
    }

    // Get trunk for delete check
    let trunk = ref_store.get_trunk()?;

    let mut state = ListState::default();
    // Start with current branch selected
    let current_idx = find_current_branch_index(&rows);
    state.select(Some(current_idx));

    // Initialize fuzzy search state
    let mut search_state = SearchState::new(rows.len());

    let mut pending_action = TuiAction::None;

    loop {
        // Get selected branch from FILTERED indices
        let selected_branch = state
            .selected()
            .and_then(|filtered_idx| {
                let filtered = search_state.filtered_indices();
                filtered
                    .get(filtered_idx)
                    .and_then(|&original_idx| rows.get(original_idx))
            })
            .map(|r| r.name.clone());

        terminal.draw(|f| {
            // Create layout with search box, main area, and help bar
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Search box
                    Constraint::Min(0),    // Main content
                    Constraint::Length(5), // Help bar
                ])
                .split(f.area());

            // Render search input box
            let search_widget = render_search_box(search_state.query());
            f.render_widget(search_widget, chunks[0]);

            // Split main area into list and details
            let main_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(60), // Branch list
                    Constraint::Percentage(40), // Details panel
                ])
                .split(chunks[1]);

            // Render branch list with fuzzy search
            render_branch_list(f, main_chunks[0], &rows, &mut state, &mut search_state);

            // Render details panel
            if let Some(ref branch) = selected_branch {
                render_details_panel(f, main_chunks[1], ref_store, branch, current_branch, gateway);
            }

            // Render help bar
            render_help_bar(f, chunks[2], selected_branch.as_deref(), current_branch);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    // Character input - add to search query (exclude command keys)
                    KeyCode::Char(c)
                        if !matches!(c, 'q' | 'j' | 'k' | 'g' | 'G' | 'c' | 'd' | 'u' | 'n' | 't' | 'b' | '.') =>
                    {
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
                            pending_action = TuiAction::None;
                            break;
                        } else {
                            search_state.clear();
                            search_state.filter(&rows, |branch| &branch.name);

                            // Restore selection to current branch
                            let current_idx = find_current_branch_index(&rows);
                            state.select(Some(current_idx));
                        }
                    }

                    // Quit
                    KeyCode::Char('q') => {
                        pending_action = TuiAction::None;
                        break;
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

                    // Jump to top/bottom of filtered list
                    KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::NONE) => {
                        state.select(if search_state.filtered_indices().is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                    }
                    KeyCode::Char('G') => {
                        state.select(if search_state.filtered_indices().is_empty() {
                            None
                        } else {
                            Some(search_state.filtered_indices().len().saturating_sub(1))
                        });
                    }
                    KeyCode::Home => {
                        state.select(if search_state.filtered_indices().is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                    }
                    KeyCode::End => {
                        state.select(if search_state.filtered_indices().is_empty() {
                            None
                        } else {
                            Some(search_state.filtered_indices().len().saturating_sub(1))
                        });
                    }

                    // Actions
                    KeyCode::Enter | KeyCode::Char('c') => {
                        // Checkout selected branch (or just exit if on current)
                        if let Some(branch) = selected_branch.as_ref() {
                            if branch != current_branch {
                                pending_action = TuiAction::Checkout;
                            }
                        }
                        break;
                    }
                    KeyCode::Char('d') => {
                        // Delete selected branch (if not current or trunk)
                        if let Some(branch) = selected_branch.as_ref() {
                            if branch != current_branch && trunk.as_ref() != Some(branch) {
                                pending_action = TuiAction::Delete;
                                break;
                            }
                        }
                    }
                    KeyCode::Char('u') => {
                        // Navigate up in stack
                        pending_action = TuiAction::NavigateUp;
                        break;
                    }
                    KeyCode::Char('n') => {
                        // Navigate down in stack
                        pending_action = TuiAction::NavigateDown;
                        break;
                    }
                    KeyCode::Char('t') => {
                        // Jump to top of stack
                        pending_action = TuiAction::NavigateTop;
                        break;
                    }
                    KeyCode::Char('b') => {
                        // Jump to bottom of stack
                        pending_action = TuiAction::NavigateBottom;
                        break;
                    }

                    // Jump to current branch in filtered list
                    KeyCode::Char('.') => {
                        // Find current branch in original rows
                        if let Some(original_idx) = rows.iter().position(|r| r.is_current) {
                            // Find its position in filtered indices
                            let filtered = search_state.filtered_indices();
                            if let Some(filtered_idx) = filtered.iter().position(|&idx| idx == original_idx) {
                                state.select(Some(filtered_idx));
                            }
                        }
                    }

                    _ => {}
                }
            }
        }
    }

    // Handle pending action after exiting TUI
    match pending_action {
        TuiAction::Checkout => {
            // Map filtered index to original index
            if let Some(filtered_idx) = state.selected() {
                let filtered = search_state.filtered_indices();
                if let Some(&original_idx) = filtered.get(filtered_idx) {
                    if let Some(branch) = rows.get(original_idx) {
                        println!("Checking out {}...", branch.name);
                        let _ = Command::new("dm").args(["checkout", &branch.name]).status();
                    }
                }
            }
        }
        TuiAction::Delete => {
            // Map filtered index to original index
            if let Some(filtered_idx) = state.selected() {
                let filtered = search_state.filtered_indices();
                if let Some(&original_idx) = filtered.get(filtered_idx) {
                    if let Some(branch) = rows.get(original_idx) {
                        println!("Deleting {}...", branch.name);
                        let _ = Command::new("dm").args(["delete", &branch.name]).status();
                    }
                }
            }
        }
        TuiAction::NavigateUp => {
            let _ = Command::new("dm").args(["up"]).status();
        }
        TuiAction::NavigateDown => {
            let _ = Command::new("dm").args(["down"]).status();
        }
        TuiAction::NavigateTop => {
            let _ = Command::new("dm").args(["top"]).status();
        }
        TuiAction::NavigateBottom => {
            let _ = Command::new("dm").args(["bottom"]).status();
        }
        TuiAction::None => {}
    }

    Ok(())
}

/// Render the branch list with tree visualization and fuzzy search filtering
/// Uses simple vertical format: trunk at bottom, branches above
fn render_branch_list(
    f: &mut ratatui::Frame,
    area: Rect,
    rows: &[BranchDisplay],
    state: &mut ListState,
    search_state: &mut SearchState,
) {
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

                // Use depth-based indentation with simple vertical lines
                let indent = format_indent(branch.depth);

                // Current branch marker (using shared constants)
                let marker = if branch.is_current {
                    MARKER_CURRENT
                } else {
                    MARKER_OTHER
                };

                // Needs restack indicator
                let restack_indicator = if branch.needs_restack { " (needs restack)" } else { "" };

                // Time indicator
                let time_indicator = if !branch.commit_time.is_empty() {
                    format!(" ({})", branch.commit_time)
                } else {
                    String::new()
                };

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

                // Use shared highlighting logic for branch name
                spans.extend(highlight_matches(&branch.name, match_indices, base_style));

                // Add indicators
                spans.push(Span::styled(restack_indicator, base_style));
                spans.push(Span::styled(time_indicator, base_style));

                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    // Add match count to title
    let total = rows.len();
    let shown = filtered.len();
    let title = if search_state.is_empty() {
        format!(" Stack ({} branches) ", total)
    } else {
        format!(" Stack ({} of {} branches) ", shown, total)
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

    f.render_stateful_widget(list, area, state);
}

/// Render the details panel showing branch information
fn render_details_panel(
    f: &mut ratatui::Frame,
    area: Rect,
    ref_store: &RefStore,
    branch: &str,
    current_branch: &str,
    gateway: &GitGateway,
) {
    let (hash, message, time) = get_commit_info(gateway, branch);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Branch: ", Style::default().add_modifier(Modifier::BOLD)),
            if branch == current_branch {
                Span::styled(branch, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                Span::raw(branch)
            },
        ]),
        Line::from(""),
    ];

    // Parent info
    let parent = ref_store.get_parent(branch).ok().flatten();
    let trunk = ref_store.get_trunk().ok().flatten();

    if let Some(ref parent) = parent {
        lines.push(Line::from(vec![
            Span::styled("Parent: ", Style::default().fg(Color::Cyan)),
            Span::raw(parent),
        ]));
    } else if trunk.as_deref() != Some(branch) {
        // Branch is not trunk but has no parent - likely orphan or trunk itself
        lines.push(Line::from(vec![
            Span::styled("Parent: ", Style::default().fg(Color::Cyan)),
            Span::styled("(trunk)", Style::default().fg(Color::Yellow)),
        ]));
    }

    // Children info
    let children = ref_store.get_children(branch).unwrap_or_default();
    if !children.is_empty() {
        let mut sorted_children: Vec<_> = children.into_iter().collect();
        sorted_children.sort();
        let children_str: Vec<&str> = sorted_children.iter().map(|s| s.as_str()).collect();
        lines.push(Line::from(vec![
            Span::styled("Children: ", Style::default().fg(Color::Cyan)),
            Span::raw(children_str.join(", ")),
        ]));
    }

    // Frozen status
    if let Ok(true) = ref_store.is_frozen(branch) {
        lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::Cyan)),
            Span::styled("frozen", Style::default().fg(Color::LightBlue)),
        ]));
    }

    // PR URL if available (from cache)
    let cache = Cache::load().ok();
    let pr_url = cache.as_ref().and_then(|c| c.get_pr_url(branch).map(|s| s.to_string()));

    if let Some(url) = pr_url {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("PR: ", Style::default().fg(Color::Magenta)),
            Span::raw(url),
        ]));
    }

    // Remote sync status
    let sync_status: Option<(String, Color)> = match gateway.check_remote_sync(branch) {
        Ok(BranchSyncState::InSync) => Some(("✓ in sync".to_string(), Color::Green)),
        Ok(BranchSyncState::Ahead(n)) => {
            let s = if n == 1 { "" } else { "s" };
            Some((format!("{} commit{} ahead", n, s), Color::Yellow))
        }
        Ok(BranchSyncState::Behind(n)) => {
            let s = if n == 1 { "" } else { "s" };
            Some((format!("{} commit{} behind", n, s), Color::Red))
        }
        Ok(BranchSyncState::Diverged {
            local_ahead,
            remote_ahead,
        }) => Some((
            format!("diverged (+{} local, +{} remote)", local_ahead, remote_ahead),
            Color::Red,
        )),
        Ok(BranchSyncState::NoRemote) => Some(("not pushed".to_string(), Color::DarkGray)),
        Err(_) => None,
    };

    if let Some((status, color)) = sync_status {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Remote: ", Style::default().fg(Color::Cyan)),
            Span::styled(status, Style::default().fg(color)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "─── Latest Commit ───",
        Style::default().fg(Color::DarkGray),
    )]));
    lines.push(Line::from(""));

    // Commit info
    if !hash.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Hash: ", Style::default().fg(Color::Yellow)),
            Span::raw(&hash),
        ]));
    }

    if !time.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Time: ", Style::default().fg(Color::Yellow)),
            Span::raw(&time),
        ]));
    }

    if !message.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Message: ",
            Style::default().fg(Color::Yellow),
        )]));
        lines.push(Line::from(vec![Span::raw(format!("  {}", message))]));
    }

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Details ")
                .title_style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

/// Render the help bar with context-aware shortcuts
fn render_help_bar(f: &mut ratatui::Frame, area: Rect, selected_branch: Option<&str>, current_branch: &str) {
    let is_on_current = selected_branch == Some(current_branch);

    // Line 1: Search and main actions
    let search_help = Span::styled("Type to filter  ", Style::default().fg(Color::Cyan));

    let checkout_help = if is_on_current {
        Span::styled("Enter/c: (on current)  ", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled("Enter/c: Checkout  ", Style::default().fg(Color::Cyan))
    };

    let delete_help = if is_on_current {
        Span::styled("d: (can't delete current)  ", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled("d: Delete  ", Style::default().fg(Color::Red))
    };

    let quit_help = Span::styled("Esc: Clear/Quit  ", Style::default().fg(Color::DarkGray));

    // Line 2: Navigation
    let nav_help = Span::styled("↑/↓ j/k: Navigate  ", Style::default().fg(Color::DarkGray));
    let jump_help = Span::styled("g/G: Top/Bottom  .: Current  ", Style::default().fg(Color::DarkGray));

    let stack_help = Span::styled(
        "u/n: Up/Down stack  t/b: Top/Bottom stack  ",
        Style::default().fg(Color::Yellow),
    );

    let quit_key = Span::styled("q: Quit", Style::default().fg(Color::DarkGray));

    let line1 = Line::from(vec![search_help, checkout_help, delete_help, quit_help]);
    let line2 = Line::from(vec![nav_help, jump_help, stack_help, quit_key]);

    let help = Paragraph::new(vec![line1, line2]).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Keyboard Shortcuts ")
            .title_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(help, area);
}
