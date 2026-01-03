use crate::cache::Cache;
use crate::forge::get_forge;
use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;
use crate::stack_viz::update_all_stack_visualizations;
use anyhow::Result;
use colored::Colorize;
use std::collections::HashSet;

/// Validation error types for RefStore
#[derive(Debug, Clone)]
pub enum DiagnosticError {
    /// Circular dependency detected in branch relationships
    Cycle(Vec<String>),
    /// Branch references a parent that doesn't exist in refs
    OrphanedParent { branch: String, parent: String },
    /// Trunk branch doesn't exist in git
    MissingTrunk(String),
    /// Branch is tracked but doesn't exist in git
    TrackedBranchMissing(String),
    /// Parent ref contains corrupted data (CRITICAL-8 detection)
    CorruptedRef { branch: String, error: String },
}

/// Run diagnostics on the stack metadata
pub fn run(fix: bool, fix_viz: bool) -> Result<()> {
    // Handle --fix-viz first (independent of regular diagnostics)
    if fix_viz {
        return run_fix_viz();
    }

    println!("{} Running diagnostics...\n", "🔍".blue());

    let ref_store = RefStore::new()?;
    let gateway = GitGateway::new()?;

    let errors = validate_refs(&ref_store, &gateway)?;

    if errors.is_empty() {
        println!("{} All checks passed!", "✓".green().bold());
        println!("\nStack metadata is healthy.");
        return Ok(());
    }

    // Display errors
    println!("{} Found {} issues:\n", "✗".red().bold(), errors.len());

    for (i, error) in errors.iter().enumerate() {
        match error {
            DiagnosticError::Cycle(cycle) => {
                println!("{}. {} Circular dependency detected:", i + 1, "⚠".yellow());
                println!("   {}", cycle.join(" → ").yellow());
            }
            DiagnosticError::OrphanedParent { branch, parent } => {
                println!(
                    "{}. {} Branch '{}' references non-existent parent '{}'",
                    i + 1,
                    "⚠".yellow(),
                    branch.cyan(),
                    parent.cyan()
                );
            }
            DiagnosticError::MissingTrunk(trunk) => {
                println!(
                    "{}. {} Trunk branch '{}' doesn't exist in git",
                    i + 1,
                    "⚠".yellow(),
                    trunk.cyan()
                );
            }
            DiagnosticError::TrackedBranchMissing(branch) => {
                println!(
                    "{}. {} Branch '{}' is tracked but doesn't exist in git",
                    i + 1,
                    "⚠".yellow(),
                    branch.cyan()
                );
            }
            DiagnosticError::CorruptedRef { branch, error } => {
                println!(
                    "{}. {} Branch '{}' has corrupted parent metadata",
                    i + 1,
                    "⚠".yellow(),
                    branch.cyan()
                );
                println!("   {}", error.dimmed());
            }
        }
    }

    if fix {
        println!("\n{} Attempting automatic repair...", "🔧".blue());
        let unfixed_count = attempt_fix(&ref_store, &gateway, &errors)?;

        if unfixed_count > 0 {
            anyhow::bail!(
                "{} issue{} could not be automatically fixed",
                unfixed_count,
                if unfixed_count == 1 { "" } else { "s" }
            );
        }
        Ok(())
    } else {
        println!(
            "\n{} Run '{} doctor --fix' to attempt automatic repair.",
            "💡".blue(),
            program_name()
        );
        anyhow::bail!(
            "{} issue{} found",
            errors.len(),
            if errors.len() == 1 { "" } else { "s" }
        );
    }
}

/// Validate refs for issues
fn validate_refs(ref_store: &RefStore, gateway: &GitGateway) -> Result<Vec<DiagnosticError>> {
    let mut errors = Vec::new();

    // Check trunk exists
    if let Some(trunk) = ref_store.get_trunk()? {
        if !gateway.branch_exists(&trunk)? {
            errors.push(DiagnosticError::MissingTrunk(trunk.clone()));
        }

        // Get all tracked branches - use list_tracked_branches to catch orphaned branches and cycles
        // (collect_branches_dfs only finds branches reachable from trunk)
        let all_branches = ref_store.list_tracked_branches()?;

        for branch in &all_branches {
            // Skip trunk
            if branch == &trunk {
                continue;
            }

            // Check branch exists in git
            if !gateway.branch_exists(branch)? {
                errors.push(DiagnosticError::TrackedBranchMissing(branch.clone()));
                continue;
            }

            // Check parent exists and is tracked
            // Use unchecked getter to allow inspection of corrupted refs
            match ref_store.get_parent_unchecked(branch)? {
                Some(parent) => {
                    // Validate parent name first (CRITICAL-8 corruption detection)
                    use crate::ref_store::validate_parent_name;
                    if let Err(e) = validate_parent_name(&parent, branch) {
                        errors.push(DiagnosticError::CorruptedRef {
                            branch: branch.clone(),
                            error: e.to_string(),
                        });
                        continue; // Skip further validation for corrupted refs
                    }

                    // Parent name is valid, check if it's tracked and exists
                    if parent != trunk {
                        // Check if parent is tracked
                        let parent_is_tracked = ref_store.is_tracked(&parent)? || parent == trunk;
                        if !parent_is_tracked {
                            errors.push(DiagnosticError::OrphanedParent {
                                branch: branch.clone(),
                                parent: parent.clone(),
                            });
                        }
                        // Also check if parent exists in git
                        if parent_is_tracked && !gateway.branch_exists(&parent)? {
                            errors.push(DiagnosticError::OrphanedParent {
                                branch: branch.clone(),
                                parent,
                            });
                        }
                    }
                }
                None => {
                    // Branch has no parent (could be untracked or trunk)
                }
            }
        }

        // Check for cycles
        if let Some(cycle) = detect_cycle(ref_store, &all_branches)? {
            errors.push(DiagnosticError::Cycle(cycle));
        }
    }

    Ok(errors)
}

/// Detect cycles in parent relationships
fn detect_cycle(ref_store: &RefStore, branches: &[String]) -> Result<Option<Vec<String>>> {
    let trunk = ref_store.get_trunk()?.unwrap_or_default();

    for start in branches {
        let mut visited = HashSet::new();
        let mut path = Vec::new();
        let mut current = start.clone();

        loop {
            if current == trunk {
                break; // Reached trunk, no cycle
            }

            if visited.contains(&current) {
                // Found cycle - extract it from path
                if let Some(pos) = path.iter().position(|b| b == &current) {
                    let mut cycle = path[pos..].to_vec();
                    cycle.push(current);
                    return Ok(Some(cycle));
                }
                break;
            }

            visited.insert(current.clone());
            path.push(current.clone());

            // Use unchecked getter to skip corrupted refs when detecting cycles
            match ref_store.get_parent_unchecked(&current) {
                Ok(Some(parent)) => {
                    // Skip validation - we just want to follow the chain
                    // Corruption will be detected separately by validate_refs
                    current = parent;
                }
                Ok(None) => break, // No parent, end of chain
                Err(_) => break,   // Error reading ref, end of chain
            }
        }
    }

    Ok(None)
}

/// Attempt to fix an orphaned parent relationship.
///
/// Strategy:
/// 1. Check if the missing parent exists on remote ({remote}/{parent})
/// 2. If remote parent exists:
///    a. Check if it's an ancestor of the child branch → parent was rebased into child, reparent to trunk
///    b. Check if it's an ancestor of trunk → parent was merged, reparent to trunk
/// 3. If no remote parent, fall back to trunk if child is based on trunk
///
/// Returns:
/// - Ok(Some(new_parent)) if successfully reparented
/// - Ok(None) if couldn't fix automatically
/// - Err if something went wrong
fn try_fix_orphaned_parent(
    ref_store: &RefStore,
    gateway: &GitGateway,
    branch: &str,
    missing_parent: &str,
    trunk: &str,
) -> Result<Option<String>> {
    let remote_parent = format!("{}/{}", gateway.remote(), missing_parent);

    // Check if the missing parent exists on remote
    if gateway.resolve_ref(&remote_parent).is_ok() {
        // Remote parent exists! Check if its commits are already in the child
        // This happens after a restack that rebased the child onto trunk

        if gateway.is_ancestor(&remote_parent, branch)? {
            // The remote parent's commit is an ancestor of our branch
            // This means the child already contains all of the parent's work
            // (likely via a rebase/restack that incorporated the parent's changes)
            // Safe to reparent to trunk
            ref_store.set_parent(branch, trunk)?;
            return Ok(Some(trunk.to_string()));
        }

        // Check if the remote parent was merged into trunk
        if gateway.is_ancestor(&remote_parent, trunk)? {
            // Parent was merged into trunk - safe to reparent to trunk
            ref_store.set_parent(branch, trunk)?;
            return Ok(Some(trunk.to_string()));
        }

        // Remote parent exists but wasn't rebased into child or merged into trunk
        // Fall through to check if child is based on trunk anyway
    }

    // Check if the branch is based on trunk
    // This catches cases where:
    // - The child was rebased onto trunk (losing connection to old parent)
    // - The old parent exists on remote but is now stale
    // - There's no remote parent at all
    if gateway.is_ancestor(trunk, branch)? {
        // Trunk is an ancestor of branch, so it's safe to reparent to trunk
        ref_store.set_parent(branch, trunk)?;
        return Ok(Some(trunk.to_string()));
    }

    // Couldn't determine a safe fix
    // This happens when:
    // - Branch is not based on trunk (has unrelated history)
    // - Or something else unusual is going on
    Ok(None)
}

/// Attempt to fix issues. Returns the number of issues that could not be fixed.
fn attempt_fix(ref_store: &RefStore, gateway: &GitGateway, errors: &[DiagnosticError]) -> Result<usize> {
    let mut fixed_count = 0;
    let mut failed_count = 0;

    let trunk = ref_store.get_trunk()?.unwrap_or_default();

    for error in errors {
        match error {
            DiagnosticError::OrphanedParent { branch, parent } => {
                match try_fix_orphaned_parent(ref_store, gateway, branch, parent, &trunk) {
                    Ok(Some(new_parent)) => {
                        println!("  {} Fixed '{}': reparented to '{}'", "✓".green(), branch, new_parent);
                        fixed_count += 1;
                    }
                    Ok(None) => {
                        println!(
                            "  {}: Orphaned parent for '{}' (manual intervention required)",
                            "⚠".yellow(),
                            branch
                        );
                        failed_count += 1;
                    }
                    Err(e) => {
                        println!("  {}: Failed to fix '{}': {}", "✗".red(), branch, e);
                        failed_count += 1;
                    }
                }
            }
            DiagnosticError::Cycle(_) => {
                println!("  {}: Cycle detected (manual intervention required)", "⚠".yellow());
                failed_count += 1;
            }
            DiagnosticError::MissingTrunk(_) => {
                println!(
                    "  {}: Missing trunk branch (manual intervention required)",
                    "⚠".yellow()
                );
                failed_count += 1;
            }
            DiagnosticError::TrackedBranchMissing(branch) => {
                println!("  Fixing: Removing tracking for non-existent branch '{}'...", branch);

                // Remove the parent ref for this branch
                match ref_store.remove_parent(branch) {
                    Ok(()) => {
                        fixed_count += 1;
                        println!("  {} Fixed", "✓".green());
                    }
                    Err(e) => {
                        failed_count += 1;
                        println!("  {} Failed: {}", "✗".red(), e);
                    }
                }
            }
            DiagnosticError::CorruptedRef { branch, .. } => {
                println!("  Fixing: Removing corrupted parent ref for '{}'...", branch);

                // Remove the corrupted parent ref
                match ref_store.remove_parent(branch) {
                    Ok(()) => {
                        fixed_count += 1;
                        println!("  {} Fixed", "✓".green());
                    }
                    Err(e) => {
                        failed_count += 1;
                        println!("  {} Failed: {}", "✗".red(), e);
                    }
                }
            }
        }
    }

    println!("\n{} Repair complete:", "📊".blue());
    println!("  {} Fixed automatically", fixed_count.to_string().green());
    println!("  {} Require manual intervention", failed_count.to_string().yellow());

    if failed_count > 0 {
        let prog = program_name();
        println!("\n{} Some issues require manual intervention.", "💡".blue());
        println!("  Review the issues above and run:");
        println!("  • {} track <branch>    - Track an existing branch", prog);
        println!("  • {} delete <branch>   - Remove a problematic branch", prog);
        println!("  • {} move <branch>     - Fix parent relationships", prog);
    }

    Ok(failed_count)
}

/// Update stack visualization in all PRs
fn run_fix_viz() -> Result<()> {
    println!("{} Updating stack visualization in PRs...\n", "🔧".blue());

    let ref_store = RefStore::new()?;
    let cache = Cache::load().unwrap_or_default();

    // Get forge
    let forge = get_forge(None)?;

    // Check auth
    forge.check_auth()?;

    // Update all stack visualizations (verbose mode)
    let updated = update_all_stack_visualizations(&ref_store, &cache, forge.as_ref(), true)?;

    if updated > 0 {
        println!(
            "\n{} Updated stack info in {} PR{}",
            "✓".green().bold(),
            updated,
            if updated == 1 { "" } else { "s" }
        );
    } else {
        println!(
            "\n{} No PRs needed updating (no tracked branches have open PRs)",
            "ℹ".blue()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    fn create_branch(repo: &Repository, name: &str) -> Result<()> {
        let head = repo.head()?.peel_to_commit()?;
        repo.branch(name, &head, false)?;
        Ok(())
    }

    #[test]
    fn test_doctor_healthy_repo() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());

        // Initialize git repo using helper
        let _repo = init_test_repo(dir.path())?;

        // Set trunk in refs
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        // Should pass doctor check
        let result = run(false, false);
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_doctor_detects_missing_branch() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());

        // Initialize git repo using helper
        let _repo = init_test_repo(dir.path())?;

        // Set up refs with a branch that doesn't exist in git
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("non-existent", "main")?;

        // Should detect the missing branch
        let gateway = GitGateway::new()?;
        let errors = validate_refs(&ref_store, &gateway)?;

        assert!(!errors.is_empty());
        assert!(errors
            .iter()
            .any(|e| matches!(e, DiagnosticError::TrackedBranchMissing(_))));

        Ok(())
    }

    #[test]
    fn test_doctor_returns_error_when_issues_found() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());

        // Initialize git repo using helper
        let _repo = init_test_repo(dir.path())?;

        // Set up refs with a branch that doesn't exist in git
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("non-existent", "main")?;

        // Doctor should return an error (exit code 1) when issues are found
        let result = run(false, false);
        assert!(result.is_err(), "doctor should return error when issues are found");

        Ok(())
    }

    #[test]
    fn test_doctor_fix_removes_missing_branches() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());

        // Initialize git repo using helper
        let repo = init_test_repo(dir.path())?;

        // Create branches first so we can set parent relationships
        create_branch(&repo, "feature-1")?;
        create_branch(&repo, "feature-2")?;

        // Set up refs with branches
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature-1", "main")?;
        ref_store.set_parent("feature-2", "feature-1")?;

        // Now delete the branches to simulate "tracked but missing" state
        repo.find_branch("feature-1", git2::BranchType::Local)?.delete()?;
        repo.find_branch("feature-2", git2::BranchType::Local)?.delete()?;

        // Validate - should find TrackedBranchMissing errors
        let gateway = GitGateway::new()?;
        let errors = validate_refs(&ref_store, &gateway)?;

        let missing_count = errors
            .iter()
            .filter(|e| matches!(e, DiagnosticError::TrackedBranchMissing(_)))
            .count();
        assert_eq!(missing_count, 2, "Expected 2 TrackedBranchMissing errors");

        // Run fix
        attempt_fix(&ref_store, &gateway, &errors)?;

        // Verify branches were removed from refs
        assert!(
            ref_store.get_parent("feature-1")?.is_none(),
            "feature-1 should have been removed"
        );
        assert!(
            ref_store.get_parent("feature-2")?.is_none(),
            "feature-2 should have been removed"
        );

        Ok(())
    }

    #[test]
    fn test_doctor_fix_orphaned_parent_reparents_to_trunk() -> Result<()> {
        let dir = tempdir()?;
        let _ctx = TestRepoContext::new(dir.path());

        // Initialize git repo using helper
        let repo = init_test_repo(dir.path())?;

        // Create feature-1 branch that will exist in git
        create_branch(&repo, "feature-1")?;
        // Create a parent branch that we'll delete later to simulate orphaned state
        create_branch(&repo, "non-existent-parent")?;

        // Set up refs with parent relationship
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;
        ref_store.set_parent("feature-1", "non-existent-parent")?;

        // Now delete the parent branch to simulate "orphaned parent" state
        repo.find_branch("non-existent-parent", git2::BranchType::Local)?
            .delete()?;

        // Validate - should find OrphanedParent error
        let gateway = GitGateway::new()?;
        let errors = validate_refs(&ref_store, &gateway)?;

        assert!(!errors.is_empty());
        assert!(errors
            .iter()
            .any(|e| matches!(e, DiagnosticError::OrphanedParent { .. })));

        // Run fix - should reparent to trunk since feature-1 is based on main
        attempt_fix(&ref_store, &gateway, &errors)?;

        // Verify feature-1 was reparented to main
        assert_eq!(
            ref_store.get_parent("feature-1")?,
            Some("main".to_string()),
            "feature-1 should be reparented to main"
        );

        Ok(())
    }

    // CRITICAL-8 + doctor integration tests

    #[test]
    fn test_doctor_can_inspect_corrupted_ref() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create branch with corrupted parent ref (empty blob)
        create_branch(&repo, "feature")?;
        let empty_blob = repo.blob(b"")?;
        repo.reference("refs/diamond/parent/feature", empty_blob, true, "corrupt")?;

        // Test: doctor should be able to read the corrupted value without crashing
        let raw_parent = ref_store.get_parent_unchecked("feature")?;
        assert_eq!(raw_parent, Some("".to_string())); // Empty string, not error

        // Test: doctor can validate separately
        use crate::ref_store::validate_parent_name;
        let validation = validate_parent_name(&raw_parent.unwrap(), "feature");
        assert!(validation.is_err());
        assert!(validation.unwrap_err().to_string().contains("empty value"));

        Ok(())
    }

    #[test]
    fn test_doctor_detects_corrupted_refs() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        let gateway = GitGateway::new()?;

        // Set trunk (required for validate_refs to run)
        ref_store.set_trunk("main")?;

        // Create branch with corrupted parent ref (empty blob)
        create_branch(&repo, "feature")?;
        let empty_blob = repo.blob(b"")?;
        repo.reference("refs/diamond/parent/feature", empty_blob, true, "corrupt")?;

        // Run doctor diagnostics
        let errors = validate_refs(&ref_store, &gateway)?;

        // Should detect corruption
        assert_eq!(errors.len(), 1, "Should detect exactly one error");
        match &errors[0] {
            DiagnosticError::CorruptedRef { branch, error } => {
                assert_eq!(branch, "feature");
                assert!(
                    error.contains("empty value"),
                    "Error message should mention 'empty value', got: {}",
                    error
                );
            }
            _ => panic!("Expected CorruptedRef error, got: {:?}", errors[0]),
        }

        Ok(())
    }

    #[test]
    fn test_doctor_fix_removes_corrupted_refs() -> Result<()> {
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Set trunk
        ref_store.set_trunk("main")?;

        // Create branch with corrupted parent ref (empty blob)
        create_branch(&repo, "feature")?;
        let empty_blob = repo.blob(b"")?;
        repo.reference("refs/diamond/parent/feature", empty_blob, true, "corrupt")?;

        // Verify corruption exists
        let result = ref_store.get_parent("feature");
        assert!(result.is_err(), "Should have corrupted ref");
        assert!(result.unwrap_err().to_string().contains("Corrupted metadata"));

        // Run doctor --fix
        run(true, false)?;

        // Verify corruption is fixed (ref removed)
        assert!(ref_store.get_parent("feature")?.is_none(), "Ref should be removed");
        assert!(!ref_store.is_tracked("feature")?, "Branch should not be tracked");

        Ok(())
    }

    #[test]
    fn test_normal_command_rejects_corrupted_ref() -> Result<()> {
        // Verify CRITICAL-8 is still enforced for normal operations
        let dir = tempdir()?;
        let repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;

        // Create branch with corrupted parent ref (empty blob)
        create_branch(&repo, "feature")?;
        let empty_blob = repo.blob(b"")?;
        repo.reference("refs/diamond/parent/feature", empty_blob, true, "corrupt")?;

        // Test: Normal get_parent() should fail with validation error
        let result = ref_store.get_parent("feature");
        assert!(result.is_err(), "Normal commands should reject corrupted refs");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Corrupted metadata"),
            "Error should mention corruption, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("empty value"),
            "Error should describe the corruption type, got: {}",
            err_msg
        );

        Ok(())
    }
}
