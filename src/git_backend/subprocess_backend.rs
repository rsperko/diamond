//! Subprocess-based implementation of GitBackend.
//!
//! This backend uses git CLI commands for all operations.
//! It works on any repository format, including reftable.
//!
//! # Performance Note
//!
//! TODO(perf): Each operation spawns a new git subprocess, adding ~10-50ms overhead.
//! For a typical `dm sync`, this adds 100-1000ms total latency vs git2.
//! Consider caching frequently accessed data (branch list, current branch) to reduce
//! subprocess calls. Cache should be invalidated on operations that modify branches.
//! See: agent_notes/code_review_20260102/code_quality.md Issue #1

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{GitBackend, Oid, RefFormat};

/// Subprocess-based backend implementation
#[allow(dead_code)] // Fields used by trait methods that aren't called yet
pub struct SubprocessBackend {
    git_dir: PathBuf,
    workdir: PathBuf,
    ref_format: RefFormat,
}

impl SubprocessBackend {
    /// Open a repository at the given path
    pub fn open(path: &Path) -> Result<Self> {
        // Get git directory
        let output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(path)
            .output()
            .context("Failed to find git directory")?;

        if !output.status.success() {
            anyhow::bail!("Not a git repository");
        }

        let git_dir_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let git_dir = if Path::new(&git_dir_str).is_absolute() {
            PathBuf::from(git_dir_str)
        } else {
            path.join(git_dir_str)
        };

        // Get working directory
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(path)
            .output()
            .context("Failed to find working directory")?;

        if !output.status.success() {
            anyhow::bail!("Not a working tree");
        }

        let workdir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());

        // Detect ref format
        let ref_format = super::detect_ref_format(path)?;

        Ok(Self {
            git_dir,
            workdir,
            ref_format,
        })
    }

    /// Run a git command and return output
    fn run_git(&self, args: &[&str]) -> Result<std::process::Output> {
        Command::new("git")
            .args(args)
            .current_dir(&self.workdir)
            .output()
            .context(format!("Failed to run git {}", args.join(" ")))
    }

    /// Run a git command and check for success
    fn run_git_success(&self, args: &[&str]) -> Result<()> {
        let output = self.run_git(args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
        }

        Ok(())
    }

    /// Run a git command and return stdout as string
    #[allow(dead_code)] // Used by trait methods that aren't called yet
    fn run_git_stdout(&self, args: &[&str]) -> Result<String> {
        let output = self.run_git(args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

impl GitBackend for SubprocessBackend {
    fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    fn workdir(&self) -> &Path {
        &self.workdir
    }

    fn ref_format(&self) -> RefFormat {
        self.ref_format
    }

    // =========================================================================
    // Branch operations
    // =========================================================================

    fn get_current_branch(&self) -> Result<String> {
        self.run_git_stdout(&["symbolic-ref", "--short", "HEAD"])
            .context("Failed to get current branch (HEAD may be detached)")
    }

    fn is_on_branch(&self) -> Result<bool> {
        let output = self.run_git(&["symbolic-ref", "--short", "HEAD"])?;
        Ok(output.status.success())
    }

    fn create_branch(&self, name: &str) -> Result<()> {
        self.run_git_success(&["checkout", "-b", name])
    }

    fn create_branch_at(&self, name: &str, at_ref: &str) -> Result<()> {
        self.run_git_success(&["branch", name, at_ref])
    }

    fn branch_exists(&self, name: &str) -> Result<bool> {
        let refname = format!("refs/heads/{}", name);
        let output = self.run_git(&["show-ref", "--verify", "--quiet", &refname])?;
        Ok(output.status.success())
    }

    fn checkout_branch(&self, name: &str) -> Result<()> {
        self.run_git_success(&["checkout", name])
    }

    fn checkout_branch_force(&self, name: &str) -> Result<()> {
        self.run_git_success(&["checkout", "-f", name])
    }

    fn list_branches(&self) -> Result<Vec<String>> {
        let output = self.run_git_stdout(&["for-each-ref", "--format=%(refname:short)", "refs/heads/"])?;

        Ok(output
            .lines()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect())
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        self.run_git_success(&["branch", "-D", name])
    }

    fn rename_branch(&self, old_name: &str, new_name: &str) -> Result<()> {
        self.run_git_success(&["branch", "-m", old_name, new_name])
    }

    // =========================================================================
    // Commit operations
    // =========================================================================

    fn stage_all(&self) -> Result<()> {
        self.run_git_success(&["add", "-A"])
    }

    fn stage_updates(&self) -> Result<()> {
        self.run_git_success(&["add", "-u"])
    }

    fn stage_file(&self, path: &str) -> Result<()> {
        self.run_git_success(&["add", "--", path])
    }

    fn commit(&self, message: &str) -> Result<()> {
        self.run_git_success(&["commit", "-m", message])
    }

    fn amend_commit(&self, message: Option<&str>) -> Result<()> {
        match message {
            Some(msg) => self.run_git_success(&["commit", "--amend", "-m", msg]),
            None => self.run_git_success(&["commit", "--amend", "--no-edit"]),
        }
    }

    // =========================================================================
    // Ref operations
    // =========================================================================

    fn create_reference(&self, name: &str, target: &Oid, force: bool, _msg: &str) -> Result<()> {
        if force {
            self.run_git_success(&["update-ref", name, target.as_str()])
        } else {
            // Check if ref exists first
            let output = self.run_git(&["show-ref", "--verify", name])?;
            if output.status.success() {
                anyhow::bail!("Reference '{}' already exists", name);
            }
            self.run_git_success(&["update-ref", name, target.as_str()])
        }
    }

    fn delete_reference(&self, name: &str) -> Result<()> {
        // Idempotent - succeeds even if ref doesn't exist
        let output = self.run_git(&["update-ref", "-d", name])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "not found" errors for idempotent behavior
            if !stderr.contains("not exist") && !stderr.contains("not found") && !stderr.contains("No such ref") {
                anyhow::bail!("git update-ref -d {} failed: {}", name, stderr.trim());
            }
        }

        Ok(())
    }

    fn find_reference(&self, name: &str) -> Result<Option<(String, Oid)>> {
        let output = self.run_git(&["show-ref", "--verify", name])?;

        if !output.status.success() {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.trim();
        if line.is_empty() {
            return Ok(None);
        }

        // Format: "sha1 refname"
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            Ok(Some((parts[1].to_string(), Oid::from_str_unchecked(parts[0]))))
        } else {
            Ok(None)
        }
    }

    fn list_references(&self, pattern: &str) -> Result<Vec<(String, Oid)>> {
        let output = self.run_git(&["for-each-ref", "--format=%(objectname) %(refname)", pattern])?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut refs = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                refs.push((parts[1].to_string(), Oid::from_str_unchecked(parts[0])));
            }
        }

        Ok(refs)
    }

    // =========================================================================
    // Blob operations
    // =========================================================================

    fn create_blob(&self, content: &[u8]) -> Result<Oid> {
        use std::io::Write;
        use std::process::Stdio;

        let mut child = Command::new("git")
            .args(["hash-object", "-w", "--stdin"])
            .current_dir(&self.workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("Failed to spawn git hash-object")?;

        child
            .stdin
            .as_mut()
            .context("Failed to get stdin")?
            .write_all(content)
            .context("Failed to write to stdin")?;

        let output = child.wait_with_output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to create blob");
        }

        let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Oid::from_str_unchecked(&oid))
    }

    fn read_blob(&self, oid: &Oid) -> Result<Vec<u8>> {
        let output = self.run_git(&["cat-file", "blob", oid.as_str()])?;

        if !output.status.success() {
            anyhow::bail!("Failed to read blob {}", oid);
        }

        Ok(output.stdout)
    }

    // =========================================================================
    // Validation / status operations
    // =========================================================================

    fn has_uncommitted_changes(&self) -> Result<bool> {
        let output = self.run_git_stdout(&["status", "--porcelain"])?;
        Ok(!output.is_empty())
    }

    fn has_staged_changes(&self) -> Result<bool> {
        let output = self.run_git(&["diff", "--cached", "--quiet"])?;
        Ok(!output.status.success())
    }

    fn has_staged_or_modified_changes(&self) -> Result<bool> {
        let output = self.run_git_stdout(&["status", "--porcelain"])?;

        // Check each line - skip untracked files (start with ??)
        for line in output.lines() {
            if !line.starts_with("??") {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn get_merge_base(&self, a: &str, b: &str) -> Result<Oid> {
        let oid = self.run_git_stdout(&["merge-base", a, b])?;
        Ok(Oid::from_str_unchecked(&oid))
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        let output = self.run_git(&["merge-base", "--is-ancestor", ancestor, descendant])?;
        Ok(output.status.success())
    }

    fn is_branch_merged(&self, branch: &str, into: &str) -> Result<bool> {
        self.is_ancestor(branch, into)
    }

    // =========================================================================
    // Commit info operations
    // =========================================================================

    fn get_ref_sha(&self, reference: &str) -> Result<Oid> {
        let sha = self.run_git_stdout(&["rev-parse", reference])?;
        Ok(Oid::from_str_unchecked(&sha))
    }

    fn get_short_sha(&self, reference: &str) -> Result<String> {
        self.run_git_stdout(&["rev-parse", "--short", reference])
    }

    fn get_commit_subject(&self, reference: &str) -> Result<String> {
        self.run_git_stdout(&["log", "-1", "--format=%s", reference])
    }

    fn get_commit_time_relative(&self, reference: &str) -> Result<String> {
        self.run_git_stdout(&["log", "-1", "--format=%cr", reference])
    }

    fn get_commit_count_since(&self, base: &str) -> Result<usize> {
        let output = self.run_git_stdout(&["rev-list", "--count", &format!("{}..HEAD", base)])?;
        output.parse().context("Failed to parse commit count")
    }
}
