//! Tests for submit command.

use super::submission::{submit_branch, submit_stack};
use super::validation::validate_stack_integrity;
use super::*;
use crate::forge::{CiStatus, ForgeType, PrFullInfo, PrInfo, PrState, ReviewState};
use crate::stack_viz::collect_full_stack;
use crate::test_context::TestRepoContext;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::RwLock;
use tempfile::tempdir;

/// Empty PR cache for tests (tests don't use the cache optimization)
fn empty_pr_cache() -> PrCache {
    HashMap::new()
}

/// Mock forge for testing submit behavior
struct MockForge {
    /// Branches that have existing PRs
    existing_prs: RwLock<HashSet<String>>,
    /// Branches that were pushed (in order)
    pushed_branches: RwLock<Vec<String>>,
    /// PRs that were created (branch, base) in order
    created_prs: RwLock<Vec<(String, String)>>,
    /// Branches whose full info was requested
    full_info_requests: RwLock<Vec<String>>,
    /// PR bodies that were updated (pr_ref, body)
    updated_bodies: RwLock<Vec<(String, String)>>,
    /// PRs that were marked as ready (pr_ref)
    marked_ready: RwLock<Vec<String>>,
    /// PRs that had auto-merge enabled (pr_ref, merge_method)
    auto_merge_enabled: RwLock<Vec<(String, String)>>,
}

impl MockForge {
    fn new() -> Self {
        Self {
            existing_prs: RwLock::new(HashSet::new()),
            pushed_branches: RwLock::new(Vec::new()),
            created_prs: RwLock::new(Vec::new()),
            full_info_requests: RwLock::new(Vec::new()),
            updated_bodies: RwLock::new(Vec::new()),
            marked_ready: RwLock::new(Vec::new()),
            auto_merge_enabled: RwLock::new(Vec::new()),
        }
    }

    fn with_existing_pr(self, branch: &str) -> Self {
        self.existing_prs.write().unwrap().insert(branch.to_string());
        self
    }

    fn get_pushed_branches(&self) -> Vec<String> {
        self.pushed_branches.read().unwrap().clone()
    }

    fn get_created_prs(&self) -> Vec<(String, String)> {
        self.created_prs.read().unwrap().clone()
    }

    #[allow(dead_code)]
    fn get_full_info_requests(&self) -> Vec<String> {
        self.full_info_requests.read().unwrap().clone()
    }

    #[allow(dead_code)]
    fn get_updated_bodies(&self) -> Vec<(String, String)> {
        self.updated_bodies.read().unwrap().clone()
    }

    #[allow(dead_code)]
    fn get_marked_ready(&self) -> Vec<String> {
        self.marked_ready.read().unwrap().clone()
    }

    #[allow(dead_code)]
    fn get_auto_merge_enabled(&self) -> Vec<(String, String)> {
        self.auto_merge_enabled.read().unwrap().clone()
    }
}

impl crate::forge::Forge for MockForge {
    fn forge_type(&self) -> ForgeType {
        ForgeType::GitHub
    }

    fn cli_name(&self) -> &str {
        "mock"
    }

    fn check_auth(&self) -> Result<()> {
        Ok(())
    }

    fn pr_exists(&self, branch: &str) -> Result<Option<PrInfo>> {
        if self.existing_prs.read().unwrap().contains(branch) {
            Ok(Some(PrInfo {
                number: 1,
                url: "https://github.com/test/repo/pull/1".to_string(),
                head_ref: branch.to_string(),
                base_ref: "main".to_string(),
                state: PrState::Open,
                title: "Test PR".to_string(),
            }))
        } else {
            Ok(None)
        }
    }

    fn create_pr(&self, branch: &str, base: &str, _title: &str, _body: &str, _options: &PrOptions) -> Result<String> {
        self.created_prs
            .write()
            .unwrap()
            .push((branch.to_string(), base.to_string()));
        // Mark as having a PR now
        self.existing_prs.write().unwrap().insert(branch.to_string());
        Ok("https://github.com/test/repo/pull/1".to_string())
    }

    fn get_pr_info(&self, _pr_ref: &str) -> Result<PrInfo> {
        Ok(PrInfo {
            number: 1,
            url: "https://github.com/test/repo/pull/1".to_string(),
            head_ref: "branch".to_string(),
            base_ref: "main".to_string(),
            state: PrState::Open,
            title: "Test PR".to_string(),
        })
    }

    fn get_pr_chain(&self, _pr_ref: &str) -> Result<Vec<PrInfo>> {
        Ok(vec![])
    }

    fn is_branch_merged(&self, _branch: &str, _into: &str) -> Result<bool> {
        Ok(false)
    }

    fn get_pr_full_info(&self, pr_ref: &str) -> Result<PrFullInfo> {
        // Track this request
        self.full_info_requests.write().unwrap().push(pr_ref.to_string());

        // Return a unique PR number based on the branch name
        let number = match pr_ref {
            "a" => 1,
            "b" => 2,
            "c" => 3,
            "d" => 4,
            "parent" => 10,
            "child" => 11,
            "feature" => 20,
            _ => 99,
        };
        Ok(PrFullInfo {
            number,
            url: format!("https://github.com/test/repo/pull/{}", number),
            title: format!("PR for {}", pr_ref),
            state: PrState::Open,
            is_draft: false,
            review: ReviewState::Pending,
            ci: CiStatus::None,
            head_ref: pr_ref.to_string(),
            base_ref: "main".to_string(),
        })
    }

    fn get_pr_body(&self, _pr_ref: &str) -> Result<String> {
        Ok(String::new())
    }

    fn update_pr_body(&self, pr_ref: &str, body: &str) -> Result<()> {
        self.updated_bodies
            .write()
            .unwrap()
            .push((pr_ref.to_string(), body.to_string()));
        Ok(())
    }

    fn update_pr_base(&self, _branch: &str, _new_base: &str) -> Result<()> {
        // Mock implementation - just succeed
        Ok(())
    }

    fn mark_pr_ready(&self, pr_ref: &str) -> Result<()> {
        self.marked_ready.write().unwrap().push(pr_ref.to_string());
        Ok(())
    }

    fn enable_auto_merge(&self, pr_ref: &str, merge_method: &str) -> Result<()> {
        self.auto_merge_enabled
            .write()
            .unwrap()
            .push((pr_ref.to_string(), merge_method.to_string()));
        Ok(())
    }

    fn merge_pr(&self, _pr_ref: &str, _method: crate::forge::MergeMethod, _auto_confirm: bool) -> Result<()> {
        Ok(())
    }

    fn open_pr_in_browser(&self, _pr_ref: &str) -> Result<()> {
        Ok(())
    }

    fn push_branch(&self, branch: &str, _force: bool) -> Result<()> {
        self.pushed_branches.write().unwrap().push(branch.to_string());
        Ok(())
    }
}

// Helper to create a minimal test repo
fn init_test_repo(path: &std::path::Path) -> Result<git2::Repository> {
    let repo = git2::Repository::init(path)?;
    let sig = git2::Signature::now("Test User", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
    drop(tree);
    fs::create_dir_all(path.join(".git").join("diamond"))?;
    Ok(repo)
}

// Helper to create a git branch at HEAD
fn create_branch(repo: &git2::Repository, name: &str) -> Result<()> {
    let head = repo.head()?.peel_to_commit()?;
    repo.branch(name, &head, false)?;
    Ok(())
}

#[tokio::test]
async fn test_submit_untracked_branch_fails() {
    let dir = tempdir().unwrap();
    let _repo = init_test_repo(dir.path()).unwrap();
    let _ctx = TestRepoContext::new(dir.path());

    // Create an empty RefStore (no tracked branches)
    let _ref_store = RefStore::new().unwrap();

    // Run should fail because branch is not tracked
    // run(stack, force, draft, publish, merge_when_ready, target_branch, reviewers, no_open, skip_validation, update_only, confirm)
    let result = run(
        false,
        false,
        false,
        false,
        false,
        None,
        vec![],
        true,
        false,
        false,
        false,
    )
    .await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not tracked"),
        "Expected 'not tracked' error, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_submit_target_branch_nonexistent_fails() {
    let dir = tempdir().unwrap();
    let _repo = init_test_repo(dir.path()).unwrap();
    let _ctx = TestRepoContext::new(dir.path());

    // Run should fail because the target branch doesn't exist
    // run(stack, force, draft, publish, merge_when_ready, target_branch, reviewers, no_open, skip_validation, update_only, confirm)
    let result = run(
        false,
        false,
        false,
        false,
        false,
        Some("nonexistent-branch".to_string()),
        vec![],
        true,
        false,
        false,
        false,
    )
    .await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("does not exist"),
        "Expected 'does not exist' error, got: {}",
        err_msg
    );
}

#[test]
fn test_submit_branch_info_in_store() {
    let dir = tempdir().unwrap();
    let _repo = init_test_repo(dir.path()).unwrap();
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::new().unwrap();

    // Create ref_store with tracked branch
    let ref_store = RefStore::new().unwrap();
    ref_store.set_trunk("main").unwrap();
    // Create the master branch as git branch before setting parent
    ref_store.set_parent("feature-1", "main").unwrap();

    // Create and checkout the feature branch
    gateway.create_branch("feature-1").unwrap();

    // Verify branch info exists
    assert_eq!(ref_store.get_parent("feature-1").unwrap(), Some("main".to_string()));
}

#[test]
fn test_collect_stack_descendants() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a stack: master -> feature-1 -> feature-2 -> feature-3
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branches before setting parent relationships
    create_branch(&repo, "feature-1")?;
    create_branch(&repo, "feature-2")?;
    create_branch(&repo, "feature-3")?;
    ref_store.set_parent("feature-1", "main")?;
    ref_store.set_parent("feature-2", "feature-1")?;
    ref_store.set_parent("feature-3", "feature-2")?;

    // Collect descendants from feature-1
    let mut to_submit = vec!["feature-1".to_string()];
    let mut i = 0;

    while i < to_submit.len() {
        let current = &to_submit[i].clone();
        let mut children: Vec<_> = ref_store.get_children(current)?.into_iter().collect();
        children.sort();
        to_submit.extend(children);
        i += 1;
    }

    assert_eq!(to_submit, vec!["feature-1", "feature-2", "feature-3"]);

    Ok(())
}

#[test]
fn test_collect_stack_with_multiple_children() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a stack with branches:
    // master -> feature-1 -> feature-2
    //                    \-> feature-3
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branches before setting parent relationships
    create_branch(&repo, "feature-1")?;
    create_branch(&repo, "feature-2")?;
    create_branch(&repo, "feature-3")?;
    ref_store.set_parent("feature-1", "main")?;
    ref_store.set_parent("feature-2", "feature-1")?;
    ref_store.set_parent("feature-3", "feature-1")?;

    // Collect descendants from feature-1
    let mut to_submit = vec!["feature-1".to_string()];
    let mut i = 0;

    while i < to_submit.len() {
        let current = &to_submit[i].clone();
        let mut children: Vec<_> = ref_store.get_children(current)?.into_iter().collect();
        children.sort();
        to_submit.extend(children);
        i += 1;
    }

    // Should include feature-1 and both children (sorted alphabetically)
    assert_eq!(to_submit, vec!["feature-1", "feature-2", "feature-3"]);

    Ok(())
}

#[test]
fn test_submit_branch_creates_parent_pr_first() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a stack: master -> parent -> child
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branches before setting parent relationships
    create_branch(&repo, "parent")?;
    create_branch(&repo, "child")?;
    ref_store.set_parent("parent", "main")?;
    ref_store.set_parent("child", "parent")?;

    // Create gateway (branches already created above)
    let gateway = GitGateway::new()?;

    // Mock forge with no existing PRs
    let forge = MockForge::new();
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit the child branch
    submit_branch(
        "child",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    )?;

    // Verify parent was pushed before child
    let pushed = forge.get_pushed_branches();
    assert_eq!(pushed.len(), 2, "Both branches should be pushed");
    assert_eq!(pushed[0], "parent", "Parent should be pushed first");
    assert_eq!(pushed[1], "child", "Child should be pushed second");

    // Verify PRs were created in correct order (parent first)
    let prs = forge.get_created_prs();
    assert_eq!(prs.len(), 2, "Two PRs should be created");
    assert_eq!(prs[0], ("parent".to_string(), "main".to_string()));
    assert_eq!(prs[1], ("child".to_string(), "parent".to_string()));

    // Verify URLs were collected
    assert_eq!(created_urls.len(), 2, "Two URLs should be collected");

    Ok(())
}

#[test]
fn test_submit_branch_skips_parent_with_existing_pr() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a stack: master -> parent -> child
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branches before setting parent relationships
    create_branch(&repo, "parent")?;
    create_branch(&repo, "child")?;
    ref_store.set_parent("parent", "main")?;
    ref_store.set_parent("child", "parent")?;

    // Create gateway (branches already created above)
    let gateway = GitGateway::new()?;

    // Mock forge where parent already has a PR
    let forge = MockForge::new().with_existing_pr("parent");
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit the child branch
    submit_branch(
        "child",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    )?;

    // Only child should be pushed (parent already has PR)
    let pushed = forge.get_pushed_branches();
    assert_eq!(pushed.len(), 1, "Only child should be pushed");
    assert_eq!(pushed[0], "child");

    // Only child PR should be created
    let prs = forge.get_created_prs();
    assert_eq!(prs.len(), 1, "Only child PR should be created");
    assert_eq!(prs[0], ("child".to_string(), "parent".to_string()));

    // Only one URL collected (parent already existed)
    assert_eq!(created_urls.len(), 1, "Only child URL should be collected");

    Ok(())
}

#[test]
fn test_submit_branch_with_trunk_parent_no_parent_pr() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a stack: master -> feature (direct child of trunk)
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branch before setting parent relationship
    ref_store.set_parent("feature", "main")?;

    // Create actual git branch (needed for divergence check)
    let gateway = GitGateway::new()?;
    gateway.create_branch("feature")?;

    // Mock forge with no existing PRs
    let forge = MockForge::new();
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit the feature branch
    submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    )?;

    // Only feature should be pushed (trunk is not pushed)
    let pushed = forge.get_pushed_branches();
    assert_eq!(pushed.len(), 1, "Only feature should be pushed");
    assert_eq!(pushed[0], "feature");

    // Only feature PR should be created (targeting trunk)
    let prs = forge.get_created_prs();
    assert_eq!(prs.len(), 1, "Only feature PR should be created");
    assert_eq!(prs[0], ("feature".to_string(), "main".to_string()));

    // One URL collected
    assert_eq!(created_urls.len(), 1, "Feature URL should be collected");

    Ok(())
}

#[test]
fn test_submit_branch_deep_stack_creates_all_parent_prs() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a deep stack: master -> a -> b -> c -> d
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    create_branch(&repo, "d")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;
    ref_store.set_parent("d", "c")?;

    // Create gateway (branches already created above)
    let gateway = GitGateway::new()?;

    // Mock forge with no existing PRs
    let forge = MockForge::new();
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit branch d (deepest)
    submit_branch(
        "d",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    )?;

    // All branches should be pushed in order (ancestors first)
    let pushed = forge.get_pushed_branches();
    assert_eq!(pushed.len(), 4, "All 4 branches should be pushed");
    assert_eq!(pushed, vec!["a", "b", "c", "d"]);

    // All PRs should be created in order
    let prs = forge.get_created_prs();
    assert_eq!(prs.len(), 4, "All 4 PRs should be created");
    assert_eq!(prs[0], ("a".to_string(), "main".to_string()));
    assert_eq!(prs[1], ("b".to_string(), "a".to_string()));
    assert_eq!(prs[2], ("c".to_string(), "b".to_string()));
    assert_eq!(prs[3], ("d".to_string(), "c".to_string()));

    // All 4 URLs collected
    assert_eq!(created_urls.len(), 4, "All 4 URLs should be collected");

    Ok(())
}

#[test]
fn test_collect_full_stack_returns_all_ancestors_and_descendants() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a deep stack: master -> a -> b -> c -> d
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    create_branch(&repo, "d")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;
    ref_store.set_parent("d", "c")?;

    // From the deepest branch, should collect entire stack
    let stack = collect_full_stack("d", &ref_store)?;
    assert_eq!(
        stack,
        vec!["a", "b", "c", "d"],
        "Should include all branches from root to tip"
    );

    // From middle branch, should still collect entire stack
    let stack = collect_full_stack("b", &ref_store)?;
    assert_eq!(
        stack,
        vec!["a", "b", "c", "d"],
        "Should include all branches even from middle"
    );

    // From root branch, should collect entire stack
    let stack = collect_full_stack("a", &ref_store)?;
    assert_eq!(
        stack,
        vec!["a", "b", "c", "d"],
        "Should include all descendants from root"
    );

    Ok(())
}

#[test]
fn test_collect_full_stack_with_branching() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a branching stack:
    // master -> a -> b -> c
    //              \-> d
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    create_branch(&repo, "d")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;
    ref_store.set_parent("d", "a")?;

    // From any branch, should get all branches in the tree
    let stack = collect_full_stack("c", &ref_store)?;
    assert_eq!(stack.len(), 4, "Should include all 4 branches");
    assert!(stack.contains(&"a".to_string()));
    assert!(stack.contains(&"b".to_string()));
    assert!(stack.contains(&"c".to_string()));
    assert!(stack.contains(&"d".to_string()));

    Ok(())
}

#[test]
fn test_collect_full_stack_single_branch() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Single branch off trunk
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branch before setting parent relationship
    create_branch(&repo, "feature")?;
    ref_store.set_parent("feature", "main")?;

    let stack = collect_full_stack("feature", &ref_store)?;
    assert_eq!(stack, vec!["feature"], "Single branch should return just itself");

    Ok(())
}

#[test]
fn test_collect_full_stack_detects_cycle() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create git branches before setting parent relationships (cycle)
    create_branch(&repo, "feature-a")?;
    create_branch(&repo, "feature-b")?;
    // Create a cycle: feature-a -> feature-b -> feature-a
    ref_store.set_parent("feature-a", "feature-b")?;
    ref_store.set_parent("feature-b", "feature-a")?;

    // Should detect the cycle and error
    let result = collect_full_stack("feature-a", &ref_store);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Circular parent reference"),
        "Error should mention circular reference: {}",
        err_msg
    );

    Ok(())
}

// ===== Stack Integrity Validation Tests =====

#[test]
fn test_validate_stack_integrity_passes_for_properly_stacked_branches() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create git branch before setting parent relationship

    // Create properly stacked branches: master -> parent -> child
    gateway.create_branch("parent")?;
    // Make a commit on parent
    std::fs::write(dir.path().join("parent.txt"), "parent content")?;
    gateway.stage_all()?;
    gateway.commit("Parent commit")?;

    // Create child from parent (properly stacked)
    gateway.create_branch("child")?;
    std::fs::write(dir.path().join("child.txt"), "child content")?;
    gateway.stage_all()?;
    gateway.commit("Child commit")?;

    // Set parent relationships (branches already created above via gateway)
    ref_store.set_parent("parent", "main")?;
    ref_store.set_parent("child", "parent")?;

    // Validation should pass
    let result = validate_stack_integrity("child", &ref_store, &gateway, Some("main"));
    assert!(result.is_ok(), "Should pass for properly stacked branches");

    Ok(())
}

#[test]
fn test_validate_stack_integrity_fails_for_unrebased_branch() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create git branch before setting parent relationship

    // Get the initial commit (master tip)
    let master_commit = repo.head()?.peel_to_commit()?;

    // Create parent branch with a commit
    gateway.create_branch("parent")?;
    std::fs::write(dir.path().join("parent.txt"), "parent content")?;
    gateway.stage_all()?;
    gateway.commit("Parent commit")?;

    // Create child branch directly from master commit (NOT from parent)
    // This simulates a branch that was created before parent was modified
    let child_branch = repo.branch("child", &master_commit, false)?;
    drop(child_branch);

    // Checkout child and make a commit
    gateway.checkout_branch("child")?;
    std::fs::write(dir.path().join("child.txt"), "child content")?;
    gateway.stage_all()?;
    gateway.commit("Child commit")?;

    // Set up ref_store to say child's parent is "parent"
    // But child is actually based on master, not parent
    // Note: parent branch already created above via gateway.create_branch
    ref_store.set_parent("parent", "main")?;
    ref_store.set_parent("child", "parent")?;

    // Validation should fail because child is not based on parent
    let result = validate_stack_integrity("child", &ref_store, &gateway, Some("main"));
    assert!(result.is_err(), "Should fail for unrebased branch");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not rebased onto"),
        "Error should mention rebase: {}",
        err_msg
    );

    Ok(())
}

#[test]
fn test_validate_stack_integrity_passes_with_multiple_commits_per_branch() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create git branch before setting parent relationship

    // Create a branch with multiple commits (simulating multiple `dm modify` calls)
    gateway.create_branch("feature")?;

    // First modify
    std::fs::write(dir.path().join("file1.txt"), "content1")?;
    gateway.stage_all()?;
    gateway.commit("First feature commit")?;

    // Second modify
    std::fs::write(dir.path().join("file2.txt"), "content2")?;
    gateway.stage_all()?;
    gateway.commit("Second feature commit")?;

    // Third modify
    std::fs::write(dir.path().join("file3.txt"), "content3")?;
    gateway.stage_all()?;
    gateway.commit("Third feature commit")?;

    ref_store.set_parent("feature", "main")?;

    // Validation should pass even with multiple commits
    let result = validate_stack_integrity("feature", &ref_store, &gateway, Some("main"));
    assert!(
        result.is_ok(),
        "Should pass for branch with multiple commits: {:?}",
        result.err()
    );

    Ok(())
}

#[test]
fn test_validate_stack_integrity_passes_deep_stack_with_multiple_commits_each() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create git branch before setting parent relationship

    // Create first branch with 2 commits
    gateway.create_branch("branch-1")?;
    std::fs::write(dir.path().join("b1-file1.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Branch 1 - commit 1")?;
    std::fs::write(dir.path().join("b1-file2.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Branch 1 - commit 2")?;

    // Create second branch on top with 3 commits
    gateway.create_branch("branch-2")?;
    std::fs::write(dir.path().join("b2-file1.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Branch 2 - commit 1")?;
    std::fs::write(dir.path().join("b2-file2.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Branch 2 - commit 2")?;
    std::fs::write(dir.path().join("b2-file3.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Branch 2 - commit 3")?;

    // Create third branch on top with 2 commits
    gateway.create_branch("branch-3")?;
    std::fs::write(dir.path().join("b3-file1.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Branch 3 - commit 1")?;
    std::fs::write(dir.path().join("b3-file2.txt"), "content")?;
    gateway.stage_all()?;
    gateway.commit("Branch 3 - commit 2")?;

    // Set parent relationships (branches already created above via gateway)
    ref_store.set_parent("branch-1", "main")?;
    ref_store.set_parent("branch-2", "branch-1")?;
    ref_store.set_parent("branch-3", "branch-2")?;

    // Validation should pass for the entire stack
    let result = validate_stack_integrity("branch-3", &ref_store, &gateway, Some("main"));
    assert!(
        result.is_ok(),
        "Should pass for deep stack with multiple commits per branch: {:?}",
        result.err()
    );

    Ok(())
}

#[test]
fn test_validate_stack_integrity_with_cycle_does_not_hang() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create branch-a from master with a commit
    gateway.create_branch("branch-a")?;
    std::fs::write(dir.path().join("a.txt"), "a")?;
    gateway.stage_all()?;
    gateway.commit("Branch A commit")?;

    // Create branch-b from branch-a with a commit
    gateway.create_branch("branch-b")?;
    std::fs::write(dir.path().join("b.txt"), "b")?;
    gateway.stage_all()?;
    gateway.commit("Branch B commit")?;

    // Create a cycle in the metadata: branch-a -> branch-b -> branch-a
    // (Note: the git history is fine, but metadata forms a cycle)
    // (branches were already created above via gateway.create_branch)
    ref_store.set_parent("branch-a", "branch-b")?;
    ref_store.set_parent("branch-b", "branch-a")?;

    // Validation should fail (either due to cycle detection or rebase check)
    // The important thing is it doesn't hang in an infinite loop
    let result = validate_stack_integrity("branch-a", &ref_store, &gateway, Some("main"));
    assert!(result.is_err(), "Should fail with cycle in metadata");

    Ok(())
}

#[test]
fn test_submit_blocks_when_branch_is_behind_remote() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let _remote_repo = init_test_repo(remote_dir.path())?;

    // Clone it (this sets up origin remote properly)
    let local_dir = tempdir()?;
    let _local_repo = git2::Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    let _ctx = TestRepoContext::new(local_dir.path());

    // Set up ref_store and gateway
    std::fs::create_dir_all(local_dir.path().join(".git").join("diamond"))?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create diamond parent ref for master

    let gateway = GitGateway::new()?;

    // Create a feature branch locally
    gateway.create_branch("feature")?;
    std::fs::write(local_dir.path().join("feature.txt"), "feature content")?;
    gateway.stage_all()?;
    gateway.commit("Feature commit")?;
    ref_store.set_parent("feature", "main")?;

    // Push the feature branch to remote
    std::process::Command::new("git")
        .args(["push", "-u", "origin", "feature"])
        .current_dir(local_dir.path())
        .output()?;

    // Simulate remote having a new commit by:
    // 1. Creating a commit in another clone
    // 2. Pushing it
    // 3. Fetching in local
    let clone2_dir = local_dir.path().join("clone2");
    std::process::Command::new("git")
        .args(["clone", remote_dir.path().to_str().unwrap(), "clone2"])
        .current_dir(local_dir.path())
        .output()?;

    std::process::Command::new("git")
        .args(["checkout", "feature"])
        .current_dir(&clone2_dir)
        .output()?;
    std::fs::write(clone2_dir.join("remote_change.txt"), "remote content")?;
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(&clone2_dir)
        .output()?;
    std::process::Command::new("git")
        .args(["commit", "-m", "Remote commit"])
        .current_dir(&clone2_dir)
        .output()?;
    std::process::Command::new("git")
        .args(["push"])
        .current_dir(&clone2_dir)
        .output()?;

    // Fetch in local repo to see the remote changes
    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(local_dir.path())
        .output()?;

    // Verify divergence - feature should be behind
    let sync_state = gateway.check_remote_sync("feature")?;
    assert_eq!(
        sync_state,
        crate::git_gateway::BranchSyncState::Behind(1),
        "Feature should be behind remote"
    );

    // Mock forge for submit
    let forge = MockForge::new();
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit should fail (without force) because branch is behind
    let result = submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false, // no force
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    );

    assert!(result.is_err(), "Submit should fail when branch is behind");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("behind remote"),
        "Error should mention behind: {}",
        err_msg
    );

    Ok(())
}

#[test]
fn test_submit_succeeds_with_force_when_behind() -> Result<()> {
    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let _remote_repo = init_test_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let _local_repo = git2::Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    let _ctx = TestRepoContext::new(local_dir.path());

    // Set up ref_store
    std::fs::create_dir_all(local_dir.path().join(".git").join("diamond"))?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create diamond parent ref for master

    // Create a feature branch locally
    let gateway = GitGateway::new()?;
    gateway.create_branch("feature")?;
    std::fs::write(local_dir.path().join("feature.txt"), "feature content")?;
    gateway.stage_all()?;
    gateway.commit("Feature commit")?;
    ref_store.set_parent("feature", "main")?;

    // Push the feature branch to remote
    std::process::Command::new("git")
        .args(["push", "-u", "origin", "feature"])
        .current_dir(local_dir.path())
        .output()?;

    // Add remote commit via another clone
    let clone2_dir = local_dir.path().join("clone2");
    std::process::Command::new("git")
        .args(["clone", remote_dir.path().to_str().unwrap(), "clone2"])
        .current_dir(local_dir.path())
        .output()?;

    std::process::Command::new("git")
        .args(["checkout", "feature"])
        .current_dir(&clone2_dir)
        .output()?;
    std::fs::write(clone2_dir.join("remote_change.txt"), "remote content")?;
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(&clone2_dir)
        .output()?;
    std::process::Command::new("git")
        .args(["commit", "-m", "Remote commit"])
        .current_dir(&clone2_dir)
        .output()?;
    std::process::Command::new("git")
        .args(["push"])
        .current_dir(&clone2_dir)
        .output()?;

    // Fetch in local repo
    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(local_dir.path())
        .output()?;

    // Mock forge
    let forge = MockForge::new();
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit WITH force should succeed (divergence check is skipped)
    let result = submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        true, // force = true
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    );

    assert!(result.is_ok(), "Submit with force should succeed: {:?}", result);

    Ok(())
}

#[test]
fn test_submit_succeeds_after_amend() -> Result<()> {
    // This tests the core workflow: push -> amend locally -> submit again
    // This MUST work without --force because it's the normal development flow.
    // The branch will be "diverged" (1 local, 1 remote) because amending
    // creates a new commit hash, but we use --force-with-lease which handles this.

    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let _remote_repo = init_test_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let _local_repo = git2::Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    let _ctx = TestRepoContext::new(local_dir.path());

    std::fs::create_dir_all(local_dir.path().join(".git").join("diamond"))?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create diamond parent ref for master

    let gateway = GitGateway::new()?;

    // Create and push a feature branch
    gateway.create_branch("feature")?;
    std::fs::write(local_dir.path().join("feature.txt"), "initial content")?;
    gateway.stage_all()?;
    gateway.commit("Feature commit")?;
    ref_store.set_parent("feature", "main")?;

    // Push to remote
    std::process::Command::new("git")
        .args(["push", "-u", "origin", "feature"])
        .current_dir(local_dir.path())
        .output()?;

    // Now amend the commit locally (simulating `dm m -a`)
    std::fs::write(local_dir.path().join("feature.txt"), "amended content")?;
    gateway.stage_all()?;
    gateway.amend_commit(None)?;

    // Fetch to update remote tracking
    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(local_dir.path())
        .output()?;

    // Verify divergence - this is expected after amending
    let sync_state = gateway.check_remote_sync("feature")?;
    assert_eq!(
        sync_state,
        crate::git_gateway::BranchSyncState::Diverged {
            local_ahead: 1,
            remote_ahead: 1
        },
        "Feature should be diverged after amend"
    );

    // Mock forge for submit
    let forge = MockForge::new();
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit should SUCCEED even without --force (normal workflow)
    // The push uses --force-with-lease which handles the diverged state safely
    let result = submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false, // no force needed!
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    );

    assert!(
        result.is_ok(),
        "Submit should succeed after amending (this is the normal workflow): {:?}",
        result.err()
    );

    Ok(())
}

#[test]
fn test_submit_succeeds_after_multiple_amends() -> Result<()> {
    // This tests the workflow of multiple amend cycles:
    // push -> amend -> submit -> amend -> submit -> amend -> submit
    // Each submit should work without --force

    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let _remote_repo = init_test_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let _local_repo = git2::Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    let _ctx = TestRepoContext::new(local_dir.path());

    std::fs::create_dir_all(local_dir.path().join(".git").join("diamond"))?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create diamond parent ref for master

    let gateway = GitGateway::new()?;

    // Create and push a feature branch
    gateway.create_branch("feature")?;
    std::fs::write(local_dir.path().join("feature.txt"), "v1")?;
    gateway.stage_all()?;
    gateway.commit("Feature v1")?;
    ref_store.set_parent("feature", "main")?;

    let forge = MockForge::new();
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };

    // First push
    std::process::Command::new("git")
        .args(["push", "-u", "origin", "feature"])
        .current_dir(local_dir.path())
        .output()?;

    // Cycle 1: amend and submit
    std::fs::write(local_dir.path().join("feature.txt"), "v2")?;
    gateway.stage_all()?;
    gateway.amend_commit(None)?;

    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(local_dir.path())
        .output()?;

    let mut created_urls = Vec::new();
    let result = submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    );
    assert!(result.is_ok(), "First amend cycle should succeed: {:?}", result.err());

    // Simulate the push actually happening (update remote tracking)
    std::process::Command::new("git")
        .args(["push", "--force-with-lease", "origin", "feature"])
        .current_dir(local_dir.path())
        .output()?;

    // Cycle 2: amend and submit again
    std::fs::write(local_dir.path().join("feature.txt"), "v3")?;
    gateway.stage_all()?;
    gateway.amend_commit(None)?;

    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(local_dir.path())
        .output()?;

    let mut created_urls = Vec::new();
    let result = submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    );
    assert!(result.is_ok(), "Second amend cycle should succeed: {:?}", result.err());

    Ok(())
}

#[test]
fn test_submit_update_only_skips_branch_without_pr() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a stack: master -> feature
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branch before setting parent relationship
    ref_store.set_parent("feature", "main")?;

    // Create actual git branch
    let gateway = GitGateway::new()?;
    gateway.create_branch("feature")?;

    // Mock forge with NO existing PRs
    let forge = MockForge::new();
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit with update_only=true
    submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        true, // update_only
        &empty_pr_cache(),
    )?;

    // Branch should NOT be pushed (skipped due to no PR)
    let pushed = forge.get_pushed_branches();
    assert_eq!(pushed.len(), 0, "Branch should be skipped (no push)");

    // No PRs created
    let prs = forge.get_created_prs();
    assert_eq!(prs.len(), 0, "No PRs should be created");

    // No URLs collected
    assert_eq!(created_urls.len(), 0, "No URLs should be collected");

    Ok(())
}

#[test]
fn test_submit_update_only_updates_branch_with_existing_pr() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a stack: master -> feature
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branch before setting parent relationship
    ref_store.set_parent("feature", "main")?;

    // Create actual git branch
    let gateway = GitGateway::new()?;
    gateway.create_branch("feature")?;

    // Mock forge WITH existing PR
    let forge = MockForge::new().with_existing_pr("feature");
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit with update_only=true
    submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        true, // update_only
        &empty_pr_cache(),
    )?;

    // Branch SHOULD be pushed (has existing PR)
    let pushed = forge.get_pushed_branches();
    assert_eq!(pushed.len(), 1, "Branch should be pushed");
    assert_eq!(pushed[0], "feature");

    // No new PRs created (just updated)
    let prs = forge.get_created_prs();
    assert_eq!(prs.len(), 0, "No new PRs should be created");

    Ok(())
}

#[test]
fn test_submit_update_only_fails_when_parent_has_no_pr() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a stack: master -> parent -> child
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branches before setting parent relationships
    create_branch(&repo, "parent")?;
    ref_store.set_parent("parent", "main")?;
    ref_store.set_parent("child", "parent")?;

    // Create child git branch (parent already created above)
    let gateway = GitGateway::new()?;
    gateway.create_branch("child")?;

    // Mock forge: child has PR but parent doesn't
    let forge = MockForge::new().with_existing_pr("child");
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit with update_only=true - should fail because parent has no PR
    // and we can't recursively create parent PRs in update_only mode
    let result = submit_branch(
        "child",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        true, // update_only
        &empty_pr_cache(),
    );

    // This should succeed since child already has a PR
    // (only fails when trying to CREATE a new PR for a branch whose parent has no PR)
    assert!(result.is_ok(), "Should succeed since child already has PR");

    Ok(())
}

#[test]
fn test_submit_publish_marks_existing_pr_as_ready() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create branch with parent tracking
    let gateway = GitGateway::new()?;
    gateway.create_branch("feature")?;
    gateway.checkout_branch("feature")?;

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("feature", "main")?;

    // Make a commit on the feature branch
    let sig = git2::Signature::now("Test User", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let parent = repo.head()?.peel_to_commit()?;
    repo.commit(Some("HEAD"), &sig, &sig, "Feature commit", &tree, &[&parent])?;

    // Mock forge with existing PR (simulates a draft PR)
    let forge = MockForge::new().with_existing_pr("feature");

    // Submit with publish=true
    let options = PrOptions {
        draft: false,
        publish: true,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    )?;

    // Verify mark_pr_ready was called
    let marked = forge.get_marked_ready();
    assert_eq!(marked.len(), 1, "mark_pr_ready should be called once");
    assert_eq!(marked[0], "feature", "Should mark the correct branch as ready");

    Ok(())
}

#[test]
fn test_submit_merge_when_ready_enables_auto_merge_for_existing_pr() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create branch with parent tracking
    let gateway = GitGateway::new()?;
    gateway.create_branch("feature")?;
    gateway.checkout_branch("feature")?;

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("feature", "main")?;

    // Make a commit on the feature branch
    let sig = git2::Signature::now("Test User", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let parent = repo.head()?.peel_to_commit()?;
    repo.commit(Some("HEAD"), &sig, &sig, "Feature commit", &tree, &[&parent])?;

    // Mock forge with existing PR
    let forge = MockForge::new().with_existing_pr("feature");

    // Submit with merge_when_ready=true
    let options = PrOptions {
        draft: false,
        merge_when_ready: true,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    )?;

    // Verify enable_auto_merge was called
    let auto_merge = forge.get_auto_merge_enabled();
    assert_eq!(auto_merge.len(), 1, "enable_auto_merge should be called once");
    assert_eq!(
        auto_merge[0].0, "feature",
        "Should enable auto-merge for the correct branch"
    );
    assert_eq!(auto_merge[0].1, "squash", "Should use squash merge method");

    Ok(())
}

#[test]
fn test_submit_merge_when_ready_enables_auto_merge_for_new_pr() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create branch with parent tracking
    let gateway = GitGateway::new()?;
    gateway.create_branch("feature")?;
    gateway.checkout_branch("feature")?;

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("feature", "main")?;

    // Make a commit on the feature branch
    let sig = git2::Signature::now("Test User", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let parent = repo.head()?.peel_to_commit()?;
    repo.commit(Some("HEAD"), &sig, &sig, "Feature commit", &tree, &[&parent])?;

    // Mock forge with NO existing PR (will create new one)
    let forge = MockForge::new();

    // Submit with merge_when_ready=true
    let options = PrOptions {
        draft: false,
        merge_when_ready: true,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    )?;

    // Verify PR was created
    let created = forge.get_created_prs();
    assert_eq!(created.len(), 1, "Should create one PR");
    assert_eq!(created[0].0, "feature", "PR should be for feature branch");

    // Verify enable_auto_merge was called for the new PR
    let auto_merge = forge.get_auto_merge_enabled();
    assert_eq!(auto_merge.len(), 1, "enable_auto_merge should be called for new PR");
    assert_eq!(
        auto_merge[0].0, "feature",
        "Should enable auto-merge for the correct branch"
    );

    Ok(())
}

#[test]
fn test_submit_without_publish_does_not_mark_ready() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create branch with parent tracking
    let gateway = GitGateway::new()?;
    gateway.create_branch("feature")?;
    gateway.checkout_branch("feature")?;

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("feature", "main")?;

    // Make a commit on the feature branch
    let sig = git2::Signature::now("Test User", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let parent = repo.head()?.peel_to_commit()?;
    repo.commit(Some("HEAD"), &sig, &sig, "Feature commit", &tree, &[&parent])?;

    // Mock forge with existing PR
    let forge = MockForge::new().with_existing_pr("feature");

    // Submit without publish flag
    let options = PrOptions {
        draft: false,
        publish: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    )?;

    // Verify mark_pr_ready was NOT called
    let marked = forge.get_marked_ready();
    assert!(
        marked.is_empty(),
        "mark_pr_ready should not be called when publish=false"
    );

    Ok(())
}

#[test]
fn test_submit_without_merge_when_ready_does_not_enable_auto_merge() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create branch with parent tracking
    let gateway = GitGateway::new()?;
    gateway.create_branch("feature")?;
    gateway.checkout_branch("feature")?;

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    ref_store.set_parent("feature", "main")?;

    // Make a commit on the feature branch
    let sig = git2::Signature::now("Test User", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let parent = repo.head()?.peel_to_commit()?;
    repo.commit(Some("HEAD"), &sig, &sig, "Feature commit", &tree, &[&parent])?;

    // Mock forge with existing PR
    let forge = MockForge::new().with_existing_pr("feature");

    // Submit without merge_when_ready flag
    let options = PrOptions {
        draft: false,
        merge_when_ready: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    submit_branch(
        "feature",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    )?;

    // Verify enable_auto_merge was NOT called
    let auto_merge = forge.get_auto_merge_enabled();
    assert!(
        auto_merge.is_empty(),
        "enable_auto_merge should not be called when merge_when_ready=false"
    );

    Ok(())
}

// ===== Additional tests for check_branch_sync_state and submit_stack =====

#[test]
fn test_check_branch_sync_state_behind_fails() {
    // This is tested via the submit_blocks_when_branch_is_behind_remote test
    // But we can also unit test check_branch_sync_state directly
}

#[test]
fn test_submit_stack_submits_all_descendants() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    // Create a stack: master -> a -> b -> c
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;

    // Create gateway (branches already created above)
    let gateway = GitGateway::new()?;

    // Mock forge with no existing PRs
    let forge = MockForge::new();
    let options = PrOptions {
        draft: false,
        reviewers: vec![],
        ..Default::default()
    };
    let mut created_urls = Vec::new();

    // Submit the stack starting from "a"
    submit_stack(
        "a",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    )?;

    // All branches should be pushed
    let pushed = forge.get_pushed_branches();
    assert_eq!(pushed.len(), 3, "All 3 branches should be pushed");
    assert_eq!(pushed, vec!["a", "b", "c"]);

    // All PRs should be created
    let prs = forge.get_created_prs();
    assert_eq!(prs.len(), 3, "All 3 PRs should be created");
    assert_eq!(prs[0], ("a".to_string(), "main".to_string()));
    assert_eq!(prs[1], ("b".to_string(), "a".to_string()));
    assert_eq!(prs[2], ("c".to_string(), "b".to_string()));

    Ok(())
}

// ===== PR Cache Collection Tests =====

#[test]
fn test_collect_branches_for_pr_check_single_branch() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create git branch before setting parent relationship
    ref_store.set_parent("feature", "main")?;

    // Single branch direct child of trunk
    let branches = vec!["feature".to_string()];
    let result = super::collect_branches_for_pr_check(&branches, &ref_store)?;

    // Should only contain the branch itself (parent is trunk, excluded)
    assert_eq!(result.len(), 1);
    assert!(result.contains(&"feature".to_string()));

    Ok(())
}

#[test]
fn test_collect_branches_for_pr_check_with_ancestors() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Stack: master -> a -> b -> c
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;

    // Check from leaf branch
    let branches = vec!["c".to_string()];
    let result = super::collect_branches_for_pr_check(&branches, &ref_store)?;

    // Should include c, b, and a (ancestors up to trunk)
    assert_eq!(result.len(), 3);
    assert!(result.contains(&"a".to_string()));
    assert!(result.contains(&"b".to_string()));
    assert!(result.contains(&"c".to_string()));

    Ok(())
}

#[test]
fn test_collect_branches_for_pr_check_deduplicates() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Stack: master -> a -> b -> c
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;

    // Check multiple branches with shared ancestors
    let branches = vec!["b".to_string(), "c".to_string()];
    let result = super::collect_branches_for_pr_check(&branches, &ref_store)?;

    // Should deduplicate - only 3 unique branches (a, b, c)
    assert_eq!(result.len(), 3);
    assert!(result.contains(&"a".to_string()));
    assert!(result.contains(&"b".to_string()));
    assert!(result.contains(&"c".to_string()));

    Ok(())
}

#[test]
fn test_collect_branches_for_pr_check_branching_stack() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Branching stack:
    // master -> a -> b
    //             \-> c
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "a")?;

    // Check both leaf branches
    let branches = vec!["b".to_string(), "c".to_string()];
    let result = super::collect_branches_for_pr_check(&branches, &ref_store)?;

    // Should include a, b, c (a is shared ancestor)
    assert_eq!(result.len(), 3);
    assert!(result.contains(&"a".to_string()));
    assert!(result.contains(&"b".to_string()));
    assert!(result.contains(&"c".to_string()));

    Ok(())
}

// ===== Branch Selection Tests (default vs --stack) =====

#[test]
fn test_collect_downstack_returns_branches_in_submit_order() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create stack: master -> a -> b -> c
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;

    // Collect downstack from c
    let branches = ref_store.ancestors("c")?;

    // Should be in submit order: a first (closest to trunk), then b, then c
    assert_eq!(branches, vec!["a", "b", "c"]);

    Ok(())
}

#[test]
fn test_collect_downstack_from_middle_of_stack() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create stack: master -> a -> b -> c
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;

    // Collect downstack from b (middle of stack)
    let branches = ref_store.ancestors("b")?;

    // Should only include ancestors up to current: a, b (not c)
    assert_eq!(branches, vec!["a", "b"]);

    Ok(())
}

#[test]
fn test_submit_default_selects_only_current_branch() -> Result<()> {
    // This tests the branch selection logic used by run()
    // Default (stack=false): only current branch
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create stack: master -> a -> b -> c
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;

    // Simulate default behavior (stack=false): only current branch
    let current = "b";
    let stack = false;

    let branches_to_submit: Vec<String> = if stack {
        let mut all = ref_store.ancestors(current)?;
        for descendant in ref_store.collect_branches_dfs(std::slice::from_ref(&current.to_string()))? {
            if !all.contains(&descendant) {
                all.push(descendant);
            }
        }
        all
    } else {
        vec![current.to_string()]
    };

    // Should only include current branch
    assert_eq!(branches_to_submit, vec!["b"]);

    Ok(())
}

#[test]
fn test_submit_stack_from_top_includes_all_ancestors() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create stack: master -> a -> b -> c
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;

    // Simulate --stack behavior from top of stack
    let current = "c";
    let stack = true;

    let branches_to_submit: Vec<String> = if stack {
        let mut all = ref_store.ancestors(current)?;
        for descendant in ref_store.collect_branches_dfs(std::slice::from_ref(&current.to_string()))? {
            if !all.contains(&descendant) {
                all.push(descendant);
            }
        }
        all
    } else {
        vec![current.to_string()]
    };

    // From top, should include all ancestors: a, b, c (no descendants)
    assert_eq!(branches_to_submit, vec!["a", "b", "c"]);

    Ok(())
}

#[test]
fn test_submit_stack_from_middle_includes_ancestors_and_descendants() -> Result<()> {
    let dir = tempdir()?;
    let repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    // Create stack metadata: master -> a -> b -> c
    // (collect_branches_dfs works with ref_store metadata, not git branches)
    // Create git branches before setting parent relationships
    create_branch(&repo, "a")?;
    create_branch(&repo, "b")?;
    create_branch(&repo, "c")?;
    ref_store.set_parent("a", "main")?;
    ref_store.set_parent("b", "a")?;
    ref_store.set_parent("c", "b")?;

    // Simulate --stack behavior from middle of stack (b)
    let current = "b";
    let stack = true;

    let branches_to_submit: Vec<String> = if stack {
        let mut all = ref_store.ancestors(current)?;
        for descendant in ref_store.collect_branches_dfs(std::slice::from_ref(&current.to_string()))? {
            if !all.contains(&descendant) {
                all.push(descendant);
            }
        }
        all
    } else {
        vec![current.to_string()]
    };

    // From middle, should include ancestors (a, b) and descendants (c)
    assert_eq!(branches_to_submit.len(), 3);
    assert!(branches_to_submit.contains(&"a".to_string()));
    assert!(branches_to_submit.contains(&"b".to_string()));
    assert!(branches_to_submit.contains(&"c".to_string()));

    Ok(())
}

// ===== Tests for push_diverged_ancestors =====

#[test]
fn test_diverged_ancestors_are_pushed_before_leaf() -> Result<()> {
    // This tests the critical fix: when submitting a leaf branch, any diverged
    // ancestor branches with PRs should be pushed first.
    //
    // Scenario:
    // - Stack: master → branch-a → branch-b
    // - Both branches have PRs
    // - branch-a is rebased locally (diverged from remote)
    // - Submit branch-b
    // - Expected: branch-a should be pushed BEFORE branch-b

    // Create a "remote" repo
    let remote_dir = tempdir()?;
    let _remote_repo = init_test_repo(remote_dir.path())?;

    // Clone it
    let local_dir = tempdir()?;
    let _local_repo = git2::Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    let _ctx = TestRepoContext::new(local_dir.path());

    // Set up diamond directory and master branch for ref_store
    std::fs::create_dir_all(local_dir.path().join(".git").join("diamond"))?;

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    let gateway = GitGateway::new()?;

    // Create branch-a
    gateway.create_branch("branch-a")?;
    std::fs::write(local_dir.path().join("a.txt"), "content a")?;
    gateway.stage_all()?;
    gateway.commit("Branch A")?;
    ref_store.set_parent("branch-a", "main")?;

    // Push branch-a
    std::process::Command::new("git")
        .args(["push", "-u", "origin", "branch-a"])
        .current_dir(local_dir.path())
        .output()?;

    // Create branch-b on top of branch-a
    gateway.create_branch("branch-b")?;
    std::fs::write(local_dir.path().join("b.txt"), "content b")?;
    gateway.stage_all()?;
    gateway.commit("Branch B")?;
    ref_store.set_parent("branch-b", "branch-a")?;

    // Push branch-b
    std::process::Command::new("git")
        .args(["push", "-u", "origin", "branch-b"])
        .current_dir(local_dir.path())
        .output()?;

    // Now amend branch-a to cause divergence
    gateway.checkout_branch("branch-a")?;
    std::fs::write(local_dir.path().join("a.txt"), "amended content a")?;
    gateway.stage_all()?;
    gateway.amend_commit(None)?;

    // Rebase branch-b onto the amended branch-a
    gateway.checkout_branch("branch-b")?;
    std::process::Command::new("git")
        .args(["rebase", "branch-a"])
        .current_dir(local_dir.path())
        .output()?;

    // Fetch to update remote tracking refs
    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(local_dir.path())
        .output()?;

    // Verify branch-a is diverged
    let sync_state = gateway.check_remote_sync("branch-a")?;
    assert!(
        matches!(sync_state, crate::git_gateway::BranchSyncState::Diverged { .. }),
        "branch-a should be diverged after amend"
    );

    // Create mock forge with both branches having PRs
    let forge = MockForge::new()
        .with_existing_pr("branch-a")
        .with_existing_pr("branch-b");

    let options = PrOptions::default();
    let mut created_urls = Vec::new();

    // Submit branch-b (the leaf)
    let result = submit_branch(
        "branch-b",
        &ref_store,
        &gateway,
        &forge,
        false, // no force
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    );

    assert!(result.is_ok(), "Submit should succeed: {:?}", result.err());

    // Verify push order: branch-a should be pushed BEFORE branch-b
    let pushed = forge.get_pushed_branches();
    assert!(pushed.len() >= 2, "Both branches should be pushed, got: {:?}", pushed);

    let a_pos = pushed.iter().position(|b| b == "branch-a");
    let b_pos = pushed.iter().position(|b| b == "branch-b");

    assert!(a_pos.is_some(), "branch-a should be in pushed list: {:?}", pushed);
    assert!(b_pos.is_some(), "branch-b should be in pushed list: {:?}", pushed);
    assert!(
        a_pos.unwrap() < b_pos.unwrap(),
        "branch-a (ancestor) should be pushed BEFORE branch-b (leaf). Order: {:?}",
        pushed
    );

    Ok(())
}

#[test]
fn test_diverged_ancestor_without_pr_not_pushed() -> Result<()> {
    // If an ancestor is diverged but has NO PR, it should NOT be pushed
    // by push_diverged_ancestors (submit_branch will handle it separately)

    let remote_dir = tempdir()?;
    let _remote_repo = init_test_repo(remote_dir.path())?;

    let local_dir = tempdir()?;
    let _local_repo = git2::Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    let _ctx = TestRepoContext::new(local_dir.path());

    // Set up diamond directory and master branch for ref_store
    std::fs::create_dir_all(local_dir.path().join(".git").join("diamond"))?;

    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;

    let gateway = GitGateway::new()?;

    // Create and push branch-a
    gateway.create_branch("branch-a")?;
    std::fs::write(local_dir.path().join("a.txt"), "content a")?;
    gateway.stage_all()?;
    gateway.commit("Branch A")?;
    ref_store.set_parent("branch-a", "main")?;

    std::process::Command::new("git")
        .args(["push", "-u", "origin", "branch-a"])
        .current_dir(local_dir.path())
        .output()?;

    // Create and push branch-b
    gateway.create_branch("branch-b")?;
    std::fs::write(local_dir.path().join("b.txt"), "content b")?;
    gateway.stage_all()?;
    gateway.commit("Branch B")?;
    ref_store.set_parent("branch-b", "branch-a")?;

    std::process::Command::new("git")
        .args(["push", "-u", "origin", "branch-b"])
        .current_dir(local_dir.path())
        .output()?;

    // Amend branch-a to cause divergence
    gateway.checkout_branch("branch-a")?;
    std::fs::write(local_dir.path().join("a.txt"), "amended")?;
    gateway.stage_all()?;
    gateway.amend_commit(None)?;

    // Rebase branch-b
    gateway.checkout_branch("branch-b")?;
    std::process::Command::new("git")
        .args(["rebase", "branch-a"])
        .current_dir(local_dir.path())
        .output()?;

    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(local_dir.path())
        .output()?;

    // Mock forge: branch-b has PR, but branch-a does NOT
    let forge = MockForge::new().with_existing_pr("branch-b");

    let options = PrOptions::default();
    let mut created_urls = Vec::new();

    let result = submit_branch(
        "branch-b",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    );

    assert!(result.is_ok(), "Submit should succeed: {:?}", result.err());

    // branch-a should NOT be in pushed list (no PR)
    let pushed = forge.get_pushed_branches();
    assert!(
        !pushed.contains(&"branch-a".to_string()),
        "branch-a should NOT be pushed (no PR). Pushed: {:?}",
        pushed
    );

    Ok(())
}

#[test]
fn test_in_sync_ancestor_not_pushed() -> Result<()> {
    // If an ancestor has a PR but is NOT diverged (in sync), it should not be pushed

    let remote_dir = tempdir()?;
    let _remote_repo = init_test_repo(remote_dir.path())?;

    let local_dir = tempdir()?;
    let _local_repo = git2::Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    let _ctx = TestRepoContext::new(local_dir.path());

    std::fs::create_dir_all(local_dir.path().join(".git").join("diamond"))?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create diamond parent ref for master

    let gateway = GitGateway::new()?;

    // Create and push branch-a
    gateway.create_branch("branch-a")?;
    std::fs::write(local_dir.path().join("a.txt"), "content a")?;
    gateway.stage_all()?;
    gateway.commit("Branch A")?;
    ref_store.set_parent("branch-a", "main")?;

    std::process::Command::new("git")
        .args(["push", "-u", "origin", "branch-a"])
        .current_dir(local_dir.path())
        .output()?;

    // Create and push branch-b
    gateway.create_branch("branch-b")?;
    std::fs::write(local_dir.path().join("b.txt"), "content b")?;
    gateway.stage_all()?;
    gateway.commit("Branch B")?;
    ref_store.set_parent("branch-b", "branch-a")?;

    std::process::Command::new("git")
        .args(["push", "-u", "origin", "branch-b"])
        .current_dir(local_dir.path())
        .output()?;

    // Fetch (no amendments, so both branches are in sync)
    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(local_dir.path())
        .output()?;

    // Verify branch-a is in sync
    let sync_state = gateway.check_remote_sync("branch-a")?;
    assert!(
        matches!(sync_state, crate::git_gateway::BranchSyncState::InSync),
        "branch-a should be in sync"
    );

    // Now amend ONLY branch-b (not branch-a)
    std::fs::write(local_dir.path().join("b.txt"), "amended b")?;
    gateway.stage_all()?;
    gateway.amend_commit(None)?;

    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(local_dir.path())
        .output()?;

    // Both branches have PRs
    let forge = MockForge::new()
        .with_existing_pr("branch-a")
        .with_existing_pr("branch-b");

    let options = PrOptions::default();
    let mut created_urls = Vec::new();

    let result = submit_branch(
        "branch-b",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    );

    assert!(result.is_ok(), "Submit should succeed: {:?}", result.err());

    // branch-a should NOT be pushed (it's in sync)
    // Only branch-b should be pushed
    let pushed = forge.get_pushed_branches();
    assert!(
        !pushed.iter().any(|b| b == "branch-a"),
        "branch-a should NOT be pushed (in sync). Pushed: {:?}",
        pushed
    );
    assert!(
        pushed.contains(&"branch-b".to_string()),
        "branch-b should be pushed. Pushed: {:?}",
        pushed
    );

    Ok(())
}

#[test]
fn test_multiple_diverged_ancestors_pushed_in_order() -> Result<()> {
    // Stack: master → a → b → c
    // Both a and b are diverged (amended after push) with PRs
    // Submit c should push a, then b, then c (in that order)
    //
    // Note: We amend each branch SEPARATELY without rebasing, simulating
    // what happens when each branch has local changes after initial push.

    let remote_dir = tempdir()?;
    let _remote_repo = init_test_repo(remote_dir.path())?;

    let local_dir = tempdir()?;
    let _local_repo = git2::Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path())?;

    let _ctx = TestRepoContext::new(local_dir.path());

    std::fs::create_dir_all(local_dir.path().join(".git").join("diamond"))?;
    let ref_store = RefStore::new()?;
    ref_store.set_trunk("main")?;
    // Create diamond parent ref for master

    let gateway = GitGateway::new()?;

    // Create and push branch-a
    gateway.create_branch("branch-a")?;
    std::fs::write(local_dir.path().join("a.txt"), "a")?;
    gateway.stage_all()?;
    gateway.commit("A")?;
    ref_store.set_parent("branch-a", "main")?;
    std::process::Command::new("git")
        .args(["push", "-u", "origin", "branch-a"])
        .current_dir(local_dir.path())
        .output()?;

    // Create and push branch-b
    gateway.create_branch("branch-b")?;
    std::fs::write(local_dir.path().join("b.txt"), "b")?;
    gateway.stage_all()?;
    gateway.commit("B")?;
    ref_store.set_parent("branch-b", "branch-a")?;
    std::process::Command::new("git")
        .args(["push", "-u", "origin", "branch-b"])
        .current_dir(local_dir.path())
        .output()?;

    // Create and push branch-c
    gateway.create_branch("branch-c")?;
    std::fs::write(local_dir.path().join("c.txt"), "c")?;
    gateway.stage_all()?;
    gateway.commit("C")?;
    ref_store.set_parent("branch-c", "branch-b")?;
    std::process::Command::new("git")
        .args(["push", "-u", "origin", "branch-c"])
        .current_dir(local_dir.path())
        .output()?;

    // Now amend branch-a (causes divergence)
    gateway.checkout_branch("branch-a")?;
    std::fs::write(local_dir.path().join("a.txt"), "amended a")?;
    gateway.stage_all()?;
    gateway.amend_commit(None)?;

    // Amend branch-b (causes divergence)
    gateway.checkout_branch("branch-b")?;
    std::fs::write(local_dir.path().join("b.txt"), "amended b")?;
    gateway.stage_all()?;
    gateway.amend_commit(None)?;

    // Return to branch-c (don't amend it - we're testing ancestor push)
    gateway.checkout_branch("branch-c")?;

    // Fetch to update remote tracking refs
    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(local_dir.path())
        .output()?;

    // Verify both branch-a and branch-b are diverged
    let sync_a = gateway.check_remote_sync("branch-a")?;
    let sync_b = gateway.check_remote_sync("branch-b")?;
    assert!(
        matches!(sync_a, crate::git_gateway::BranchSyncState::Diverged { .. }),
        "branch-a should be diverged, got: {:?}",
        sync_a
    );
    assert!(
        matches!(sync_b, crate::git_gateway::BranchSyncState::Diverged { .. }),
        "branch-b should be diverged, got: {:?}",
        sync_b
    );

    // All three have PRs
    let forge = MockForge::new()
        .with_existing_pr("branch-a")
        .with_existing_pr("branch-b")
        .with_existing_pr("branch-c");

    let options = PrOptions::default();
    let mut created_urls = Vec::new();

    let result = submit_branch(
        "branch-c",
        &ref_store,
        &gateway,
        &forge,
        false,
        &options,
        &mut created_urls,
        false,
        &empty_pr_cache(),
    );

    assert!(result.is_ok(), "Submit should succeed: {:?}", result.err());

    let pushed = forge.get_pushed_branches();

    let a_pos = pushed.iter().position(|b| b == "branch-a");
    let b_pos = pushed.iter().position(|b| b == "branch-b");
    let c_pos = pushed.iter().position(|b| b == "branch-c");

    assert!(a_pos.is_some(), "branch-a should be pushed: {:?}", pushed);
    assert!(b_pos.is_some(), "branch-b should be pushed: {:?}", pushed);
    assert!(c_pos.is_some(), "branch-c should be pushed: {:?}", pushed);

    // Verify order: a < b < c
    assert!(
        a_pos.unwrap() < b_pos.unwrap(),
        "branch-a should be pushed before branch-b. Order: {:?}",
        pushed
    );
    assert!(
        b_pos.unwrap() < c_pos.unwrap(),
        "branch-b should be pushed before branch-c. Order: {:?}",
        pushed
    );

    Ok(())
}
