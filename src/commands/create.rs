use crate::cache::Cache;
use crate::config::Config;
use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::stack_viz::is_dangerous_branch_name;
use crate::state::{acquire_operation_lock, OperationState};
use crate::worktree;
use anyhow::{Context, Result};
use colored::Colorize;

/// Slugify a commit message into a valid branch name component.
/// Does NOT add date prefix - that's handled by Config::format_branch_name().
fn slugify_name(message: &str) -> String {
    message
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join("_")
}

pub fn run(
    name: Option<String>,
    all: bool,
    update: bool,
    message: Option<String>,
    insert: Option<String>,
) -> Result<()> {
    // Acquire operation lock to prevent race conditions with concurrent sync/restack.
    // This is especially important for --insert which modifies refs and rebases.
    let _lock = acquire_operation_lock()?;

    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load config ({}), using defaults", e);
        Config {
            branch: Default::default(),
            remote: "origin".to_string(),
            merge: Default::default(),
        }
    });

    // Validate mutually exclusive flags
    if all && update {
        anyhow::bail!("Cannot use both -a (all) and -u (update) flags together");
    }

    // 1. Determine raw branch name (explicit or auto-generated from message)
    let raw_name = match (&name, &message) {
        (Some(n), _) => n.clone(),
        (None, Some(msg)) => slugify_name(msg),
        (None, None) => anyhow::bail!("Must provide either a branch name or a commit message to generate the name"),
    };

    // 2. Apply branch name formatting from config (prefix, date, etc.)
    // Only apply formatting when auto-generating from message, not for explicit names
    let branch_name = if name.is_some() {
        raw_name // Use explicit name as-is
    } else {
        config.format_branch_name(&raw_name)
    };

    // 3. Validate branch name is safe for PR descriptions
    if is_dangerous_branch_name(&branch_name) {
        anyhow::bail!(
            "Branch name '{}' contains dangerous patterns that could enable injection attacks.\n\
            Patterns like '](http', '](javascript', '```', '<!--', and '-->' are not allowed.\n\
            Please use a simpler branch name.",
            branch_name
        );
    }

    // 4. Check if branch already exists
    if gateway.branch_exists(&branch_name)? {
        anyhow::bail!("Branch '{}' already exists", branch_name);
    }

    // 5. Determine parent (current branch)
    let parent = gateway
        .get_current_branch_name()
        .context("Could not determine current branch to use as parent")?;

    // Validate parent exists in git (could have been deleted remotely)
    gateway
        .validate_parent_exists(&parent)
        .context("Cannot create branch with deleted parent")?;

    // Also validate that the parent's own parent exists (for non-trunk branches)
    // This prevents creating branches in a broken stack
    let trunk = ref_store.get_trunk()?;
    if Some(&parent) != trunk.as_ref() {
        if let Some(grandparent) = ref_store.get_parent(&parent)? {
            gateway
                .validate_parent_exists(&grandparent)
                .context("Cannot create branch - current branch's parent has been deleted")?;
        }
    }

    // 6. If inserting, determine the child branch(es)
    // - If insert is Some(""), auto-detect child from parent's children
    // - If insert is Some(name), use that specific child
    let insert_target: Option<String> = if let Some(ref child) = insert {
        if child.is_empty() {
            // Auto-detect: get children of current branch
            let children = ref_store.get_children(&parent)?;
            if children.is_empty() {
                anyhow::bail!(
                    "Cannot use --insert: branch '{}' has no children.\n\
                    Use --insert <child> to specify a child branch explicitly.",
                    parent
                );
            } else if children.len() > 1 {
                let mut child_list: Vec<_> = children.into_iter().collect();
                child_list.sort();
                anyhow::bail!(
                    "Cannot use --insert: branch '{}' has multiple children: {}\n\
                    Use --insert <child> to specify which child to reparent.",
                    parent,
                    child_list.join(", ")
                );
            } else {
                // Exactly one child - use it
                Some(children.into_iter().next().unwrap())
            }
        } else {
            // Explicit child specified - validate it
            if !gateway.branch_exists(child)? {
                anyhow::bail!("Child branch '{}' does not exist", child);
            }

            // Verify the child is actually a child of the current branch
            if let Some(child_parent) = ref_store.get_parent(child)? {
                if child_parent != parent {
                    anyhow::bail!(
                        "Branch '{}' is not a child of '{}' (it's a child of {})",
                        child,
                        parent,
                        child_parent
                    );
                }
            } else {
                anyhow::bail!(
                    "Branch '{}' is not tracked. Track it first with '{} track'.",
                    child,
                    program_name()
                );
            }
            Some(child.clone())
        }
    } else {
        None
    };

    // 7. Create and checkout the new branch
    println!("Creating branch '{}' from '{}'...", branch_name.green(), parent.blue());
    gateway.create_branch(&branch_name)?;

    // 8. Update Stack Metadata
    ref_store.set_parent(&branch_name, &parent)?;

    // 9. Handle -a/-u and/or -m flags independently
    if all {
        gateway.stage_all()?;
        println!("Staged all changes");
    } else if update {
        gateway.stage_updates()?;
        println!("Staged tracked file updates");
    }

    if let Some(msg) = message {
        gateway.commit(&msg)?;
        println!("Committed: {}", msg);
    }

    // 10. Update base_sha to track this branch's initial state (in cache only)
    {
        let branch_sha = gateway.get_branch_sha(&branch_name)?;
        let mut cache = Cache::load()?;
        cache.set_base_sha(&branch_name, &branch_sha);
        cache.save()?;
    }

    // 11. If inserting, re-parent the child and rebase it
    if let Some(child) = insert_target {
        // Check for worktree conflicts before rebasing
        worktree::check_branches_for_worktree_conflicts(std::slice::from_ref(&child))?;

        println!("Inserting between '{}' and '{}'...", parent.blue(), child.green());

        // Save operation state BEFORE modifying anything.
        // This enables `dm abort` to rollback if the user runs `git rebase --abort`.
        let insert_state = OperationState::new_insert(
            branch_name.clone(),
            child.clone(),
            parent.clone(), // original parent of the child
        );
        insert_state.save()?;

        // Update metadata: change child's parent to the new branch
        ref_store.reparent(&child, &branch_name)?;

        // Rebase the child onto the new branch
        let outcome = gateway.rebase_onto(&child, &branch_name)?;
        if outcome.has_conflicts() {
            println!(
                "\n{} Conflicts while rebasing '{}' onto '{}'. Resolve and run '{} continue'.",
                "!".yellow().bold(),
                child,
                branch_name,
                program_name()
            );
            return Ok(());
        }

        // Return to the new branch after rebasing
        gateway.checkout_branch(&branch_name)?;

        // Clear operation state on success
        OperationState::clear()?;

        println!(
            "{} Inserted '{}' between '{}' and '{}'",
            "✓".green().bold(),
            branch_name,
            parent,
            child
        );
    } else {
        println!("{} Checked out branch '{}'", "Success:".green().bold(), branch_name);
        println!("Stack: {} -> {}", parent, branch_name);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::ref_store::RefStore;
    use anyhow::Result;

    use std::fs;
    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_create_branch_success() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create a new branch
        run(Some("feature-1".to_string()), false, false, None, None)?;

        // Verify branch exists in git
        assert!(gateway.branch_exists("feature-1")?);

        // Verify current branch is the new one
        assert_eq!(gateway.get_current_branch_name()?, "feature-1");

        // Verify stack metadata in refs
        let ref_store = RefStore::new()?;
        assert!(ref_store.is_tracked("feature-1")?);

        Ok(())
    }

    #[test]
    fn test_create_branch_records_parent() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create first branch
        run(Some("feature-1".to_string()), false, false, None, None)?;

        // Create second branch from first
        run(Some("feature-2".to_string()), false, false, None, None)?;

        // Verify parent relationship via refs
        let ref_store = RefStore::new()?;
        let parent = ref_store.get_parent("feature-2")?;
        assert_eq!(parent, Some("feature-1".to_string()));

        // Verify parent has child
        let children = ref_store.get_children("feature-1")?;
        assert!(children.contains("feature-2"));

        Ok(())
    }

    #[test]
    fn test_create_duplicate_branch_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create a branch
        run(Some("duplicate".to_string()), false, false, None, None)?;

        // Try to create it again
        let result = run(Some("duplicate".to_string()), false, false, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));

        Ok(())
    }

    #[test]
    fn test_create_branch_from_main() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Get initial branch name
        let initial_branch = gateway.get_current_branch_name()?;

        // Create feature from main/master
        run(Some("feature".to_string()), false, false, None, None)?;

        // Verify parent is initial branch via refs
        let ref_store = RefStore::new()?;
        let parent = ref_store.get_parent("feature")?;
        assert_eq!(parent, Some(initial_branch));

        Ok(())
    }

    #[test]
    fn test_create_stack_of_three() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create chain: main -> feature-1 -> feature-2 -> feature-3
        run(Some("feature-1".to_string()), false, false, None, None)?;
        run(Some("feature-2".to_string()), false, false, None, None)?;
        run(Some("feature-3".to_string()), false, false, None, None)?;

        // Verify full chain via refs
        let ref_store = RefStore::new()?;

        assert_eq!(ref_store.get_parent("feature-3")?, Some("feature-2".to_string()));

        assert_eq!(ref_store.get_parent("feature-2")?, Some("feature-1".to_string()));
        assert!(ref_store.get_children("feature-2")?.contains("feature-3"));

        assert!(ref_store.get_children("feature-1")?.contains("feature-2"));

        Ok(())
    }

    #[test]
    fn test_create_branch_with_special_name() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create branch with special characters
        run(Some("feature/sub-branch_v2".to_string()), false, false, None, None)?;

        // Verify it was created
        assert!(gateway.branch_exists("feature/sub-branch_v2")?);

        Ok(())
    }

    #[test]
    fn test_create_preserves_existing_metadata() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create initial structure
        run(Some("feature-1".to_string()), false, false, None, None)?;

        // Add metadata to feature-1 via cache
        let mut cache = Cache::load()?;
        cache.set_pr_url("feature-1", "https://github.com/org/repo/pull/1");
        cache.save()?;

        // Create child branch
        run(Some("feature-2".to_string()), false, false, None, None)?;

        // Verify original metadata preserved in cache
        let cache = Cache::load()?;
        assert_eq!(
            cache.get_pr_url("feature-1"),
            Some("https://github.com/org/repo/pull/1")
        );

        Ok(())
    }

    #[test]
    fn test_create_with_a_without_m_just_stages() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create a file
        fs::write(dir.path().join("test.txt"), "content")?;

        // Create with -a but no -m (should stage but not commit)
        run(Some("feature".to_string()), true, false, None, None)?;

        // Verify branch exists
        assert!(gateway.branch_exists("feature")?);
        assert_eq!(gateway.get_current_branch_name()?, "feature");

        Ok(())
    }

    #[test]
    fn test_create_with_m_without_a_commits_staged() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create and stage a file manually
        fs::write(dir.path().join("staged.txt"), "staged")?;
        {
            let mut index = repo.index()?;
            index.add_path(std::path::Path::new("staged.txt"))?;
            index.write()?;
        }

        // Create another file but don't stage it
        fs::write(dir.path().join("unstaged.txt"), "unstaged")?;

        // Create with -m but no -a (should commit only staged files)
        run(
            Some("feature".to_string()),
            false,
            false,
            Some("Test commit".to_string()),
            None,
        )?;

        // Verify branch and commit
        assert_eq!(gateway.get_current_branch_name()?, "feature");
        let head = repo.head()?.peel_to_commit()?;
        assert_eq!(head.message(), Some("Test commit"));

        // Verify only staged file is committed
        let tree = head.tree()?;
        assert!(tree.get_name("staged.txt").is_some());
        assert!(tree.get_name("unstaged.txt").is_none());

        Ok(())
    }

    #[test]
    fn test_create_with_am_stages_and_commits() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create a file to stage
        fs::write(dir.path().join("test.txt"), "test content")?;

        // Create branch with -am
        run(
            Some("feature".to_string()),
            true,
            false,
            Some("Test commit".to_string()),
            None,
        )?;

        // Verify we're on the feature branch
        assert_eq!(gateway.get_current_branch_name()?, "feature");

        // Verify the commit was made
        let head = repo.head()?.peel_to_commit()?;
        assert_eq!(head.message(), Some("Test commit"));

        Ok(())
    }

    #[test]
    fn test_create_autogenerate_name_from_message() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create a file
        fs::write(dir.path().join("test.txt"), "content")?;

        // Create branch with message but no name (should auto-generate)
        run(None, true, false, Some("Add new feature".to_string()), None)?;

        // Verify branch was created with slugified name (MM-DD-message_with_underscores)
        let current_branch = gateway.get_current_branch_name()?;
        // Check for pattern: XX-XX-add_new_feature (date prefix + underscored message)
        assert!(
            current_branch.ends_with("-add_new_feature"),
            "Expected branch to end with '-add_new_feature', got: {}",
            current_branch
        );
        // Verify it has the date prefix format (MM-DD-)
        let parts: Vec<&str> = current_branch.splitn(3, '-').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts[0].len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()));
        assert!(parts[1].len() == 2 && parts[1].chars().all(|c| c.is_ascii_digit()));

        Ok(())
    }

    #[test]
    fn test_create_autogenerate_name_with_special_chars() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create branch with special characters in message
        run(None, false, false, Some("Fix bug #123: URL parsing!".to_string()), None)?;

        // Verify branch name is slugified (MM-DD-message_with_underscores)
        let current_branch = gateway.get_current_branch_name()?;
        // Check for pattern: XX-XX-fix_bug_123_url_parsing
        assert!(
            current_branch.ends_with("-fix_bug_123_url_parsing"),
            "Expected branch to end with '-fix_bug_123_url_parsing', got: {}",
            current_branch
        );
        // Verify it has the date prefix format (MM-DD-)
        let parts: Vec<&str> = current_branch.splitn(3, '-').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts[0].len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()));
        assert!(parts[1].len() == 2 && parts[1].chars().all(|c| c.is_ascii_digit()));

        Ok(())
    }

    #[test]
    fn test_create_no_name_no_message_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create with neither name nor message should fail
        let result = run(None, false, false, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("branch name"));

        Ok(())
    }

    #[test]
    fn test_create_dangerous_branch_name_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // URL injection attempt
        let result = run(Some("branch](http://evil.com)".to_string()), false, false, None, None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("dangerous"),
            "Expected error about dangerous patterns, got: {}",
            err_msg
        );

        // Code block injection
        let result = run(Some("branch```code".to_string()), false, false, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dangerous"));

        // HTML comment injection
        let result = run(Some("branch<!--".to_string()), false, false, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dangerous"));

        Ok(())
    }

    #[test]
    fn test_create_insert_between_branches() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create initial stack: main -> feature-1
        run(Some("feature-1".to_string()), false, false, None, None)?;

        // Make a commit on feature-1 so we have something to rebase
        fs::write(dir.path().join("feature1.txt"), "feature 1 content")?;
        gateway.stage_all()?;
        gateway.commit("Feature 1 commit")?;

        // Go back to main and create a new branch that inserts between main and feature-1
        gateway.checkout_branch("main")?;

        // Insert between main and feature-1
        run(
            Some("new-middle".to_string()),
            false,
            false,
            None,
            Some("feature-1".to_string()),
        )?;

        // Verify the new structure via refs: main -> new-middle -> feature-1
        let ref_store = RefStore::new()?;

        // new-middle should have main as parent
        assert_eq!(ref_store.get_parent("new-middle")?, Some("main".to_string()));
        assert!(ref_store.get_children("new-middle")?.contains("feature-1"));

        // feature-1 should have new-middle as parent (was previously main)
        assert_eq!(ref_store.get_parent("feature-1")?, Some("new-middle".to_string()));

        // main should have new-middle as child (not feature-1 anymore)
        let main_children = ref_store.get_children("main")?;
        assert!(main_children.contains("new-middle"));
        assert!(!main_children.contains("feature-1"));

        // We should be back on new-middle
        let current = gateway.get_current_branch_name()?;
        assert_eq!(current, "new-middle");

        Ok(())
    }

    #[test]
    fn test_create_insert_nonexistent_child_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Try to insert with a non-existent child
        let result = run(
            Some("new-branch".to_string()),
            false,
            false,
            None,
            Some("nonexistent".to_string()),
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));

        Ok(())
    }

    #[test]
    fn test_create_insert_not_a_child_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create two branches from main
        run(Some("feature-1".to_string()), false, false, None, None)?;
        gateway.checkout_branch("main")?;
        run(Some("feature-2".to_string()), false, false, None, None)?;

        // Try to insert between feature-2 (current) and feature-1 (not a child of feature-2)
        let result = run(
            Some("new-branch".to_string()),
            false,
            false,
            None,
            Some("feature-1".to_string()),
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not a child"),
            "Expected error about not being a child, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_create_with_update_flag_only_stages_tracked() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create and commit a tracked file
        fs::write(dir.path().join("tracked.txt"), "initial")?;
        gateway.stage_all()?;
        gateway.commit("Add tracked file")?;

        // Modify the tracked file
        fs::write(dir.path().join("tracked.txt"), "modified")?;

        // Create a new untracked file
        fs::write(dir.path().join("untracked.txt"), "new")?;

        // Create with -u flag (should only stage tracked file updates)
        run(
            Some("feature".to_string()),
            false, // all
            true,  // update
            Some("Update tracked file".to_string()),
            None,
        )?;

        // Verify the commit only has tracked.txt changes
        let head = repo.head()?.peel_to_commit()?;
        let tree = head.tree()?;
        assert!(tree.get_name("tracked.txt").is_some());
        // untracked.txt should NOT be in the commit
        assert!(tree.get_name("untracked.txt").is_none());

        Ok(())
    }

    #[test]
    fn test_create_all_and_update_mutually_exclusive() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Try to use both -a and -u
        let result = run(
            Some("feature".to_string()),
            true, // all
            true, // update
            Some("Test".to_string()),
            None,
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
    fn test_create_insert_boolean_auto_detects_child() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;

        // Create initial stack: main -> feature-1
        run(Some("feature-1".to_string()), false, false, None, None)?;

        // Make a commit on feature-1
        fs::write(dir.path().join("feature1.txt"), "feature 1 content")?;
        gateway.stage_all()?;
        gateway.commit("Feature 1 commit")?;

        // Go back to main
        gateway.checkout_branch("main")?;

        // Insert with boolean --insert (auto-detect child)
        // This should insert between main and feature-1
        run(
            Some("new-middle".to_string()),
            false,
            false,
            None,
            Some("".to_string()), // Empty string indicates boolean flag usage
        )?;

        // Verify the new structure: main -> new-middle -> feature-1
        let ref_store = RefStore::new()?;

        // new-middle should have main as parent
        assert_eq!(ref_store.get_parent("new-middle")?, Some("main".to_string()));
        assert!(ref_store.get_children("new-middle")?.contains("feature-1"));

        // feature-1 should have new-middle as parent
        assert_eq!(ref_store.get_parent("feature-1")?, Some("new-middle".to_string()));

        Ok(())
    }

    #[test]
    fn test_create_insert_boolean_fails_with_no_children() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // On main with no children, --insert should fail
        let result = run(
            Some("new-branch".to_string()),
            false,
            false,
            None,
            Some("".to_string()), // Empty string indicates boolean flag usage
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("no children") || err_msg.contains("child"),
            "Expected error about no children, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_create_fails_when_current_branch_parent_deleted() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set trunk
        ref_store.set_trunk("main")?;

        // Create stack: main -> A -> B
        run(Some("A".to_string()), false, false, None, None)?;
        fs::write(dir.path().join("a.txt"), "a")?;
        gateway.stage_all()?;
        gateway.commit("A commit")?;

        run(Some("B".to_string()), false, false, None, None)?;
        fs::write(dir.path().join("b.txt"), "b")?;
        gateway.stage_all()?;
        gateway.commit("B commit")?;

        // Now we're on B, which has parent A
        // Delete A using git directly (bypassing Diamond)
        gateway.checkout_branch("main")?;
        repo.find_branch("A", git2::BranchType::Local)?.delete()?;

        // Checkout B again
        gateway.checkout_branch("B")?;

        // Try to create C from B (which has deleted parent A)
        // This should fail because B's parent (A) doesn't exist
        let result = run(Some("C".to_string()), false, false, None, None);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // Error message should mention the problem (parent deleted/doesn't exist)
        assert!(
            err_msg.contains("does not exist") || err_msg.contains("has been deleted") || err_msg.contains("parent"),
            "Expected error about parent not existing, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_create_succeeds_when_on_trunk() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Set trunk
        ref_store.set_trunk("main")?;

        // Create from trunk should always succeed (no parent to validate)
        let result = run(Some("A".to_string()), false, false, None, None);

        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_create_succeeds_when_parent_is_direct_child_of_trunk() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let gateway = GitGateway::new()?;
        let ref_store = RefStore::new()?;

        // Set trunk
        ref_store.set_trunk("main")?;

        // Create A from main
        run(Some("A".to_string()), false, false, None, None)?;
        fs::write(dir.path().join("a.txt"), "a")?;
        gateway.stage_all()?;
        gateway.commit("A commit")?;

        // Create B from A should succeed (A's parent is trunk, which always exists)
        let result = run(Some("B".to_string()), false, false, None, None);

        assert!(result.is_ok());

        Ok(())
    }
}
