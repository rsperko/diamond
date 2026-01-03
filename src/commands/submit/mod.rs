//! Submit command - push branches and create/update PRs.

mod submission;
mod validation;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};

use anyhow::Result;
use colored::Colorize;

use crate::forge::{get_async_forge, get_forge, PrInfo, PrOptions};
use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::stack_viz::{collect_full_stack, update_stack_visualization_async};

use self::submission::{submit_branch, submit_stack};
use self::validation::{check_trunk_sync, show_submit_preview_async, validate_stack_integrity};

/// PR existence cache - maps branch name to optional PR info
pub(crate) type PrCache = HashMap<String, Option<PrInfo>>;

/// Collect all branches that need PR existence checks (branches + their ancestors)
fn collect_branches_for_pr_check(branches: &[String], ref_store: &RefStore) -> Result<Vec<String>> {
    let mut all_branches = std::collections::HashSet::new();
    let trunk = ref_store.get_trunk()?;

    for branch in branches {
        // Add the branch itself
        all_branches.insert(branch.clone());

        // Walk up to trunk, collecting ancestors
        let mut current = ref_store.get_parent(branch)?;
        while let Some(parent) = current {
            if trunk.as_ref() == Some(&parent) {
                break;
            }
            all_branches.insert(parent.clone());
            current = ref_store.get_parent(&parent)?;
        }
    }

    Ok(all_branches.into_iter().collect())
}

/// Submit the current branch or stack by pushing and creating PRs (default: submit current branch only)
#[allow(clippy::too_many_arguments)]
pub async fn run(
    stack: bool,
    force: bool,
    draft: bool,
    publish: bool,
    merge_when_ready: bool,
    target_branch: Option<String>,
    reviewers: Vec<String>,
    no_open: bool,
    skip_validation: bool,
    update_only: bool,
    confirm: bool,
) -> Result<()> {
    let gateway = GitGateway::new()?;
    let current = if let Some(ref branch) = target_branch {
        // Verify the branch exists
        if !gateway.branch_exists(branch)? {
            anyhow::bail!("Branch '{}' does not exist", branch);
        }
        branch.clone()
    } else {
        gateway.get_current_branch_name()?
    };
    let ref_store = RefStore::new()?;
    let trunk = ref_store.get_trunk()?;

    // Verify branch is tracked
    let parent = ref_store.get_parent(&current)?;
    if parent.is_none() && trunk.as_ref() != Some(&current) {
        anyhow::bail!(
            "Branch '{}' is not tracked. Run '{} track' first.",
            current,
            program_name()
        );
    }

    // Get the forge for this repo (both sync and async versions)
    let forge = get_forge(None)?;
    let async_forge = get_async_forge(None)?;

    // Check auth before proceeding
    forge.check_auth()?;

    // Pre-flight validation (unless skipped)
    if !skip_validation {
        validate_stack_integrity(&current, &ref_store, &gateway, trunk.as_deref())?;
        check_trunk_sync(&gateway, trunk.as_deref())?;
    }

    let options = PrOptions {
        draft,
        publish,
        merge_when_ready,
        reviewers,
    };

    // Build list of branches to submit for preview/confirmation
    // Default: submit current branch only
    // --stack: submit downstack + all descendants (full stack)
    let branches_to_submit: Vec<String> = if stack {
        // Full stack: downstack + all descendants
        let mut all = ref_store.ancestors(&current)?;
        for descendant in ref_store.collect_branches_dfs(std::slice::from_ref(&current))? {
            if !all.contains(&descendant) {
                all.push(descendant);
            }
        }
        all
    } else {
        // Default: current branch only
        vec![current.clone()]
    };

    // Show confirmation prompt if requested (uses async for batch PR checks)
    if confirm {
        show_submit_preview_async(&branches_to_submit, &ref_store, async_forge.as_ref(), update_only).await?;

        // Check if stdin is a TTY before prompting
        if !io::stdin().is_terminal() {
            anyhow::bail!("Cannot use --confirm in non-interactive mode. Remove --confirm or use a TTY.");
        }

        print!("\nProceed? [y/N]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{} Submit cancelled", "✗".yellow());
            return Ok(());
        }
        println!();
    }

    // Pre-check PR existence for all branches (batch async for performance)
    let branches_to_check = collect_branches_for_pr_check(&branches_to_submit, &ref_store)?;
    let pr_results = async_forge.check_prs_exist(&branches_to_check).await;
    let pr_cache: PrCache = pr_results.into_iter().collect();

    // Collect newly created PR URLs
    let mut created_urls: Vec<String> = Vec::new();

    if stack {
        submit_stack(
            &current,
            &ref_store,
            &gateway,
            forge.as_ref(),
            force,
            &options,
            &mut created_urls,
            update_only,
            &pr_cache,
        )?;
    } else {
        submit_branch(
            &current,
            &ref_store,
            &gateway,
            forge.as_ref(),
            force,
            &options,
            &mut created_urls,
            update_only,
            &pr_cache,
        )?;
    }

    // Show summary
    if !created_urls.is_empty() {
        println!(
            "\n{} Created {} PR{}",
            "✓".green().bold(),
            created_urls.len(),
            if created_urls.len() == 1 { "" } else { "s" }
        );

        // Update stack visualization in all PRs (once, at the end, using async for parallelism)
        let full_stack = collect_full_stack(&current, &ref_store)?;
        // Show beautiful progress - the tracker handles the summary output
        let _updated = update_stack_visualization_async(&full_stack, async_forge.as_ref(), &ref_store, true).await?;
    }

    // Open newly created PRs in browser (unless --no-open)
    if !no_open && !created_urls.is_empty() {
        for url in &created_urls {
            if let Err(e) = open::that(url) {
                eprintln!("{} Failed to open {}: {}", "⚠".yellow(), url, e);
            }
        }
    }

    Ok(())
}
