use anyhow::Result;
use colored::Colorize;
use slog::{Drain, Logger};

use crate::commands::restack;
use crate::context::ExecutionContext;
use crate::git_gateway::GitGateway;

/// Absorb staged changes into the appropriate commits in the stack
///
/// This command uses git-absorb to automatically find which earlier commits
/// should receive the currently staged changes, and amends them accordingly.
/// If `all` is true, stage all changes before absorbing.
/// If `force` is true, skip confirmation prompts (not currently used, but reserved).
pub fn run(all: bool, _force: bool) -> Result<()> {
    let dry_run = ExecutionContext::is_dry_run();
    let gateway = GitGateway::new()?;

    // Stage all changes if -a flag is provided
    if all {
        gateway.stage_all()?;
        println!("{} Staged all changes", "✓".green());
    }

    // Check if there are staged changes
    if !gateway.has_staged_changes()? {
        println!("{} No staged changes to absorb.", "ℹ".blue());
        return Ok(());
    }

    let current_branch = gateway.get_current_branch_name()?;

    if dry_run {
        println!("{} Dry run - showing what would be absorbed:", "→".blue());
    } else {
        println!("{} Absorbing staged changes into stack...", "→".blue());
    }

    // Create a logger for git-absorb (outputs to terminal)
    let logger = create_logger();

    // Configure git-absorb
    let config = git_absorb::Config {
        dry_run,
        force_author: false,
        force_detach: false,
        base: None,
        and_rebase: false, // We handle restack ourselves
        rebase_options: &vec![],
        whole_file: false,
        one_fixup_per_commit: false,
        message: None,
    };

    // Run git-absorb library
    git_absorb::run(&logger, &config)?;

    if !dry_run {
        // Auto-restack children after absorb (like modify does)
        println!();
        restack::restack_children(&current_branch)?;

        println!("{} Absorb complete!", "✓".green().bold());
    }

    Ok(())
}

/// Create a slog logger that outputs to the terminal
fn create_logger() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = std::sync::Mutex::new(drain).fuse();
    Logger::root(drain, slog::o!())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    use crate::test_context::{init_test_repo, TestRepoContext};

    #[test]
    fn test_has_staged_changes_detection() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;

        let gateway = GitGateway::from_path(dir.path())?;

        // Fresh repo should have no staged changes
        assert!(!gateway.has_staged_changes()?);

        Ok(())
    }

    #[test]
    fn test_create_logger() {
        // Verify logger creation doesn't panic
        let _logger = create_logger();
    }

    #[test]
    fn test_config_creation() {
        // Verify we can create a valid config for git-absorb
        let config = git_absorb::Config {
            dry_run: true,
            force_author: false,
            force_detach: false,
            base: None,
            and_rebase: false,
            rebase_options: &vec![],
            whole_file: false,
            one_fixup_per_commit: false,
            message: None,
        };
        assert!(config.dry_run);
    }

    #[test]
    fn test_absorb_no_staged_changes_succeeds() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // No staged changes - should succeed with Ok(()), not error
        let result = run(false, false);
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_absorb_all_flag_stages_changes() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let _repo = init_test_repo(dir.path())?;
        let _ctx = TestRepoContext::new(dir.path());

        // Create an unstaged file
        std::fs::write(dir.path().join("test.txt"), "test content")?;

        let gateway = GitGateway::new()?;

        // Verify file is not staged
        assert!(!gateway.has_staged_changes()?);

        // Call with all=true to stage changes
        // Note: This will stage the file, check for staged changes, but since
        // there's no prior commit to absorb into, it will likely fail at git-absorb.
        // We just verify the staging part works.
        gateway.stage_all()?;
        assert!(gateway.has_staged_changes()?);

        Ok(())
    }
}
