use anyhow::Result;
use colored::Colorize;
use std::collections::HashSet;

use crate::cache::Cache;
use crate::ui;
use crate::commands::cleanup::{cleanup_merged_branches_for_sync_async, find_merged_prs_async};
use crate::context::ExecutionContext;
use crate::forge::get_async_forge;
use crate::git_gateway::GitGateway;
use crate::operation_log::{Operation, OperationRecorder};
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::stack_viz::update_stack_visualization_async;
use crate::state::{acquire_operation_lock, OperationState, OperationType};
use crate::validation::repair_orphaned_branches;
use crate::worktree;

/// Tracks the outcome of syncing branches for progressive disclosure output
#[derive(Default)]
pub(crate) struct SyncOutcome {
    /// Branches that were actually rebased
    rebased: Vec<String>,
    /// Branches that were already in sync (no rebase needed)
    already_in_sync: Vec<String>,
    /// Branch that had conflicts (if any) - stops processing
    conflict_branch: Option<String>,
    /// Branches that were skipped due to conflicts (branch_name, reason)
    skipped_branches: Vec<(String, String)>,
}

impl SyncOutcome {
    fn any_work_done(&self) -> bool {
        !self.rebased.is_empty()
    }

    fn total_branches(&self) -> usize {
        self.rebased.len() + self.already_in_sync.len()
    }
}

/// Sync stacks by rebasing onto updated trunk (default: restack after sync)
pub async fn run(
    continue_sync: bool,
    abort: bool,
    force: bool,
    no_cleanup: bool,
    restack: bool,
    verbose: bool,
) -> Result<()> {
    // Handle abort
    if abort {
        return handle_abort();
    }

    // Handle continue
    if continue_sync {
        // Get the branches from state before continuing (for viz update)
        let synced_branches = OperationState::load()?
            .map(|s| s.all_branches.clone())
            .unwrap_or_default();

        // Continue syncing - returns whether any rebasing was performed
        let any_rebased = handle_continue()?;

        // Update viz only if rebasing actually happened
        if any_rebased && !synced_branches.is_empty() {
            let ref_store = RefStore::new()?;
            if let Err(e) = update_stack_visualization_for_sync_async(&synced_branches, &ref_store).await {
                ui::warning(&format!("Could not update stack visualization in PRs: {}", e));
            }
        }
        return Ok(());
    }

    // Acquire exclusive lock to prevent concurrent Diamond operations
    let _lock = acquire_operation_lock()?;

    // Start fresh sync
    run_sync(force, no_cleanup, restack, verbose).await
}

/// Handle dm sync --abort (delegates to general abort logic)
fn handle_abort() -> Result<()> {
    let gateway = GitGateway::new()?;

    // Check if there's an operation in progress
    let state = OperationState::load()?;
    if state.is_none() {
        anyhow::bail!("No operation in progress to abort.");
    }

    let state = state.unwrap();

    // Verify it's a sync operation
    if state.operation_type != OperationType::Sync {
        anyhow::bail!(
            "A {} is in progress, not a sync. Use '{} abort' to abort any operation.",
            match state.operation_type {
                OperationType::Sync => "sync",
                OperationType::Restack => "restack",
                OperationType::Move => "move",
                OperationType::Insert => "insert",
            },
            program_name()
        );
    }

    // Abort any git rebase in progress
    if gateway.rebase_in_progress()? {
        gateway.rebase_abort()?;
    }

    // Return to original branch
    gateway.checkout_branch_worktree_safe(&state.original_branch)?;

    // Clear operation state
    OperationState::clear()?;

    ui::success_bold("Sync aborted");
    Ok(())
}

/// Handle dm sync --continue (delegates to general continue logic)
/// Returns Ok(true) if any rebasing was performed, Ok(false) if all branches were already rebased
fn handle_continue() -> Result<bool> {
    let gateway = GitGateway::new()?;

    // Check if there's an operation in progress
    let state = OperationState::load()?;
    if state.is_none() {
        anyhow::bail!(
            "No operation in progress. Run '{} sync' to start a new sync.",
            program_name()
        );
    }

    let mut state = state.unwrap();

    // Verify it's a sync operation
    if state.operation_type != OperationType::Sync {
        anyhow::bail!(
            "A {} is in progress, not a sync. Use '{} continue' to continue any operation.",
            match state.operation_type {
                OperationType::Sync => "sync",
                OperationType::Restack => "restack",
                OperationType::Move => "move",
                OperationType::Insert => "insert",
            },
            program_name()
        );
    }

    // If there's a rebase in progress, continue it
    if gateway.rebase_in_progress()? && gateway.rebase_continue()?.has_conflicts() {
        ui::warning(&format!(
            "Conflicts remain. Resolve them and run '{} continue'",
            program_name()
        ));
        return Ok(false);
    }

    // Continue with remaining branches
    // Note: no_cleanup=true because cleanup was already handled (or skipped) in initial sync
    let ref_store = RefStore::new()?;
    let outcome = continue_sync_from_state(&mut state, &ref_store, true, false)?;
    Ok(outcome.any_work_done())
}

/// Dry-run preview of sync operation
fn run_sync_dry_run(ref_store: &RefStore) -> Result<()> {
    let trunk = ref_store.require_trunk()?;

    // Find all branches that would be rebased (roots are branches whose parent is trunk)
    let all_branches = ref_store.collect_branches_dfs(std::slice::from_ref(&trunk))?;
    let roots: Vec<String> = all_branches
        .iter()
        .filter(|name| {
            if let Ok(Some(parent)) = ref_store.get_parent(name) {
                parent == trunk && name.as_str() != trunk
            } else {
                false
            }
        })
        .cloned()
        .collect();

    if roots.is_empty() {
        println!("{} No branches to sync", "[preview]".yellow().bold());
        return Ok(());
    }

    // Collect all branches in DFS order from roots
    let branches_to_rebase = ref_store.collect_branches_dfs(&roots)?;

    let gateway = GitGateway::new()?;
    println!("{} Dry run - would perform:", "[preview]".yellow().bold());
    println!("  • Fetch from {}", gateway.remote());
    println!("  • Update {} to latest", trunk.green());
    println!("  • Rebase {} branches:", branches_to_rebase.len().to_string().yellow());
    for b in &branches_to_rebase {
        let parent = ref_store.get_parent(b)?.unwrap_or_else(|| trunk.clone());
        println!("    - {} onto {}", b.green(), parent.blue());
    }
    println!("  • Update stack visualization in PRs");
    println!();
    println!("{} No changes made (dry-run mode)", "✓".green().bold());

    Ok(())
}

/// Start a fresh sync operation
async fn run_sync(force: bool, no_cleanup: bool, restack: bool, verbose: bool) -> Result<()> {
    let gateway = GitGateway::new()?;

    // Check for staged or modified changes (allow untracked files)
    if gateway.has_staged_or_modified_changes()? {
        anyhow::bail!(
            "Cannot sync with staged or modified changes.\n\
            Commit or stash your changes first:\n\
            • git add -A && git commit -m \"WIP\"\n\
            • git stash\n\
            \n\
            Note: Untracked files are OK and will be preserved."
        );
    }

    let original_branch = gateway.get_current_branch_name()?;
    let ref_store = RefStore::new()?;

    // Handle dry-run mode
    if ExecutionContext::is_dry_run() {
        return run_sync_dry_run(&ref_store);
    }

    // Verify we have a trunk
    let trunk = ref_store.require_trunk()?;

    let spin = ui::spinner(&format!("Fetching from {}...", gateway.remote()));
    match gateway.fetch_origin() {
        Ok(()) => ui::spinner_success(spin, &format!("Fetched from {}", gateway.remote())),
        Err(e) => {
            // Non-fatal: might not have remote configured or SSH auth issues
            ui::spinner_warning(spin, &format!("Could not fetch from {}: {}", gateway.remote(), e));
            ui::bullet_step("Continuing with local branches...");
        }
    }

    // Note: We intentionally do NOT fetch diamond refs here.
    // Refs travel with branches (pushed on submit, fetched on checkout).
    // This prevents overwrites when collaborators independently reparent after merges.

    // Try to fast-forward trunk
    let spin = ui::spinner(&format!("Updating {}...", ui::print_branch(&trunk)));
    match gateway.fast_forward_branch(&trunk) {
        Ok(()) => ui::spinner_success(spin, &format!("{} is up to date", trunk)),
        Err(e) => {
            // Non-fatal: trunk might have local changes or no remote
            ui::spinner_warning(spin, &format!("Could not fast-forward {}: {}", trunk, e));
        }
    }

    // Detect and fix orphaned branches BEFORE collecting the branch tree
    // This handles the case where a parent branch was merged/deleted via GitHub
    // and child branches are now "orphaned" (parent doesn't exist in git)
    repair_orphaned_branches(&gateway, &ref_store, &trunk)?;

    // Find all branches that need rebasing (roots are branches whose parent is trunk)
    let all_branches = ref_store.collect_branches_dfs(std::slice::from_ref(&trunk))?;
    let roots: Vec<String> = all_branches
        .iter()
        .filter(|name| {
            if let Ok(Some(parent)) = ref_store.get_parent(name) {
                parent == trunk && name.as_str() != trunk
            } else {
                false
            }
        })
        .cloned()
        .collect();

    if roots.is_empty() {
        ui::success_bold("No branches to sync");
        gateway.checkout_branch_worktree_safe(&original_branch)?;
        return Ok(());
    }

    // Collect all branches in parent-first order
    let mut branches_to_rebase = ref_store.collect_branches_dfs(&roots)?;

    // Validate that all branches actually exist in git
    for branch in &branches_to_rebase {
        if !gateway.branch_exists(branch)? {
            anyhow::bail!(
                "Cannot sync: branch '{}' is tracked but doesn't exist in git.\n\
                 Run '{} doctor --fix' to clean up metadata.",
                branch,
                program_name()
            );
        }
    }

    // Load cache (needed for cleanup to update base_sha for reparented branches)
    let mut cache = Cache::load().unwrap_or_default();

    // Check PR status and cleanup merged branches BEFORE sync from remote
    // This handles squash-merged PRs that git can't detect
    // Uses async for parallel PR status checks
    if !no_cleanup {
        match get_async_forge(None) {
            Ok(forge) => {
                if let Err(e) = forge.check_auth() {
                    ui::warning(&format!(
                        "Skipping PR cleanup: not authenticated ({}). Run 'gh auth status' to check.",
                        e
                    ));
                } else {
                    let merged_prs = find_merged_prs_async(forge.as_ref(), &branches_to_rebase).await;
                    if !merged_prs.is_empty() {
                        // Prompt for which branches to delete (batch selection)
                        // Unless --force is set, in which case delete all
                        let branches_to_delete = if force {
                            // Force mode: delete all merged branches without prompting
                            merged_prs.iter().map(|(branch, _)| branch.clone()).collect::<Vec<_>>()
                        } else {
                            // Interactive mode: prompt user for batch selection
                            match ui::select_branches_for_cleanup(&merged_prs) {
                                Ok(selected) => selected,
                                Err(_) => {
                                    // Non-TTY environment - bail with helpful error
                                    anyhow::bail!(
                                        "Found {} merged PR(s) but cannot prompt in non-interactive mode.\n\
                                        Use --force to automatically delete all merged branches.",
                                        merged_prs.len()
                                    );
                                }
                            }
                        };

                        // If user chose to delete some branches, proceed with cleanup
                        if !branches_to_delete.is_empty() {
                            // Filter merged_prs to only include selected branches
                            let filtered_prs: Vec<(String, crate::forge::PrInfo)> = merged_prs
                                .into_iter()
                                .filter(|(branch, _)| branches_to_delete.contains(branch))
                                .collect();

                            ui::step(&format!("Cleaning up {} merged PR(s):", filtered_prs.len()));
                            let deleted = cleanup_merged_branches_for_sync_async(
                                &gateway,
                                &ref_store,
                                &mut cache,
                                &trunk,
                                &filtered_prs,
                                Some(forge.as_ref()),
                            )
                            .await?;

                            // Recalculate branches to rebase after cleanup
                            if !deleted.is_empty() {
                                // Remove deleted branches from the list
                                branches_to_rebase.retain(|b| !deleted.contains(b));
                                println!();
                            }
                        } else {
                            ui::step("Skipped cleanup (no branches selected)");
                        }
                    }
                }
            }
            Err(_) => {
                // No forge configured - this is normal for local-only usage
            }
        }
    }

    // If no branches left after cleanup, we're done
    if branches_to_rebase.is_empty() {
        ui::success_bold("Sync complete! All branches were merged.");
        // Return to original branch if it still exists, otherwise stay on trunk
        if gateway.branch_exists(&original_branch)? {
            gateway.checkout_branch_worktree_safe(&original_branch)?;
        } else {
            ui::step(&format!(
                "Original branch '{}' was merged, staying on {}",
                original_branch, trunk
            ));
            gateway.checkout_branch_worktree_safe(&trunk)?;
        }
        return Ok(());
    }

    // If --no-restack was specified, stop here (cleanup only, no rebasing)
    if !restack {
        ui::success_bold("Sync complete (cleanup only)");
        // Return to original branch if it still exists, otherwise stay on trunk
        if gateway.branch_exists(&original_branch)? {
            gateway.checkout_branch_worktree_safe(&original_branch)?;
        } else {
            ui::step(&format!(
                "Original branch '{}' was deleted, staying on {}",
                original_branch, trunk
            ));
            gateway.checkout_branch_worktree_safe(&trunk)?;
        }
        return Ok(());
    }

    // Check for worktree conflicts before starting any rebase operations
    worktree::check_branches_for_worktree_conflicts(&branches_to_rebase)?;

    // Sync each branch from remote silently (P1-02: collaborative sync)
    // Note: Divergence is common after parent merges (GitHub rebases child PRs).
    // We don't warn because the rebase below will reconcile the differences.
    // User will push with --force after sync to update remote.
    for branch in &branches_to_rebase {
        // Sync silently - errors are fatal, success is silent
        gateway.sync_branch_from_remote(branch, force)?;
    }

    // Note: We no longer block on "external changes" detection.
    // Changes to branches (even outside Diamond) will be rebased onto the correct parent.
    // The old warning caused confusion after parent merges and normal development work.

    // Create backup refs for all branches silently BEFORE starting
    let recorder = OperationRecorder::new()?;
    for branch in &branches_to_rebase {
        let backup = gateway.create_backup_ref(branch)?;
        // Log backup creation (silent - only recorded for recovery)
        recorder.record(Operation::BackupCreated {
            branch: branch.clone(),
            backup_ref: backup.ref_name.clone(),
        })?;
    }

    // Log sync start
    recorder.record(Operation::SyncStarted {
        branches: branches_to_rebase.clone(),
    })?;

    // Create operation state for sync and save immediately
    // This ensures we can recover even if crash happens before first rebase
    let synced_branches = branches_to_rebase.clone(); // Keep for stack viz update
    let mut state = OperationState::new_sync(original_branch.clone(), branches_to_rebase.clone());
    state.save()?;

    // Start rebasing - returns outcome tracking what was done
    let outcome = continue_sync_from_state(&mut state, &ref_store, no_cleanup, verbose)?;

    // Log sync completion
    recorder.record(Operation::SyncCompleted {
        branches: state.remaining_branches.clone(),
        success: outcome.conflict_branch.is_none(),
    })?;

    // Update stack visualization in PRs only if rebasing actually happened
    // Skip if all branches were already rebased (nothing changed)
    if outcome.any_work_done() {
        let ref_store = RefStore::new()?;
        if let Err(e) = update_stack_visualization_for_sync_async(&synced_branches, &ref_store).await {
            ui::warning(&format!("Could not update stack visualization in PRs: {}", e));
        }
    }

    Ok(())
}

/// Continue syncing from saved state
/// This is public so it can be called from the standalone continue command
/// Returns SyncOutcome tracking which branches were rebased vs already in sync
pub fn continue_sync_from_state(
    state: &mut OperationState,
    ref_store: &RefStore,
    _no_cleanup: bool,
    verbose: bool,
) -> Result<SyncOutcome> {
    let gateway = GitGateway::new()?;
    let mut cache = Cache::load().unwrap_or_default();

    // Re-run repair in case state changed since crash
    let trunk = ref_store.require_trunk()?;
    repair_orphaned_branches(&gateway, ref_store, &trunk)?;

    let mut outcome = SyncOutcome::default();
    let total = state.all_branches.len();
    let mut processed = total - state.remaining_branches.len();

    // Determine "current stack" for stack-aware conflict handling
    // If on trunk: no stack concept (empty set) - all branches are "other"
    // If on feature branch: collect full dependency tree (ancestors + current + descendants)
    let current_stack_branches: HashSet<String> = if state.original_branch == trunk {
        HashSet::new() // On trunk: no stack, all branches are unrelated
    } else {
        // On feature branch: find stack root and collect all branches in the tree
        let stack_root = find_stack_root(&state.original_branch, ref_store);
        ref_store
            .collect_branches_dfs(&[stack_root])
            .unwrap_or_default()
            .into_iter()
            .collect()
    };

    // Create progress tracker (shows progress bar for 10+ branches, spinners for <10)
    let mut sync_progress = ui::SyncProgress::new(total, "Syncing stack");

    while !state.remaining_branches.is_empty() {
        let branch = state.remaining_branches.remove(0);
        state.current_branch = Some(branch.clone());
        processed += 1;

        // Determine what to rebase onto
        let trunk = ref_store.require_trunk()?;
        let onto = ref_store.get_parent(&branch)?.unwrap_or(trunk.clone());

        // Check if branch is already rebased onto target
        if gateway.is_branch_based_on(&branch, &onto)? {
            outcome.already_in_sync.push(branch.clone());

            // Only show "already in sync" messages in verbose mode or non-progress-bar mode
            if verbose || !sync_progress.is_progress_bar_mode() {
                // Build PR display with clickable link if available
                let pr_display = if let Some(pr_url) = cache.get_pr_url(&branch) {
                    if let Some(num_str) = pr_url.rsplit('/').next() {
                        if let Ok(num) = num_str.parse::<u64>() {
                            format!(" ({})", ui::hyperlink(pr_url, &format!("PR #{}", num)))
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                if sync_progress.is_progress_bar_mode() {
                    // In progress bar mode with verbose, just print (don't interfere with bar)
                    println!(
                        "  [{}/{}] {}{} already up to date",
                        processed,
                        total,
                        ui::print_branch(&branch),
                        pr_display
                    );
                } else {
                    // Spinner mode: show as normal
                    println!(
                        "  [{}/{}] {}{} already up to date",
                        processed,
                        total,
                        ui::print_branch(&branch),
                        pr_display
                    );
                }
            }
            continue;
        }

        // Start rebase (creates spinner or updates progress bar message)
        let spin = sync_progress.start_branch(&branch);

        // CHECKPOINT: Save state BEFORE rebase (crash recovery)
        state.save()?;

        // Attempt rebase using --fork-point for better handling of merged parents.
        // When a parent branch is merged, --fork-point uses the reflog to find
        // the correct fork point, avoiding conflicts from already-merged commits.
        // Falls back to regular rebase if reflog is unavailable.
        let rebase_result = gateway.rebase_fork_point(&branch, &onto)?;

        if rebase_result.has_conflicts() {
            // Stack-aware conflict handling: only stop if conflict is in YOUR stack
            let is_in_current_stack = current_stack_branches.contains(&branch);

            if is_in_current_stack {
                // STOP: Branch is in your dependency chain (ancestor, current, or descendant)
                // User needs to resolve this to continue working on their stack
                sync_progress.finish_branch_error(spin, &format!("Conflicts in {}", branch));
                ui::blank();

                // Show rich conflict message with stack context and conflicted files
                ui::display_conflict_message(
                    &branch,
                    &onto,
                    &state.remaining_branches,
                    ref_store,
                    &gateway,
                    false, // initial conflict
                )?;

                outcome.conflict_branch = Some(branch);
                return Ok(outcome);
            } else {
                // SKIP: Branch is in a different stack, doesn't block your work
                // Abort the rebase and continue with other branches
                gateway.rebase_abort()?;
                sync_progress.finish_branch_warning(spin, &format!("Skipped {} (conflicts)", branch));

                let reason = format!("conflicts with {}", onto);
                outcome.skipped_branches.push((branch.clone(), reason));

                // Skip all children too (dependency chain: can't rebase children if parent failed)
                let children = ref_store
                    .collect_branches_dfs(std::slice::from_ref(&branch))
                    .unwrap_or_default();
                for child in children {
                    if child != branch {
                        outcome
                            .skipped_branches
                            .push((child.clone(), "parent was skipped".to_string()));
                        state.remaining_branches.retain(|b| b != &child);
                    }
                }

                // Continue to next branch
                continue;
            }
        }

        // Build success message with clickable PR link if available
        let pr_display = if let Some(pr_url) = cache.get_pr_url(&branch) {
            // Extract PR number from URL (e.g., "https://github.com/owner/repo/pull/123" -> "123")
            if let Some(num_str) = pr_url.rsplit('/').next() {
                if let Ok(num) = num_str.parse::<u64>() {
                    // Create clickable hyperlink (OSC 8) - invisible in unsupported terminals
                    format!(" ({})", ui::hyperlink(pr_url, &format!("PR #{}", num)))
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        sync_progress.finish_branch_success(spin, &format!("Rebased {}{}", branch, pr_display));
        outcome.rebased.push(branch.clone());

        // Update base_sha after successful rebase
        cache.set_base_sha(&branch, &gateway.get_branch_sha(&branch)?);
        cache.save()?;
    }

    // All done - clean up
    state.current_branch = None;
    state.in_progress = false;
    OperationState::clear()?;

    // Finish progress bar (if in progress bar mode)
    // This ensures a clean transition to summary output
    drop(sync_progress);

    // Return to original branch if it still exists, otherwise stay on trunk
    let trunk = ref_store.require_trunk()?;
    if gateway.branch_exists(&state.original_branch)? {
        gateway.checkout_branch_worktree_safe(&state.original_branch)?;
    } else {
        ui::step(&format!(
            "Original branch '{}' was merged, staying on {}",
            state.original_branch, trunk
        ));
        gateway.checkout_branch_worktree_safe(&trunk)?;
    }

    // Record sync timestamp for staleness tracking
    let trunk_sha = gateway.get_branch_sha(&trunk).ok();
    let mut cache = Cache::load().unwrap_or_default();
    cache.record_sync(trunk_sha);
    if let Err(e) = cache.save() {
        ui::warning(&format!("Could not record sync state: {}", e));
    }

    // Print concise summary based on outcome
    ui::blank();
    if outcome.any_work_done() {
        ui::success_bold(&format!(
            "Sync complete ({} branch{} updated)",
            outcome.rebased.len(),
            if outcome.rebased.len() == 1 { "" } else { "es" }
        ));

        // Show "already in sync" count if not verbose
        if !verbose && !outcome.already_in_sync.is_empty() {
            println!("  • {} already up to date", outcome.already_in_sync.len());
        }
    } else {
        ui::success(&format!(
            "{} branch{} already in sync",
            outcome.total_branches(),
            if outcome.total_branches() == 1 { "" } else { "es" }
        ));
    }

    // Suggest --verbose if there were branches that didn't need updating
    if !verbose && !outcome.already_in_sync.is_empty() && outcome.any_work_done() {
        ui::step("Use --verbose (-v) to see all branch details");
    }

    // Show skipped branches if any (grouped by stack)
    if !outcome.skipped_branches.is_empty() {
        ui::blank();
        ui::warning(&format!(
            "{} branch{} could not be rebased:",
            outcome.skipped_branches.len(),
            if outcome.skipped_branches.len() == 1 { "" } else { "es" }
        ));

        // Group branches by their position in the stack
        // This helps users understand dependencies and what to fix first
        let mut displayed = std::collections::HashSet::new();

        for (branch, reason) in &outcome.skipped_branches {
            if displayed.contains(branch) {
                continue;
            }

            // Check if this branch has children that were also skipped
            let children: Vec<String> = ref_store
                .get_children(branch)
                .unwrap_or_default()
                .into_iter()
                .filter(|child| outcome.skipped_branches.iter().any(|(b, _)| b == child))
                .collect();

            // Display the branch with its reason
            let reason_display = if reason.contains("conflicts") {
                format!("{}", reason.red())
            } else {
                format!("{}", reason.yellow())
            };
            println!("  • {} ({})", branch.yellow(), reason_display);
            displayed.insert(branch.clone());

            // Display children that were blocked by this parent
            for child in children {
                let child_reason = outcome
                    .skipped_branches
                    .iter()
                    .find(|(b, _)| b == &child)
                    .map(|(_, r)| r.as_str())
                    .unwrap_or("blocked by parent");

                println!("    └─ {} ({})", child.bright_black(), child_reason.bright_black());
                displayed.insert(child);
            }
        }

        ui::blank();
        println!("Fix conflicts starting from the top of each stack, then run:");
        println!(
            "  {} checkout <branch> && {} restack --continue",
            ui::print_cmd(program_name()),
            ui::print_cmd(program_name())
        );
    }

    // Note: Stack visualization update moved to run_sync() to happen AFTER restack
    Ok(outcome)
}

/// Update stack visualization in all open PRs after sync (async version)
///
/// This version processes multiple independent stacks in parallel for better performance.
pub async fn update_stack_visualization_for_sync_async(synced_branches: &[String], ref_store: &RefStore) -> Result<()> {
    if synced_branches.is_empty() {
        return Ok(());
    }

    // Get the async forge
    let forge = match get_async_forge(None) {
        Ok(f) => f,
        Err(_) => return Ok(()), // No forge available, skip silently
    };

    // Check auth silently
    if forge.check_auth().is_err() {
        return Ok(()); // Not authenticated, skip silently
    }

    println!("\nUpdating stack visualization in PRs...");

    // Find the root of the synced stack and collect all its branches
    // (all synced branches are in the same stack)
    let first_branch = &synced_branches[0];
    let root = find_stack_root(first_branch, ref_store);
    let stack_branches = ref_store.collect_branches_dfs(std::slice::from_ref(&root))?;

    // Update just this one stack
    if let Err(e) = update_stack_visualization_async(&stack_branches, forge.as_ref(), ref_store, true).await {
        eprintln!("{} Failed to update stack starting at {}: {}", "⚠".yellow(), root, e);
    }

    Ok(())
}

/// Find the root of a stack (first branch after trunk)
fn find_stack_root(branch: &str, ref_store: &RefStore) -> String {
    let mut current = branch.to_string();
    let trunk = ref_store.get_trunk().ok().flatten();

    loop {
        if let Ok(Some(parent)) = ref_store.get_parent(&current) {
            // Check if parent is trunk
            if trunk.as_ref() == Some(&parent) {
                return current;
            }
            // Check if parent is tracked
            if ref_store.is_tracked(&parent).unwrap_or(false) {
                current = parent;
            } else {
                return current;
            }
        } else {
            return current;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    // ===== SyncOutcome Unit Tests =====

    #[test]
    fn test_sync_outcome_default_no_work_done() {
        let outcome = SyncOutcome::default();
        assert!(!outcome.any_work_done());
        assert_eq!(outcome.total_branches(), 0);
        assert!(outcome.conflict_branch.is_none());
    }

    #[test]
    fn test_sync_outcome_with_rebased_branches() {
        let outcome = SyncOutcome {
            rebased: vec!["feature-1".to_string(), "feature-2".to_string()],
            already_in_sync: vec![],
            conflict_branch: None,
            skipped_branches: vec![],
        };
        assert!(outcome.any_work_done());
        assert_eq!(outcome.total_branches(), 2);
    }

    #[test]
    fn test_sync_outcome_with_already_in_sync() {
        let outcome = SyncOutcome {
            rebased: vec![],
            already_in_sync: vec!["feature-1".to_string(), "feature-2".to_string()],
            conflict_branch: None,
            skipped_branches: vec![],
        };
        assert!(!outcome.any_work_done());
        assert_eq!(outcome.total_branches(), 2);
    }

    #[test]
    fn test_sync_outcome_mixed() {
        let outcome = SyncOutcome {
            rebased: vec!["feature-1".to_string()],
            already_in_sync: vec!["feature-2".to_string(), "feature-3".to_string()],
            conflict_branch: None,
            skipped_branches: vec![],
        };
        assert!(outcome.any_work_done());
        assert_eq!(outcome.total_branches(), 3);
    }

    #[test]
    fn test_sync_outcome_with_conflict() {
        let outcome = SyncOutcome {
            rebased: vec!["feature-1".to_string()],
            already_in_sync: vec![],
            conflict_branch: Some("feature-2".to_string()),
            skipped_branches: vec![],
        };
        assert!(outcome.any_work_done());
        assert_eq!(outcome.conflict_branch, Some("feature-2".to_string()));
    }

    // ===== Integration Tests =====

    fn create_branch(repo: &git2::Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[tokio::test]
    async fn test_sync_no_trunk_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // No trunk configured (RefStore is empty)
        let result = run(false, false, false, true, false, false).await; // restack=false, verbose=false for tests
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not initialized") || err_msg.contains("No trunk"),
            "Expected initialization error, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_sync_abort_no_operation_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // No operation in progress
        OperationState::clear().ok();

        let result = run(false, true, false, true, false, false).await; // restack=false, verbose=false for tests
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No operation in progress"));
    }

    #[tokio::test]
    async fn test_sync_continue_no_operation_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // No operation in progress
        OperationState::clear().ok();

        let result = run(true, false, false, true, false, false).await; // restack=false, verbose=false for tests
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No operation in progress"));
    }

    #[test]
    fn test_collect_branches_dfs_simple() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create git branches before setting parent relationships
        create_branch(&repo, "feature-1").unwrap();
        create_branch(&repo, "feature-2").unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();
        ref_store.set_parent("feature-2", "feature-1").unwrap();

        let result = ref_store.collect_branches_dfs(&["feature-1".to_string()]).unwrap();

        // DFS from feature-1 should include feature-2
        assert!(result.contains(&"feature-2".to_string()));
    }

    #[test]
    fn test_collect_branches_dfs_multiple_children() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create git branches before setting parent relationships
        create_branch(&repo, "feature-1").unwrap();
        create_branch(&repo, "feature-a").unwrap();
        create_branch(&repo, "feature-b").unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();
        ref_store.set_parent("feature-a", "feature-1").unwrap();
        ref_store.set_parent("feature-b", "feature-1").unwrap();

        let result = ref_store.collect_branches_dfs(&["feature-1".to_string()]).unwrap();

        // Children should be present
        assert!(result.contains(&"feature-a".to_string()));
        assert!(result.contains(&"feature-b".to_string()));
    }

    #[test]
    fn test_operation_state_save_load_for_sync() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git").join("diamond")).unwrap();

        let mut state = OperationState::new_sync(
            "main".to_string(),
            vec!["feature-2".to_string(), "feature-3".to_string()],
        );
        state.current_branch = Some("feature-1".to_string());

        state.save_to(dir.path()).unwrap();

        let loaded = OperationState::load_from(dir.path()).unwrap();
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert!(loaded.in_progress);
        assert_eq!(loaded.operation_type, OperationType::Sync);
        assert_eq!(loaded.current_branch, Some("feature-1".to_string()));
        assert_eq!(loaded.remaining_branches, vec!["feature-2", "feature-3"]);
        assert_eq!(loaded.original_branch, "main");
    }

    #[test]
    fn test_operation_state_clear_for_sync() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git").join("diamond")).unwrap();

        let state = OperationState::new_sync("main".to_string(), vec![]);

        state.save_to(dir.path()).unwrap();

        // Verify it was saved
        assert!(OperationState::load_from(dir.path()).unwrap().is_some());

        // Clear it
        OperationState::clear_from(dir.path()).unwrap();

        // Verify it's gone
        assert!(OperationState::load_from(dir.path()).unwrap().is_none());
    }

    #[tokio::test]
    async fn test_sync_abort_wrong_operation_type() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create a restack operation (not sync)
        let state = OperationState::new_restack("main".to_string(), vec!["feature-1".to_string()]);
        state.save().unwrap();

        let result = run(false, true, false, true, false, false).await; // restack=false, verbose=false for tests
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("restack is in progress, not a sync"));
    }

    #[tokio::test]
    async fn test_sync_continue_wrong_operation_type() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create a move operation (not sync)
        let state = OperationState::new_move(
            "feature".to_string(),
            vec!["feature".to_string()],
            "develop".to_string(),
            None, // old_parent
        );
        state.save().unwrap();

        let result = run(true, false, false, true, false, false).await; // restack=false, verbose=false for tests
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("move is in progress, not a sync"));
    }

    #[tokio::test]
    async fn test_sync_cleans_up_missing_tracked_branch() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create one real branch
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-1", &main_commit, false).unwrap();

        // Set up refs with feature-1 having a non-existent child
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();
        ref_store.set_parent("missing-branch", "feature-1").unwrap();

        // Try to sync - should auto-clean the stale ref for missing-branch
        let result = run_sync(false, true, false, false).await; // restack=false, verbose=false for tests
        assert!(result.is_ok(), "Sync should succeed after cleaning up stale refs");

        // Verify the stale ref was cleaned up
        let tracked = ref_store.list_tracked_branches().unwrap();
        assert!(
            !tracked.contains(&"missing-branch".to_string()),
            "missing-branch should have been cleaned up"
        );
        assert!(
            tracked.contains(&"feature-1".to_string()),
            "feature-1 should still be tracked"
        );
    }

    #[test]
    fn test_find_stack_root_finds_trunk_child() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create git branches before setting parent relationships
        create_branch(&repo, "feature-1").unwrap();
        create_branch(&repo, "feature-2").unwrap();
        create_branch(&repo, "feature-3").unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();
        ref_store.set_parent("feature-2", "feature-1").unwrap();
        ref_store.set_parent("feature-3", "feature-2").unwrap();

        // From any branch in the stack, root should be feature-1
        assert_eq!(find_stack_root("feature-3", &ref_store), "feature-1");
        assert_eq!(find_stack_root("feature-2", &ref_store), "feature-1");
        assert_eq!(find_stack_root("feature-1", &ref_store), "feature-1");
    }

    #[test]
    fn test_collect_stack_from_root_gets_all_descendants() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create git branches before setting parent relationships
        create_branch(&repo, "feature-1").unwrap();
        create_branch(&repo, "feature-2").unwrap();
        create_branch(&repo, "feature-3").unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();
        ref_store.set_parent("feature-2", "feature-1").unwrap();
        ref_store.set_parent("feature-3", "feature-1").unwrap(); // branching

        let stack = ref_store.collect_branches_dfs(&["feature-1".to_string()]).unwrap();
        assert_eq!(stack.len(), 3);
        assert!(stack.contains(&"feature-1".to_string()));
        assert!(stack.contains(&"feature-2".to_string()));
        assert!(stack.contains(&"feature-3".to_string()));
    }

    #[test]
    fn test_synced_branches_finds_full_stack() {
        // Tests that given synced branches, we find and collect the entire stack
        // This mirrors the logic in update_stack_visualization_for_sync_async
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create git branches before setting parent relationships
        create_branch(&repo, "feature-1").unwrap();
        create_branch(&repo, "feature-2").unwrap();
        create_branch(&repo, "feature-3").unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();
        ref_store.set_parent("feature-2", "feature-1").unwrap();
        ref_store.set_parent("feature-3", "feature-2").unwrap();

        // Simulate synced branches (subset of the stack)
        let synced_branches = ["feature-2".to_string(), "feature-3".to_string()];

        // Apply the logic from update_stack_visualization_for_sync_async:
        // Find root from first synced branch and collect full stack
        let first_branch = &synced_branches[0];
        let root = find_stack_root(first_branch, &ref_store);
        let stack_branches = ref_store.collect_branches_dfs(std::slice::from_ref(&root)).unwrap();

        // Root should be feature-1 (first branch after trunk)
        assert_eq!(root, "feature-1");

        // Stack should contain all 3 branches
        assert_eq!(stack_branches.len(), 3);
        assert!(stack_branches.contains(&"feature-1".to_string()));
        assert!(stack_branches.contains(&"feature-2".to_string()));
        assert!(stack_branches.contains(&"feature-3".to_string()));
    }

    #[test]
    fn test_continue_sync_returns_false_when_already_rebased() {
        // Tests that continue_sync_from_state returns Ok(false) when no rebasing is needed
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches that are already correctly based
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-1", &main_commit, false).unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();

        // Create operation state with feature-1 to sync
        let mut state = OperationState::new_sync("main".to_string(), vec!["feature-1".to_string()]);

        // Run continue_sync - outcome should show no work done because feature-1 is already based on main
        let outcome = continue_sync_from_state(&mut state, &ref_store, true, false).unwrap();
        assert!(
            !outcome.any_work_done(),
            "Expected no work done when branch is already rebased"
        );
        assert_eq!(outcome.already_in_sync.len(), 1);
        assert!(outcome.already_in_sync.contains(&"feature-1".to_string()));
    }

    // ===== Crash Recovery Tests (formerly covered by Journal) =====

    #[test]
    fn test_remaining_branches_updated_during_processing() {
        // Verifies that remaining_branches is correctly decremented as branches are processed
        // This tests the core state update logic that Journal's mark_branch_completed tested
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-1", &main_commit, false).unwrap();
        repo.branch("feature-2", &main_commit, false).unwrap();
        repo.branch("feature-3", &main_commit, false).unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();
        ref_store.set_parent("feature-2", "feature-1").unwrap();
        ref_store.set_parent("feature-3", "feature-2").unwrap();

        // Create state with 3 branches
        let mut state = OperationState::new_sync(
            "main".to_string(),
            vec![
                "feature-1".to_string(),
                "feature-2".to_string(),
                "feature-3".to_string(),
            ],
        );

        // Verify initial state
        assert_eq!(state.remaining_branches.len(), 3);
        assert_eq!(state.all_branches.len(), 3);

        // Process branches (they're already based correctly, so no rebase needed)
        let outcome = continue_sync_from_state(&mut state, &ref_store, true, false).unwrap();

        // After processing, remaining_branches should be empty
        assert!(
            state.remaining_branches.is_empty(),
            "remaining_branches should be empty after processing all branches"
        );

        // all_branches should still contain all 3 (for abort recovery)
        assert_eq!(
            state.all_branches.len(),
            3,
            "all_branches should preserve original list"
        );

        // All 3 branches should be in already_in_sync (since they were already based correctly)
        assert_eq!(outcome.already_in_sync.len(), 3);
    }

    #[test]
    fn test_crash_recovery_preserves_partial_progress() {
        // Simulates a crash mid-operation and verifies state can be recovered
        // This is the core crash recovery scenario that Journal was designed for
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-1", &main_commit, false).unwrap();
        repo.branch("feature-2", &main_commit, false).unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();
        ref_store.set_parent("feature-2", "feature-1").unwrap();

        // Create state simulating partial progress (1 of 2 branches processed)
        let mut state = OperationState::new_sync(
            "main".to_string(),
            vec!["feature-1".to_string(), "feature-2".to_string()],
        );

        // Simulate processing first branch manually
        state.remaining_branches.remove(0); // Remove feature-1
        state.current_branch = Some("feature-2".to_string());

        // CRITICAL: Save state (this is the checkpoint that enables crash recovery)
        state.save().unwrap();

        // Simulate "crash" by dropping state and reloading
        drop(state);

        // Reload state (simulating restart after crash)
        let loaded = OperationState::load().unwrap();
        assert!(loaded.is_some(), "State should persist after save");

        let loaded = loaded.unwrap();
        assert_eq!(
            loaded.remaining_branches,
            vec!["feature-2"],
            "Partial progress should be preserved: only feature-2 remaining"
        );
        assert_eq!(
            loaded.all_branches,
            vec!["feature-1", "feature-2"],
            "Original branch list should be preserved for abort"
        );
        assert!(loaded.in_progress, "Should still be in progress");
    }

    #[test]
    fn test_state_cleared_after_successful_completion() {
        // Verifies state is properly cleared after successful sync completion
        // This replaces Journal's test_complete_removes_file
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create a simple branch
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-1", &main_commit, false).unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();

        // Create and save state
        let mut state = OperationState::new_sync("main".to_string(), vec!["feature-1".to_string()]);
        state.save().unwrap();

        // Verify state exists
        assert!(
            OperationState::load().unwrap().is_some(),
            "State should exist before processing"
        );

        // Process (will complete since branch is already based correctly)
        let _ = continue_sync_from_state(&mut state, &ref_store, true, false).unwrap();

        // State should be cleared after successful completion
        assert!(
            OperationState::load().unwrap().is_none(),
            "State should be cleared after successful completion"
        );
    }
}
