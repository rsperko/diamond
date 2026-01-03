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
