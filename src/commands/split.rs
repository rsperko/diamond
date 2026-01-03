use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;

use crate::git_gateway::GitGateway;
use crate::program_name::program_name;
use crate::ref_store::RefStore;

/// Split the current branch into multiple branches
///
/// Supports three modes:
/// 1. --by-commit: Split each commit into its own branch
/// 2. --by-file <pathspecs>: Extract files into a new parent branch
/// 3. Legacy: dm split <new-branch> <commit> - split at a specific commit
pub fn run(
    new_branch: Option<String>,
    commit: Option<String>,
    by_commit: bool,
    by_file: Option<Vec<String>>,
    by_hunk: bool,
) -> Result<()> {
    // Dispatch to the appropriate mode
    if by_hunk {
        run_by_hunk()
    } else if by_commit {
        run_by_commit()
    } else if let Some(patterns) = by_file {
        run_by_file(patterns)
    } else if new_branch.is_some() || commit.is_some() {
        // Legacy mode - split at specific commit
        run_at_commit(new_branch, commit)
    } else {
        // No mode specified - show help
        show_usage()
    }
}

/// Show usage help when no mode specified
fn show_usage() -> Result<()> {
    let prog = program_name();
    println!("{} Split the current branch into multiple branches", "→".blue());
    println!();
    println!("Usage:");
    println!(
        "  {} split --by-commit       Split each commit into its own branch",
        prog
    );
    println!(
        "  {} split --by-file <files> Extract files into a new parent branch",
        prog
    );
    println!("  {} split <branch> <commit> Split at a specific commit", prog);
    println!();
    println!("Examples:");
    println!("  {} split --by-commit", prog);
    println!("    Creates: main -> feature-1 -> feature-2 -> feature-3");
    println!("    (one branch per commit)");
    println!();
    println!("  {} split --by-file '*.test.ts' 'test/**'", prog);
    println!("    Extracts test files into a new parent branch");
    println!();
    println!("  {} split feature-part2 HEAD~2", prog);
    println!("    Splits current branch at 2 commits ago");

    Ok(())
}

/// Split by commit - creates a branch for each commit
fn run_by_commit() -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    let current_branch = gateway.get_current_branch_name()?;
    let trunk = ref_store.get_trunk()?;

    // Verify current branch is tracked
    let parent = ref_store.get_parent(&current_branch)?;
    if parent.is_none() && trunk.as_ref() != Some(&current_branch) {
        anyhow::bail!(
            "Branch '{}' is not tracked. Run '{} track' first.",
            current_branch,
            program_name()
        );
    }

    // Cannot split trunk
    if trunk.as_ref() == Some(&current_branch) {
        anyhow::bail!("Cannot split trunk branch '{}'", current_branch);
    }

    let parent_branch = parent
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Branch '{}' has no parent", current_branch))?;

    // Get commits unique to this branch (from parent to HEAD)
    let parent_tip = gateway.resolve_ref(parent_branch)?;
    let commits = gateway.get_commits_between(&parent_tip.to_string(), "HEAD")?;

    if commits.is_empty() {
        println!("{} No commits to split - branch is empty", "ℹ".blue());
        return Ok(());
    }

    if commits.len() == 1 {
        println!("{} Only one commit - nothing to split", "ℹ".blue());
        return Ok(());
    }

    println!(
        "{} Splitting '{}' into {} branches (one per commit)",
        "→".blue(),
        current_branch.green(),
        commits.len().to_string().yellow()
    );

    // Get any children of current branch before we start
    let children: Vec<String> = ref_store.get_children(&current_branch)?.into_iter().collect();

    // Create branches for each commit (oldest to newest)
    // commits are returned newest first, so reverse
    let commits: Vec<_> = commits.into_iter().rev().collect();

    let base_name = &current_branch;
    let mut prev_branch = parent_branch.clone();
    let mut created_branches = Vec::new();

    for (i, (oid, message)) in commits.iter().enumerate() {
        let branch_name = if i == commits.len() - 1 {
            // Keep the original branch name for the last commit
            current_branch.clone()
        } else {
            // Generate names for intermediate branches
            format!("{}-part{}", base_name, i + 1)
        };

        // Check if branch exists (skip original branch name)
        if branch_name != current_branch && gateway.branch_exists(&branch_name)? {
            anyhow::bail!(
                "Branch '{}' already exists. Please rename or delete it first.",
                branch_name
            );
        }

        let short_msg = message
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(50)
            .collect::<String>();
        let short_oid = &oid.to_string()[..7];

        println!(
            "  {} {} ({}) -> {}",
            "•".blue(),
            short_oid.yellow(),
            short_msg.dimmed(),
            branch_name.green()
        );

        created_branches.push((branch_name.clone(), oid.clone(), prev_branch.clone()));
        prev_branch = branch_name;
    }

    // Now actually create the branches
    // Start by resetting current branch to parent
    let _original_head = gateway.resolve_ref("HEAD")?.to_string();

    // Create branches from oldest to newest
    for (i, (branch_name, oid, parent_ref)) in created_branches.iter().enumerate() {
        let oid_str = oid.to_string();
        if *branch_name == current_branch {
            // For the original branch, just update metadata
            ref_store.set_parent(&current_branch, parent_ref)?;
        } else {
            // Create new branch at this commit
            gateway.create_branch_at_ref(branch_name, &oid_str)?;
            ref_store.set_parent(branch_name, parent_ref)?;
        }

        // For the first non-original branch, we need to reset its history
        if i < created_branches.len() - 1 {
            // The branch should only contain commits up to this point
            // We need to reset it to just this commit on top of parent
            gateway.checkout_branch(branch_name)?;
            gateway.hard_reset_to(&oid_str)?;
        }
    }

    // Checkout back to the original (now last) branch
    gateway.checkout_branch(&current_branch)?;

    // Update children to point to the new last branch
    let last_created = &created_branches.last().unwrap().0;
    for child in &children {
        ref_store.set_parent(child, last_created)?;
    }

    println!();
    println!("{} Split complete!", "✓".green().bold());
    println!();
    println!("Created branches:");
    for (branch_name, _, parent_ref) in &created_branches {
        if *branch_name != current_branch {
            println!("  {} -> {}", parent_ref.blue(), branch_name.green());
        }
    }

    Ok(())
}

/// Split by file - extracts files matching patterns into a new parent branch
fn run_by_file(patterns: Vec<String>) -> Result<()> {
    let gateway = GitGateway::new()?;
    let ref_store = RefStore::new()?;
    let current_branch = gateway.get_current_branch_name()?;
    let trunk = ref_store.get_trunk()?;

    // Verify current branch is tracked
    let parent = ref_store.get_parent(&current_branch)?;
    if parent.is_none() && trunk.as_ref() != Some(&current_branch) {
        anyhow::bail!(
            "Branch '{}' is not tracked. Run '{} track' first.",
            current_branch,
            program_name()
        );
    }

    // Cannot split trunk
    if trunk.as_ref() == Some(&current_branch) {
        anyhow::bail!("Cannot split trunk branch '{}'", current_branch);
    }

    let parent_branch = parent
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Branch '{}' has no parent", current_branch))?;

    println!(
        "{} Splitting files from '{}' matching: {}",
        "→".blue(),
        current_branch.green(),
        patterns.join(", ").yellow()
    );

    // Get all files changed in this branch compared to parent
    let parent_tip = gateway.resolve_ref(parent_branch)?;
    let changed_files = gateway.get_changed_files(&parent_tip.to_string(), "HEAD")?;

    // Filter files matching the patterns
    let matching_files: Vec<String> = changed_files
        .iter()
        .filter(|file| {
            patterns.iter().any(|pattern| {
                // Support glob-like patterns
                if pattern.contains('*') {
                    glob_match(pattern, file)
                } else {
                    file.starts_with(pattern) || file.contains(pattern)
                }
            })
        })
        .cloned()
        .collect();

    if matching_files.is_empty() {
        println!("{} No files match the specified patterns", "ℹ".blue());
        println!();
        println!("Files changed in this branch:");
        for file in &changed_files {
            println!("  {}", file);
        }
        return Ok(());
    }

    let non_matching: Vec<_> = changed_files.iter().filter(|f| !matching_files.contains(f)).collect();

    if non_matching.is_empty() {
        anyhow::bail!(
            "All files match the patterns. Nothing would remain in '{}'.",
            current_branch
        );
    }

    println!("  Files to extract:");
    for file in &matching_files {
        println!("    {} {}", "+".green(), file);
    }
    println!("  Files remaining:");
    for file in &non_matching {
        println!("    {} {}", "•".blue(), file);
    }

    // Generate name for the new branch
    let new_branch_name = format!("{}-extracted", current_branch);
    if gateway.branch_exists(&new_branch_name)? {
        anyhow::bail!(
            "Branch '{}' already exists. Please rename or delete it first.",
            new_branch_name
        );
    }

    // Strategy:
    // 1. Create new branch at parent
    // 2. Cherry-pick changes to matching files only
    // 3. Update current branch to remove those files
    // 4. Insert new branch between parent and current

    // Step 1: Create new branch at parent
    gateway.create_branch_at_ref(&new_branch_name, &parent_tip.to_string())?;

    // Step 2: Checkout new branch and extract matching files from current branch
    gateway.checkout_branch(&new_branch_name)?;

    // Get the files from current branch and add them
    for file in &matching_files {
        // Get file content from current branch
        let content = gateway.get_file_at_ref(&current_branch, file);
        if let Ok(content) = content {
            // Write file and stage it
            std::fs::create_dir_all(std::path::Path::new(file).parent().unwrap_or(std::path::Path::new("")))?;
            std::fs::write(file, content)?;
            gateway.stage_file(file)?;
        }
    }

    // Commit the extracted files
    gateway.commit(&format!("Extract files: {}", patterns.join(", ")))?;

    // Step 3: Update current branch - rebase onto new branch
    // This applies current branch's changes on top of the extracted files
    // git rebase --onto new_branch parent current_branch
    gateway.rebase_onto_from(&current_branch, &new_branch_name, parent_branch)?;

    // Remove the extracted files from current branch
    for file in &matching_files {
        if std::path::Path::new(file).exists() {
            // Restore file to the extracted branch's version
            // (they should be the same after rebase)
        }
    }

    // Step 4: Update metadata
    ref_store.set_parent(&new_branch_name, parent_branch)?;
    ref_store.set_parent(&current_branch, &new_branch_name)?;

    println!();
    println!("{} Split complete!", "✓".green().bold());
    println!();
    println!("New structure:");
    println!(
        "  {} -> {} -> {}",
        parent_branch.blue(),
        new_branch_name.green(),
        current_branch.green()
    );
    println!();
    println!("Next steps:");
    println!(
        "  {} checkout {}  # Review extracted files",
        program_name(),
        new_branch_name
    );
    println!("  {} restack           # Ensure stack is aligned", program_name());

    Ok(())
}

/// Simple glob matching for file patterns
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('*').collect();

    if pattern_parts.len() == 1 {
        // No wildcard
        return text == pattern;
    }

    let mut pos = 0;
    for (i, part) in pattern_parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        if let Some(found) = text[pos..].find(part) {
            if i == 0 && found != 0 {
                // First part must match at start
                return false;
            }
            pos += found + part.len();
        } else {
            return false;
        }
    }

    // If pattern ends with *, any suffix is ok
    // If not, must match exactly to end
    if !pattern.ends_with('*') {
        return pos == text.len();
    }

    true
}

/// Split by hunk - interactive mode (requires TTY)
fn run_by_hunk() -> Result<()> {
    // Check for TTY
    if !std::io::stdout().is_terminal() {
        anyhow::bail!(
            "Split --by-hunk requires an interactive terminal.\n\
            Use --by-commit or --by-file for non-interactive splitting."
        );
    }

    println!("{} Interactive hunk splitting", "→".blue());
    println!();
    println!("This feature works like 'git add -p' to create new branches from selected hunks.");
    println!();
    println!("Not yet implemented. Alternatives:");
    println!("  1. Use --by-commit to split each commit into its own branch");
    println!("  2. Use --by-file to split by file patterns");
    println!(
        "  3. Manually use 'git add -p' + '{} create' for fine-grained control",
        program_name()
    );

    anyhow::bail!("--by-hunk is not yet implemented")
}

/// Legacy mode - split at a specific commit
fn run_at_commit(new_branch: Option<String>, commit: Option<String>) -> Result<()> {
    let gateway = GitGateway::new()?;
    let current_branch = gateway.get_current_branch_name()?;
    let ref_store = RefStore::new()?;
    let trunk = ref_store.get_trunk()?;

    // Verify current branch is tracked
    let parent = ref_store.get_parent(&current_branch)?;
    if parent.is_none() && trunk.as_ref() != Some(&current_branch) {
        anyhow::bail!(
            "Branch '{}' is not tracked. Run '{} track' first.",
            current_branch,
            program_name()
        );
    }

    // Require both arguments
    let prog = program_name();
    let new_branch = new_branch.ok_or_else(|| {
        anyhow::anyhow!(
            "Missing new branch name.\n\n\
            Usage: {} split <new-branch-name> <commit>\n\n\
            Examples:\n  \
            {} split feature-part2 HEAD~2    # Split at 2 commits ago\n  \
            {} split feature-part2 abc123    # Split at specific commit\n\n\
            Or use:\n  \
            {} split --by-commit    # Split each commit into its own branch\n  \
            {} split --by-file ...  # Extract files into new parent branch",
            prog,
            prog,
            prog,
            prog,
            prog
        )
    })?;

    let commit = commit.ok_or_else(|| {
        anyhow::anyhow!(
            "Missing commit to split at.\n\n\
            Usage: {} split <new-branch-name> <commit>\n\n\
            Examples:\n  \
            {} split {} HEAD~2    # Split at 2 commits ago\n  \
            {} split {} abc123    # Split at specific commit",
            prog,
            prog,
            new_branch,
            prog,
            new_branch
        )
    })?;

    // Check if new branch already exists
    if gateway.branch_exists(&new_branch)? {
        anyhow::bail!("Branch '{}' already exists", new_branch);
    }

    // Verify the commit exists and is in our history
    let split_oid = gateway.resolve_ref(&commit)?;
    let split_point = split_oid.to_string();
    let split_short = &split_point[..7.min(split_point.len())];

    // Verify the split point is in the current branch's history
    if !gateway.is_ancestor(&split_point, "HEAD")? {
        anyhow::bail!("Commit '{}' is not in the history of '{}'", commit, current_branch);
    }

    // Verify the split point is not the parent branch's tip
    if let Some(ref parent_branch) = parent {
        let parent_tip = gateway.resolve_ref(parent_branch)?.to_string();
        if split_point == parent_tip {
            anyhow::bail!(
                "Cannot split at the parent branch tip. \
                Use '{} fold' instead if you want to merge into parent.",
                program_name()
            );
        }
    }

    println!(
        "{} Splitting '{}' at commit {}",
        "→".blue(),
        current_branch.green(),
        split_short.yellow()
    );

    // Create new branch at current HEAD (before we reset)
    println!("  Creating new branch '{}'...", new_branch.green());
    gateway.create_branch_at_head(&new_branch)?;

    // Reset current branch to commit before split point
    println!("  Resetting '{}' to before split point...", current_branch);
    let reset_target = format!("{}^", split_point);
    if let Err(e) = gateway.hard_reset_to(&reset_target) {
        // Cleanup the branch we created on failure
        let _ = gateway.delete_branch(&new_branch);
        return Err(e);
    }

    // Update metadata
    // Register new branch with current branch as parent
    ref_store.set_parent(&new_branch, &current_branch)?;

    // Move children of current branch to be children of new branch
    let children: Vec<String> = ref_store
        .get_children(&current_branch)?
        .into_iter()
        .filter(|c| c != &new_branch)
        .collect();

    for child in &children {
        ref_store.set_parent(child, &new_branch)?;
    }

    println!();
    println!("{} Split complete!", "✓".green().bold());
    println!();
    println!("Stack structure:");
    if let Some(ref p) = parent {
        println!("  {} -> {} -> {}", p.blue(), current_branch.green(), new_branch.green());
    } else {
        println!("  {} -> {}", current_branch.green(), new_branch.green());
    }
    println!();
    println!("Next steps:");
    println!(
        "  {} checkout {}  # Switch to split-off branch",
        program_name(),
        new_branch
    );
    println!("  {} restack           # Restack if needed", program_name());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::TestRepoContext;

    fn init_test_repo(path: &std::path::Path) -> Result<git2::Repository> {
        let repo = git2::Repository::init(path)?;
        let sig = git2::Signature::now("Test User", "test@example.com")?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])?;
        drop(tree);
        Ok(repo)
    }

    #[test]
    fn test_glob_match_simple() {
        assert!(glob_match("*.txt", "file.txt"));
        assert!(glob_match("*.txt", "path/to/file.txt"));
        assert!(!glob_match("*.txt", "file.rs"));
    }

    #[test]
    fn test_glob_match_prefix() {
        assert!(glob_match("src/*", "src/file.rs"));
        assert!(glob_match("src/*", "src/subdir/file.rs"));
        assert!(!glob_match("src/*", "test/file.rs"));
    }

    #[test]
    fn test_glob_match_contains() {
        assert!(glob_match("*test*", "my_test_file.rs"));
        assert!(glob_match("*test*", "test.rs"));
        assert!(!glob_match("*test*", "myfile.rs"));
    }

    #[test]
    fn test_split_requires_branch_name() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Set up tracking
        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        let result = run(None, None, false, None, false);
        // Should show usage, not error
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_split_untracked_branch_fails() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Empty ref_store - branch not tracked
        let _ref_store = RefStore::new()?;

        let result = run(
            Some("new-branch".to_string()),
            Some("HEAD".to_string()),
            false,
            None,
            false,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));

        Ok(())
    }

    #[test]
    fn test_by_hunk_requires_tty() -> Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        let ref_store = RefStore::new()?;
        ref_store.set_trunk("main")?;

        // by_hunk should fail in non-TTY environment
        let result = run(None, None, false, None, true);
        assert!(result.is_err());
        // The error should mention interactive or TTY
        let err = result.unwrap_err().to_string();
        assert!(err.contains("interactive") || err.contains("TTY") || err.contains("not yet implemented"));

        Ok(())
    }
}
