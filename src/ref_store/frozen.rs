//! Frozen branch operations for RefStore.

use anyhow::{Context, Result};

use super::{RefStore, FROZEN_REF_PREFIX};

#[allow(dead_code)]
impl RefStore {
    /// Check if a branch is frozen
    ///
    /// Frozen branches cannot be modified locally (e.g., with `dm modify`)
    pub fn is_frozen(&self, branch: &str) -> Result<bool> {
        let ref_name = format!("{}{}", FROZEN_REF_PREFIX, branch);
        Ok(self.gateway.find_reference(&ref_name)?.is_some())
    }

    /// Set or clear the frozen state of a branch
    ///
    /// When frozen=true, creates refs/diamond/frozen/<branch>
    /// When frozen=false, deletes that ref
    pub fn set_frozen(&self, branch: &str, frozen: bool) -> Result<()> {
        let ref_name = format!("{}{}", FROZEN_REF_PREFIX, branch);

        if frozen {
            // Create blob with empty content (we only care about ref existence)
            let blob_oid = self
                .gateway
                .create_blob(b"")
                .context("Failed to create frozen marker blob")?;

            self.gateway
                .create_reference(
                    &ref_name,
                    &blob_oid,
                    true, // force: overwrite if exists
                    &format!("dm: freeze {}", branch),
                )
                .context(format!("Failed to freeze {}", branch))?;
        } else {
            // Delete the ref if it exists (idempotent)
            self.gateway
                .delete_reference(&ref_name)
                .context(format!("Failed to unfreeze {}", branch))?;
        }

        Ok(())
    }

    /// List all frozen branches (sorted alphabetically)
    pub fn list_frozen_branches(&self) -> Result<Vec<String>> {
        let mut branches = Vec::new();

        let pattern = format!("{}*", FROZEN_REF_PREFIX);
        for (ref_name, _) in self.gateway.list_references(&pattern)? {
            if let Some(branch) = ref_name.strip_prefix(FROZEN_REF_PREFIX) {
                branches.push(branch.to_string());
            }
        }

        branches.sort();
        Ok(branches)
    }
}
