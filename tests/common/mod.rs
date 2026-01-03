use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Helper to get the path to the dm binary
pub fn dm_binary() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push("dm");
    path
}

/// Helper to initialize a test git repository
#[allow(dead_code)]
pub fn init_test_repo(dir: &Path) -> Result<()> {
    // Initialize git repo
    Command::new("git").args(["init"]).current_dir(dir).output()?;

    // Configure git
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(dir)
        .output()?;

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(dir)
        .output()?;

    // Prevent editors from blocking tests
    Command::new("git")
        .args(["config", "core.editor", "true"])
        .current_dir(dir)
        .output()?;

    // Prevent sequence editor from blocking (for rebase)
    Command::new("git")
        .args(["config", "sequence.editor", "true"])
        .current_dir(dir)
        .output()?;

    // Create initial commit
    fs::write(dir.join("README.md"), "# Test Repo")?;
    Command::new("git").args(["add", "."]).current_dir(dir).output()?;

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(dir)
        .output()?;

    // Initialize diamond
    Command::new(dm_binary()).args(["init"]).current_dir(dir).output()?;

    Ok(())
}

/// Helper to run dm command and return output
pub fn run_dm(dir: &Path, args: &[&str]) -> Result<std::process::Output> {
    use std::process::Stdio;
    Ok(Command::new(dm_binary())
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()?)
}

/// Helper to get current git branch
#[allow(dead_code)]
pub fn get_current_branch(dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(dir)
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Helper to get last commit message
#[allow(dead_code)]
pub fn get_last_commit_message(dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["log", "-1", "--pretty=format:%s"])
        .current_dir(dir)
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Helper to run git commands directly (bypassing Diamond)
#[allow(dead_code)]
pub fn run_git(dir: &Path, args: &[&str]) -> Result<std::process::Output> {
    Ok(Command::new("git").args(args).current_dir(dir).output()?)
}

/// Helper to check if branch exists in git
#[allow(dead_code)]
pub fn git_branch_exists(dir: &Path, name: &str) -> Result<bool> {
    let output = run_git(dir, &["branch", "--list", name])?;
    Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

/// Helper to get commit hash for a branch
#[allow(dead_code)]
pub fn get_commit_hash(dir: &Path, branch: &str) -> Result<String> {
    let output = run_git(dir, &["rev-parse", branch])?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Helper to create operation_state.json with specific content
#[allow(dead_code)]
pub fn create_operation_state(dir: &Path, state: &serde_json::Value) -> Result<()> {
    let diamond_dir = dir.join(".git/diamond");
    if !diamond_dir.exists() {
        fs::create_dir_all(&diamond_dir)?;
    }
    let path = diamond_dir.join("operation_state.json");
    fs::write(path, serde_json::to_string_pretty(state)?)?;
    Ok(())
}

/// Helper to delete operation_state.json
#[allow(dead_code)]
pub fn delete_operation_state(dir: &Path) -> Result<()> {
    let path = dir.join(".git/diamond/operation_state.json");
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// Helper to create a file and commit it
#[allow(dead_code)]
pub fn create_file_and_commit(dir: &Path, filename: &str, content: &str, message: &str) -> Result<()> {
    fs::write(dir.join(filename), content)?;
    run_git(dir, &["add", filename])?;
    run_git(dir, &["commit", "-m", message])?;
    Ok(())
}

/// Helper to check if git rebase is in progress
#[allow(dead_code)]
pub fn git_rebase_in_progress(dir: &Path) -> Result<bool> {
    let git_dir = dir.join(".git");
    Ok(git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists())
}

/// Helper to get operation state
#[allow(dead_code)]
pub fn get_operation_state(dir: &Path) -> Result<Option<serde_json::Value>> {
    let path = dir.join(".git/diamond/operation_state.json");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&content)?))
}

/// Helper to check if a branch is tracked in refs
#[allow(dead_code)]
pub fn is_branch_tracked_in_refs(dir: &Path, branch: &str) -> Result<bool> {
    let output = run_git(dir, &["show-ref", &format!("refs/diamond/parent/{}", branch)])?;
    Ok(output.status.success())
}

/// Helper to get the parent of a branch from refs
#[allow(dead_code)]
pub fn get_parent_from_refs(dir: &Path, branch: &str) -> Result<Option<String>> {
    let output = run_git(dir, &["cat-file", "-p", &format!("refs/diamond/parent/{}", branch)])?;
    if output.status.success() {
        let parent = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if parent.is_empty() {
            Ok(None)
        } else {
            Ok(Some(parent))
        }
    } else {
        Ok(None)
    }
}

/// Helper to set the parent of a branch in refs (for testing)
#[allow(dead_code)]
pub fn set_parent_in_refs(dir: &Path, branch: &str, parent: &str) -> Result<()> {
    // Create a blob with the parent name by writing to a temp file
    let temp_file = dir.join(".git/diamond/temp_parent");
    fs::create_dir_all(dir.join(".git/diamond"))?;
    fs::write(&temp_file, parent)?;
    let output = run_git(dir, &["hash-object", "-w", temp_file.to_str().unwrap()])?;
    fs::remove_file(temp_file)?;
    let blob_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Update the ref
    run_git(
        dir,
        &["update-ref", &format!("refs/diamond/parent/{}", branch), &blob_hash],
    )?;
    Ok(())
}

/// Helper to remove tracking for a branch in refs (for testing)
#[allow(dead_code)]
pub fn remove_branch_tracking(dir: &Path, branch: &str) -> Result<()> {
    run_git(dir, &["update-ref", "-d", &format!("refs/diamond/parent/{}", branch)])?;
    Ok(())
}
