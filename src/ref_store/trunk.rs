//! Trunk configuration operations for RefStore.

use anyhow::{Context, Result};

use crate::program_name::program_name;

use super::{RefStore, TRUNK_REF};

#[allow(dead_code)]
impl RefStore {
    /// Set the trunk branch
    ///
    /// Creates: refs/diamond/config/trunk -> blob("<branch>")
    pub fn set_trunk(&self, branch: &str) -> Result<()> {
        // Validate that the branch actually exists
        // This prevents orphaned trunk refs pointing to non-existent branches
        if !self.gateway.branch_exists(branch)? {
            anyhow::bail!("Branch '{}' does not exist. Cannot set as trunk.", branch);
        }

        // Create blob containing trunk branch name
        let blob_oid = self
            .gateway
            .create_blob(branch.as_bytes())
            .context("Failed to create trunk blob")?;

        // Create/update ref pointing to the blob
        self.gateway
            .create_reference(
                TRUNK_REF,
                &blob_oid,
                true, // force
                &format!("dm: set trunk to {}", branch),
            )
            .context("Failed to set trunk ref")?;

        Ok(())
    }

    /// Get the trunk branch
    pub fn get_trunk(&self) -> Result<Option<String>> {
        self.read_ref_as_string(TRUNK_REF)
    }

    /// Get trunk or return error if not initialized
    pub fn require_trunk(&self) -> Result<String> {
        self.get_trunk()?
            .ok_or_else(|| anyhow::anyhow!("Diamond is not initialized. Run `{} init` first.", program_name()))
    }

    /// Check if Diamond is initialized (trunk is set)
    pub fn is_initialized(&self) -> Result<bool> {
        Ok(self.get_trunk()?.is_some())
    }
}
