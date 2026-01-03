use anyhow::{Context, Result};
use colored::Colorize;

use crate::cache::Cache;
use crate::forge::{get_forge, PrInfo};
use crate::git_gateway::{GitGateway, SyncBranchResult};
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Get (download) a PR stack from the remote
///
/// By default, downloaded branches are frozen to prevent accidental modification.
/// Use `--unfrozen` to allow editing, or `dm unfreeze` later.
pub fn run(pr_ref: String, force: bool, unfrozen: bool) -> Result<()> {
    let gateway = GitGateway::new()?;

    // Get the forge
    let forge = get_forge(None)?;

    // Check auth
    forge.check_auth()?;

    println!("{} Getting PR info for {}...", "→".blue(), pr_ref.cyan());

    // Get the PR chain (parent-first order)
    let chain = forge.get_pr_chain(&pr_ref)?;

    if chain.is_empty() {
        anyhow::bail!("No PRs found for '{}'", pr_ref);
    }

    println!(
        "{} Found {} PR(s) in chain:",
        "✓".green(),
        chain.len().to_string().yellow()
    );
    for pr in &chain {
        println!("  • {} → {} ({})", pr.head_ref.green(), pr.base_ref.blue(), pr.url);
    }
    println!();

    // Fetch all branches
    println!("{} Fetching branches from {}...", "→".blue(), gateway.remote());
    gateway.fetch_origin()?;

    // Checkout and track each branch
    let ref_store = RefStore::new()?;
    let mut cache = Cache::load().unwrap_or_default();
    let trunk = ref_store.get_trunk()?;

    // Collect branches that we'll freeze
    let mut branches_to_freeze = Vec::new();

    for pr in &chain {
        checkout_and_track_pr(pr, &ref_store, &mut cache, trunk.as_deref(), &gateway, force)?;
        branches_to_freeze.push(pr.head_ref.clone());
    }

    cache.save()?;

    // Freeze branches by default (unless --unfrozen)
    if !unfrozen {
        for branch in &branches_to_freeze {
            ref_store.set_frozen(branch, true)?;
        }
        println!(
            "{} Froze {} branch{} (use '{} unfreeze' to allow modifications)",
            "❄".cyan(),
            branches_to_freeze.len(),
            if branches_to_freeze.len() == 1 { "" } else { "es" },
            program_name()
        );
    }

    // Checkout the tip (last in chain, which is the original PR)
    let tip = chain.last().unwrap();
    gateway.checkout_branch(&tip.head_ref)?;

    println!();
    println!(
        "{} Downloaded stack. Now on '{}'{}",
        "✓".green().bold(),
        tip.head_ref.green(),
        if unfrozen { "" } else { " (frozen)" }
    );

    Ok(())
}

/// Checkout a branch from remote and track it, or sync if it already exists
fn checkout_and_track_pr(
    pr: &PrInfo,
    ref_store: &RefStore,
    cache: &mut Cache,
    trunk: Option<&str>,
    gateway: &GitGateway,
    force: bool,
) -> Result<()> {
    let branch = &pr.head_ref;

    // Check if branch exists locally
    if !gateway.branch_exists(branch)? {
        // Create local tracking branch
        println!("  {} Creating local branch '{}'...", "→".blue(), branch.green());
        create_tracking_branch(branch, gateway.remote())?;
    } else {
        // Branch exists - sync it from remote
        match gateway.sync_branch_from_remote(branch, force)? {
            SyncBranchResult::Updated(n) => {
                println!(
                    "  {} Updated '{}' with {} commit(s) from remote",
                    "↓".blue(),
                    branch.green(),
                    n.to_string().yellow()
                );
            }
            SyncBranchResult::AlreadySynced => {
                println!("  {} Branch '{}' is up to date", "✓".green(), branch);
            }
            SyncBranchResult::LocalAhead(n) => {
                println!(
                    "  {} Branch '{}' has {} local commit(s) not on remote",
                    "!".yellow(),
                    branch.yellow(),
                    n.to_string().yellow()
                );
            }
            SyncBranchResult::ForceSynced => {
                println!("  {} Overwrote '{}' with remote (force)", "⚠".yellow(), branch.yellow());
            }
            SyncBranchResult::Diverged {
                local_ahead,
                remote_ahead,
            } => {
                println!(
                    "  {} Branch '{}' has diverged (+{} local, +{} remote)",
                    "⚠".yellow(),
                    branch.yellow(),
                    local_ahead,
                    remote_ahead
                );
                println!("    Use '--force' to overwrite local with remote");
            }
            SyncBranchResult::NoRemote => {
                println!("  {} Branch '{}' has no remote tracking", "?".yellow(), branch.yellow());
            }
        }
    }

    // Register parent relationship (skip if parent is trunk)
    if trunk != Some(&pr.base_ref) {
        ref_store.set_parent(branch, &pr.base_ref)?;
    } else {
        // Branch's parent is trunk, set it explicitly
        ref_store.set_parent(branch, &pr.base_ref)?;
    }

    // Store PR URL in cache
    cache.set_pr_url(branch, &pr.url);

    Ok(())
}

/// Create a local branch tracking the remote
fn create_tracking_branch(branch: &str, remote: &str) -> Result<()> {
    let status = std::process::Command::new("git")
        .args(["checkout", "-b", branch, &format!("{}/{}", remote, branch)])
        .status()
        .context("Failed to run git checkout")?;

    if !status.success() {
        anyhow::bail!("Failed to create tracking branch '{}'", branch);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::TestRepoContext;

    fn init_test_repo(path: &std::path::Path) -> Result<git2::Repository> {
        let repo = git2::Repository::init(path)?;
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
        drop(tree);
        Ok(repo)
    }

    fn create_branch(repo: &git2::Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[test]
    fn test_get_no_origin_fails() {
        let dir = tempdir().unwrap();
        let _repo = init_test_repo(dir.path()).unwrap();
        let _ctx = TestRepoContext::new(dir.path());

        // Create ref_store with trunk
        let ref_store = RefStore::new().unwrap();
        ref_store.set_trunk("main").unwrap();

        // Try to get a PR - should fail because no origin remote
        let result = run("123".to_string(), false, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_checkout_and_track_pr_registers_branch() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        let mut cache = Cache::load().unwrap_or_default();

        let pr = PrInfo {
            number: 123,
            url: "https://github.com/user/repo/pull/123".to_string(),
            head_ref: "feature-1".to_string(),
            base_ref: "main".to_string(),
            state: crate::forge::PrState::Open,
            title: "Test PR".to_string(),
        };

        // Simulate registration without checkout
        ref_store.set_parent("feature-1", "main")?;
        cache.set_pr_url("feature-1", &pr.url);

        assert_eq!(ref_store.get_parent("feature-1")?, Some("main".to_string()));
        assert_eq!(
            cache.get_pr_url("feature-1"),
            Some("https://github.com/user/repo/pull/123")
        );

        Ok(())
    }

    #[test]
    fn test_pr_chain_registration() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create "main" branch if it doesn't exist (it may already be the default branch)
        if repo.find_branch("main", git2::BranchType::Local).is_err() {
            create_branch(&repo, "main")?;
        }
        // Create "feature-1" branch (the parent of feature-2)
        create_branch(&repo, "feature-1")?;

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        let mut cache = Cache::load().unwrap_or_default();

        // Simulate a chain: main -> feature-1 -> feature-2
        let chain = vec![
            PrInfo {
                number: 1,
                url: "https://github.com/user/repo/pull/1".to_string(),
                head_ref: "feature-1".to_string(),
                base_ref: "main".to_string(),
                state: crate::forge::PrState::Open,
                title: "First PR".to_string(),
            },
            PrInfo {
                number: 2,
                url: "https://github.com/user/repo/pull/2".to_string(),
                head_ref: "feature-2".to_string(),
                base_ref: "feature-1".to_string(),
                state: crate::forge::PrState::Open,
                title: "Second PR".to_string(),
            },
        ];

        // Register branches as the get command would
        for pr in &chain {
            ref_store.set_parent(&pr.head_ref, &pr.base_ref)?;
            cache.set_pr_url(&pr.head_ref, &pr.url);
        }

        // Verify chain structure
        assert_eq!(ref_store.get_parent("feature-1")?, Some("main".to_string()));
        assert_eq!(ref_store.get_parent("feature-2")?, Some("feature-1".to_string()));

        let children = ref_store.get_children("feature-1")?;
        assert!(children.contains("feature-2"));

        Ok(())
    }

    /// Helper to create a test repo with origin remote for sync tests
    /// Returns (local_dir, origin_dir)
    fn setup_local_and_remote() -> Result<(tempfile::TempDir, tempfile::TempDir)> {
        // Create "origin" repo with initial commit (bare so we can push to it)
        let origin_dir = tempdir()?;
        {
            // Initialize non-bare first to create a commit
            let origin_repo = git2::Repository::init(origin_dir.path())?;
            let sig = git2::Signature::now("Test", "test@test.com")?;
            let tree_id = origin_repo.index()?.write_tree()?;
            let tree = origin_repo.find_tree(tree_id)?;
            origin_repo.commit(Some("HEAD"), &sig, &sig, "Initial", &tree, &[])?;
        }

        // Clone to create local repo with proper tracking
        let local_dir = tempdir()?;
        git2::Repository::clone(origin_dir.path().to_str().unwrap(), local_dir.path())?;

        Ok((local_dir, origin_dir))
    }

    /// Helper to add a commit to a repo
    fn add_commit(repo: &git2::Repository, message: &str) -> Result<git2::Oid> {
        let sig = git2::Signature::now("Test", "test@test.com")?;
        let head = repo.head()?.peel_to_commit()?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head])?;
        Ok(oid)
    }

    #[test]
    fn test_get_syncs_behind_branch() -> Result<()> {
        let (local_dir, origin_dir) = setup_local_and_remote()?;
        let _ctx = TestRepoContext::new(local_dir.path());

        let local_repo = git2::Repository::open(local_dir.path())?;
        let origin_repo = git2::Repository::open(origin_dir.path())?;

        // Create feature-1 branch on origin first
        {
            let head = origin_repo.head()?.peel_to_commit()?;
            origin_repo.branch("feature-1", &head, false)?;
        }

        // Fetch in local to get origin/feature-1
        {
            let mut remote = local_repo.find_remote("origin")?;
            remote.fetch(&["feature-1"], None, None)?;
        }

        // Create local feature-1 tracking origin/feature-1
        {
            let remote_ref = local_repo.find_reference("refs/remotes/origin/feature-1")?;
            let commit = remote_ref.peel_to_commit()?;
            local_repo.branch("feature-1", &commit, false)?;
        }

        // Add a commit to origin's feature-1 (simulating coworker push)
        {
            let feature_ref = origin_repo.find_reference("refs/heads/feature-1")?;
            let commit = feature_ref.peel_to_commit()?;
            let sig = git2::Signature::now("Coworker", "coworker@test.com")?;
            let tree_id = origin_repo.index()?.write_tree()?;
            let tree = origin_repo.find_tree(tree_id)?;
            origin_repo.commit(
                Some("refs/heads/feature-1"),
                &sig,
                &sig,
                "Coworker change",
                &tree,
                &[&commit],
            )?;
        }

        // Fetch to get the remote changes
        let gateway = GitGateway::new()?;
        gateway.fetch_origin()?;

        // Setup Diamond tracking
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        let mut cache = Cache::load().unwrap_or_default();

        let pr = PrInfo {
            number: 1,
            url: "https://github.com/test/repo/pull/1".to_string(),
            head_ref: "feature-1".to_string(),
            base_ref: "main".to_string(),
            state: crate::forge::PrState::Open,
            title: "Test".to_string(),
        };

        // Call checkout_and_track_pr - should sync the branch
        checkout_and_track_pr(&pr, &ref_store, &mut cache, Some("main"), &gateway, false)?;

        // Local branch should now have the remote commit
        let local_feature = local_repo.find_reference("refs/heads/feature-1")?;
        let remote_feature = local_repo.find_reference("refs/remotes/origin/feature-1")?;
        assert_eq!(
            local_feature.target().unwrap(),
            remote_feature.target().unwrap(),
            "Local branch should be updated to match remote"
        );

        Ok(())
    }

    #[test]
    fn test_get_warns_on_diverged_branch() -> Result<()> {
        let (local_dir, origin_dir) = setup_local_and_remote()?;
        let _ctx = TestRepoContext::new(local_dir.path());

        let local_repo = git2::Repository::open(local_dir.path())?;
        let origin_repo = git2::Repository::open(origin_dir.path())?;

        // Create feature-1 branch on origin first
        {
            let head = origin_repo.head()?.peel_to_commit()?;
            origin_repo.branch("feature-1", &head, false)?;
        }

        // Fetch in local to get origin/feature-1
        {
            let mut remote = local_repo.find_remote("origin")?;
            remote.fetch(&["feature-1"], None, None)?;
        }

        // Create local feature-1 tracking origin/feature-1
        {
            let remote_ref = local_repo.find_reference("refs/remotes/origin/feature-1")?;
            let commit = remote_ref.peel_to_commit()?;
            local_repo.branch("feature-1", &commit, false)?;
        }

        // Add a commit to origin's feature-1 (simulating coworker push)
        {
            let feature_ref = origin_repo.find_reference("refs/heads/feature-1")?;
            let commit = feature_ref.peel_to_commit()?;
            let sig = git2::Signature::now("Coworker", "coworker@test.com")?;
            let tree_id = origin_repo.index()?.write_tree()?;
            let tree = origin_repo.find_tree(tree_id)?;
            origin_repo.commit(
                Some("refs/heads/feature-1"),
                &sig,
                &sig,
                "Remote change",
                &tree,
                &[&commit],
            )?;
        }

        // Add a commit to local feature-1 (creating divergence)
        {
            local_repo.set_head("refs/heads/feature-1")?;
            local_repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
            add_commit(&local_repo, "Local change")?;
        }

        // Fetch to get the remote changes
        let gateway = GitGateway::new()?;
        gateway.fetch_origin()?;

        // Record local branch position before sync
        let local_before = local_repo.find_reference("refs/heads/feature-1")?.target().unwrap();

        // Setup Diamond tracking
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        let mut cache = Cache::load().unwrap_or_default();

        let pr = PrInfo {
            number: 1,
            url: "https://github.com/test/repo/pull/1".to_string(),
            head_ref: "feature-1".to_string(),
            base_ref: "main".to_string(),
            state: crate::forge::PrState::Open,
            title: "Test".to_string(),
        };

        // Call without force - should warn but not modify
        checkout_and_track_pr(&pr, &ref_store, &mut cache, Some("main"), &gateway, false)?;

        // Local branch should NOT have changed (diverged without force)
        let local_after = local_repo.find_reference("refs/heads/feature-1")?.target().unwrap();
        assert_eq!(
            local_before, local_after,
            "Local branch should not be modified when diverged without force"
        );

        Ok(())
    }

    #[test]
    fn test_get_force_overwrites_diverged_branch() -> Result<()> {
        let (local_dir, origin_dir) = setup_local_and_remote()?;
        let _ctx = TestRepoContext::new(local_dir.path());

        let local_repo = git2::Repository::open(local_dir.path())?;
        let origin_repo = git2::Repository::open(origin_dir.path())?;

        // Create feature-1 branch on origin first
        {
            let head = origin_repo.head()?.peel_to_commit()?;
            origin_repo.branch("feature-1", &head, false)?;
        }

        // Fetch in local to get origin/feature-1
        {
            let mut remote = local_repo.find_remote("origin")?;
            remote.fetch(&["feature-1"], None, None)?;
        }

        // Create local feature-1 tracking origin/feature-1
        {
            let remote_ref = local_repo.find_reference("refs/remotes/origin/feature-1")?;
            let commit = remote_ref.peel_to_commit()?;
            local_repo.branch("feature-1", &commit, false)?;
        }

        // Add a commit to origin's feature-1 (simulating coworker push)
        {
            let feature_ref = origin_repo.find_reference("refs/heads/feature-1")?;
            let commit = feature_ref.peel_to_commit()?;
            let sig = git2::Signature::now("Coworker", "coworker@test.com")?;
            let tree_id = origin_repo.index()?.write_tree()?;
            let tree = origin_repo.find_tree(tree_id)?;
            origin_repo.commit(
                Some("refs/heads/feature-1"),
                &sig,
                &sig,
                "Remote change",
                &tree,
                &[&commit],
            )?;
        }

        // Add a commit to local feature-1 (creating divergence)
        {
            local_repo.set_head("refs/heads/feature-1")?;
            local_repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
            add_commit(&local_repo, "Local change")?;
        }

        // Fetch to get the remote changes
        let gateway = GitGateway::new()?;
        gateway.fetch_origin()?;

        // Setup Diamond tracking
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        let mut cache = Cache::load().unwrap_or_default();

        let pr = PrInfo {
            number: 1,
            url: "https://github.com/test/repo/pull/1".to_string(),
            head_ref: "feature-1".to_string(),
            base_ref: "main".to_string(),
            state: crate::forge::PrState::Open,
            title: "Test".to_string(),
        };

        // Call WITH force - should overwrite local with remote
        checkout_and_track_pr(&pr, &ref_store, &mut cache, Some("main"), &gateway, true)?;

        // Local branch should now match remote
        let local_feature = local_repo.find_reference("refs/heads/feature-1")?;
        let remote_feature = local_repo.find_reference("refs/remotes/origin/feature-1")?;
        assert_eq!(
            local_feature.target().unwrap(),
            remote_feature.target().unwrap(),
            "Local branch should be overwritten to match remote with --force"
        );

        Ok(())
    }
}
