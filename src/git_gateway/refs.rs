//! Reference and blob operations for GitGateway.
//!
//! These operations delegate to the GitBackend, which handles the
//! reftable vs files format difference internally.

use anyhow::Result;

use super::GitGateway;

// Re-export the canonical types from git_backend
// This keeps all type definitions in one place while maintaining
// the existing public API for git_gateway users.
pub use crate::git_backend::{Oid, RefFormat};

impl GitGateway {
    // === Reference Operations ===

    /// Find a reference by name
    ///
    /// Returns None if the ref doesn't exist.
    pub fn find_reference(&self, name: &str) -> Result<Option<Oid>> {
        match self.backend.find_reference(name)? {
            Some((_, oid)) => Ok(Some(oid)),
            None => Ok(None),
        }
    }

    /// List all references matching a glob pattern
    ///
    /// Pattern should be like "refs/diamond/parent/*"
    pub fn list_references(&self, glob_pattern: &str) -> Result<Vec<(String, Oid)>> {
        self.backend.list_references(glob_pattern)
    }

    /// Create or update a reference
    ///
    /// If force is true, overwrites existing ref. Otherwise fails if ref exists.
    pub fn create_reference(&self, name: &str, target: &Oid, force: bool, msg: &str) -> Result<()> {
        self.backend.create_reference(name, target, force, msg)
    }

    /// Delete a reference (idempotent - succeeds even if ref doesn't exist)
    pub fn delete_reference(&self, name: &str) -> Result<()> {
        self.backend.delete_reference(name)
    }

    // === Blob Operations ===

    /// Create a blob with the given content
    pub fn create_blob(&self, content: &[u8]) -> Result<Oid> {
        self.backend.create_blob(content)
    }

    /// Read a blob's content
    pub fn read_blob(&self, oid: &Oid) -> Result<Vec<u8>> {
        self.backend.read_blob(oid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_gateway::GitGateway;
    use crate::test_context::{init_test_repo, TestRepoContext};
    use tempfile::tempdir;

    /// Check if git version supports reftable (2.45+)
    fn git_supports_reftable() -> bool {
        let output = std::process::Command::new("git").args(["--version"]).output().ok();

        if let Some(output) = output {
            let version = String::from_utf8_lossy(&output.stdout);
            // Parse "git version 2.45.0" or similar
            if let Some(v) = version.strip_prefix("git version ") {
                let parts: Vec<&str> = v.trim().split('.').collect();
                if parts.len() >= 2 {
                    let major: u32 = parts[0].parse().unwrap_or(0);
                    let minor: u32 = parts[1].parse().unwrap_or(0);
                    return major > 2 || (major == 2 && minor >= 45);
                }
            }
        }
        false
    }

    // === Format Detection Tests ===

    #[test]
    fn test_detect_format_files_repo() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let format = GitGateway::detect_format(dir.path())?;
        assert_eq!(format, RefFormat::Files);

        Ok(())
    }

    #[test]
    fn test_detect_format_reftable_repo() -> anyhow::Result<()> {
        if !git_supports_reftable() {
            eprintln!("Skipping reftable test - git version < 2.45");
            return Ok(());
        }

        let dir = tempdir()?;

        // Create reftable repo using git CLI
        let status = std::process::Command::new("git")
            .args(["init", "-b", "main", "--ref-format=reftable"])
            .current_dir(dir.path())
            .status()?;
        assert!(status.success(), "Failed to create reftable repo");

        let _ctx = TestRepoContext::new(dir.path());

        let format = GitGateway::detect_format(dir.path())?;
        assert_eq!(format, RefFormat::Reftable);

        Ok(())
    }

    // === Subprocess Ref Operations Tests ===
    // These test the subprocess code paths by forcing subprocess backend

    /// Create a GitGateway that forces subprocess mode (simulates reftable)
    fn create_subprocess_gateway(path: &std::path::Path) -> anyhow::Result<GitGateway> {
        // First create a normal gateway to get the paths
        let normal = GitGateway::from_path(path)?;

        // Create a subprocess backend
        let backend = Box::new(crate::git_backend::SubprocessBackend::open(path)?);

        // Create gateway with subprocess backend (simulates reftable)
        Ok(GitGateway {
            backend,
            git_dir: normal.git_dir.clone(),
            workdir: normal.workdir.clone(),
            remote: "origin".to_string(),
            format: RefFormat::Reftable, // Pretend it's reftable
        })
    }

    #[test]
    fn test_subprocess_create_and_read_blob() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = create_subprocess_gateway(dir.path())?;

        // Create a blob
        let content = b"test blob content";
        let oid = gateway.create_blob(content)?;

        // Read it back
        let read_content = gateway.read_blob(&oid)?;
        assert_eq!(read_content, content);

        Ok(())
    }

    #[test]
    fn test_subprocess_create_and_find_reference() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = create_subprocess_gateway(dir.path())?;

        // Create a blob first
        let content = b"parent-branch-name";
        let blob_oid = gateway.create_blob(content)?;

        // Create a reference pointing to the blob
        let ref_name = "refs/diamond/parent/test-branch";
        gateway.create_reference(ref_name, &blob_oid, false, "test ref")?;

        // Find it
        let found = gateway.find_reference(ref_name)?;
        assert!(found.is_some());
        assert_eq!(found.unwrap(), blob_oid);

        Ok(())
    }

    #[test]
    fn test_subprocess_list_references() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = create_subprocess_gateway(dir.path())?;

        // Create multiple refs
        let blob1 = gateway.create_blob(b"main")?;
        let blob2 = gateway.create_blob(b"feature")?;

        gateway.create_reference("refs/diamond/parent/branch-a", &blob1, false, "test")?;
        gateway.create_reference("refs/diamond/parent/branch-b", &blob2, false, "test")?;

        // List them
        let refs = gateway.list_references("refs/diamond/parent/*")?;
        assert_eq!(refs.len(), 2);

        let ref_names: Vec<&str> = refs.iter().map(|(name, _)| name.as_str()).collect();
        assert!(ref_names.contains(&"refs/diamond/parent/branch-a"));
        assert!(ref_names.contains(&"refs/diamond/parent/branch-b"));

        Ok(())
    }

    #[test]
    fn test_subprocess_delete_reference() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = create_subprocess_gateway(dir.path())?;

        // Create a ref
        let blob = gateway.create_blob(b"main")?;
        let ref_name = "refs/diamond/parent/to-delete";
        gateway.create_reference(ref_name, &blob, false, "test")?;

        // Verify it exists
        assert!(gateway.find_reference(ref_name)?.is_some());

        // Delete it
        gateway.delete_reference(ref_name)?;

        // Verify it's gone
        assert!(gateway.find_reference(ref_name)?.is_none());

        Ok(())
    }

    #[test]
    fn test_subprocess_delete_nonexistent_reference_is_idempotent() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = create_subprocess_gateway(dir.path())?;

        // Delete a ref that doesn't exist - should not error
        let result = gateway.delete_reference("refs/diamond/parent/nonexistent");
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_subprocess_find_nonexistent_reference() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = create_subprocess_gateway(dir.path())?;

        // Find a ref that doesn't exist - should return None, not error
        let found = gateway.find_reference("refs/diamond/parent/nonexistent")?;
        assert!(found.is_none());

        Ok(())
    }

    // === Reftable Integration Test ===
    // This tests the full stack on an actual reftable repo

    #[test]
    fn test_reftable_repo_full_refstore_operations() -> anyhow::Result<()> {
        if !git_supports_reftable() {
            eprintln!("Skipping reftable integration test - git version < 2.45");
            return Ok(());
        }

        let dir = tempdir()?;

        // Create reftable repo with initial commit
        std::process::Command::new("git")
            .args(["init", "-b", "main", "--ref-format=reftable"])
            .current_dir(dir.path())
            .status()?;

        // Configure git user (needed for commit in CI)
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(dir.path())
            .status()?;
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir.path())
            .status()?;

        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "Initial commit"])
            .current_dir(dir.path())
            .status()?;

        // Create a branch
        std::process::Command::new("git")
            .args(["branch", "feature"])
            .current_dir(dir.path())
            .status()?;

        let _ctx = TestRepoContext::new(dir.path());

        // Create RefStore - should work on reftable repo
        let ref_store = crate::ref_store::RefStore::from_path(dir.path())?;

        // Set trunk
        ref_store.set_trunk("main")?;
        assert_eq!(ref_store.get_trunk()?, Some("main".to_string()));

        // Set parent
        ref_store.set_parent("feature", "main")?;
        assert_eq!(ref_store.get_parent("feature")?, Some("main".to_string()));

        // Get children
        let children = ref_store.get_children("main")?;
        assert!(children.contains("feature"));

        // Remove parent
        ref_store.remove_parent("feature")?;
        assert_eq!(ref_store.get_parent("feature")?, None);

        Ok(())
    }

    // === Oid Tests ===

    #[test]
    fn test_oid_from_str_valid() {
        let hash = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let oid = Oid::from_str(hash).unwrap();
        assert_eq!(oid.as_str(), hash);
    }

    #[test]
    fn test_oid_from_str_with_whitespace() {
        let hash = "  a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2  \n";
        let oid = Oid::from_str(hash).unwrap();
        assert_eq!(oid.as_str(), "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2");
    }

    #[test]
    fn test_oid_from_str_invalid_length() {
        let result = Oid::from_str("abc123");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("length"));
    }

    #[test]
    fn test_oid_from_str_invalid_chars() {
        let result = Oid::from_str("g1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"); // 'g' is not hex
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("non-hex"));
    }

    #[test]
    fn test_oid_roundtrip_git2() -> anyhow::Result<()> {
        let hash = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let oid = Oid::from_str(hash)?;

        let git2_oid = oid.to_git2()?;
        let roundtrip = Oid::from(git2_oid);

        assert_eq!(roundtrip.as_str(), hash);
        Ok(())
    }
}
