//! Diamond-specific ref operations for GitGateway.

use anyhow::Context;
use anyhow::Result;

use super::verbose_cmd;
use super::GitGateway;
use crate::ref_store::PARENT_REF_PREFIX;

impl GitGateway {
    /// Push a diamond parent ref to a remote
    ///
    /// Pushes refs/diamond/parent/<branch> to enable collaboration.
    /// The ref contains the symbolic pointer to the parent branch.
    pub fn push_diamond_ref_to(&self, branch: &str, remote: &str) -> Result<()> {
        let ref_name = format!("{}{}", PARENT_REF_PREFIX, branch);

        verbose_cmd("push", &["--force", remote, &ref_name]);

        // Diamond refs point to blob objects (parent branch names), not commits.
        // Git requires --force to update refs that point to non-commit objects.
        let output = std::process::Command::new("git")
            .args(["push", "--force", remote, &ref_name])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git push for diamond ref")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Non-fatal: ref might already be up to date or remote might not exist
            if !stderr.contains("up-to-date") && !stderr.contains("Everything up-to-date") {
                anyhow::bail!("Failed to push diamond ref: {}", stderr.trim());
            }
        }

        Ok(())
    }

    /// Push a diamond parent ref to the configured remote (convenience method)
    pub fn push_diamond_ref(&self, branch: &str) -> Result<()> {
        self.push_diamond_ref_to(branch, &self.remote)
    }

    /// Delete a diamond parent ref from a remote
    ///
    /// Removes refs/diamond/parent/<branch> when a branch is deleted.
    pub fn delete_remote_diamond_ref_from(&self, branch: &str, remote: &str) -> Result<()> {
        let ref_name = format!("{}{}", PARENT_REF_PREFIX, branch);
        let delete_refspec = format!(":{}", ref_name);

        verbose_cmd("push", &[remote, "--delete", &ref_name]);

        let output = std::process::Command::new("git")
            .args(["push", remote, &delete_refspec])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git push --delete for diamond ref")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Non-fatal: ref might not exist on remote
            if !stderr.contains("not found") && !stderr.contains("does not exist") {
                anyhow::bail!("Failed to delete remote diamond ref: {}", stderr.trim());
            }
        }

        Ok(())
    }

    /// Delete a diamond parent ref from the configured remote (convenience method)
    pub fn delete_remote_diamond_ref(&self, branch: &str) -> Result<()> {
        self.delete_remote_diamond_ref_from(branch, &self.remote)
    }

    /// Fetch a single branch's diamond ref from a remote
    ///
    /// Used when checking out a branch to get its parent relationship from remote.
    /// This is a best-effort operation - failures are non-fatal.
    pub fn fetch_diamond_ref_for_branch_from(&self, branch: &str, remote: &str) -> Result<()> {
        let ref_spec = format!("{0}{1}:{0}{1}", PARENT_REF_PREFIX, branch);

        verbose_cmd("fetch", &[remote, &ref_spec]);

        let output = std::process::Command::new("git")
            .args(["fetch", remote, &ref_spec])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git fetch for diamond ref")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Non-fatal: ref might not exist on remote yet
            if !stderr.contains("no matching refs")
                && !stderr.contains("couldn't find remote ref")
                && !stderr.is_empty()
            {
                anyhow::bail!("Failed to fetch diamond ref: {}", stderr.trim());
            }
        }

        Ok(())
    }

    /// Fetch a single branch's diamond ref from the configured remote (convenience method)
    pub fn fetch_diamond_ref_for_branch(&self, branch: &str) -> Result<()> {
        self.fetch_diamond_ref_for_branch_from(branch, &self.remote)
    }

    /// Configure remote to automatically fetch diamond refs
    ///
    /// Adds refspec to .git/config so `git fetch <remote>` includes diamond refs.
    /// This is called by `dm init` to set up collaboration.
    pub fn configure_diamond_refspec_for(&self, remote: &str) -> Result<()> {
        // Check if refspec already exists
        let config_key = format!("remote.{}.fetch", remote);
        let output = std::process::Command::new("git")
            .args(["config", "--get-all", &config_key])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to check existing refspecs")?;

        let existing = String::from_utf8_lossy(&output.stdout);
        if existing.contains("refs/diamond/*:refs/diamond/*") {
            // Already configured
            return Ok(());
        }

        // Add the refspec
        let output = std::process::Command::new("git")
            .args(["config", "--add", &config_key, "+refs/diamond/*:refs/diamond/*"])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to add diamond refspec")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to configure diamond refspec: {}", stderr.trim());
        }

        Ok(())
    }

    /// Configure the configured remote to automatically fetch diamond refs (convenience method)
    pub fn configure_diamond_refspec(&self) -> Result<()> {
        self.configure_diamond_refspec_for(&self.remote)
    }

    /// Push the trunk config ref to a remote
    ///
    /// Pushes refs/diamond/config/trunk so team members know the trunk branch.
    #[allow(dead_code)] // Will be used in Phase 2 init
    pub fn push_trunk_ref_to(&self, remote: &str) -> Result<()> {
        let ref_name = "refs/diamond/config/trunk";

        verbose_cmd("push", &[remote, ref_name]);

        let output = std::process::Command::new("git")
            .args(["push", remote, ref_name])
            .current_dir(&self.workdir)
            .output()
            .context("Failed to run git push for trunk ref")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("up-to-date") && !stderr.contains("Everything up-to-date") {
                anyhow::bail!("Failed to push trunk ref: {}", stderr.trim());
            }
        }

        Ok(())
    }

    /// Push the trunk config ref to the configured remote (convenience method)
    #[allow(dead_code)] // Will be used in Phase 2 init
    pub fn push_trunk_ref(&self) -> Result<()> {
        self.push_trunk_ref_to(&self.remote)
    }

    /// Prune orphaned diamond refs
    ///
    /// Removes refs/diamond/parent/<branch> for branches that no longer exist.
    /// Returns the list of pruned ref names.
    #[allow(dead_code)] // Will be used in Phase 2 doctor/sync --prune
    pub fn prune_orphaned_diamond_refs(&self) -> Result<Vec<String>> {
        let mut pruned = Vec::new();

        // List all parent refs
        let pattern = format!("{}*", PARENT_REF_PREFIX);
        let refs = self.list_references(&pattern)?;

        for (name, _oid) in refs {
            if let Some(branch) = name.strip_prefix(PARENT_REF_PREFIX) {
                if !self.branch_exists(branch)? {
                    self.delete_reference(&name)?;
                    pruned.push(name);
                }
            }
        }

        Ok(pruned)
    }
}
