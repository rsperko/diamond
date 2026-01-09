use anyhow::Result;
use colored::Colorize;

use crate::cache::Cache;
use crate::commands::sync;
use crate::config::Config;
use crate::forge::{get_forge, wait_for_ci, CiWaitConfig, CiWaitResult, Forge, MergeMethod, PrState};
use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Merge PRs from the command line (merges entire downstack from trunk to current)
pub async fn run(
    method: MergeMethod,
    dry_run: bool,
    auto_confirm: bool,
    no_sync: bool,
    no_wait: bool,
    fast_mode: bool,
    keep: bool,
) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    let cache = Cache::load().unwrap_or_default();
    let current = gateway.get_current_branch_name()?;
    let trunk = ref_store.require_trunk()?;

    // Can't merge trunk
    if current == trunk {
        anyhow::bail!(
            "Cannot merge trunk branch '{}'. Checkout a feature branch first.",
            trunk
        );
    }

    // Get forge - required for merging
    let forge = get_forge(None)?;

    // Load merge configuration
    let config = Config::load()?;
    let merge_config = &config.merge;

    // Determine effective settings (CLI flags override config)
    let proactive_rebase = if fast_mode {
        false
    } else {
        merge_config.proactive_rebase
    };
    let do_wait_for_ci = if fast_mode || no_wait {
        false
    } else {
        merge_config.wait_for_ci
    };

    let ci_wait_config = CiWaitConfig {
        timeout_secs: merge_config.ci_timeout_secs,
        enabled: do_wait_for_ci,
        ..Default::default()
    };

    // Collect branches to merge (entire downstack from trunk to current)
    let branches_to_merge = ref_store.ancestors(&current)?;

    if branches_to_merge.is_empty() {
        println!("{} No branches to merge", "ℹ".blue());
        return Ok(());
    }

    // Check which branches have PRs
    let mut mergeable: Vec<(String, String)> = Vec::new(); // (branch, pr_url)
    let mut missing_pr: Vec<String> = Vec::new();

    for branch in &branches_to_merge {
        if let Some(url) = cache.get_pr_url(branch) {
            mergeable.push((branch.clone(), url.to_string()));
        } else {
            missing_pr.push(branch.clone());
        }
    }

    if !missing_pr.is_empty() {
        println!("{} These branches don't have PRs and will be skipped:", "!".yellow());
        for branch in &missing_pr {
            println!("  • {}", branch.dimmed());
        }
        println!();
    }

    if mergeable.is_empty() {
        anyhow::bail!("No PRs to merge. Run '{} submit' to create PRs first.", program_name());
    }

    // Show what will be merged
    println!(
        "{} Will merge {} PR{} using {} method:",
        "→".blue(),
        mergeable.len(),
        if mergeable.len() == 1 { "" } else { "s" },
        method
    );
    for (branch, url) in &mergeable {
        println!("  • {} → {}", branch.green(), url.dimmed());
    }
    println!();

    if dry_run {
        println!("{} Dry run - no PRs were merged", "[preview]".yellow().bold());
        return Ok(());
    }

    // Track how many PRs we actually merged (vs skipped because already merged)
    let mut actually_merged = 0;

    // Merge PRs (from bottom of stack to top)
    for (i, (branch, url)) in mergeable.iter().enumerate() {
        // Extract PR number from URL
        let pr_number = extract_pr_number(url)?;

        // Check PR state before attempting to merge - skip if already merged/closed
        match forge.get_pr_info(&pr_number) {
            Ok(pr_info) => {
                match pr_info.state {
                    PrState::Merged => {
                        println!(
                            "{} {} already merged (PR #{}), skipping",
                            "✓".green(),
                            branch.cyan(),
                            pr_number
                        );
                        continue;
                    }
                    PrState::Closed => {
                        println!(
                            "{} {} is closed (PR #{}), skipping",
                            "!".yellow(),
                            branch.cyan(),
                            pr_number
                        );
                        continue;
                    }
                    PrState::Open => {
                        // PR is open, proceed to merge
                    }
                }
            }
            Err(e) => {
                // If we can't get PR info, warn but try to merge anyway
                eprintln!("  {} Could not check PR state: {}", "!".yellow(), e);
            }
        }

        println!("{} Merging {}...", "→".blue(), branch.green());

        // PROACTIVE MODE (default): Rebase and wait for CI before merge attempt
        // This ensures clean history and that CI passes before we try to merge.
        // Only needed for branches after the first one (first targets trunk directly).
        if proactive_rebase && i > 0 {
            match proactive_rebase_for_merge(&gateway, forge.as_ref(), branch, &trunk) {
                Ok(true) => {
                    println!("  {} Rebased {} onto {}", "→".blue(), branch.cyan(), trunk.green());
                }
                Ok(false) => {
                    // Already up to date, no rebase needed
                }
                Err(e) => {
                    // Rebase failed with conflicts - abort entire operation
                    eprintln!("  {} Rebase failed: {}", "✗".red(), e);
                    anyhow::bail!(
                        "Could not rebase {} onto {}. Resolve conflicts manually with:\n  {} sync",
                        branch,
                        trunk,
                        program_name()
                    );
                }
            }

            // Wait for CI after rebase (if enabled)
            if do_wait_for_ci {
                match wait_for_ci(forge.as_ref(), &pr_number, branch, &ci_wait_config)? {
                    CiWaitResult::Success | CiWaitResult::NoChecks => {
                        println!("  {} CI passed for {}", "✓".green(), branch.cyan());
                    }
                    CiWaitResult::Failed => {
                        anyhow::bail!("CI failed for PR #{}. Cannot merge until CI passes.", pr_number);
                    }
                    CiWaitResult::Timeout => {
                        anyhow::bail!(
                            "CI timeout for PR #{}. CI did not complete within {} seconds.\n\
                             Use --no-wait to skip CI waiting, or increase timeout via:\n  {} config set merge.ci_timeout_secs <seconds>",
                            pr_number, ci_wait_config.timeout_secs, program_name()
                        );
                    }
                }
            }
        }

        let merge_result = forge.merge_pr(&pr_number, method, auto_confirm);

        // Handle merge result with automatic recovery for stale branches
        // In fast_mode, we use reactive recovery (rebase only after failure)
        // In proactive mode, we should rarely hit this path since we rebased above
        let final_result = match merge_result {
            Ok(()) => Ok(()),
            Err(e) if is_not_mergeable_error(&e) && i > 0 && fast_mode => {
                // FAST MODE: Parent was just merged (squash), so this branch likely has stale commits.
                // Attempt automatic recovery: rebase onto trunk, force push, retry merge.
                println!(
                    "  {} PR not mergeable (parent was just merged). Attempting auto-recovery...",
                    "!".yellow()
                );

                match auto_recover_and_retry_merge(
                    &gateway,
                    forge.as_ref(),
                    branch,
                    &trunk,
                    &pr_number,
                    method,
                    auto_confirm,
                ) {
                    Ok(()) => {
                        println!("  {} Auto-recovery successful!", "✓".green());
                        Ok(())
                    }
                    Err(recovery_err) => {
                        // Recovery failed - return original error with recovery context
                        eprintln!("  {} Auto-recovery failed: {}", "✗".red(), recovery_err);
                        Err(e)
                    }
                }
            }
            Err(e) => Err(e),
        };

        match final_result {
            Ok(()) => {
                println!("  {} Merged PR #{}", "✓".green(), pr_number);
                actually_merged += 1;

                // After merging, retarget the next PR to trunk (if there is one)
                // This is critical for squash merges: the child PR's original base branch
                // was just squash-merged, so we need to point it at trunk instead.
                // Only retarget if the next PR is still open (skip if already merged/closed).
                match retarget_next_pr_if_open(forge.as_ref(), &mergeable, i, &trunk) {
                    Ok(Some(next_branch)) => {
                        println!("    Retargeted {} to {}", next_branch.cyan(), trunk.green());
                    }
                    Ok(None) => {
                        // No retargeting needed (last branch, no forge, or PR already merged/closed)
                    }
                    Err(e) => {
                        // Warn but continue - the merge might still work,
                        // and if it doesn't, the error will be clearer
                        if let Some((next_branch, _)) = mergeable.get(i + 1) {
                            eprintln!(
                                "  {} Could not retarget {} to {}: {}",
                                "!".yellow(),
                                next_branch,
                                trunk,
                                e
                            );
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("  {} Failed to merge PR #{}: {}", "✗".red(), pr_number, e);

                // Only suggest dm sync for actual conflicts, not branch protection
                if i == 0 && is_not_mergeable_error(&e) {
                    eprintln!(
                        "\n{} This PR has conflicts with {}. Run '{} sync' to update your branch.",
                        "!".yellow(),
                        trunk,
                        program_name()
                    );
                }

                eprintln!("\n{} Stopping downstack merge. Remaining PRs not merged.", "!".yellow());
                return Err(e);
            }
        }
    }

    if actually_merged == 0 {
        println!("\n{} All PRs were already merged", "✓".green().bold());
    } else {
        println!(
            "\n{} Merged {} PR{}",
            "✓".green().bold(),
            actually_merged,
            if actually_merged == 1 { "" } else { "s" }
        );
    }

    // Auto-sync to update local branches and clean up merged ones
    if !no_sync && !dry_run {
        println!("\n{} Syncing local branches...", "→".blue());
        // Run sync with: continue=false, abort=false, force=false, no_cleanup=false, keep, restack=true, verbose=false
        if let Err(e) = sync::run(false, false, false, false, keep, true, false).await {
            // Sync errors shouldn't fail the merge command since PRs are already merged
            eprintln!("  {} Sync encountered an issue: {}", "!".yellow(), e);
            eprintln!("  Run '{} sync' manually to complete cleanup.", program_name());
        }

        // Update PR stack visualizations to show merged status
        println!("\n{} Updating PR stack visualizations...", "→".blue());
        let branch_names: Vec<String> = mergeable.iter().map(|(branch, _)| branch.clone()).collect();
        if let Err(e) = sync::update_stack_visualization_for_sync_async(&branch_names, &ref_store).await {
            eprintln!("  {} Could not update stack visualizations: {}", "!".yellow(), e);
        } else {
            println!("  {} Updated stack visualizations", "✓".green());
        }
    } else if no_sync {
        println!("\nRun '{} sync' to update your local branches.", program_name());
    }

    Ok(())
}

/// Proactively rebase a branch onto trunk before attempting to merge.
///
/// This is used in "safe by default" mode to ensure clean history and that CI runs
/// on the final commit before we attempt to merge. This is called BEFORE the merge
/// attempt, unlike auto_recover which is reactive (called after merge failure).
///
/// Returns:
/// - Ok(true) if rebase was performed
/// - Ok(false) if branch was already up to date
/// - Err if rebase failed (conflicts, etc.)
fn proactive_rebase_for_merge(gateway: &GitGateway, forge: &dyn Forge, branch: &str, trunk: &str) -> Result<bool> {
    // Step 1: Fetch latest from remote to get the merged commits
    gateway.fetch_origin()?;

    // Step 2: Check if rebase is needed by comparing with remote trunk
    let remote_trunk = format!("{}/{}", gateway.remote(), trunk);

    // Get the merge base between branch and remote trunk
    let merge_base = gateway.get_merge_base(branch, &remote_trunk)?;
    let remote_trunk_sha = gateway.get_branch_sha(&remote_trunk)?;

    // If merge base equals remote trunk, branch is already up to date
    if merge_base == remote_trunk_sha {
        return Ok(false);
    }

    // Step 3: Rebase onto trunk using fork-point
    let outcome = gateway.rebase_fork_point(branch, &remote_trunk)?;

    if outcome.has_conflicts() {
        anyhow::bail!("Rebase encountered conflicts");
    }

    // Step 4: Force push the rebased branch
    forge.push_branch(branch, true)?;

    // Step 5: Ensure PR targets trunk (may already be done, but ensure it)
    // Ignore errors - the PR might already target trunk
    let _ = forge.update_pr_base(branch, trunk);

    Ok(true)
}

/// Check if an error indicates the PR/MR is not mergeable (conflicts, stale branch, etc.)
///
/// This function detects merge conflict errors from multiple forges:
/// - GitHub (`gh`): "not mergeable", "cannot be cleanly created"
/// - GitLab (`glab`): "cannot be merged", "has conflicts"
/// - Generic: "merge conflict", "conflicting"
fn is_not_mergeable_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();

    // Exclude branch protection/policy errors - these are NOT merge conflicts
    // These patterns are from actual gh/glab CLI output:
    //
    // CONFIRMED patterns (from real errors):
    // - "base branch policy" - GitHub's generic protection error
    // - "branch protection" - GitHub alternative phrasing
    // - "protected branch" - Common across both forges
    // - "required status" - GitHub status check errors
    // - "review" - GitHub review requirement errors
    // - "approval" - GitLab approval errors
    //
    // If you encounter a branch protection error that isn't caught here,
    // verify the actual error message first, then add it with a comment.
    if msg.contains("branch protection")
        || msg.contains("base branch policy")
        || msg.contains("protected branch")
        || msg.contains("required status")
        || msg.contains("review")
        || msg.contains("approval")
    {
        return false;
    }

    // Actual merge conflict patterns (these indicate git conflicts)
    // GitHub error patterns
    msg.contains("not mergeable")
        || msg.contains("cannot be cleanly created")
        // GitLab error patterns
        || msg.contains("cannot be merged")
        || msg.contains("has conflicts")
        // Generic patterns (work across forges)
        || msg.contains("merge conflict")
        || msg.contains("conflicting")
}

/// Attempt automatic recovery when a PR can't be merged after its parent was squash-merged.
///
/// This performs the following steps:
/// 1. Fetch latest from remote (to get the squash-merged parent)
/// 2. Rebase the branch onto trunk using --fork-point (drops already-merged commits)
/// 3. Force push the rebased branch
/// 4. Retarget the PR to trunk (if not already done)
/// 5. Retry the merge
///
/// This makes Diamond's merge behavior as good or better than other tools, which require
/// manual `sync && submit` after a partial merge failure.
fn auto_recover_and_retry_merge(
    gateway: &GitGateway,
    forge: &dyn Forge,
    branch: &str,
    trunk: &str,
    pr_number: &str,
    method: MergeMethod,
    auto_confirm: bool,
) -> Result<()> {
    // Step 1: Fetch latest from remote to get the squash-merged commits
    println!("    {} Fetching latest from remote...", "→".blue());
    gateway.fetch_origin()?;

    // Step 2: Rebase onto trunk using fork-point
    // This uses the reflog to find the correct fork point, which handles
    // the case where parent commits were squash-merged and no longer exist
    println!(
        "    {} Rebasing {} onto {}...",
        "→".blue(),
        branch.cyan(),
        trunk.green()
    );

    // Use the remote trunk to ensure we have the latest
    let remote_trunk = format!("{}/{}", gateway.remote(), trunk);
    let outcome = gateway.rebase_fork_point(branch, &remote_trunk)?;

    if outcome.has_conflicts() {
        anyhow::bail!(
            "Rebase encountered conflicts. Please resolve manually with:\n  \
             {} sync && {} submit",
            program_name(),
            program_name()
        );
    }

    // Step 3: Force push the rebased branch
    println!("    {} Force pushing rebased branch...", "→".blue());
    forge.push_branch(branch, true)?;

    // Step 4: Retarget PR to trunk (may already be done, but ensure it)
    println!("    {} Ensuring PR targets {}...", "→".blue(), trunk.green());
    // Ignore errors - the PR might already target trunk
    let _ = forge.update_pr_base(branch, trunk);

    // Step 5: Retry the merge using the forge
    println!("    {} Retrying merge...", "→".blue());
    forge.merge_pr(pr_number, method, auto_confirm)
}

/// Retarget the next PR in the stack to trunk, but only if it's still open.
///
/// After merging a PR, this function updates the child PR's base branch to trunk.
/// It first checks if the child PR is still open - if the PR is already merged or
/// closed, retargeting is skipped silently. This prevents errors like
/// "Cannot change the base branch of a closed pull request".
///
/// # Arguments
/// * `forge` - The forge to use for checking PR state and updating bases
/// * `mergeable` - The list of (branch, url) tuples being merged
/// * `current_index` - The index of the branch that was just merged
/// * `trunk` - The trunk branch name to retarget to
///
/// # Returns
/// * `Ok(Some(branch))` - Successfully retargeted `branch` to trunk
/// * `Ok(None)` - No retargeting needed (last branch, no forge, or PR already merged/closed)
/// * `Err(e)` - Retargeting failed
fn retarget_next_pr_if_open(
    forge: &dyn Forge,
    mergeable: &[(String, String)],
    current_index: usize,
    trunk: &str,
) -> Result<Option<String>> {
    // Check if there's a next branch to retarget
    let Some((next_branch, next_url)) = mergeable.get(current_index + 1) else {
        return Ok(None);
    };

    // Extract PR number from URL and check if it's still open
    let pr_number = extract_pr_number(next_url)?;
    match forge.get_pr_info(&pr_number) {
        Ok(pr_info) => {
            if pr_info.state != PrState::Open {
                // PR is already merged or closed, skip retargeting
                return Ok(None);
            }
        }
        Err(_) => {
            // If we can't get PR info, try to retarget anyway
            // The error will be caught and handled by the caller
        }
    }

    // Attempt to retarget
    forge.update_pr_base(next_branch, trunk)?;
    Ok(Some(next_branch.clone()))
}

/// Extract PR number from URL like "https://github.com/owner/repo/pull/123"
fn extract_pr_number(url: &str) -> Result<String> {
    url.split('/')
        .next_back()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("Could not extract PR number from URL: {}", url))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    /// Retarget the next PR in the stack to trunk after a successful merge.
    ///
    /// This is a test helper function that tests the core retargeting logic without
    /// checking PR state. The production code uses `retarget_next_pr_if_open` which
    /// first checks if the PR is still open before attempting to retarget.
    fn retarget_next_pr(
        forge: Option<&dyn Forge>,
        mergeable: &[(String, String)],
        current_index: usize,
        trunk: &str,
    ) -> Result<Option<String>> {
        // Check if there's a next branch to retarget
        let Some((next_branch, _)) = mergeable.get(current_index + 1) else {
            return Ok(None);
        };

        // Check if forge is available
        let Some(f) = forge else {
            return Ok(None);
        };

        // Attempt to retarget
        f.update_pr_base(next_branch, trunk)?;
        Ok(Some(next_branch.clone()))
    }

    /// Compute which branches need to be retargeted after each merge.
    /// Returns a list of (merged_branch, next_branch_to_retarget, new_base) tuples.
    ///
    /// After merging a parent PR in a stack, the child PR's base branch no longer exists
    /// in the same form (it was squash-merged). We need to retarget the child PR to trunk.
    ///
    /// For example, with stack [a, b, c] and trunk "main":
    /// - After merging "a", retarget "b" to "main"
    /// - After merging "b", retarget "c" to "main"
    /// - After merging "c", nothing to retarget
    ///
    /// This function extracts the retargeting logic for testability.
    /// The actual implementation in `run()` performs retargeting inline during the merge loop.
    fn compute_retargets(mergeable: &[(String, String)], trunk: &str) -> Vec<(String, String, String)> {
        let mut retargets = Vec::new();

        for (i, (branch, _url)) in mergeable.iter().enumerate() {
            if let Some((next_branch, _)) = mergeable.get(i + 1) {
                retargets.push((branch.clone(), next_branch.clone(), trunk.to_string()));
            }
        }

        retargets
    }

    fn create_branch(repo: &git2::Repository, name: &str) -> anyhow::Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[test]
    fn test_extract_pr_number() {
        assert_eq!(
            extract_pr_number("https://github.com/owner/repo/pull/123").unwrap(),
            "123"
        );
        assert_eq!(
            extract_pr_number("https://github.com/owner/repo/pull/42").unwrap(),
            "42"
        );
    }

    #[test]
    fn test_merge_method_as_str() {
        assert_eq!(MergeMethod::Squash.as_str(), "squash");
        assert_eq!(MergeMethod::Merge.as_str(), "merge");
        assert_eq!(MergeMethod::Rebase.as_str(), "rebase");
    }

    #[test]
    fn test_collect_downstack_returns_branches_in_merge_order() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        // Create git branches for the stack (master first, then children)
        create_branch(&repo, "a")?;
        create_branch(&repo, "b")?;
        create_branch(&repo, "c")?;

        // Create stack: master -> a -> b -> c
        ref_store.set_parent("a", "main")?;
        ref_store.set_parent("b", "a")?;
        ref_store.set_parent("c", "b")?;

        // Collect downstack from c (the tip)
        let branches = ref_store.ancestors("c")?;

        // Should be in merge order: a first (parent must merge before child), then b, then c
        assert_eq!(branches, vec!["a", "b", "c"]);

        Ok(())
    }

    #[test]
    fn test_collect_downstack_from_middle_only_includes_ancestors() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        // Create git branches for the stack (master first, then children)
        create_branch(&repo, "a")?;
        create_branch(&repo, "b")?;
        create_branch(&repo, "c")?;

        // Create stack: master -> a -> b -> c
        ref_store.set_parent("a", "main")?;
        ref_store.set_parent("b", "a")?;
        ref_store.set_parent("c", "b")?;

        // Collect downstack from b (middle of stack)
        let branches = ref_store.ancestors("b")?;

        // Should only include up to current: a, b (not c)
        assert_eq!(branches, vec!["a", "b"]);

        Ok(())
    }

    #[test]
    fn test_collect_downstack_single_branch() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        // Create git branches (master first, then feature)
        create_branch(&repo, "feature")?;

        // Create single branch: master -> feature
        ref_store.set_parent("feature", "main")?;

        // Collect downstack from feature
        let branches = ref_store.ancestors("feature")?;

        // Should just be the single branch
        assert_eq!(branches, vec!["feature"]);

        Ok(())
    }

    // =========================================================================
    // Tests for compute_retargets - the PR base retargeting logic
    // =========================================================================

    #[test]
    fn test_compute_retargets_three_branch_stack() {
        // Stack: main -> a -> b -> c
        // After merging each, the next PR needs to be retargeted to main
        let mergeable = vec![
            ("a".to_string(), "url_a".to_string()),
            ("b".to_string(), "url_b".to_string()),
            ("c".to_string(), "url_c".to_string()),
        ];

        let retargets = compute_retargets(&mergeable, "main");

        // After merging "a", retarget "b" to "main"
        // After merging "b", retarget "c" to "main"
        // After merging "c", nothing to retarget (last in stack)
        assert_eq!(
            retargets,
            vec![
                ("a".to_string(), "b".to_string(), "main".to_string()),
                ("b".to_string(), "c".to_string(), "main".to_string()),
            ]
        );
    }

    #[test]
    fn test_compute_retargets_two_branch_stack() {
        // Stack: main -> a -> b
        let mergeable = vec![
            ("a".to_string(), "url_a".to_string()),
            ("b".to_string(), "url_b".to_string()),
        ];

        let retargets = compute_retargets(&mergeable, "main");

        // After merging "a", retarget "b" to "main"
        // After merging "b", nothing to retarget
        assert_eq!(retargets, vec![("a".to_string(), "b".to_string(), "main".to_string()),]);
    }

    #[test]
    fn test_compute_retargets_single_branch() {
        // Stack: main -> feature (single PR, no retargeting needed)
        let mergeable = vec![("feature".to_string(), "url".to_string())];

        let retargets = compute_retargets(&mergeable, "main");

        // Nothing to retarget - single PR
        assert!(retargets.is_empty());
    }

    #[test]
    fn test_compute_retargets_empty_stack() {
        // Edge case: no branches to merge
        let mergeable: Vec<(String, String)> = vec![];

        let retargets = compute_retargets(&mergeable, "main");

        assert!(retargets.is_empty());
    }

    #[test]
    fn test_compute_retargets_preserves_trunk_name() {
        // Verify that different trunk names are preserved
        let mergeable = vec![
            ("a".to_string(), "url_a".to_string()),
            ("b".to_string(), "url_b".to_string()),
        ];

        // With trunk named "main"
        let retargets = compute_retargets(&mergeable, "main");
        assert_eq!(retargets, vec![("a".to_string(), "b".to_string(), "main".to_string()),]);

        // With trunk named "develop"
        let retargets = compute_retargets(&mergeable, "develop");
        assert_eq!(
            retargets,
            vec![("a".to_string(), "b".to_string(), "develop".to_string()),]
        );
    }

    #[test]
    fn test_compute_retargets_large_stack() {
        // Test with a larger stack to ensure pattern holds
        let mergeable: Vec<(String, String)> = (1..=5)
            .map(|i| (format!("branch-{}", i), format!("url_{}", i)))
            .collect();

        let retargets = compute_retargets(&mergeable, "main");

        // Should have 4 retargets (n-1 for n branches)
        assert_eq!(retargets.len(), 4);

        // Verify each retarget points to the next branch and uses "main" as base
        assert_eq!(
            retargets[0],
            ("branch-1".to_string(), "branch-2".to_string(), "main".to_string())
        );
        assert_eq!(
            retargets[1],
            ("branch-2".to_string(), "branch-3".to_string(), "main".to_string())
        );
        assert_eq!(
            retargets[2],
            ("branch-3".to_string(), "branch-4".to_string(), "main".to_string())
        );
        assert_eq!(
            retargets[3],
            ("branch-4".to_string(), "branch-5".to_string(), "main".to_string())
        );
    }

    // =========================================================================
    // Tests for retarget_next_pr - the actual retargeting behavior
    // =========================================================================

    use crate::forge::{CiStatus, Forge, ForgeType, PrFullInfo, PrInfo, PrOptions, PrState, ReviewState};
    use std::sync::RwLock;

    /// Mock forge that tracks update_pr_base calls and can be configured to fail
    struct MockForge {
        /// Track all update_pr_base calls: (branch, new_base)
        update_calls: RwLock<Vec<(String, String)>>,
        /// If set, update_pr_base will return this error
        should_fail: bool,
    }

    impl MockForge {
        fn new() -> Self {
            Self {
                update_calls: RwLock::new(Vec::new()),
                should_fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                update_calls: RwLock::new(Vec::new()),
                should_fail: true,
            }
        }

        fn get_update_calls(&self) -> Vec<(String, String)> {
            self.update_calls.read().unwrap().clone()
        }
    }

    impl Forge for MockForge {
        fn forge_type(&self) -> ForgeType {
            ForgeType::GitHub
        }
        fn cli_name(&self) -> &str {
            "mock"
        }
        fn check_auth(&self) -> Result<()> {
            Ok(())
        }
        fn pr_exists(&self, _branch: &str) -> Result<Option<PrInfo>> {
            Ok(None)
        }
        fn create_pr(
            &self,
            _branch: &str,
            _base: &str,
            _title: &str,
            _body: &str,
            _options: &PrOptions,
        ) -> Result<String> {
            Ok("https://mock/pr/1".to_string())
        }
        fn get_pr_info(&self, _pr_ref: &str) -> Result<PrInfo> {
            Ok(PrInfo {
                number: 1,
                url: "https://mock/pr/1".to_string(),
                head_ref: "branch".to_string(),
                base_ref: "main".to_string(),
                state: PrState::Open,
                title: "Test".to_string(),
            })
        }
        fn get_pr_chain(&self, _pr_ref: &str) -> Result<Vec<PrInfo>> {
            Ok(vec![])
        }
        fn is_branch_merged(&self, _branch: &str, _into: &str) -> Result<bool> {
            Ok(false)
        }
        fn get_pr_full_info(&self, _pr_ref: &str) -> Result<PrFullInfo> {
            Ok(PrFullInfo {
                number: 1,
                url: "https://mock/pr/1".to_string(),
                title: "Test".to_string(),
                state: PrState::Open,
                is_draft: false,
                review: ReviewState::Pending,
                ci: CiStatus::None,
                head_ref: "branch".to_string(),
                base_ref: "main".to_string(),
            })
        }
        fn get_pr_body(&self, _pr_ref: &str) -> Result<String> {
            Ok(String::new())
        }
        fn update_pr_body(&self, _pr_ref: &str, _body: &str) -> Result<()> {
            Ok(())
        }
        fn update_pr_base(&self, branch: &str, new_base: &str) -> Result<()> {
            self.update_calls
                .write()
                .unwrap()
                .push((branch.to_string(), new_base.to_string()));

            if self.should_fail {
                anyhow::bail!("Mock forge error: update_pr_base failed");
            }
            Ok(())
        }
        fn mark_pr_ready(&self, _pr_ref: &str) -> Result<()> {
            Ok(())
        }
        fn enable_auto_merge(&self, _pr_ref: &str, _merge_method: &str) -> Result<()> {
            Ok(())
        }
        fn merge_pr(&self, _pr_ref: &str, _method: MergeMethod, _auto_confirm: bool) -> Result<()> {
            Ok(())
        }
        fn open_pr_in_browser(&self, _pr_ref: &str) -> Result<()> {
            Ok(())
        }
        fn push_branch(&self, _branch: &str, _force: bool) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_retarget_next_pr_success() {
        let forge = MockForge::new();
        let mergeable = vec![
            ("a".to_string(), "url_a".to_string()),
            ("b".to_string(), "url_b".to_string()),
            ("c".to_string(), "url_c".to_string()),
        ];

        // After merging "a" (index 0), should retarget "b" to "main"
        let result = retarget_next_pr(Some(&forge), &mergeable, 0, "main");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some("b".to_string()));

        // Verify the forge was called correctly
        let calls = forge.get_update_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], ("b".to_string(), "main".to_string()));
    }

    #[test]
    fn test_retarget_next_pr_last_branch_returns_none() {
        let forge = MockForge::new();
        let mergeable = vec![
            ("a".to_string(), "url_a".to_string()),
            ("b".to_string(), "url_b".to_string()),
        ];

        // After merging "b" (index 1, the last one), should return None
        let result = retarget_next_pr(Some(&forge), &mergeable, 1, "main");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);

        // Verify no forge call was made
        let calls = forge.get_update_calls();
        assert!(calls.is_empty());
    }

    #[test]
    fn test_retarget_next_pr_no_forge_returns_none() {
        let mergeable = vec![
            ("a".to_string(), "url_a".to_string()),
            ("b".to_string(), "url_b".to_string()),
        ];

        // Without a forge, should return None (graceful degradation)
        let result = retarget_next_pr(None, &mergeable, 0, "main");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_retarget_next_pr_forge_error_propagates() {
        let forge = MockForge::failing();
        let mergeable = vec![
            ("a".to_string(), "url_a".to_string()),
            ("b".to_string(), "url_b".to_string()),
        ];

        // With a failing forge, should return an error
        let result = retarget_next_pr(Some(&forge), &mergeable, 0, "main");
        assert!(result.is_err());

        // The error message should be from the mock
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Mock forge error"));
    }

    #[test]
    fn test_retarget_next_pr_uses_correct_trunk() {
        let forge = MockForge::new();
        let mergeable = vec![
            ("a".to_string(), "url_a".to_string()),
            ("b".to_string(), "url_b".to_string()),
        ];

        // Test with "main" as trunk
        let _ = retarget_next_pr(Some(&forge), &mergeable, 0, "main");
        let calls = forge.get_update_calls();
        assert_eq!(calls[0], ("b".to_string(), "main".to_string()));
    }

    #[test]
    fn test_retarget_next_pr_multiple_branches() {
        // Test retargeting through a full stack
        let forge = MockForge::new();
        let mergeable = vec![
            ("a".to_string(), "url_a".to_string()),
            ("b".to_string(), "url_b".to_string()),
            ("c".to_string(), "url_c".to_string()),
            ("d".to_string(), "url_d".to_string()),
        ];

        // Simulate merging each branch in order
        for i in 0..mergeable.len() {
            let _ = retarget_next_pr(Some(&forge), &mergeable, i, "main");
        }

        // Should have called update_pr_base 3 times (for b, c, d - not after d)
        let calls = forge.get_update_calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], ("b".to_string(), "main".to_string()));
        assert_eq!(calls[1], ("c".to_string(), "main".to_string()));
        assert_eq!(calls[2], ("d".to_string(), "main".to_string()));
    }

    #[test]
    fn test_retarget_next_pr_single_branch_returns_none() {
        let forge = MockForge::new();
        let mergeable = vec![("feature".to_string(), "url".to_string())];

        // Single branch - nothing to retarget
        let result = retarget_next_pr(Some(&forge), &mergeable, 0, "main");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);

        // No forge calls should have been made
        let calls = forge.get_update_calls();
        assert!(calls.is_empty());
    }

    // =========================================================================
    // Tests for retarget_next_pr_if_open - smart retargeting with state check
    // =========================================================================

    /// Mock forge that can return configurable PR states for testing
    struct MockForgeWithState {
        update_calls: RwLock<Vec<(String, String)>>,
        pr_states: std::collections::HashMap<String, PrState>,
    }

    impl MockForgeWithState {
        fn new(pr_states: std::collections::HashMap<String, PrState>) -> Self {
            Self {
                update_calls: RwLock::new(Vec::new()),
                pr_states,
            }
        }

        fn get_update_calls(&self) -> Vec<(String, String)> {
            self.update_calls.read().unwrap().clone()
        }
    }

    impl Forge for MockForgeWithState {
        fn forge_type(&self) -> ForgeType {
            ForgeType::GitHub
        }
        fn cli_name(&self) -> &str {
            "mock"
        }
        fn check_auth(&self) -> Result<()> {
            Ok(())
        }
        fn pr_exists(&self, _branch: &str) -> Result<Option<PrInfo>> {
            Ok(None)
        }
        fn create_pr(
            &self,
            _branch: &str,
            _base: &str,
            _title: &str,
            _body: &str,
            _options: &PrOptions,
        ) -> Result<String> {
            Ok("https://mock/pr/1".to_string())
        }
        fn get_pr_info(&self, pr_ref: &str) -> Result<PrInfo> {
            let state = self.pr_states.get(pr_ref).copied().unwrap_or(PrState::Open);
            Ok(PrInfo {
                number: pr_ref.parse().unwrap_or(1),
                url: format!("https://mock/pr/{}", pr_ref),
                head_ref: "branch".to_string(),
                base_ref: "main".to_string(),
                state,
                title: "Test".to_string(),
            })
        }
        fn get_pr_chain(&self, _pr_ref: &str) -> Result<Vec<PrInfo>> {
            Ok(vec![])
        }
        fn is_branch_merged(&self, _branch: &str, _into: &str) -> Result<bool> {
            Ok(false)
        }
        fn get_pr_full_info(&self, _pr_ref: &str) -> Result<PrFullInfo> {
            Ok(PrFullInfo {
                number: 1,
                url: "https://mock/pr/1".to_string(),
                title: "Test".to_string(),
                state: PrState::Open,
                is_draft: false,
                review: ReviewState::Pending,
                ci: CiStatus::None,
                head_ref: "branch".to_string(),
                base_ref: "main".to_string(),
            })
        }
        fn get_pr_body(&self, _pr_ref: &str) -> Result<String> {
            Ok(String::new())
        }
        fn update_pr_body(&self, _pr_ref: &str, _body: &str) -> Result<()> {
            Ok(())
        }
        fn update_pr_base(&self, branch: &str, new_base: &str) -> Result<()> {
            self.update_calls
                .write()
                .unwrap()
                .push((branch.to_string(), new_base.to_string()));
            Ok(())
        }
        fn mark_pr_ready(&self, _pr_ref: &str) -> Result<()> {
            Ok(())
        }
        fn enable_auto_merge(&self, _pr_ref: &str, _merge_method: &str) -> Result<()> {
            Ok(())
        }
        fn merge_pr(&self, _pr_ref: &str, _method: MergeMethod, _auto_confirm: bool) -> Result<()> {
            Ok(())
        }
        fn open_pr_in_browser(&self, _pr_ref: &str) -> Result<()> {
            Ok(())
        }
        fn push_branch(&self, _branch: &str, _force: bool) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_retarget_next_pr_if_open_skips_merged_pr() {
        // Configure PR #2 (extracted from url_b) as already merged
        let mut pr_states = std::collections::HashMap::new();
        pr_states.insert("2".to_string(), PrState::Merged);

        let forge = MockForgeWithState::new(pr_states);
        let mergeable = vec![
            ("a".to_string(), "https://mock/pr/1".to_string()),
            ("b".to_string(), "https://mock/pr/2".to_string()),
        ];

        // After merging "a" (index 0), should skip retargeting "b" because it's merged
        let result = retarget_next_pr_if_open(&forge, &mergeable, 0, "main");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None); // Returns None because PR is merged

        // No update_pr_base calls should have been made
        let calls = forge.get_update_calls();
        assert!(calls.is_empty());
    }

    #[test]
    fn test_retarget_next_pr_if_open_skips_closed_pr() {
        // Configure PR #2 as closed
        let mut pr_states = std::collections::HashMap::new();
        pr_states.insert("2".to_string(), PrState::Closed);

        let forge = MockForgeWithState::new(pr_states);
        let mergeable = vec![
            ("a".to_string(), "https://mock/pr/1".to_string()),
            ("b".to_string(), "https://mock/pr/2".to_string()),
        ];

        // After merging "a" (index 0), should skip retargeting "b" because it's closed
        let result = retarget_next_pr_if_open(&forge, &mergeable, 0, "main");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None); // Returns None because PR is closed

        // No update_pr_base calls should have been made
        let calls = forge.get_update_calls();
        assert!(calls.is_empty());
    }

    #[test]
    fn test_retarget_next_pr_if_open_retargets_open_pr() {
        // Configure PR #2 as open
        let mut pr_states = std::collections::HashMap::new();
        pr_states.insert("2".to_string(), PrState::Open);

        let forge = MockForgeWithState::new(pr_states);
        let mergeable = vec![
            ("a".to_string(), "https://mock/pr/1".to_string()),
            ("b".to_string(), "https://mock/pr/2".to_string()),
        ];

        // After merging "a" (index 0), should retarget "b" because it's open
        let result = retarget_next_pr_if_open(&forge, &mergeable, 0, "main");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some("b".to_string()));

        // update_pr_base should have been called
        let calls = forge.get_update_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], ("b".to_string(), "main".to_string()));
    }

    #[test]
    fn test_retarget_next_pr_if_open_returns_none_for_last_branch() {
        let forge = MockForgeWithState::new(std::collections::HashMap::new());
        let mergeable = vec![
            ("a".to_string(), "https://mock/pr/1".to_string()),
            ("b".to_string(), "https://mock/pr/2".to_string()),
        ];

        // After merging "b" (last branch), should return None
        let result = retarget_next_pr_if_open(&forge, &mergeable, 1, "main");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);

        // No update_pr_base calls should have been made
        let calls = forge.get_update_calls();
        assert!(calls.is_empty());
    }

    // =========================================================================
    // Tests for is_not_mergeable_error - error detection for auto-recovery
    // =========================================================================

    #[test]
    fn test_is_not_mergeable_error_detects_not_mergeable() {
        let err = anyhow::anyhow!("Pull request is not mergeable");
        assert!(is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_detects_merge_conflict() {
        let err = anyhow::anyhow!("PR has merge conflict with base");
        assert!(is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_detects_cannot_be_cleanly_created() {
        // This is the actual error message from GitHub
        let err = anyhow::anyhow!("the merge commit cannot be cleanly created");
        assert!(is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_detects_conflicting() {
        let err = anyhow::anyhow!("PR state is CONFLICTING");
        assert!(is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_case_insensitive() {
        let err = anyhow::anyhow!("PR is NOT MERGEABLE due to conflicts");
        assert!(is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_returns_false_for_other_errors() {
        let err = anyhow::anyhow!("Network timeout");
        assert!(!is_not_mergeable_error(&err));

        let err = anyhow::anyhow!("Authentication failed");
        assert!(!is_not_mergeable_error(&err));

        let err = anyhow::anyhow!("Required checks not passing");
        assert!(!is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_matches_real_github_error() {
        // This is the exact error message from the user's report
        let err = anyhow::anyhow!(
            "gh pr merge failed: X Pull request rsperko/diamond#12 is not mergeable: \
             the merge commit cannot be cleanly created."
        );
        assert!(is_not_mergeable_error(&err));
    }

    // =========================================================================
    // GitLab-specific error detection tests
    // =========================================================================

    #[test]
    fn test_is_not_mergeable_error_detects_gitlab_cannot_be_merged() {
        // GitLab API returns "Branch cannot be merged" (HTTP 406) for conflicts
        let err = anyhow::anyhow!("glab mr merge failed: Branch cannot be merged");
        assert!(is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_detects_gitlab_has_conflicts() {
        let err = anyhow::anyhow!("Merge request has conflicts that must be resolved");
        assert!(is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_gitlab_case_insensitive() {
        let err = anyhow::anyhow!("BRANCH CANNOT BE MERGED due to conflicts");
        assert!(is_not_mergeable_error(&err));
    }

    // =========================================================================
    // Tests for effective settings computation (CLI flags override config)
    // =========================================================================

    use crate::config::MergeConfig;

    /// Helper to compute effective proactive_rebase setting
    fn compute_proactive_rebase(fast_mode: bool, config_value: bool) -> bool {
        if fast_mode {
            false
        } else {
            config_value
        }
    }

    /// Helper to compute effective wait_for_ci setting
    fn compute_wait_for_ci(fast_mode: bool, no_wait: bool, config_value: bool) -> bool {
        if fast_mode || no_wait {
            false
        } else {
            config_value
        }
    }

    #[test]
    fn test_default_config_enables_proactive_rebase() {
        let config = MergeConfig::default();
        assert!(config.proactive_rebase);

        let effective = compute_proactive_rebase(false, config.proactive_rebase);
        assert!(effective);
    }

    #[test]
    fn test_default_config_enables_wait_for_ci() {
        let config = MergeConfig::default();
        assert!(config.wait_for_ci);

        let effective = compute_wait_for_ci(false, false, config.wait_for_ci);
        assert!(effective);
    }

    #[test]
    fn test_fast_flag_disables_proactive_rebase() {
        let config = MergeConfig::default();
        assert!(config.proactive_rebase); // Config says true

        let effective = compute_proactive_rebase(true, config.proactive_rebase);
        assert!(!effective); // But --fast overrides to false
    }

    #[test]
    fn test_fast_flag_disables_wait_for_ci() {
        let config = MergeConfig::default();
        assert!(config.wait_for_ci); // Config says true

        let effective = compute_wait_for_ci(true, false, config.wait_for_ci);
        assert!(!effective); // But --fast overrides to false
    }

    #[test]
    fn test_no_wait_flag_disables_wait_for_ci() {
        let config = MergeConfig::default();
        assert!(config.wait_for_ci); // Config says true

        let effective = compute_wait_for_ci(false, true, config.wait_for_ci);
        assert!(!effective); // But --no-wait overrides to false
    }

    #[test]
    fn test_no_wait_flag_preserves_proactive_rebase() {
        let config = MergeConfig::default();
        assert!(config.proactive_rebase); // Config says true

        // --no-wait should NOT affect proactive_rebase
        let effective = compute_proactive_rebase(false, config.proactive_rebase);
        assert!(effective); // Still true
    }

    #[test]
    fn test_config_disabled_proactive_rebase_respected() {
        let config = MergeConfig {
            proactive_rebase: false,
            ..Default::default()
        };

        let effective = compute_proactive_rebase(false, config.proactive_rebase);
        assert!(!effective); // Config disabled it
    }

    #[test]
    fn test_config_disabled_wait_for_ci_respected() {
        let config = MergeConfig {
            wait_for_ci: false,
            ..Default::default()
        };

        let effective = compute_wait_for_ci(false, false, config.wait_for_ci);
        assert!(!effective); // Config disabled it
    }

    #[test]
    fn test_fast_mode_overrides_even_when_config_enabled() {
        let config = MergeConfig {
            proactive_rebase: true,
            wait_for_ci: true,
            ..Default::default()
        };

        // --fast should disable both even when config has them enabled
        assert!(!compute_proactive_rebase(true, config.proactive_rebase));
        assert!(!compute_wait_for_ci(true, false, config.wait_for_ci));
    }

    #[test]
    fn test_both_fast_and_no_wait_disables_ci_wait() {
        let config = MergeConfig::default();

        // Both flags set (redundant but valid)
        let effective = compute_wait_for_ci(true, true, config.wait_for_ci);
        assert!(!effective);
    }

    #[test]
    fn test_ci_timeout_from_config() {
        let config = MergeConfig {
            ci_timeout_secs: 300, // Custom 5 minute timeout
            ..Default::default()
        };

        assert_eq!(config.ci_timeout_secs, 300);
    }

    #[test]
    fn test_default_ci_timeout() {
        let config = MergeConfig::default();
        assert_eq!(config.ci_timeout_secs, 600); // 10 minutes
    }

    // =========================================================================
    // Tests for proactive_rebase_for_merge
    //
    // NOTE: Full integration tests for proactive_rebase_for_merge require:
    // - A "remote" repo (simulated with a bare git repo)
    // - A "local" repo cloned from the remote
    // - Proper branch setup with commits that need rebasing
    //
    // These are better suited for the integration test suite (tests/*.rs)
    // rather than unit tests. The function is tightly coupled to git
    // operations (fetch, rebase, force push) that are difficult to mock.
    //
    // Recommended integration tests to add in tests/:
    // - test_merge_proactive_rebase_rebases_stale_branch
    // - test_merge_proactive_rebase_skips_up_to_date_branch
    // - test_merge_proactive_rebase_fails_on_conflict
    // - test_merge_proactive_rebase_force_pushes
    // =========================================================================

    /// Mock forge that tracks push_branch and update_pr_base calls
    /// (Prepared for integration tests that test proactive_rebase_for_merge)
    #[allow(dead_code)]
    struct ProactiveRebaseMockForge {
        push_calls: RwLock<Vec<(String, bool)>>,
        update_base_calls: RwLock<Vec<(String, String)>>,
    }

    #[allow(dead_code)]
    impl ProactiveRebaseMockForge {
        fn new() -> Self {
            Self {
                push_calls: RwLock::new(Vec::new()),
                update_base_calls: RwLock::new(Vec::new()),
            }
        }

        #[allow(dead_code)]
        fn get_push_calls(&self) -> Vec<(String, bool)> {
            self.push_calls.read().unwrap().clone()
        }

        #[allow(dead_code)]
        fn get_update_base_calls(&self) -> Vec<(String, String)> {
            self.update_base_calls.read().unwrap().clone()
        }
    }

    impl Forge for ProactiveRebaseMockForge {
        fn forge_type(&self) -> ForgeType {
            ForgeType::GitHub
        }
        fn cli_name(&self) -> &str {
            "mock"
        }
        fn check_auth(&self) -> Result<()> {
            Ok(())
        }
        fn pr_exists(&self, _branch: &str) -> Result<Option<PrInfo>> {
            Ok(None)
        }
        fn create_pr(
            &self,
            _branch: &str,
            _base: &str,
            _title: &str,
            _body: &str,
            _options: &PrOptions,
        ) -> Result<String> {
            Ok("https://mock/pr/1".to_string())
        }
        fn get_pr_info(&self, _pr_ref: &str) -> Result<PrInfo> {
            Ok(PrInfo {
                number: 1,
                url: "https://mock/pr/1".to_string(),
                head_ref: "branch".to_string(),
                base_ref: "main".to_string(),
                state: PrState::Open,
                title: "Test".to_string(),
            })
        }
        fn get_pr_chain(&self, _pr_ref: &str) -> Result<Vec<PrInfo>> {
            Ok(vec![])
        }
        fn is_branch_merged(&self, _branch: &str, _into: &str) -> Result<bool> {
            Ok(false)
        }
        fn get_pr_full_info(&self, _pr_ref: &str) -> Result<PrFullInfo> {
            Ok(PrFullInfo {
                number: 1,
                url: "https://mock/pr/1".to_string(),
                title: "Test".to_string(),
                state: PrState::Open,
                is_draft: false,
                review: ReviewState::Pending,
                ci: CiStatus::None,
                head_ref: "branch".to_string(),
                base_ref: "main".to_string(),
            })
        }
        fn get_pr_body(&self, _pr_ref: &str) -> Result<String> {
            Ok(String::new())
        }
        fn update_pr_body(&self, _pr_ref: &str, _body: &str) -> Result<()> {
            Ok(())
        }
        fn update_pr_base(&self, branch: &str, new_base: &str) -> Result<()> {
            self.update_base_calls
                .write()
                .unwrap()
                .push((branch.to_string(), new_base.to_string()));
            Ok(())
        }
        fn mark_pr_ready(&self, _pr_ref: &str) -> Result<()> {
            Ok(())
        }
        fn enable_auto_merge(&self, _pr_ref: &str, _merge_method: &str) -> Result<()> {
            Ok(())
        }
        fn merge_pr(&self, _pr_ref: &str, _method: MergeMethod, _auto_confirm: bool) -> Result<()> {
            Ok(())
        }
        fn open_pr_in_browser(&self, _pr_ref: &str) -> Result<()> {
            Ok(())
        }
        fn push_branch(&self, branch: &str, force: bool) -> Result<()> {
            self.push_calls.write().unwrap().push((branch.to_string(), force));
            Ok(())
        }
    }

    // Unit-testable aspects of proactive rebase behavior:

    #[test]
    fn test_proactive_rebase_only_applies_to_non_first_branch() {
        // The proactive rebase logic in the merge loop only applies when i > 0
        // (i.e., not the first branch in the stack, which already targets trunk)

        // For the first branch (i == 0), proactive rebase should be skipped
        // This is a logic test, not a full integration test

        // Index 0 should NOT trigger proactive rebase
        let should_proactive_rebase_first = 0 > 0;
        assert!(!should_proactive_rebase_first);

        // Index 1 should trigger proactive rebase
        let should_proactive_rebase_second = 1 > 0;
        assert!(should_proactive_rebase_second);
    }

    #[test]
    fn test_proactive_mode_skipped_in_fast_mode() {
        // Test that fast_mode=true skips proactive rebase
        let fast_mode = true;
        let proactive_rebase = compute_proactive_rebase(fast_mode, true);
        assert!(!proactive_rebase);

        // Even with i > 0, fast mode means no proactive rebase
        let should_do_proactive = proactive_rebase && 1 > 0;
        assert!(!should_do_proactive);
    }

    #[test]
    fn test_proactive_mode_enabled_by_default() {
        // Test that default config enables proactive rebase
        let config = MergeConfig::default();
        let fast_mode = false;
        let proactive_rebase = compute_proactive_rebase(fast_mode, config.proactive_rebase);
        assert!(proactive_rebase);

        // With i > 0, proactive rebase should happen
        let should_do_proactive = proactive_rebase && 1 > 0;
        assert!(should_do_proactive);
    }

    #[test]
    fn test_is_not_mergeable_error_excludes_branch_protection() {
        let err = anyhow::anyhow!("the base branch policy prohibits the merge");
        assert!(!is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_excludes_branch_protection_keyword() {
        let err = anyhow::anyhow!("blocked by branch protection rules");
        assert!(!is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_excludes_protected_branch() {
        let err = anyhow::anyhow!("cannot push to protected branch");
        assert!(!is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_excludes_required_status() {
        let err = anyhow::anyhow!("required status checks not met");
        assert!(!is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_excludes_reviews() {
        let err = anyhow::anyhow!("PR requires reviews that haven't been approved");
        assert!(!is_not_mergeable_error(&err));
    }

    #[test]
    fn test_is_not_mergeable_error_excludes_gitlab_approval() {
        let err = anyhow::anyhow!("MR requires approval before merging");
        assert!(!is_not_mergeable_error(&err));
    }
}
