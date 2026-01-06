use crate::commands::restack;
use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use anyhow::{Context, Result};

/// Modify current branch (stage and commit/amend)
///
/// Behavior:
/// - If -a: stages all changes
/// - If -u: stages only updates to already-tracked files (like git add -u)
/// - If -c: create new commit (uses -m message or opens editor)
/// - If -e: open editor to edit commit message
/// - If -m without -c: amend with new message
/// - If --reset-author: reset the commit author to current user
/// - If -i/--interactive-rebase: open interactive rebase from parent
/// - If --into <branch>: amend changes into a downstack branch instead of current
/// - If no -m and no -c: amend existing commit preserving its message
///
/// After amending, automatically restacks any child branches
#[allow(clippy::too_many_arguments)]
pub fn run(
    all: bool,
    update: bool,
    message: Option<String>,
    force_commit: bool,
    edit: bool,
    reset_author: bool,
    interactive_rebase: bool,
    into: Option<String>,
    patch: bool,
) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;

    // Silent cleanup of orphaned refs (handles branches deleted via git/IDE)
    if let Err(_e) = crate::validation::silent_cleanup_orphaned_refs(&gateway) {}

    // Validate mutually exclusive flags
    if all && update {
        anyhow::bail!("Cannot use both -a (all) and -u (update) flags together");
    }
    if patch && (all || update) {
        anyhow::bail!("Cannot use --patch with -a/--all or -u/--update (patch mode is interactive)");
    }

    let current_branch = gateway.get_current_branch_name()?;

    // Check if we're on trunk - cannot modify trunk directly
    let trunk = ref_store.get_trunk()?;
    if Some(&current_branch) == trunk.as_ref() && into.is_none() {
        anyhow::bail!(
            "Cannot perform this operation on the trunk branch '{}'.\n\
            Create a new branch with '{} create <name>' first.",
            current_branch,
            program_name()
        );
    }

    // Validate current branch's parent exists (for non-trunk branches)
    if Some(&current_branch) != trunk.as_ref() {
        if let Some(parent) = ref_store.get_parent(&current_branch)? {
            gateway
                .validate_parent_exists(&parent)
                .context("Cannot modify branch with deleted parent")?;
        }
    }

    // Handle --into flag: amend changes into a downstack branch
    if let Some(ref target_branch) = into {
        return run_into(
            &gateway,
            &ref_store,
            &current_branch,
            target_branch,
            all,
            update,
            message,
            patch,
        );
    }

    // Check if current branch is frozen
    if ref_store.is_frozen(&current_branch)? {
        anyhow::bail!(
            "Branch '{}' is frozen. Use '{} unfreeze' to allow modifications.",
            current_branch,
            program_name()
        );
    }

    // Handle interactive rebase first (mutually exclusive with other operations)
    if interactive_rebase {
        let parent = ref_store
            .get_parent(&current_branch)?
            .ok_or_else(|| anyhow::anyhow!("Branch '{}' has no parent. Cannot interactive rebase.", current_branch))?;

        println!("Opening interactive rebase from '{}'...", parent);
        gateway.interactive_rebase(&parent)?;
        println!("Interactive rebase complete. Restacking children...");
        restack::restack_children(&current_branch)?;
        return Ok(());
    }

    // Stage changes based on flags
    if all {
        gateway.stage_all()?;
        println!("Staged all changes");
    } else if update {
        gateway.stage_updates()?;
        println!("Staged tracked file updates");
    } else if patch {
        gateway.stage_patch()?;
        println!("Staged selected hunks");
    }

    // Handle --reset-author
    if reset_author {
        gateway.amend_reset_author(message.as_deref())?;
        println!("Reset author on commit");
        gateway.show_commit_diffstat()?;
        restack::restack_children(&current_branch)?;
        return Ok(());
    }

    // Handle --edit (open editor for commit message)
    if edit {
        gateway.amend_with_editor()?;
        println!("Amended commit with edited message");
        gateway.show_commit_diffstat()?;
        restack::restack_children(&current_branch)?;
        return Ok(());
    }

    // Determine action based on flags
    // -c flag controls commit vs amend, -m just provides the message
    let did_amend = match (force_commit, message) {
        (true, Some(msg)) => {
            // -c with -m: create new commit with message
            gateway.commit(&msg)?;
            println!("Committed: {}", msg);
            gateway.show_commit_diffstat()?;
            false
        }
        (true, None) => {
            // -c without -m: create new commit using editor
            gateway.commit_with_editor()?;
            println!("Created new commit");
            gateway.show_commit_diffstat()?;
            false
        }
        (false, Some(msg)) => {
            // -m without -c: amend with new message
            gateway.amend_commit(Some(&msg))?;
            println!("Amended commit: {}", msg);
            gateway.show_commit_diffstat()?;
            restack::restack_children(&current_branch)?;
            true
        }
        (false, None) => {
            // No -c, no -m: amend preserving existing message
            gateway.amend_commit(None)?;
            println!("Amended commit (message preserved)");
            gateway.show_commit_diffstat()?;
            restack::restack_children(&current_branch)?;
            true
        }
    };

    // Note: restack is now done inside the amend branches above
    let _ = did_amend; // suppress unused warning

    Ok(())
}

/// Handle --into flag: amend changes into a downstack branch
#[allow(clippy::too_many_arguments)]
fn run_into(
    gateway: &GitGateway,
    ref_store: &RefStore,
    current_branch: &str,
    target_branch: &str,
    all: bool,
    update: bool,
    message: Option<String>,
    patch: bool,
) -> Result<()> {
    // Verify target branch exists
    if !gateway.branch_exists(target_branch)? {
        anyhow::bail!("Branch '{}' does not exist", target_branch);
    }

    // Verify target branch is in the downstack (is an ancestor of current branch)
    if !is_in_downstack(ref_store, current_branch, target_branch)? {
        anyhow::bail!(
            "Branch '{}' is not in the downstack of '{}'. Can only modify ancestor branches.",
            target_branch,
            current_branch
        );
    }

    // Check if target branch is frozen
    if ref_store.is_frozen(target_branch)? {
        anyhow::bail!(
            "Branch '{}' is frozen. Use '{} unfreeze' to allow modifications.",
            target_branch,
            program_name()
        );
    }

    // Switch to target branch first (worktree changes are carried over)
    println!("Switching to '{}' to apply changes...", target_branch);
    gateway.checkout_branch_worktree_safe(target_branch)?;

    // Stage changes AFTER switching branches (so they apply to target)
    if all {
        gateway.stage_all()?;
        println!("Staged all changes");
    } else if update {
        gateway.stage_updates()?;
        println!("Staged tracked file updates");
    } else if patch {
        gateway.stage_patch()?;
        println!("Staged selected hunks");
    }

    // Amend the commit
    match message {
        Some(msg) => {
            gateway.amend_commit(Some(&msg))?;
            println!("Amended commit on '{}': {}", target_branch, msg);
            gateway.show_commit_diffstat()?;
        }
        None => {
            gateway.amend_commit(None)?;
            println!("Amended commit on '{}' (message preserved)", target_branch);
            gateway.show_commit_diffstat()?;
        }
    }

    // Restack from target branch to update all descendants
    restack::restack_children(target_branch)?;

    // Return to original branch
    gateway.checkout_branch_worktree_safe(current_branch)?;
    println!("Returned to '{}'", current_branch);

    Ok(())
}

/// Check if target_branch is in the downstack (ancestors) of current_branch
fn is_in_downstack(ref_store: &RefStore, current_branch: &str, target_branch: &str) -> Result<bool> {
    let trunk = ref_store.get_trunk()?;

    // Walk up the parent chain from current branch with cycle detection
    let mut branch = current_branch.to_string();
    let mut seen = std::collections::HashSet::new();
    seen.insert(branch.clone());

    while let Some(parent) = ref_store.get_parent(&branch)? {
        if parent == target_branch {
            return Ok(true);
        }
        // Cycle detection
        if !seen.insert(parent.clone()) {
            anyhow::bail!(
                "Circular parent reference detected at '{}'. Run 'dm cleanup' to repair metadata.",
                parent
            );
        }
        // Stop at trunk
        if Some(parent.clone()) == trunk {
            // Check if target is trunk
            return Ok(target_branch == parent);
        }
        branch = parent;
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_gateway::GitGateway;
    use crate::ref_store::RefStore;

    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    use crate::test_context::{init_test_repo_with_branch, TestRepoContext};

    #[test]
    fn test_modify_without_message_amends() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create initial file and commit
        fs::write(dir.path().join("file1.txt"), "initial")?;
        gateway.stage_all()?;
        gateway.commit("Original message")?;

        // Stage another file
        fs::write(dir.path().join("file2.txt"), "new")?;
        {
            let mut index = repo.index()?;
            index.add_path(Path::new("file2.txt"))?;
            index.write()?;
        }

        // Modify without -a and without message (should amend)
        run(false, false, None, false, false, false, false, None, false)?;

        // Verify message was preserved
        let head = repo.head()?.peel_to_commit()?;
        assert_eq!(head.message(), Some("Original message"));

        Ok(())
    }

    #[test]
    fn test_modify_with_message_amends() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create initial commit
        fs::write(dir.path().join("file1.txt"), "initial")?;
        gateway.stage_all()?;
        gateway.commit("Original message")?;

        let original_head = repo.head()?.peel_to_commit()?.id();

        // Create a new file
        fs::write(dir.path().join("test.txt"), "test content")?;

        // Modify with -am (should AMEND with new message, not create new commit)
        run(
            true,
            false,
            Some("Updated message".to_string()),
            false,
            false,
            false,
            false,
            None,
            false,
        )?;

        // Verify commit was amended (same parent, different hash)
        let head = repo.head()?.peel_to_commit()?;
        assert_eq!(head.message(), Some("Updated message"));
        // The commit hash should be different (amended)
        assert_ne!(head.id(), original_head);
        // But the parent count should be 1 (still on top of Initial commit)
        assert_eq!(head.parent_count(), 1);

        Ok(())
    }

    #[test]
    fn test_modify_with_commit_flag_creates_new() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create initial commit
        fs::write(dir.path().join("file1.txt"), "initial")?;
        gateway.stage_all()?;
        gateway.commit("First commit")?;

        // Create a new file
        fs::write(dir.path().join("test.txt"), "test content")?;

        // Modify with -c -m (should CREATE new commit)
        run(
            true,
            false,
            Some("Second commit".to_string()),
            true,
            false,
            false,
            false,
            None,
            false,
        )?;

        // Verify a new commit was created (2 commits total after Initial)
        let head = repo.head()?.peel_to_commit()?;
        assert_eq!(head.message(), Some("Second commit"));
        // Should have parent (First commit)
        assert_eq!(head.parent_count(), 1);
        let parent = head.parent(0)?;
        assert_eq!(parent.message(), Some("First commit"));

        Ok(())
    }

    #[test]
    fn test_modify_stages_all() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create multiple files
        fs::write(dir.path().join("file1.txt"), "content1")?;
        fs::write(dir.path().join("file2.txt"), "content2")?;

        // Modify with -am
        run(
            true,
            false,
            Some("Multiple files".to_string()),
            false,
            false,
            false,
            false,
            None,
            false,
        )?;

        // Verify both files are in commit
        let head = repo.head()?.peel_to_commit()?;
        let tree = head.tree()?;
        assert!(tree.get_name("file1.txt").is_some());
        assert!(tree.get_name("file2.txt").is_some());

        Ok(())
    }

    #[test]
    fn test_modify_with_message_amends_not_creates() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create initial commit on branch
        fs::write(dir.path().join("file1.txt"), "content1")?;
        gateway.stage_all()?;
        gateway.commit("First commit")?;

        // Count commits before
        let head_before = repo.head()?.peel_to_commit()?;
        let parent_before = head_before.parent(0)?; // Initial commit from test setup

        // Add more content and amend with -m (no -c)
        fs::write(dir.path().join("file2.txt"), "content2")?;
        run(
            true,
            false,
            Some("Amended message".to_string()),
            false,
            false,
            false,
            false,
            None,
            false,
        )?;

        // Verify we AMENDED (not created new) - should still have same parent
        let head_after = repo.head()?.peel_to_commit()?;
        assert_eq!(head_after.message(), Some("Amended message"));
        assert_eq!(head_after.parent_count(), 1);
        // Parent should be the same Initial commit
        assert_eq!(head_after.parent(0)?.id(), parent_before.id());

        Ok(())
    }

    #[test]
    fn test_modify_a_without_message_amends_preserving_message() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create initial file and commit
        fs::write(dir.path().join("file1.txt"), "initial content")?;
        gateway.stage_all()?;
        gateway.commit("Original commit message")?;

        // Make changes
        fs::write(dir.path().join("file2.txt"), "new content")?;

        // Modify with -a but no message (should amend and preserve message)
        run(true, false, None, false, false, false, false, None, false)?;

        // Verify message was preserved
        let head = repo.head()?.peel_to_commit()?;
        assert_eq!(head.message(), Some("Original commit message"));

        // Verify both files are now in the amended commit
        let tree = head.tree()?;
        assert!(tree.get_name("file1.txt").is_some());
        assert!(tree.get_name("file2.txt").is_some());

        Ok(())
    }

    #[test]
    fn test_modify_frozen_branch_fails() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        // Initialize dm and create a tracked feature branch
        let ref_store = RefStore::from_path(dir.path())?;
        ref_store.set_trunk("main")?;

        // Create and checkout feature branch
        let head = repo.head()?.peel_to_commit()?;
        repo.branch("feature", &head, false)?;
        repo.set_head("refs/heads/feature")?;
        ref_store.set_parent("feature", "main")?;

        // Freeze the branch
        ref_store.set_frozen("feature", true)?;

        // Make changes
        fs::write(dir.path().join("file.txt"), "content")?;

        // Try to modify - should fail
        let result = run(
            true,
            false,
            Some("Should fail".to_string()),
            false,
            false,
            false,
            false,
            None,
            false,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("frozen"), "Error should mention frozen: {}", err_msg);
        assert!(
            err_msg.contains("unfreeze"),
            "Error should mention unfreeze: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_modify_into_downstack_branch() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Initialize dm
        ref_store.set_trunk("main")?;

        // Create parent branch with a commit
        gateway.create_branch("parent")?;
        ref_store.set_parent("parent", "main")?;
        fs::write(dir.path().join("parent.txt"), "parent content")?;
        gateway.stage_all()?;
        gateway.commit("Parent commit")?;

        // Create child branch with a commit
        gateway.create_branch("child")?;
        ref_store.set_parent("child", "parent")?;
        fs::write(dir.path().join("child.txt"), "child content")?;
        gateway.stage_all()?;
        gateway.commit("Child commit")?;

        // We're on child, modify into parent
        fs::write(dir.path().join("fix.txt"), "fix for parent")?;
        run(
            true,
            false,
            Some("Fixed parent".to_string()),
            false,
            false,
            false,
            false,
            Some("parent".to_string()),
            false,
        )?;

        // Verify we're back on child
        assert_eq!(gateway.get_current_branch_name()?, "child");

        // Verify parent commit was amended
        gateway.checkout_branch_worktree_safe("parent")?;
        let repo = git2::Repository::open(dir.path())?;
        let head = repo.head()?.peel_to_commit()?;
        assert_eq!(head.message(), Some("Fixed parent"));

        // Verify fix.txt is in parent's tree
        let tree = head.tree()?;
        assert!(tree.get_name("fix.txt").is_some());

        Ok(())
    }

    #[test]
    fn test_modify_into_nonexistent_branch_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Initialize dm
        ref_store.set_trunk("main")?;

        // Create a branch
        gateway.create_branch("feature")?;
        ref_store.set_parent("feature", "main")?;

        // Try to modify into nonexistent branch
        let result = run(
            false,
            false,
            Some("Test".to_string()),
            false,
            false,
            false,
            false,
            Some("nonexistent".to_string()),
            false,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));

        Ok(())
    }

    #[test]
    fn test_modify_into_non_downstack_branch_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Initialize dm
        ref_store.set_trunk("main")?;

        // Create two parallel branches from main
        gateway.create_branch("branch-a")?;
        ref_store.set_parent("branch-a", "main")?;

        gateway.checkout_branch_worktree_safe("main")?;
        gateway.create_branch("branch-b")?;
        ref_store.set_parent("branch-b", "main")?;

        // From branch-b, try to modify into branch-a (not in downstack)
        let result = run(
            false,
            false,
            Some("Test".to_string()),
            false,
            false,
            false,
            false,
            Some("branch-a".to_string()),
            false,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not in the downstack"),
            "Expected error about downstack, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_is_in_downstack_detects_cycle() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Initialize dm
        ref_store.set_trunk("main")?;

        // Create two branches
        gateway.create_branch("branch-a")?;
        gateway.checkout_branch_worktree_safe("main")?;
        gateway.create_branch("branch-b")?;

        // Create a cycle in metadata: branch-a -> branch-b -> branch-a
        ref_store.set_parent("branch-a", "branch-b")?;
        ref_store.set_parent("branch-b", "branch-a")?;

        // is_in_downstack should detect the cycle and error
        let result = is_in_downstack(&ref_store, "branch-a", "main");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Circular parent reference"),
            "Expected circular reference error, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_modify_into_frozen_branch_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Initialize dm
        ref_store.set_trunk("main")?;

        // Create parent branch
        gateway.create_branch("parent")?;
        ref_store.set_parent("parent", "main")?;
        fs::write(dir.path().join("parent.txt"), "content")?;
        gateway.stage_all()?;
        gateway.commit("Parent commit")?;

        // Create child branch
        gateway.create_branch("child")?;
        ref_store.set_parent("child", "parent")?;
        fs::write(dir.path().join("child.txt"), "content")?;
        gateway.stage_all()?;
        gateway.commit("Child commit")?;

        // Freeze the parent branch
        ref_store.set_frozen("parent", true)?;

        // Try to modify into frozen parent
        fs::write(dir.path().join("fix.txt"), "fix")?;
        let result = run(
            true,
            false,
            Some("Fix".to_string()),
            false,
            false,
            false,
            false,
            Some("parent".to_string()),
            false,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("frozen"), "Expected frozen error, got: {}", err_msg);

        Ok(())
    }

    #[test]
    fn test_modify_with_update_flag_only_stages_tracked() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Initialize dm
        ref_store.set_trunk("main")?;

        // Create and track a feature branch
        gateway.create_branch("feature")?;
        ref_store.set_parent("feature", "main")?;

        // Create and commit a tracked file
        fs::write(dir.path().join("tracked.txt"), "initial")?;
        gateway.stage_all()?;
        gateway.commit("Add tracked file")?;

        // Modify the tracked file
        fs::write(dir.path().join("tracked.txt"), "modified")?;

        // Create a new untracked file
        fs::write(dir.path().join("untracked.txt"), "new")?;

        // Modify with -u flag (should only stage tracked file updates)
        run(
            false,
            true,
            Some("Update tracked".to_string()),
            false,
            false,
            false,
            false,
            None,
            false,
        )?;

        // Verify the commit only has tracked.txt
        let head = repo.head()?.peel_to_commit()?;
        let tree = head.tree()?;
        assert!(tree.get_name("tracked.txt").is_some());
        // untracked.txt should NOT be in the commit
        assert!(tree.get_name("untracked.txt").is_none());

        Ok(())
    }

    #[test]
    fn test_modify_all_and_update_mutually_exclusive() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        // Try to use both -a and -u
        let result = run(
            true, // all
            true, // update
            Some("Test".to_string()),
            false,
            false,
            false,
            false,
            None,
            false,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("-a") && err_msg.contains("-u"),
            "Expected error about mutually exclusive flags, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_modify_on_trunk_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Initialize dm with trunk
        ref_store.set_trunk("main")?;

        // Make changes on trunk
        fs::write(dir.path().join("test.txt"), "content")?;

        // Try to modify on trunk - should fail
        let result = run(
            true,
            false,
            Some("Should fail".to_string()),
            false,
            false,
            false,
            false,
            None,
            false,
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("trunk") || err_msg.contains("Cannot"),
            "Expected error about trunk, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_modify_auto_repairs_when_parent_deleted() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set trunk
        ref_store.set_trunk("main")?;

        // Create stack: main -> A -> B
        gateway.create_branch("A")?;
        ref_store.set_parent("A", "main")?;
        gateway.checkout_branch_worktree_safe("A")?;
        fs::write(dir.path().join("a.txt"), "a")?;
        gateway.stage_all()?;
        gateway.commit("A commit")?;

        gateway.create_branch("B")?;
        ref_store.set_parent("B", "A")?;
        gateway.checkout_branch_worktree_safe("B")?;
        fs::write(dir.path().join("b.txt"), "b")?;
        gateway.stage_all()?;
        gateway.commit("B commit")?;

        // Delete A using git directly (bypassing Diamond)
        gateway.checkout_branch_worktree_safe("main")?;
        repo.find_branch("A", git2::BranchType::Local)?.delete()?;

        // Checkout B again
        gateway.checkout_branch_worktree_safe("B")?;

        // Make a change
        fs::write(dir.path().join("change.txt"), "change")?;

        // Modify B - should succeed because modify auto-repairs orphaned metadata
        // The silent_cleanup_orphaned_refs call reparents B to trunk when A is missing
        let result = run(
            true,  // all
            false, // update
            Some("Update B".to_string()),
            false, // force_commit
            false, // edit
            false, // reset_author
            false, // interactive_rebase
            None,  // into
            false, // patch
        );

        assert!(
            result.is_ok(),
            "Modify should succeed after auto-repair, got: {:?}",
            result
        );

        // B should now be reparented to trunk (main) since A was deleted
        let new_parent = ref_store.get_parent("B")?;
        assert_eq!(
            new_parent,
            Some("main".to_string()),
            "B should be reparented to trunk after A was deleted"
        );

        Ok(())
    }

    #[test]
    fn test_modify_succeeds_when_parent_exists() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo_with_branch(dir.path(), "main")?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set trunk
        ref_store.set_trunk("main")?;

        // Create stack: main -> A -> B
        gateway.create_branch("A")?;
        ref_store.set_parent("A", "main")?;
        gateway.checkout_branch_worktree_safe("A")?;
        fs::write(dir.path().join("a.txt"), "a")?;
        gateway.stage_all()?;
        gateway.commit("A commit")?;

        gateway.create_branch("B")?;
        ref_store.set_parent("B", "A")?;
        gateway.checkout_branch_worktree_safe("B")?;
        fs::write(dir.path().join("b.txt"), "b")?;
        gateway.stage_all()?;
        gateway.commit("B commit")?;

        // Make a change
        fs::write(dir.path().join("change.txt"), "change")?;

        // Modify B - should succeed since A (parent) exists
        let result = run(
            true,  // all
            false, // update
            Some("Update B".to_string()),
            false, // force_commit
            false, // edit
            false, // reset_author
            false, // interactive_rebase
            None,  // into
            false, // patch
        );

        assert!(result.is_ok());

        Ok(())
    }
}
