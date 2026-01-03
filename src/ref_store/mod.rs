//! RefStore provides git-native parent tracking using blob-based refs.
//!
//! Parent relationships are stored as:
//!   refs/diamond/parent/<child-branch> -> blob containing "<parent-branch>"
//!
//! This approach survives push/fetch operations, unlike
//! symbolic refs which get resolved to commit SHAs during transfer.
//!
//! Children are DERIVED by scanning all parent refs (not stored).
//!
//! Trunk configuration is stored as:
//!   refs/diamond/config/trunk -> blob containing "<trunk-branch>"

mod frozen;
mod lock;
mod parent;
mod traversal;
mod trunk;

#[cfg(test)]
mod tests;

pub use lock::RefStoreLockGuard;
pub use parent::validate_parent_name;

use crate::git_gateway::GitGateway;
use anyhow::{Context, Result};
use std::path::Path;

/// Prefix for parent relationship refs
pub(crate) const PARENT_REF_PREFIX: &str = "refs/diamond/parent/";
/// Ref for trunk configuration
pub(crate) const TRUNK_REF: &str = "refs/diamond/config/trunk";
/// Prefix for frozen branch refs
pub(crate) const FROZEN_REF_PREFIX: &str = "refs/diamond/frozen/";

/// RefStore manages stack metadata using git refs pointing to blobs.
///
/// Refs travel with fetch/push operations, enabling collaboration.
/// Uses GitGateway which handles both "files" and "reftable" ref formats.
pub struct RefStore {
    pub(crate) gateway: GitGateway,
}

#[allow(dead_code)]
impl RefStore {
    /// Create a new RefStore from the current directory
    ///
    /// In test mode, uses the thread-local test repository path if set via `TestRepoContext`.
    pub fn new() -> Result<Self> {
        #[cfg(test)]
        {
            if let Some(path) = crate::test_context::test_repo_path() {
                return Self::from_path(&path);
            }

            // SAFETY CHECK: In tests, we should ALWAYS have a test repo path set
            // If we get here, a test is about to modify the diamond repository itself
            panic!(
                "SAFETY VIOLATION: RefStore::new() called in test without TestRepoContext!\n\
                 This would modify the diamond repository. Use TestRepoContext in your test:\n\
                 \n\
                 let dir = tempdir()?;\n\
                 let _repo = init_test_repo(dir.path())?;\n\
                 let _ctx = TestRepoContext::new(dir.path());  // <- ADD THIS\n\
                 let ref_store = RefStore::new()?;"
            );
        }

        #[cfg(not(test))]
        {
            let gateway =
                GitGateway::new().context("Not a git repository. Run this command from within a git repository.")?;
            Ok(Self { gateway })
        }
    }

    /// Create a RefStore from a specific path (for testing)
    pub fn from_path(path: &Path) -> Result<Self> {
        let gateway = GitGateway::from_path(path).context("Failed to open repository")?;
        Ok(Self { gateway })
    }

    /// Acquire an exclusive lock on the ref store.
    ///
    /// This should be used before performing operations that modify
    /// multiple refs or need to be atomic. The lock is released when
    /// the returned guard is dropped.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let ref_store = RefStore::new()?;
    /// let _lock = ref_store.lock()?;
    /// // Perform multiple operations atomically
    /// ref_store.set_parent("feature", "main")?;
    /// ref_store.set_parent("feature-2", "feature")?;
    /// // Lock released here
    /// ```
    pub fn lock(&self) -> Result<RefStoreLockGuard> {
        RefStoreLockGuard::acquire(self.gateway.git_dir())
    }

    /// Try to acquire a lock without blocking.
    ///
    /// Returns `Ok(Some(guard))` if acquired, `Ok(None)` if another
    /// process holds the lock.
    pub fn try_lock(&self) -> Result<Option<RefStoreLockGuard>> {
        RefStoreLockGuard::try_acquire(self.gateway.git_dir())
    }

    /// Read a ref and return its blob content as a string
    pub(crate) fn read_ref_as_string(&self, ref_name: &str) -> Result<Option<String>> {
        match self.gateway.find_reference(ref_name)? {
            Some(oid) => {
                // Read the blob
                let blob_content = self
                    .gateway
                    .read_blob(&oid)
                    .context(format!("Failed to read blob for ref {}", ref_name))?;

                // Convert to string
                let content = String::from_utf8(blob_content).context(format!("Invalid UTF-8 in ref {}", ref_name))?;

                Ok(Some(content))
            }
            None => Ok(None),
        }
    }

    /// Reparent a branch to a new parent
    pub fn reparent(&self, branch: &str, new_parent: &str) -> Result<()> {
        self.set_parent(branch, new_parent)
    }

    /// Check if a branch is tracked (has a parent ref)
    pub fn is_tracked(&self, branch: &str) -> Result<bool> {
        // Use unchecked getter to avoid propagating corruption errors
        // A branch with corrupted parent metadata should still be considered "tracked"
        // (it has a parent ref, even if the content is invalid)
        Ok(self.get_parent_unchecked(branch)?.is_some())
    }

    /// Remove a branch and reparent its children to its parent (grandparent)
    ///
    /// This is used when deleting a middle branch in a stack.
    ///
    /// This operation acquires an exclusive lock to ensure atomicity
    /// when modifying multiple refs.
    pub fn remove_branch_reparent(&self, branch: &str) -> Result<()> {
        // Acquire lock for the duration of this compound operation
        let _lock = self.lock()?;

        // Get the branch's parent (will become the new parent for children)
        let parent = self.get_parent(branch)?;

        // Get children of this branch
        let children = self.get_children(branch)?;

        // Reparent each child to the grandparent
        for child in children {
            if let Some(ref p) = parent {
                self.set_parent(&child, p)?;
            } else {
                // If branch has no parent (shouldn't happen for non-trunk), remove child's parent
                self.remove_parent(&child)?;
            }
        }

        // Remove the branch's parent ref
        self.remove_parent(branch)?;

        Ok(())
    }

    /// Register a branch with a parent.
    ///
    /// This is equivalent to set_parent but with an Option for the parent.
    pub fn register_branch(&self, branch: &str, parent: Option<&str>) -> Result<()> {
        match parent {
            Some(p) => self.set_parent(branch, p),
            None => {
                // If no parent specified, remove any existing parent ref
                // (This happens for trunk registration)
                self.remove_parent(branch)
            }
        }
    }

    /// Remove a branch (just removes its parent ref)
    ///
    /// Note: This does NOT reparent children. Use remove_branch_reparent for that.
    pub fn remove_branch(&self, branch: &str) -> Result<()> {
        self.remove_parent(branch)
    }

    /// Clear all Diamond tracking data (for reset)
    ///
    /// This removes:
    /// - All parent refs (untracks all branches)
    /// - All frozen refs
    /// - Trunk configuration
    ///
    /// Used by `init --reset` to start fresh.
    ///
    /// This operation acquires an exclusive lock to ensure atomicity.
    pub fn clear_all(&self) -> Result<()> {
        // Acquire lock for the duration of this compound operation
        let _lock = self.lock()?;

        // Remove all parent refs
        let pattern = format!("{}*", PARENT_REF_PREFIX);
        let refs_to_delete = self.gateway.list_references(&pattern)?;

        for (ref_name, _) in refs_to_delete {
            self.gateway.delete_reference(&ref_name).ok(); // Ignore errors
        }

        // Remove all frozen refs
        let pattern = format!("{}*", FROZEN_REF_PREFIX);
        let refs_to_delete = self.gateway.list_references(&pattern)?;

        for (ref_name, _) in refs_to_delete {
            self.gateway.delete_reference(&ref_name).ok(); // Ignore errors
        }

        // Remove trunk config
        self.gateway.delete_reference(TRUNK_REF).ok();

        Ok(())
    }
}
