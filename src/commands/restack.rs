use anyhow::Result;
use colored::Colorize;
use std::io::{self, IsTerminal, Write};

use crate::cache::Cache;
use crate::context::ExecutionContext;
use crate::forge::{get_async_forge, ReviewState};
use crate::git_gateway::GitGateway;
use crate::operation_log::{Operation, OperationRecorder};
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::state::{acquire_operation_lock, OperationState};
use crate::ui;
use crate::validation::repair_orphaned_branches;
use crate::worktree;

/// Scope of restack operation
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RestackScope {
    /// Restack all tracked branches (default when no flags)
    All,
    /// Restack only the specified branch
    Only,
    /// Restack branch and all descendants
    Upstack,
    /// Restack all ancestors down to trunk
    Downstack,
}

/// Restack branches without fetching from remote
/// This is useful after amending a parent branch and needing to update descendants
///
/// When `called_from_sync` is true, skips redundant work (backups, external change detection)
/// since sync already performed these steps.
pub async fn run(
    branch: Option<String>,
    only: bool,
    downstack: bool,
    upstack: bool,
    force: bool,
    skip_approved: bool,
    called_from_sync: bool,
) -> Result<()> {
    // Acquire exclusive lock to prevent concurrent Diamond operations
    let _lock = acquire_operation_lock()?;

    // Determine scope from flags
    let scope = if only {
        RestackScope::Only
    } else if downstack {
        RestackScope::Downstack
    } else if upstack {
        RestackScope::Upstack
    } else if branch.is_some() {
        // If branch is specified without other flags, default to upstack
        RestackScope::Upstack
    } else {
        RestackScope::All
    };

    run_restack(branch, scope, force, skip_approved, called_from_sync).await
}

async fn run_restack(
    target_branch: Option<String>,
    scope: RestackScope,
    force: bool,
    skip_approved: bool,
    called_from_sync: bool,
) -> Result<()> {
    let gateway = GitGateway::new()?;
    gateway.require_clean_for_rebase()?;

    let original_branch = gateway.get_current_branch_name()?;
    let ref_store = RefStore::new()?;

    // Verify we have a trunk
    let trunk = ref_store.require_trunk()?;

    // Detect and fix orphaned branches BEFORE collecting the branch tree
    // This handles the case where a parent branch was merged/deleted via GitHub
    // and child branches are now "orphaned" (parent doesn't exist in git)
    repair_orphaned_branches(&gateway, &ref_store, &trunk)?;

    // Determine the starting branch
    let start_branch = target_branch.unwrap_or_else(|| original_branch.clone());

    // Build the list of branches to rebase based on scope
    let mut branches_to_rebase: Vec<String> = match scope {
        RestackScope::All => {
            // Find all branches that need rebasing - collect from trunk
            let all_branches = ref_store.collect_branches_dfs(std::slice::from_ref(&trunk))?;

            // Filter to only roots (direct children of trunk) and their descendants
            let roots: Vec<String> = all_branches
                .iter()
                .filter(|name| ref_store.get_parent(name).ok().flatten().as_ref() == Some(&trunk))
                .cloned()
                .collect();

            if roots.is_empty() {
                println!("{} No branches to restack", "✓".green().bold());
                return Ok(());
            }

            // Collect all descendants in DFS order
            ref_store.collect_branches_dfs(&roots)?
        }
        RestackScope::Only => {
            // Just the specified branch
            if start_branch == trunk {
                anyhow::bail!("Cannot restack trunk branch '{}'", trunk);
            }
            vec![start_branch.clone()]
        }
        RestackScope::Upstack => {
            // The branch and all descendants
            if start_branch == trunk {
                anyhow::bail!("Cannot restack trunk branch '{}'", trunk);
            }
            ref_store.collect_branches_dfs(std::slice::from_ref(&start_branch))?
        }
        RestackScope::Downstack => {
            // All ancestors from this branch down to trunk (excluding trunk)
            if start_branch == trunk {
                anyhow::bail!("Cannot restack trunk branch '{}'", trunk);
            }
            let mut ancestors = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut current = start_branch.clone();
            while let Some(parent) = ref_store.get_parent(&current)? {
                // Cycle detection
                if !seen.insert(current.clone()) {
                    anyhow::bail!(
                        "Circular parent reference detected at '{}'. Run 'dm cleanup' to repair metadata.",
                        current
                    );
                }
                if current != trunk {
                    ancestors.push(current.clone());
                }
                if parent == trunk {
                    break;
                }
                current = parent;
            }
            // Reverse so we rebase from closest-to-trunk first
            ancestors.reverse();
            ancestors
        }
    };

    if branches_to_rebase.is_empty() {
        println!("{} No branches to restack", "✓".green().bold());
        return Ok(());
    }

    // Validate that all branches actually exist in git
    for branch in &branches_to_rebase {
        if !gateway.branch_exists(branch)? {
            anyhow::bail!(
                "Cannot restack: branch '{}' is tracked but doesn't exist in git.\n\
                 Run '{} doctor --fix' to clean up metadata.",
                branch,
                program_name()
            );
        }
    }

    // Check for worktree conflicts before starting any rebase operations
    worktree::check_branches_for_worktree_conflicts(&branches_to_rebase)?;

    // Check for approved PRs (only if not in dry-run and not forced)
    // Uses async for parallel PR status checks
    if !ExecutionContext::is_dry_run() && !force {
        let approved_branches = check_approved_prs_async(&branches_to_rebase).await;

        if !approved_branches.is_empty() {
            if skip_approved {
                // Remove approved branches from the list
                println!(
                    "{} Skipping {} branch(es) with approved PRs:",
                    "→".blue(),
                    approved_branches.len().to_string().yellow()
                );
                for (branch, approval_count) in &approved_branches {
                    println!(
                        "  {} {} ({} approval{})",
                        "↳".cyan(),
                        branch.green(),
                        approval_count,
                        if *approval_count == 1 { "" } else { "s" }
                    );
                }
                println!();

                let approved_set: std::collections::HashSet<&String> =
                    approved_branches.iter().map(|(b, _)| b).collect();
                branches_to_rebase.retain(|b| !approved_set.contains(b));

                if branches_to_rebase.is_empty() {
                    println!("{} No branches to restack (all have approved PRs)", "✓".green().bold());
                    return Ok(());
                }
            } else {
                // Warn the user and ask for confirmation
                println!(
                    "{} This will reset approvals on {} PR{}:",
                    "⚠".yellow().bold(),
                    approved_branches.len(),
                    if approved_branches.len() == 1 { "" } else { "s" }
                );
                for (branch, approval_count) in &approved_branches {
                    println!(
                        "  {} {} ({} approval{})",
                        "↳".cyan(),
                        branch.green(),
                        approval_count,
                        if *approval_count == 1 { "" } else { "s" }
                    );
                }
                println!();

                if !io::stdin().is_terminal() {
                    anyhow::bail!(
                        "Restack would reset approvals. Use --force or --skip-approved in non-interactive mode."
                    );
                }

                print!("Continue anyway? [y/N]: ");
                io::stdout().flush().ok();

                let mut input = String::new();
                io::stdin().read_line(&mut input).ok();

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("\n{} Use --skip-approved to skip approved PRs.", "Tip:".cyan().bold());
                    anyhow::bail!("Aborted.");
                }
                println!();
            }
        }
    }

    // Detect external changes (skip in dry-run since we're not making changes)
    // Also skip when called from sync - sync already modified branches, so "external changes" are expected
    if !ExecutionContext::is_dry_run() && !called_from_sync {
        let cache = Cache::load().unwrap_or_default();
        let external_changes = detect_external_changes_ref(&gateway, &ref_store, &cache, &branches_to_rebase)?;
        handle_external_changes_ref(&external_changes, force)?;
    }

    // Handle dry-run mode
    if ExecutionContext::is_dry_run() {
        println!(
            "{} Dry run - would restack {} branches:",
            "[preview]".yellow().bold(),
            branches_to_rebase.len().to_string().yellow()
        );
        for b in &branches_to_rebase {
            let parent = ref_store.get_parent(b)?.unwrap_or_else(|| trunk.clone());
            println!("  • {} onto {}", b.green(), parent.blue());
        }
        println!();
        println!("{} No changes made (dry-run mode)", "✓".green().bold());
        return Ok(());
    }

    println!(
        "{} Restacking {} branches:",
        "→".blue(),
        branches_to_rebase.len().to_string().yellow()
    );
    for b in &branches_to_rebase {
        println!("  • {}", b.green());
    }
    println!();

    // Create backup refs for all branches BEFORE starting
    // Skip when called from sync - sync already created backups
    let recorder = OperationRecorder::new()?;
    if !called_from_sync {
        println!("{} Creating backups...", "→".blue());
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
    }

    // Log restack start
    recorder.record(Operation::RestackStarted {
        branches: branches_to_rebase.clone(),
    })?;

    // Create operation state for restack and save immediately
    // This ensures we can recover even if crash happens before first rebase
    let mut state = OperationState::new_restack(original_branch.clone(), branches_to_rebase.clone());
    state.save()?;

    // Start rebasing
    let result = continue_restack_from_state(&mut state, &ref_store);

    // Log restack completion
    recorder.record(Operation::RestackCompleted {
        branches: state.remaining_branches.clone(),
        success: result.is_ok(),
    })?;

    result
}

/// Continue restacking from saved state
/// This is public so it can be called from the standalone continue command
pub fn continue_restack_from_state(state: &mut OperationState, ref_store: &RefStore) -> Result<()> {
    let gateway = GitGateway::new()?;
    let mut cache = Cache::load().unwrap_or_default();
    let trunk = ref_store.require_trunk()?;

    // Re-run repair in case state changed since crash
    repair_orphaned_branches(&gateway, ref_store, &trunk)?;

    // Calculate total branches for progress counter
    // We need to track how many branches were completed before we started
    // (in case this is a resume from conflict)
    let total_branches = state.completed_branches.len() + state.remaining_branches.len();
    let mut completed = state.completed_branches.len();

    while !state.remaining_branches.is_empty() {
        let branch = state.remaining_branches.remove(0);
        state.current_branch = Some(branch.clone());

        // Determine what to rebase onto
        let onto = ref_store.get_parent(&branch)?.unwrap_or_else(|| trunk.clone());

        // Check if branch is already rebased onto target (crash recovery)
        if gateway.is_branch_based_on(&branch, &onto)? {
            println!(
                "{} [{}/{}] {} already restacked on {}",
                "✓".green(),
                completed + 1,
                total_branches,
                branch,
                onto
            );
            completed += 1;
            state.completed_branches.push(branch);
            continue;
        }

        print!(
            "{} [{}/{}] Restacking {} on {}... ",
            "→".blue(),
            completed + 1,
            total_branches,
            branch.green(),
            onto.blue()
        );
        io::stdout().flush().ok();

        // CHECKPOINT: Save state BEFORE rebase (crash recovery)
        state.save()?;

        // Attempt rebase
        let rebase_result = gateway.rebase_onto(&branch, &onto)?;

        if rebase_result.has_conflicts() {
            // State already saved above, show rich conflict message
            println!(); // End the "Restacking..." line
            println!();

            ui::display_conflict_message(
                &branch,
                &onto,
                &state.remaining_branches,
                ref_store,
                &gateway,
                false, // initial conflict
            )?;

            return Ok(());
        }

        println!("{}", "✓".green());
        completed += 1;
        state.completed_branches.push(branch.clone());

        // Update base_sha after successful rebase
        cache.set_base_sha(&branch, &gateway.get_branch_sha(&branch)?);
        cache.save()?;
    }

    // All done - clean up
    state.current_branch = None;
    state.in_progress = false;
    OperationState::clear()?;

    // Return to original branch
    gateway.checkout_branch_worktree_safe(&state.original_branch)?;

    println!();
    println!("{} Restack complete!", "✓".green().bold());
    Ok(())
}

/// Check which branches have approved PRs (async batch version)
///
/// This version uses batch API calls for better performance with many branches.
/// Returns a list of (branch_name, approval_count) pairs.
async fn check_approved_prs_async(branches: &[String]) -> Vec<(String, usize)> {
    let forge = match get_async_forge(None) {
        Ok(f) => f,
        Err(_) => return Vec::new(), // Can't check approvals without forge
    };

    // Batch fetch all PR info in parallel
    let pr_infos = forge.get_prs_full_info(branches).await;

    // Filter to only approved PRs
    pr_infos
        .into_iter()
        .filter(|info| info.review == ReviewState::Approved)
        .map(|info| (info.head_ref.clone(), 1))
        .collect()
}

/// Restack only the children of the specified branch
/// Called after modify to auto-update descendants
pub fn restack_children(parent_branch: &str) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    // Get children of the current branch
    let children: Vec<String> = ref_store.get_children(parent_branch)?.into_iter().collect();

    if children.is_empty() {
        return Ok(()); // No children to restack
    }

    // Collect all descendants in order (children + their descendants)
    let branches_to_rebase = ref_store.collect_branches_dfs(&children)?;

    // Filter to only existing branches
    let branches_to_rebase: Vec<String> = branches_to_rebase
        .into_iter()
        .filter(|b| gateway.branch_exists(b).unwrap_or(false))
        .collect();

    if branches_to_rebase.is_empty() {
        return Ok(());
    }

    // Show what we're restacking
    for branch in &branches_to_rebase {
        print!("Restacked {} on {}.", branch.green(), parent_branch.blue());
    }
    println!();

    // Rebase each child branch onto its parent
    // We use --fork-point to correctly handle the case where the parent was amended
    for (idx, branch) in branches_to_rebase.iter().enumerate() {
        let onto = ref_store
            .get_parent(branch)?
            .unwrap_or_else(|| parent_branch.to_string());

        // Rebase using --fork-point which uses reflog to find the correct base
        // This handles the case where the parent branch has been amended
        let rebase_result = gateway.rebase_fork_point(branch, &onto)?;
        if rebase_result.has_conflicts() {
            // Calculate remaining branches
            let remaining_branches: Vec<String> = branches_to_rebase.iter().skip(idx + 1).cloned().collect();

            // Show rich conflict message
            println!();
            ui::display_conflict_message(
                branch,
                &onto,
                &remaining_branches,
                &ref_store,
                &gateway,
                false, // initial conflict
            )?;

            // Save state for continue
            let mut state = OperationState::new_restack(parent_branch.to_string(), branches_to_rebase.clone());
            state.current_branch = Some(branch.clone());
            state.remaining_branches = remaining_branches;
            state.save()?;
            return Ok(());
        }
    }

    // Return to original branch
    gateway.checkout_branch_worktree_safe(parent_branch)?;

    Ok(())
}

/// Represents a branch that was modified externally (outside of Diamond)
pub struct ExternalChangeRef {
    pub branch: String,
    pub stored_sha: String,
    pub current_sha: String,
}

/// Detect branches that have been modified since the last Diamond operation (RefStore version)
pub fn detect_external_changes_ref(
    gateway: &GitGateway,
    _ref_store: &RefStore,
    cache: &Cache,
    branches: &[String],
) -> Result<Vec<ExternalChangeRef>> {
    let mut changes = Vec::new();

    for branch in branches {
        if let Some(stored_sha) = cache.get_base_sha(branch) {
            let current_sha = gateway.get_branch_sha(branch)?;
            if stored_sha != current_sha {
                changes.push(ExternalChangeRef {
                    branch: branch.clone(),
                    stored_sha: stored_sha.to_string(),
                    current_sha,
                });
            }
        }
        // If no stored SHA, this is a legacy branch - skip detection
    }

    Ok(changes)
}

/// Handle external changes - show warning and proceed (RefStore version)
///
/// External changes are common in stacked PR workflows (e.g., rebasing via GitHub UI,
/// using git directly). Diamond warns about them but proceeds, since the user explicitly
/// invoked restack to align the stack.
pub fn handle_external_changes_ref(changes: &[ExternalChangeRef], _force: bool) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }

    // Display warning (informational only - we proceed regardless)
    println!("{} External changes detected:", "!".yellow().bold());
    for change in changes {
        println!(
            "  • {} (was {}, now {})",
            change.branch.yellow(),
            &change.stored_sha[..7],
            &change.current_sha[..7]
        );
    }
    println!();
    println!("These branches were modified outside of Diamond.");
    println!("{} Proceeding with restack...\n", "→".blue());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::{PrFullInfo, PrState, CiStatus};

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    // === Async function logic tests ===

    #[test]
    fn test_approved_pr_filtering_logic() {
        // Test the filtering logic used by check_approved_prs_async
        // Given batch PR info results, verify correct filtering to approved PRs

        let pr_infos = vec![
            PrFullInfo {
                number: 1,
                url: "url1".to_string(),
                head_ref: "branch-approved".to_string(),
                base_ref: "main".to_string(),
                state: PrState::Open,
                title: "Approved PR".to_string(),
                review: ReviewState::Approved,
                ci: CiStatus::Success,
                is_draft: false,
            },
            PrFullInfo {
                number: 2,
                url: "url2".to_string(),
                head_ref: "branch-changes-requested".to_string(),
                base_ref: "main".to_string(),
                state: PrState::Open,
                title: "Changes Requested PR".to_string(),
                review: ReviewState::ChangesRequested,
                ci: CiStatus::Success,
                is_draft: false,
            },
            PrFullInfo {
                number: 3,
                url: "url3".to_string(),
                head_ref: "branch-pending".to_string(),
                base_ref: "main".to_string(),
                state: PrState::Open,
                title: "Pending Review PR".to_string(),
                review: ReviewState::Pending,
                ci: CiStatus::Success,
                is_draft: false,
            },
        ];

        // Apply the same filtering logic as check_approved_prs_async
        let approved: Vec<(String, usize)> = pr_infos
            .into_iter()
            .filter(|info| info.review == ReviewState::Approved)
            .map(|info| (info.head_ref.clone(), 1))
            .collect();

        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].0, "branch-approved");
    }

    #[test]
    fn test_approved_pr_filtering_empty() {
        let pr_infos: Vec<PrFullInfo> = vec![];

        let approved: Vec<(String, usize)> = pr_infos
            .into_iter()
            .filter(|info| info.review == ReviewState::Approved)
            .map(|info| (info.head_ref.clone(), 1))
            .collect();

        assert!(approved.is_empty());
    }

    #[test]
    fn test_approved_pr_filtering_none_approved() {
        let pr_infos = vec![
            PrFullInfo {
                number: 1,
                url: "url".to_string(),
                head_ref: "branch-1".to_string(),
                base_ref: "main".to_string(),
                state: PrState::Open,
                title: "PR 1".to_string(),
                review: ReviewState::Pending,
                ci: CiStatus::Success,
                is_draft: false,
            },
            PrFullInfo {
                number: 2,
                url: "url".to_string(),
                head_ref: "branch-2".to_string(),
                base_ref: "main".to_string(),
                state: PrState::Open,
                title: "PR 2".to_string(),
                review: ReviewState::ChangesRequested,
                ci: CiStatus::Success,
                is_draft: false,
            },
        ];

        let approved: Vec<(String, usize)> = pr_infos
            .into_iter()
            .filter(|info| info.review == ReviewState::Approved)
            .map(|info| (info.head_ref.clone(), 1))
            .collect();

        assert!(approved.is_empty());
    }

    // === Original tests ===

    #[tokio::test]
    async fn test_restack_no_trunk_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // No trunk set
        let result = run(None, false, false, false, false, false, false).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not initialized") || err_msg.contains("No trunk"),
            "Expected initialization error, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_restack_no_branches() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Set trunk only
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        let result = run(None, false, false, false, false, false, false).await;
        assert!(result.is_ok());
    }

    fn create_branch(repo: &git2::Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[test]
    fn test_collect_branches_dfs_ref() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create git branches before setting parent relationships
        create_branch(&repo, "feature-1")?;
        create_branch(&repo, "feature-2")?;

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature-1", "main")?;
        ref_store.set_parent("feature-2", "feature-1")?;

        let result = ref_store.collect_branches_dfs(&["feature-1".to_string()])?;

        assert!(result.contains(&"feature-2".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn test_restack_cleans_up_missing_tracked_branch() {
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

        // Try to restack - should auto-clean the stale ref for missing-branch
        let result = run_restack(None, RestackScope::All, false, false, false).await;
        assert!(result.is_ok(), "Restack should succeed after cleaning up stale refs");

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

    #[tokio::test]
    async fn test_restack_only_scope() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create branches
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-1", &main_commit, false).unwrap();

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();

        // Checkout feature-1
        let obj = repo.revparse_single("feature-1").unwrap();
        repo.checkout_tree(&obj, None).unwrap();
        repo.set_head("refs/heads/feature-1").unwrap();

        // Try restack with --only flag - should restack only feature-1
        let result = run(None, true, false, false, false, false, false).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_restack_trunk_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Try to restack trunk with --only should fail
        let result = run(Some("main".to_string()), true, false, false, false, false, false).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot restack trunk"));
    }
}
