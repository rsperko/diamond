use anyhow::{Context, Result};

use crate::cache::Cache;
use crate::forge::{get_forge, AsyncForge, PrInfo, PrState};
#[cfg(test)]
use crate::forge::Forge;
use crate::git_gateway::{GitGateway, RebaseOutcome};
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::ui;

/// Clean up branches that have been merged to trunk
pub fn run(force: bool) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    // Try to get forge for updating PR bases (best effort)
    let forge = get_forge(None).ok();

    // Verify we have a trunk
    let trunk = ref_store
        .get_trunk()?
        .with_context(|| format!("No trunk configured. Run '{} init' first.", program_name()))?;

    // Get current branch to avoid deleting it
    let current_branch = gateway.get_current_branch_name()?;

    // Find merged branches (only checks tracked branches)
    let merged_branches = find_merged_branches(&gateway, &ref_store, &trunk)?;

    // Filter out current branch (trunk already excluded by find_merged_branches)
    let candidates: Vec<String> = merged_branches.into_iter().filter(|b| b != &current_branch).collect();

    if candidates.is_empty() {
        ui::success_bold("No merged branches to clean up");
        return Ok(());
    }

    // Load cache for PR URLs
    let cache = Cache::load().unwrap_or_default();

    // Show what will be deleted
    ui::step(&format!("Found {} merged branch(es):", candidates.len()));
    for branch in &candidates {
        let pr_url = cache.get_pr_url(branch);

        if let Some(url) = pr_url {
            ui::bullet(&format!("{} ({})", ui::print_branch(branch), ui::print_url(url)));
        } else {
            ui::bullet(&ui::print_branch(branch));
        }
    }
    ui::blank();

    // Confirm unless --force
    if !force && !ui::confirm("These branches will be deleted locally. Continue?", false)? {
        ui::warning("Cleanup cancelled");
        return Ok(());
    }

    // Delete branches and update metadata
    // IMPORTANT: We restack children BEFORE deleting parent to avoid orphaning
    // children if restack encounters conflicts
    let mut deleted_count = 0;

    for branch in &candidates {
        // Get parent and children BEFORE any modifications
        let parent = ref_store.get_parent(branch)?;
        let children: Vec<String> = ref_store.get_children(branch)?.into_iter().collect();
        let restack_onto = parent.as_ref().unwrap_or(&trunk);

        // PHASE 1: Restack all children FIRST (before deleting parent)
        // This ensures children are never orphaned if restack fails
        let mut restack_failed = false;
        for child in &children {
            if gateway.branch_exists(child).unwrap_or(false) {
                ui::bullet_step(&format!(
                    "Restacking {} onto {}",
                    ui::print_branch(child),
                    ui::print_parent(restack_onto)
                ));
                match gateway.rebase_fork_point(child, restack_onto) {
                    Ok(RebaseOutcome::Success) => {
                        // Success - update metadata now that rebase succeeded
                        ref_store.reparent(child, restack_onto)?;

                        // Update PR base on GitHub if forge is available and PR is open
                        if let Some(ref forge) = forge {
                            if let Ok(Some(pr_info)) = forge.pr_exists(child) {
                                if pr_info.state == PrState::Open {
                                    if let Err(e) = forge.update_pr_base(child, restack_onto) {
                                        ui::warning(&format!("Could not update PR base for {}: {}", child, e));
                                    }
                                }
                            }
                        }
                    }
                    Ok(RebaseOutcome::Conflicts) => {
                        // Conflict - stop cleanup to let user resolve
                        ui::warning(&format!(
                            "Conflict while restacking '{}'. Resolve and run '{} continue'.",
                            child,
                            program_name()
                        ));
                        restack_failed = true;
                        break;
                    }
                    Err(e) => {
                        ui::warning(&format!("Failed to restack {}: {}", child, e));
                        restack_failed = true;
                        break;
                    }
                }
            } else {
                // Branch doesn't exist in git, just update metadata
                ref_store.reparent(child, restack_onto)?;
            }
        }

        // If restack failed, skip deleting this branch (parent still exists for recovery)
        if restack_failed {
            ui::warning(&format!("Skipping deletion of '{}' - resolve conflicts first", branch));
            continue;
        }

        // PHASE 2: Now safe to delete the parent branch
        // Switch to trunk first (safe mode - fail if uncommitted changes)
        gateway.checkout_branch_safe(&trunk)?;

        match gateway.delete_branch(branch) {
            Ok(()) => {
                // Remove branch from refs
                ref_store.remove_parent(branch)?;
                deleted_count += 1;
                ui::bullet_success(&format!("Deleted {}", branch));
            }
            Err(e) => {
                ui::bullet_error(&format!("Failed to delete {}: {}", branch, e));
            }
        }
    }

    ui::blank();
    ui::success_bold(&format!("Cleanup complete! Deleted {} branch(es)", deleted_count));

    Ok(())
}

/// Find branches that have been fully merged into trunk (git-based detection)
/// Note: This doesn't work for squash merges - use find_merged_prs_async() instead
pub fn find_merged_branches(gateway: &GitGateway, ref_store: &RefStore, trunk: &str) -> Result<Vec<String>> {
    let mut merged = Vec::new();

    // Get all tracked branches from refs
    let all_branches = ref_store.collect_branches_dfs(&[trunk.to_string()])?;

    for branch_name in all_branches {
        // Skip trunk itself
        if branch_name == trunk {
            continue;
        }

        // Check if branch exists in git and is merged
        if gateway.branch_exists(&branch_name)? && gateway.is_branch_merged(&branch_name, trunk)? {
            merged.push(branch_name);
        }
    }

    Ok(merged)
}

/// Find branches with merged PRs via forge API (async batch version)
///
/// This version uses batch API calls for better performance with many branches.
pub async fn find_merged_prs_async(forge: &dyn AsyncForge, branches: &[String]) -> Vec<(String, PrInfo)> {
    // Batch check all PRs in parallel
    let pr_results = forge.check_prs_exist(branches).await;

    // Filter to only merged PRs
    pr_results
        .into_iter()
        .filter_map(|(branch, pr_info)| {
            pr_info.and_then(|info| {
                if info.state == PrState::Merged {
                    Some((branch, info))
                } else {
                    None
                }
            })
        })
        .collect()
}

/// Clean up merged branches for sync (auto-reparents children, updates cache)
/// Returns the list of deleted branch names
///
/// If a forge is provided, also updates PR bases on GitHub for reparented children.
///
/// Note: This is the sync version, used only by unit tests.
/// Production code uses `cleanup_merged_branches_for_sync_async`.
#[cfg(test)]
pub fn cleanup_merged_branches_for_sync(
    gateway: &GitGateway,
    ref_store: &RefStore,
    cache: &mut Cache,
    trunk: &str,
    merged_branches: &[(String, PrInfo)],
    forge: Option<&dyn Forge>,
) -> Result<Vec<String>> {
    let mut deleted = Vec::new();

    for (branch, pr_info) in merged_branches {
        // Get parent and children BEFORE deleting
        let parent = ref_store.get_parent(branch)?;
        let children: Vec<String> = ref_store.get_children(branch)?.into_iter().collect();

        // Determine what to reparent children to
        let trunk_string = trunk.to_string();
        let reparent_to = parent.as_ref().unwrap_or(&trunk_string);

        // Delete the git branch
        match gateway.delete_branch(branch) {
            Ok(()) => {
                ui::bullet_success(&format!("Deleted {} (PR #{} merged)", branch, pr_info.number));

                // Reparent children to grandparent (or trunk)
                for child in &children {
                    ref_store.reparent(child, reparent_to)?;
                    println!(
                        "    Reparented {} → {}",
                        ui::print_branch(child),
                        ui::print_parent(reparent_to)
                    );

                    // Update PR base on GitHub if forge is available and PR is open
                    if let Some(forge) = forge {
                        if let Ok(Some(pr_info)) = forge.pr_exists(child) {
                            // Only update base for open PRs - merged/closed PRs can't be modified
                            if pr_info.state == PrState::Open {
                                if let Err(e) = forge.update_pr_base(child, reparent_to) {
                                    ui::warning(&format!("Could not update PR base for {}: {}", child, e));
                                }
                            }
                        }
                    }

                    // Update base_sha for reparented child
                    if let Ok(sha) = gateway.get_branch_sha(child) {
                        cache.set_base_sha(child, &sha);
                    }
                }

                // Remove branch from refs (local only - remote ref cleanup happens on next submit)
                ref_store.remove_parent(branch)?;
                deleted.push(branch.clone());
            }
            Err(e) => {
                ui::warning(&format!("Failed to delete {}: {}", branch, e));
            }
        }
    }

    // Save cache with updated base_sha values
    if !deleted.is_empty() {
        cache.save()?;
    }

    Ok(deleted)
}

/// Clean up merged branches for sync (async version with batch PR updates)
///
/// This version uses batch API calls for checking PR existence and updating
/// PR bases, providing better performance when cleaning up branches with
/// many children.
pub async fn cleanup_merged_branches_for_sync_async(
    gateway: &GitGateway,
    ref_store: &RefStore,
    cache: &mut Cache,
    trunk: &str,
    merged_branches: &[(String, PrInfo)],
    forge: Option<&dyn AsyncForge>,
) -> Result<Vec<String>> {
    let mut deleted = Vec::new();

    for (branch, pr_info) in merged_branches {
        // Get parent and children BEFORE deleting
        let parent = ref_store.get_parent(branch)?;
        let children: Vec<String> = ref_store.get_children(branch)?.into_iter().collect();

        // Determine what to reparent children to
        let trunk_string = trunk.to_string();
        let reparent_to = parent.as_ref().unwrap_or(&trunk_string);

        // Delete the git branch
        match gateway.delete_branch(branch) {
            Ok(()) => {
                ui::bullet_success(&format!("Deleted {} (PR #{} merged)", branch, pr_info.number));

                // Reparent children to grandparent (or trunk)
                for child in &children {
                    ref_store.reparent(child, reparent_to)?;
                    println!(
                        "    Reparented {} → {}",
                        ui::print_branch(child),
                        ui::print_parent(reparent_to)
                    );

                    // Update base_sha for reparented child
                    if let Ok(sha) = gateway.get_branch_sha(child) {
                        cache.set_base_sha(child, &sha);
                    }
                }

                // Batch update PR bases if forge is available
                if let Some(forge) = forge {
                    if !children.is_empty() {
                        // First, batch check which children have open PRs
                        let pr_results = forge.check_prs_exist(&children).await;

                        // Collect children with open PRs that need base updates
                        let base_updates: Vec<(String, String)> = pr_results
                            .into_iter()
                            .filter_map(|(child, pr_info)| {
                                pr_info.and_then(|info| {
                                    if info.state == PrState::Open {
                                        Some((child, reparent_to.to_string()))
                                    } else {
                                        None
                                    }
                                })
                            })
                            .collect();

                        // Batch update all PR bases in parallel
                        if !base_updates.is_empty() {
                            let updated = forge.update_pr_bases(&base_updates).await;
                            let failed = base_updates.len() - updated;
                            if failed > 0 {
                                ui::warning(&format!(
                                    "Could not update PR base for {} of {} children",
                                    failed,
                                    base_updates.len()
                                ));
                            }
                        }
                    }
                }

                // Remove branch from refs (local only - remote ref cleanup happens on next submit)
                ref_store.remove_parent(branch)?;
                deleted.push(branch.clone());
            }
            Err(e) => {
                ui::warning(&format!("Failed to delete {}: {}", branch, e));
            }
        }
    }

    // Save cache with updated base_sha values
    if !deleted.is_empty() {
        cache.save()?;
    }

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    fn create_branch_with_commit(repo: &git2::Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    // === Async function logic tests ===
    // These test the filtering/transformation logic used by async functions

    #[test]
    fn test_merged_pr_filtering_logic() {
        // Test the filtering logic used by find_merged_prs_async
        // Given batch results, verify correct filtering to merged PRs

        let results: Vec<(String, Option<PrInfo>)> = vec![
            (
                "branch-merged".to_string(),
                Some(PrInfo {
                    number: 1,
                    url: "https://github.com/test/repo/pull/1".to_string(),
                    head_ref: "branch-merged".to_string(),
                    base_ref: "main".to_string(),
                    state: PrState::Merged,
                    title: "Merged PR".to_string(),
                }),
            ),
            (
                "branch-open".to_string(),
                Some(PrInfo {
                    number: 2,
                    url: "https://github.com/test/repo/pull/2".to_string(),
                    head_ref: "branch-open".to_string(),
                    base_ref: "main".to_string(),
                    state: PrState::Open,
                    title: "Open PR".to_string(),
                }),
            ),
            (
                "branch-closed".to_string(),
                Some(PrInfo {
                    number: 3,
                    url: "https://github.com/test/repo/pull/3".to_string(),
                    head_ref: "branch-closed".to_string(),
                    base_ref: "main".to_string(),
                    state: PrState::Closed,
                    title: "Closed PR".to_string(),
                }),
            ),
            ("branch-no-pr".to_string(), None),
        ];

        // Apply the same filtering logic as find_merged_prs_async
        let merged: Vec<(String, PrInfo)> = results
            .into_iter()
            .filter_map(|(branch, pr_info)| {
                pr_info.and_then(|info| {
                    if info.state == PrState::Merged {
                        Some((branch, info))
                    } else {
                        None
                    }
                })
            })
            .collect();

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].0, "branch-merged");
        assert_eq!(merged[0].1.number, 1);
    }

    #[test]
    fn test_merged_pr_filtering_empty_input() {
        let results: Vec<(String, Option<PrInfo>)> = vec![];

        let merged: Vec<(String, PrInfo)> = results
            .into_iter()
            .filter_map(|(branch, pr_info)| {
                pr_info.and_then(|info| {
                    if info.state == PrState::Merged {
                        Some((branch, info))
                    } else {
                        None
                    }
                })
            })
            .collect();

        assert!(merged.is_empty());
    }

    #[test]
    fn test_merged_pr_filtering_all_open() {
        // Edge case: all PRs are open, none merged
        let results: Vec<(String, Option<PrInfo>)> = vec![
            (
                "branch-1".to_string(),
                Some(PrInfo {
                    number: 1,
                    url: "url".to_string(),
                    head_ref: "branch-1".to_string(),
                    base_ref: "main".to_string(),
                    state: PrState::Open,
                    title: "PR 1".to_string(),
                }),
            ),
            (
                "branch-2".to_string(),
                Some(PrInfo {
                    number: 2,
                    url: "url".to_string(),
                    head_ref: "branch-2".to_string(),
                    base_ref: "main".to_string(),
                    state: PrState::Open,
                    title: "PR 2".to_string(),
                }),
            ),
        ];

        let merged: Vec<(String, PrInfo)> = results
            .into_iter()
            .filter_map(|(branch, pr_info)| {
                pr_info.and_then(|info| {
                    if info.state == PrState::Merged {
                        Some((branch, info))
                    } else {
                        None
                    }
                })
            })
            .collect();

        assert!(merged.is_empty());
    }

    #[test]
    fn test_open_pr_filtering_for_base_updates() {
        // Test the filtering logic used by cleanup_merged_branches_for_sync_async
        // to find children with open PRs that need base updates

        let results: Vec<(String, Option<PrInfo>)> = vec![
            (
                "child-open".to_string(),
                Some(PrInfo {
                    number: 1,
                    url: "url".to_string(),
                    head_ref: "child-open".to_string(),
                    base_ref: "parent".to_string(),
                    state: PrState::Open,
                    title: "Open PR".to_string(),
                }),
            ),
            (
                "child-merged".to_string(),
                Some(PrInfo {
                    number: 2,
                    url: "url".to_string(),
                    head_ref: "child-merged".to_string(),
                    base_ref: "parent".to_string(),
                    state: PrState::Merged,
                    title: "Merged PR".to_string(),
                }),
            ),
            ("child-no-pr".to_string(), None),
        ];

        let new_base = "grandparent";

        // Apply the same filtering logic as cleanup_merged_branches_for_sync_async
        let base_updates: Vec<(String, String)> = results
            .into_iter()
            .filter_map(|(child, pr_info)| {
                pr_info.and_then(|info| {
                    if info.state == PrState::Open {
                        Some((child, new_base.to_string()))
                    } else {
                        None
                    }
                })
            })
            .collect();

        assert_eq!(base_updates.len(), 1);
        assert_eq!(base_updates[0].0, "child-open");
        assert_eq!(base_updates[0].1, "grandparent");
    }

    // === Original tests ===

    #[test]
    fn test_cleanup_no_merged_branches() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Should succeed with no branches to clean
        let result = run(true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cleanup_no_trunk_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // No trunk set
        let result = run(true);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No trunk"));
    }

    #[test]
    fn test_cleanup_merged_branches_for_sync_deletes_branch() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create a branch
        create_branch_with_commit(&repo, "feature-1").unwrap();

        let gateway = GitGateway::new().unwrap();
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("feature-1", "main").unwrap();

        let mut cache = Cache::load().unwrap_or_default();

        // Create a fake PrInfo for the merged branch
        let pr_info = PrInfo {
            number: 1,
            url: "https://github.com/test/repo/pull/1".to_string(),
            head_ref: "feature-1".to_string(),
            base_ref: "main".to_string(),
            state: PrState::Merged,
            title: "Test PR".to_string(),
        };

        let merged = vec![("feature-1".to_string(), pr_info)];

        // Verify branch exists before cleanup
        assert!(gateway.branch_exists("feature-1").unwrap());

        // Run cleanup
        let result = cleanup_merged_branches_for_sync(&gateway, &ref_store, &mut cache, "main", &merged, None).unwrap();

        // Verify branch was deleted
        assert_eq!(result, vec!["feature-1".to_string()]);
        assert!(!gateway.branch_exists("feature-1").unwrap());

        // Verify metadata was removed
        assert!(ref_store.get_parent("feature-1").unwrap().is_none());
    }

    #[test]
    fn test_cleanup_merged_branches_for_sync_reparents_children() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create parent and child branches
        create_branch_with_commit(&repo, "parent-branch").unwrap();
        create_branch_with_commit(&repo, "child-branch").unwrap();

        let gateway = GitGateway::new().unwrap();
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("parent-branch", "main").unwrap();
        ref_store.set_parent("child-branch", "parent-branch").unwrap();

        let mut cache = Cache::load().unwrap_or_default();

        // Create a fake PrInfo for the merged parent
        let pr_info = PrInfo {
            number: 1,
            url: "https://github.com/test/repo/pull/1".to_string(),
            head_ref: "parent-branch".to_string(),
            base_ref: "main".to_string(),
            state: PrState::Merged,
            title: "Test PR".to_string(),
        };

        let merged = vec![("parent-branch".to_string(), pr_info)];

        // Run cleanup
        let result = cleanup_merged_branches_for_sync(&gateway, &ref_store, &mut cache, "main", &merged, None).unwrap();

        // Verify parent was deleted
        assert_eq!(result, vec!["parent-branch".to_string()]);
        assert!(!gateway.branch_exists("parent-branch").unwrap());

        // Verify child was reparented to main (grandparent)
        let child_parent = ref_store.get_parent("child-branch").unwrap();
        assert_eq!(child_parent, Some("main".to_string()));

        // Verify child still exists
        assert!(gateway.branch_exists("child-branch").unwrap());
    }

    #[test]
    fn test_cleanup_merged_branches_for_sync_updates_base_sha() {
        let dir = tempdir().unwrap();
        let repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create parent and child branches
        create_branch_with_commit(&repo, "parent-branch").unwrap();
        create_branch_with_commit(&repo, "child-branch").unwrap();

        let gateway = GitGateway::new().unwrap();
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();
        ref_store.set_parent("parent-branch", "main").unwrap();
        ref_store.set_parent("child-branch", "parent-branch").unwrap();

        let mut cache = Cache::load().unwrap_or_default();

        // Create a fake PrInfo for the merged parent
        let pr_info = PrInfo {
            number: 1,
            url: "https://github.com/test/repo/pull/1".to_string(),
            head_ref: "parent-branch".to_string(),
            base_ref: "main".to_string(),
            state: PrState::Merged,
            title: "Test PR".to_string(),
        };

        let merged = vec![("parent-branch".to_string(), pr_info)];

        // Run cleanup
        cleanup_merged_branches_for_sync(&gateway, &ref_store, &mut cache, "main", &merged, None).unwrap();

        // Verify base_sha was updated for the reparented child
        let child_sha = gateway.get_branch_sha("child-branch").unwrap();
        let cached_sha = cache.get_base_sha("child-branch");
        assert_eq!(cached_sha, Some(child_sha.as_str()));
    }
}
