//! Thread-local test context for parallel test execution.
//!
//! This module provides a way to run tests in parallel without using `std::env::set_current_dir()`,
//! which is a process-wide operation that requires serial execution (the `#[serial]` attribute).
//!
//! Instead, tests use `TestRepoContext` to set a thread-local repository path that is used by
//! `GitGateway::new()`, `RefStore::new()`, and other components when running in test mode.
//!
//! # Example
//!
//! ```ignore
//! #[test]  // No #[serial] needed!
//! fn test_create_branch() -> Result<()> {
//!     let dir = tempdir()?;
//!     let _repo = init_test_repo(dir.path())?;
//!     let _ctx = TestRepoContext::new(dir.path());  // Sets thread-local path
//!
//!     // GitGateway::new() will use dir.path() instead of CWD
//!     let gateway = GitGateway::new()?;
//!     gateway.create_branch("feature")?;
//!
//!     Ok(())
//! }
//! ```

use std::cell::RefCell;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

#[cfg(test)]
use anyhow::Result;

thread_local! {
    /// Thread-local storage for the test repository path.
    /// This is only accessed during tests and allows parallel test execution.
    static TEST_REPO_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// RAII guard for setting the test repository path in the current thread.
///
/// When created, this sets the thread-local test repository path.
/// When dropped, it clears the path, ensuring cleanup even on panic.
///
/// The `PhantomData<*const ()>` makes this type `!Send`, preventing it from
/// being moved across thread boundaries and causing confusion.
pub struct TestRepoContext {
    _phantom: PhantomData<*const ()>,
}

impl TestRepoContext {
    /// Create a new test context for the given repository path.
    ///
    /// This sets the thread-local repository path that will be used by
    /// `GitGateway::new()`, `RefStore::new()`, and other components.
    pub fn new(path: &Path) -> Self {
        TEST_REPO_PATH.with(|p| *p.borrow_mut() = Some(path.to_path_buf()));
        Self { _phantom: PhantomData }
    }
}

impl Drop for TestRepoContext {
    fn drop(&mut self) {
        TEST_REPO_PATH.with(|p| *p.borrow_mut() = None);
    }
}

/// Get the current thread-local test repository path, if set.
///
/// This is called by components like `GitGateway::new()` and `RefStore::new()`
/// when running in test mode to use the test repository instead of the CWD.
pub(crate) fn test_repo_path() -> Option<PathBuf> {
    TEST_REPO_PATH.with(|p| p.borrow().clone())
}

/// Initialize a test repository with consistent "main" branch naming.
///
/// This is a shared helper for all unit tests to avoid code duplication.
/// Creates a git repository with an initial commit on the "main" branch,
/// ensuring consistency across all platforms (CI defaults to "master" without this).
///
/// Also creates the `.git/diamond/` directory for Diamond metadata.
#[cfg(test)]
pub fn init_test_repo(path: &Path) -> Result<git2::Repository> {
    use std::fs;

    // Initialize with explicit branch name for consistency across environments
    let repo = git2::Repository::init(path)?;

    // Configure git user for the test repo (needed for commits in CI)
    // This ensures any subsequent commits in tests will succeed
    let mut config = repo.config()?;
    config.set_str("user.name", "Test User")?;
    config.set_str("user.email", "test@example.com")?;
    drop(config); // Release borrow before commit

    // Create initial commit on main branch
    let sig = git2::Signature::now("Test User", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
    drop(tree);

    // Rename the branch to main for consistency (handle both master and main defaults)
    {
        let mut branch = repo
            .find_branch("master", git2::BranchType::Local)
            .or_else(|_| repo.find_branch("main", git2::BranchType::Local))?;
        if branch.name()?.unwrap_or("") == "master" {
            branch.rename("main", false)?;
        }
    } // Drop branch here to release borrow

    // Create Diamond metadata directory
    fs::create_dir_all(path.join(".git").join("diamond"))?;

    Ok(repo)
}

/// Initialize a test repository with a custom branch name.
///
/// This is useful for tests that need to verify behavior with non-standard branch names
/// (e.g., testing `dm init` on repos with branches other than "main").
#[cfg(test)]
pub fn init_test_repo_with_branch(path: &Path, branch_name: &str) -> Result<git2::Repository> {
    use std::fs;

    let repo = git2::Repository::init(path)?;

    // Configure git user (needed for commits in CI)
    let mut config = repo.config()?;
    config.set_str("user.name", "Test User")?;
    config.set_str("user.email", "test@example.com")?;
    config.set_str("init.defaultBranch", branch_name)?;
    drop(config);

    let sig = git2::Signature::now("Test User", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let refname = format!("refs/heads/{}", branch_name);
    repo.commit(Some(&refname), &sig, &sig, "Initial commit", &tree, &[])?;
    drop(tree);

    repo.set_head(&refname)?;

    // Create Diamond metadata directory
    fs::create_dir_all(path.join(".git").join("diamond"))?;

    Ok(repo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_context_sets_and_clears_path() {
        let dir = tempdir().unwrap();

        // Path should be None initially
        assert!(test_repo_path().is_none());

        {
            let _ctx = TestRepoContext::new(dir.path());
            // Path should be set within the context
            assert_eq!(test_repo_path(), Some(dir.path().to_path_buf()));
        }

        // Path should be cleared after context is dropped
        assert!(test_repo_path().is_none());
    }

    #[test]
    fn test_context_clears_on_panic() {
        let dir = tempdir().unwrap();

        let result = std::panic::catch_unwind(|| {
            let _ctx = TestRepoContext::new(dir.path());
            assert!(test_repo_path().is_some());
            panic!("intentional panic");
        });

        assert!(result.is_err());
        // Path should be cleared even after panic
        assert!(test_repo_path().is_none());
    }

    #[test]
    fn test_nested_contexts_use_latest() {
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();

        let _ctx1 = TestRepoContext::new(dir1.path());
        assert_eq!(test_repo_path(), Some(dir1.path().to_path_buf()));

        {
            let _ctx2 = TestRepoContext::new(dir2.path());
            // Inner context overwrites outer context
            assert_eq!(test_repo_path(), Some(dir2.path().to_path_buf()));
        }

        // After inner context drops, path is None (not restored to ctx1)
        // This is expected behavior - nesting is not recommended
        assert!(test_repo_path().is_none());
    }
}
