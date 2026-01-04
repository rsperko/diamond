//! Core submission logic for branches and stacks.

use anyhow::{Context, Result};
use colored::Colorize;

use crate::cache::Cache;
use crate::forge::{Forge, PrInfo, PrOptions};
use crate::git_gateway::{BranchSyncState, GitGateway};
use crate::program_name::program_name;
use crate::ref_store::RefStore;

use super::PrCache;

/// Get PR title for a branch - uses tip commit message or falls back to branch name
pub(super) fn get_pr_title_for_branch(gateway: &GitGateway, branch: &str) -> Result<String> {
    // Try to get commit subject (first line of commit message)
    match gateway.get_commit_subject(branch) {
        Ok(subject) => Ok(subject),
        Err(_) => {
            // No commits or error - fall back to formatted branch name
            Ok(branch.replace(['-', '_'], " "))
        }
    }
}

/// Push diverged ancestor branches that have existing PRs.
///
/// When a stack is rebased locally (e.g., via `dm sync` or `dm restack`),
/// intermediate branches get new commit hashes. If we submit a leaf branch
/// without first pushing these rebased ancestors, the PR on GitHub will
/// compare against stale parent commits and show incorrect diffs.
///
/// This function walks up the parent chain and pushes any diverged branches
/// that already have PRs, ensuring GitHub's PR base branches are up to date.
fn push_diverged_ancestors(
    branch: &str,
    ref_store: &RefStore,
    gateway: &GitGateway,
    forge: &dyn Forge,
    force: bool,
    pr_cache: &PrCache,
) -> Result<()> {
    let trunk = ref_store.get_trunk()?;

    // Collect ancestors in child-to-parent order
    let mut ancestors = Vec::new();
    let mut current = ref_store.get_parent(branch)?;

    while let Some(parent) = current {
        // Stop at trunk
        if trunk.as_ref() == Some(&parent) {
            break;
        }
        ancestors.push(parent.clone());
        current = ref_store.get_parent(&parent)?;
    }

    // Reverse to get oldest-first order (push parents before children)
    ancestors.reverse();

    // For each ancestor, check if it's diverged AND has a PR (using cache)
    for ancestor in &ancestors {
        let sync_state = gateway.check_remote_sync(ancestor)?;

        if let BranchSyncState::Diverged { .. } = sync_state {
            // Check if this branch has a PR (from cache, fallback to API if not cached)
            let has_pr = pr_cache
                .get(ancestor)
                .map(|opt| opt.is_some())
                .unwrap_or_else(|| forge.pr_exists(ancestor).ok().flatten().is_some());

            if has_pr {
                println!("{} Pushing rebased ancestor {}...", "↑".blue(), ancestor.yellow());
                forge.push_branch(ancestor, force)?;

                // Also push diamond ref in case parent changed
                if let Err(e) = gateway.push_diamond_ref(ancestor) {
                    eprintln!("  {} Could not push diamond ref: {}", "!".yellow(), e);
                }
            }
        }
    }

    Ok(())
}

/// Check if a branch is safe to push (only blocks if behind remote)
///
/// Note: We intentionally allow "diverged" state because:
/// 1. The push uses --force-with-lease by default, which protects against overwriting
///    someone else's changes (fails if remote changed since last fetch)
/// 2. "Diverged" is the normal state after amending a pushed commit (common workflow)
/// 3. Should allow this workflow - just tries the push and lets git handle it
pub(super) fn check_branch_sync_state(gateway: &GitGateway, branch: &str) -> Result<()> {
    match gateway.check_remote_sync(branch)? {
        BranchSyncState::Behind(n) => {
            // Behind means remote has commits we don't have - this is unusual in a
            // stacked PR workflow and could indicate someone else pushed to our branch.
            // Block to prevent accidentally overwriting their work.
            anyhow::bail!(
                "Branch '{}' is {} commit{} behind remote.\n\
                 Run '{} sync' to pull changes first, or use '--force' to overwrite.",
                branch,
                n,
                if n == 1 { "" } else { "s" },
                program_name()
            );
        }
        // Diverged is OK - this happens after amending. The --force-with-lease push
        // will protect against overwriting changes made by others since our last fetch.
        // InSync, Ahead, NoRemote - all safe to push
        _ => Ok(()),
    }
}

/// Submit a single branch
#[allow(clippy::too_many_arguments)]
pub(super) fn submit_branch(
    branch: &str,
    ref_store: &RefStore,
    gateway: &GitGateway,
    forge: &dyn Forge,
    force: bool,
    options: &PrOptions,
    created_urls: &mut Vec<String>,
    update_only: bool,
    pr_cache: &PrCache,
) -> Result<()> {
    // First, ensure any diverged ancestor branches are pushed.
    // This prevents PRs from showing incorrect diffs when the stack was rebased locally.
    push_diverged_ancestors(branch, ref_store, gateway, forge, force, pr_cache)?;

    let trunk = ref_store.get_trunk()?;

    // Get parent for this branch
    let parent = ref_store.get_parent(branch)?;

    // Determine base branch (parent or trunk)
    let base = parent
        .as_ref()
        .or(trunk.as_ref())
        .context("Cannot determine base branch for PR")?;

    // Check if PR already exists (from cache, fallback to API if not cached)
    let existing_pr: Option<PrInfo> = pr_cache
        .get(branch)
        .cloned()
        .flatten()
        .or_else(|| forge.pr_exists(branch).ok().flatten());

    if let Some(ref pr_info) = existing_pr {
        // PR exists - push updates
        println!(
            "{} Updating PR #{} for {}...",
            "→".blue(),
            pr_info.number,
            branch.green()
        );

        // Check for remote divergence before pushing (safety check)
        if !force {
            check_branch_sync_state(gateway, branch)?;
        }

        // Push the branch
        forge.push_branch(branch, force)?;

        // Still push diamond ref in case it changed
        if let Err(e) = gateway.push_diamond_ref(branch) {
            eprintln!("  {} Could not push diamond ref: {}", "!".yellow(), e);
        }

        // Handle publish - mark draft PR as ready for review
        if options.publish {
            match forge.mark_pr_ready(branch) {
                Ok(()) => {
                    println!("  {} Marked as ready for review", "✓".green());
                }
                Err(e) => {
                    eprintln!("  {} Could not mark as ready: {}", "!".yellow(), e);
                }
            }
        }

        // Handle merge-when-ready - enable auto-merge
        if options.merge_when_ready {
            match forge.enable_auto_merge(branch, "squash") {
                Ok(()) => {
                    println!("  {} Auto-merge enabled", "✓".green());
                }
                Err(e) => {
                    eprintln!("  {} Could not enable auto-merge: {}", "!".yellow(), e);
                }
            }
        }

        println!(
            "{} Updated {}: {}",
            "✓".green().bold(),
            branch.green(),
            pr_info.url.blue()
        );
        return Ok(());
    }

    // No PR exists - check if we should skip (update_only mode)
    if update_only {
        println!("{} Skipping {} (no PR, --update-only)", "⏭".dimmed(), branch.yellow());
        return Ok(());
    }

    // Ensure parent branch has a PR before creating one for this branch
    // (If parent is a tracked branch and not trunk, submit it first)
    let is_trunk = trunk.as_ref() == Some(base);
    let is_tracked_branch = ref_store.get_parent(base)?.is_some() || trunk.as_ref() == Some(base);

    if !is_trunk && is_tracked_branch {
        // Check if parent already has a PR (from cache, fallback to API)
        let parent_has_pr = pr_cache
            .get(base)
            .map(|opt| opt.is_some())
            .unwrap_or_else(|| forge.pr_exists(base).ok().flatten().is_some());

        if !parent_has_pr {
            if update_only {
                // In update_only mode, don't recursively create parent PRs
                anyhow::bail!(
                    "Cannot create PR for '{}': parent '{}' has no PR.\n\
                     Use '{} submit --stack' without --update-only to create all PRs.",
                    branch,
                    base,
                    program_name()
                );
            }
            println!("Parent branch {} needs a PR first, submitting it...\n", base.yellow());
            // Recursively submit the parent branch
            submit_branch(
                base,
                ref_store,
                gateway,
                forge,
                force,
                options,
                created_urls,
                update_only,
                pr_cache,
            )?;
            println!(); // Add spacing after parent submission
        }
    }

    // Check for remote divergence before pushing (safety check)
    if !force {
        check_branch_sync_state(gateway, branch)?;
    }

    // Now push and create PR for this branch
    println!("Pushing {} → {}...", branch.green(), gateway.remote());
    forge.push_branch(branch, force)?;

    // Push diamond parent ref for collaboration (Phase 2)
    if let Err(e) = gateway.push_diamond_ref(branch) {
        eprintln!("  {} Could not push diamond ref: {}", "!".yellow(), e);
    }

    // Create new PR
    let draft_str = if options.draft { " (draft)" } else { "" };
    println!("Creating PR{}: {} → {}...", draft_str, branch.green(), base.blue());

    // Use tip commit message as title, fall back to branch name if no commits
    let title = get_pr_title_for_branch(gateway, branch)?;
    let body = String::new();

    let url = forge.create_pr(branch, base, &title, &body, options)?;

    // Update cache with PR URL
    let mut cache = Cache::load().unwrap_or_default();
    cache.set_pr_url(branch, &url);
    cache.save()?;

    // Handle merge-when-ready for new PRs
    if options.merge_when_ready {
        match forge.enable_auto_merge(branch, "squash") {
            Ok(()) => {
                println!("  {} Auto-merge enabled", "✓".green());
            }
            Err(e) => {
                eprintln!("  {} Could not enable auto-merge: {}", "!".yellow(), e);
            }
        }
    }

    println!("{} Created PR: {}", "✓".green().bold(), url.blue());

    // Track newly created PR for opening in browser
    created_urls.push(url);

    Ok(())
}

/// Submit all branches in the stack (parent-first order)
#[allow(clippy::too_many_arguments)]
pub(super) fn submit_stack(
    branch: &str,
    ref_store: &RefStore,
    gateway: &GitGateway,
    forge: &dyn Forge,
    force: bool,
    options: &PrOptions,
    created_urls: &mut Vec<String>,
    update_only: bool,
    pr_cache: &PrCache,
) -> Result<()> {
    // Collect all descendants in DFS order (parent-first)
    let mut to_submit = vec![branch.to_string()];
    let mut i = 0;

    while i < to_submit.len() {
        let current = &to_submit[i].clone();
        let mut children: Vec<_> = ref_store.get_children(current)?.into_iter().collect();
        children.sort();
        to_submit.extend(children);
        i += 1;
    }

    println!("Submitting stack of {} branches:", to_submit.len().to_string().yellow());
    for b in &to_submit {
        println!("  • {}", b.green());
    }
    println!();

    // Submit each branch in order (using pre-fetched PR cache)
    for b in &to_submit {
        submit_branch(
            b,
            ref_store,
            gateway,
            forge,
            force,
            options,
            created_urls,
            update_only,
            pr_cache,
        )?;
    }

    Ok(())
}
