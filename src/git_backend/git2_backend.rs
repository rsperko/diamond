//! Git2 (libgit2) implementation of GitBackend.
//!
//! This backend uses the git2 crate for fast, native git operations.
//! It only works on repositories using the "files" ref format.

use anyhow::{Context, Result};
use git2::{BranchType, IndexAddOption, Repository, Signature};
use std::path::{Path, PathBuf};

use super::{GitBackend, Oid, RefFormat};

/// Git2-based backend implementation
#[allow(dead_code)] // Fields used by trait methods that aren't called yet
pub struct Git2Backend {
    repo: Repository,
    git_dir: PathBuf,
    workdir: PathBuf,
}

impl Git2Backend {
    /// Open a repository at the given path
    pub fn open(path: &Path) -> Result<Self> {
        let repo = Repository::discover(path).context("Failed to open git repository with git2")?;

        let git_dir = repo.path().to_path_buf();
        let workdir = repo.workdir().context("Not a work tree")?.to_path_buf();

        Ok(Self { repo, git_dir, workdir })
    }

    /// Get a reference to the underlying Repository (for operations not yet abstracted)
    #[allow(dead_code)] // Available for potential future use
    pub fn repo(&self) -> &Repository {
        &self.repo
    }

    #[allow(dead_code)] // Used by trait methods that aren't called yet
    fn signature(&self) -> Result<Signature<'_>> {
        self.repo
            .signature()
            .or_else(|_| Signature::now("Diamond", "diamond@local"))
            .context("Failed to create signature")
    }
}

impl GitBackend for Git2Backend {
    fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    fn workdir(&self) -> &Path {
        &self.workdir
    }

    fn ref_format(&self) -> RefFormat {
        RefFormat::Files
    }

    // =========================================================================
    // Branch operations
    // =========================================================================

    fn get_current_branch(&self) -> Result<String> {
        let head = self.repo.head().context("Failed to get HEAD")?;

        if !head.is_branch() {
            anyhow::bail!("HEAD is detached");
        }

        head.shorthand()
            .map(|s| s.to_string())
            .context("Branch name is not valid UTF-8")
    }

    fn is_on_branch(&self) -> Result<bool> {
        match self.repo.head() {
            Ok(head) => Ok(head.is_branch()),
            Err(_) => Ok(false),
        }
    }

    fn create_branch(&self, name: &str) -> Result<()> {
        let head = self.repo.head().context("Failed to get HEAD")?;
        let commit = head.peel_to_commit().context("Failed to get HEAD commit")?;

        self.repo
            .branch(name, &commit, false)
            .context(format!("Failed to create branch '{}'", name))?;

        // Set HEAD to the new branch WITHOUT resetting the index
        // This is like `git checkout -b name` which preserves staged changes
        let refname = format!("refs/heads/{}", name);
        self.repo
            .set_head(&refname)
            .context(format!("Failed to set HEAD to '{}'", name))?;

        Ok(())
    }

    fn create_branch_at(&self, name: &str, at_ref: &str) -> Result<()> {
        let reference = self
            .repo
            .find_reference(&format!("refs/heads/{}", at_ref))
            .or_else(|_| self.repo.find_reference(at_ref))
            .context(format!("Failed to find ref '{}'", at_ref))?;

        let commit = reference.peel_to_commit().context("Failed to get commit for ref")?;

        self.repo
            .branch(name, &commit, false)
            .context(format!("Failed to create branch '{}' at '{}'", name, at_ref))?;

        Ok(())
    }

    fn branch_exists(&self, name: &str) -> Result<bool> {
        Ok(self.repo.find_branch(name, BranchType::Local).is_ok())
    }

    fn checkout_branch(&self, name: &str) -> Result<()> {
        let refname = format!("refs/heads/{}", name);
        let commit;

        // Check if local branch exists
        if let Ok(reference) = self.repo.find_reference(&refname) {
            commit = reference
                .peel_to_commit()
                .context("Failed to peel reference to commit")?;
        } else {
            // Local branch doesn't exist - check for remote tracking branch
            let remote_refname = format!("refs/remotes/origin/{}", name);
            if let Ok(remote_ref) = self.repo.find_reference(&remote_refname) {
                commit = remote_ref
                    .peel_to_commit()
                    .context("Failed to peel remote reference to commit")?;
                // Create local branch from remote
                self.repo
                    .branch(name, &commit, false)
                    .context(format!("Failed to create local branch '{}' from remote", name))?;
            } else {
                anyhow::bail!("Branch '{}' not found", name);
            }
        }

        let tree = commit.tree().context("Failed to get commit tree")?;

        // First checkout the tree to update working directory
        // Use safe mode to preserve uncommitted changes and untracked files
        let mut checkout_builder = git2::build::CheckoutBuilder::new();
        checkout_builder
            .safe() // Don't overwrite uncommitted changes
            .recreate_missing(true); // Recreate missing files from target tree
                                     // NOTE: We do NOT use .remove_untracked(true) - git never deletes untracked files on checkout!

        self.repo
            .checkout_tree(tree.as_object(), Some(&mut checkout_builder))
            .context("Failed to checkout tree")?;

        // Then set HEAD to the branch
        self.repo
            .set_head(&refname)
            .context(format!("Failed to set HEAD to '{}'", name))?;

        Ok(())
    }

    fn checkout_branch_force(&self, name: &str) -> Result<()> {
        let refname = format!("refs/heads/{}", name);
        let commit;

        // Check if local branch exists
        if let Ok(reference) = self.repo.find_reference(&refname) {
            commit = reference
                .peel_to_commit()
                .context("Failed to peel reference to commit")?;
            self.repo
                .set_head(&refname)
                .context(format!("Failed to set HEAD to '{}'", name))?;
        } else {
            // Local branch doesn't exist - check if there's a remote tracking branch
            // and create a local tracking branch from it
            let remote_refname = format!("refs/remotes/origin/{}", name);
            if let Ok(remote_ref) = self.repo.find_reference(&remote_refname) {
                commit = remote_ref
                    .peel_to_commit()
                    .context("Failed to peel remote ref to commit")?;

                // Create local branch pointing to the same commit
                self.repo
                    .branch(name, &commit, false)
                    .context(format!("Failed to create local branch '{}'", name))?;

                // Now set HEAD to the new branch
                self.repo
                    .set_head(&refname)
                    .context(format!("Failed to set HEAD to '{}'", name))?;
            } else {
                // Neither local nor remote branch exists
                return Err(anyhow::anyhow!("Branch '{}' not found locally or in remote", name));
            }
        }

        // Hard reset updates HEAD, index, AND working directory to match the commit
        let mut checkout_opts = git2::build::CheckoutBuilder::new();
        checkout_opts.force();

        self.repo
            .reset(commit.as_object(), git2::ResetType::Hard, Some(&mut checkout_opts))
            .context("Failed to checkout branch")?;

        Ok(())
    }

    fn list_branches(&self) -> Result<Vec<String>> {
        let mut branches = Vec::new();

        for branch in self.repo.branches(Some(BranchType::Local))? {
            let (branch, _) = branch?;
            if let Some(name) = branch.name()? {
                branches.push(name.to_string());
            }
        }

        Ok(branches)
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        let mut branch = self
            .repo
            .find_branch(name, BranchType::Local)
            .context(format!("Branch '{}' not found", name))?;

        branch.delete().context(format!("Failed to delete branch '{}'", name))?;

        Ok(())
    }

    fn rename_branch(&self, old_name: &str, new_name: &str) -> Result<()> {
        let mut branch = self
            .repo
            .find_branch(old_name, BranchType::Local)
            .context(format!("Branch '{}' not found", old_name))?;

        branch
            .rename(new_name, false)
            .context(format!("Failed to rename '{}' to '{}'", old_name, new_name))?;

        Ok(())
    }

    // =========================================================================
    // Commit operations
    // =========================================================================

    fn stage_all(&self) -> Result<()> {
        let mut index = self.repo.index().context("Failed to get index")?;
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .context("Failed to stage all files")?;
        index.write().context("Failed to write index")?;
        Ok(())
    }

    fn stage_updates(&self) -> Result<()> {
        let mut index = self.repo.index().context("Failed to get index")?;
        index
            .update_all(["*"].iter(), None)
            .context("Failed to stage updates")?;
        index.write().context("Failed to write index")?;
        Ok(())
    }

    fn stage_file(&self, path: &str) -> Result<()> {
        let mut index = self.repo.index().context("Failed to get index")?;
        index
            .add_path(Path::new(path))
            .context(format!("Failed to stage '{}'", path))?;
        index.write().context("Failed to write index")?;
        Ok(())
    }

    fn commit(&self, message: &str) -> Result<()> {
        let sig = self.signature()?;
        let mut index = self.repo.index()?;
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;

        let head = self.repo.head()?;
        let parent = head.peel_to_commit()?;

        self.repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])?;

        Ok(())
    }

    fn amend_commit(&self, message: Option<&str>) -> Result<()> {
        let head = self.repo.head()?;
        let commit = head.peel_to_commit()?;

        let sig = self.signature()?;
        let mut index = self.repo.index()?;
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;

        let msg = message.unwrap_or_else(|| commit.message().unwrap_or(""));

        commit.amend(Some("HEAD"), Some(&sig), Some(&sig), None, Some(msg), Some(&tree))?;

        Ok(())
    }

    // =========================================================================
    // Ref operations
    // =========================================================================

    fn create_reference(&self, name: &str, target: &Oid, force: bool, msg: &str) -> Result<()> {
        let oid = git2::Oid::from_str(target.as_str()).context("Invalid OID")?;

        self.repo
            .reference(name, oid, force, msg)
            .context(format!("Failed to create reference '{}'", name))?;

        Ok(())
    }

    fn delete_reference(&self, name: &str) -> Result<()> {
        // Idempotent - succeeds even if ref doesn't exist
        match self.repo.find_reference(name) {
            Ok(mut reference) => {
                reference
                    .delete()
                    .context(format!("Failed to delete reference '{}'", name))?;
            }
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                // Already doesn't exist, that's fine
            }
            Err(e) => return Err(e).context(format!("Failed to find reference '{}'", name)),
        }
        Ok(())
    }

    fn find_reference(&self, name: &str) -> Result<Option<(String, Oid)>> {
        match self.repo.find_reference(name) {
            Ok(reference) => {
                let oid = reference.target().context("Reference has no target")?;
                Ok(Some((name.to_string(), Oid::from(oid))))
            }
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn list_references(&self, pattern: &str) -> Result<Vec<(String, Oid)>> {
        let mut refs = Vec::new();

        for reference in self.repo.references_glob(pattern)? {
            let reference = reference?;
            if let (Some(name), Some(oid)) = (reference.name(), reference.target()) {
                refs.push((name.to_string(), Oid::from(oid)));
            }
        }

        Ok(refs)
    }

    // =========================================================================
    // Blob operations
    // =========================================================================

    fn create_blob(&self, content: &[u8]) -> Result<Oid> {
        let oid = self.repo.blob(content).context("Failed to create blob")?;

        Ok(Oid::from(oid))
    }

    fn read_blob(&self, oid: &Oid) -> Result<Vec<u8>> {
        let git_oid = git2::Oid::from_str(oid.as_str()).context("Invalid OID")?;

        let blob = self.repo.find_blob(git_oid).context("Failed to find blob")?;

        Ok(blob.content().to_vec())
    }

    // =========================================================================
    // Validation / status operations
    // =========================================================================

    fn has_uncommitted_changes(&self) -> Result<bool> {
        let mut opts = git2::StatusOptions::new();
        opts.include_ignored(false).include_untracked(true);

        let statuses = self.repo.statuses(Some(&mut opts)).context("Failed to get status")?;

        Ok(!statuses.is_empty())
    }

    fn has_staged_changes(&self) -> Result<bool> {
        let mut opts = git2::StatusOptions::new();
        opts.include_ignored(false);

        let statuses = self.repo.statuses(Some(&mut opts))?;

        for entry in statuses.iter() {
            let status = entry.status();
            if status.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED
                    | git2::Status::INDEX_TYPECHANGE,
            ) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn has_staged_or_modified_changes(&self) -> Result<bool> {
        let mut opts = git2::StatusOptions::new();
        opts.include_ignored(false);

        let statuses = self.repo.statuses(Some(&mut opts))?;

        for entry in statuses.iter() {
            let status = entry.status();
            // Check for staged changes (INDEX_*) or modified tracked files (WT_MODIFIED)
            // Exclude untracked files (WT_NEW)
            if status.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED
                    | git2::Status::INDEX_TYPECHANGE
                    | git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_RENAMED
                    | git2::Status::WT_TYPECHANGE,
            ) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn get_merge_base(&self, a: &str, b: &str) -> Result<Oid> {
        let oid_a = self.get_ref_sha(a)?;
        let oid_b = self.get_ref_sha(b)?;

        let git_oid_a = git2::Oid::from_str(oid_a.as_str())?;
        let git_oid_b = git2::Oid::from_str(oid_b.as_str())?;

        let merge_base = self
            .repo
            .merge_base(git_oid_a, git_oid_b)
            .context("Failed to find merge base")?;

        Ok(Oid::from(merge_base))
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        let oid_ancestor = self.get_ref_sha(ancestor)?;
        let oid_descendant = self.get_ref_sha(descendant)?;

        let git_oid_ancestor = git2::Oid::from_str(oid_ancestor.as_str())?;
        let git_oid_descendant = git2::Oid::from_str(oid_descendant.as_str())?;

        // ancestor is an ancestor of descendant if merge_base(ancestor, descendant) == ancestor
        match self.repo.merge_base(git_oid_ancestor, git_oid_descendant) {
            Ok(merge_base) => Ok(merge_base == git_oid_ancestor),
            Err(_) => Ok(false),
        }
    }

    fn is_branch_merged(&self, branch: &str, into: &str) -> Result<bool> {
        // A branch is merged if its tip is an ancestor of the target
        self.is_ancestor(branch, into)
    }

    // =========================================================================
    // Commit info operations
    // =========================================================================

    fn get_ref_sha(&self, reference: &str) -> Result<Oid> {
        // Try as branch first
        if let Ok(branch) = self.repo.find_branch(reference, BranchType::Local) {
            let commit = branch.get().peel_to_commit()?;
            return Ok(Oid::from(commit.id()));
        }

        // Try as reference
        if let Ok(git_ref) = self.repo.find_reference(reference) {
            let commit = git_ref.peel_to_commit()?;
            return Ok(Oid::from(commit.id()));
        }

        // Try as commit SHA
        if let Ok(oid) = git2::Oid::from_str(reference) {
            if self.repo.find_commit(oid).is_ok() {
                return Ok(Oid::from(oid));
            }
        }

        // Try revparse as last resort
        let obj = self
            .repo
            .revparse_single(reference)
            .context(format!("Failed to resolve '{}'", reference))?;

        let commit = obj.peel_to_commit()?;
        Ok(Oid::from(commit.id()))
    }

    fn get_short_sha(&self, reference: &str) -> Result<String> {
        let oid = self.get_ref_sha(reference)?;
        Ok(oid.short().to_string())
    }

    fn get_commit_subject(&self, reference: &str) -> Result<String> {
        let oid = self.get_ref_sha(reference)?;
        let git_oid = git2::Oid::from_str(oid.as_str())?;
        let commit = self.repo.find_commit(git_oid)?;

        let message = commit.message().unwrap_or("");
        let subject = message.lines().next().unwrap_or("");

        Ok(subject.to_string())
    }

    fn get_commit_time_relative(&self, reference: &str) -> Result<String> {
        let oid = self.get_ref_sha(reference)?;
        let git_oid = git2::Oid::from_str(oid.as_str())?;
        let commit = self.repo.find_commit(git_oid)?;

        let time = commit.time();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let diff = now - time.seconds();

        // Convert to human-readable
        if diff < 60 {
            Ok("just now".to_string())
        } else if diff < 3600 {
            let mins = diff / 60;
            Ok(format!("{} minute{} ago", mins, if mins == 1 { "" } else { "s" }))
        } else if diff < 86400 {
            let hours = diff / 3600;
            Ok(format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" }))
        } else if diff < 604800 {
            let days = diff / 86400;
            Ok(format!("{} day{} ago", days, if days == 1 { "" } else { "s" }))
        } else {
            let weeks = diff / 604800;
            Ok(format!("{} week{} ago", weeks, if weeks == 1 { "" } else { "s" }))
        }
    }

    fn get_commit_count_since(&self, base: &str) -> Result<usize> {
        let base_oid = self.get_ref_sha(base)?;
        let git_base_oid = git2::Oid::from_str(base_oid.as_str())?;

        let head = self.repo.head()?;
        let head_commit = head.peel_to_commit()?;

        let mut revwalk = self.repo.revwalk()?;
        revwalk.push(head_commit.id())?;
        revwalk.hide(git_base_oid)?;

        Ok(revwalk.count())
    }
}
