use anyhow::Result;
use std::process::Command;

/// Common git commands that should be passed through.
/// This list covers the most frequently used porcelain and ancillary commands.
const KNOWN_GIT_COMMANDS: &[&str] = &[
    // Main Porcelain Commands
    "add",
    "am",
    "archive",
    "bisect",
    "branch",
    "bundle",
    "checkout",
    "cherry-pick",
    "clean",
    "clone",
    "commit",
    "describe",
    "diff",
    "fetch",
    "format-patch",
    "gc",
    "grep",
    "init",
    "log",
    "merge",
    "mv",
    "notes",
    "pull",
    "push",
    "range-diff",
    "rebase",
    "reset",
    "restore",
    "revert",
    "rm",
    "shortlog",
    "show",
    "sparse-checkout",
    "stash",
    "status",
    "submodule",
    "switch",
    "tag",
    "worktree",
    // Ancillary Commands
    "blame",
    "config",
    "help",
    "reflog",
    "remote",
    "rev-parse",
    "version",
];

/// Run a git command passthrough.
///
/// If the command is a known git command, it passes through directly.
/// If unknown, it checks whether git recognizes the command before passing through.
/// If git doesn't recognize it, shows Diamond help.
pub fn run(args: Vec<String>) -> Result<()> {
    if args.is_empty() {
        show_help_and_exit();
    }

    let cmd = &args[0];

    if is_known_git_command(cmd) || is_git_recognized_command(cmd) {
        execute_passthrough(&args)
    } else {
        show_help_and_exit();
    }
}

/// Check if a command is in our known git commands list (fast path).
fn is_known_git_command(cmd: &str) -> bool {
    KNOWN_GIT_COMMANDS.contains(&cmd)
}

/// Check if git recognizes the command (slow path for aliases/obscure commands).
fn is_git_recognized_command(cmd: &str) -> bool {
    // Run from a stable directory to avoid issues with temp directories from tests
    let mut command = Command::new("git");
    command.arg(cmd).arg("--help");

    // Use home directory or root as stable location (avoids test directory issues)
    if let Some(home) = std::env::var_os("HOME") {
        command.current_dir(home);
    }

    let output = command.output();
    match output {
        Ok(o) => {
            // Check both stderr and stdout for the error message
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);
            let not_recognized = stderr.contains("is not a git command") || stdout.contains("is not a git command");
            !not_recognized
        }
        Err(_) => false,
    }
}

/// Execute git with the provided arguments, showing passthrough message.
fn execute_passthrough(args: &[String]) -> Result<()> {
    eprintln!("Passing command through to git...");
    eprintln!("Running: \"git {}\"", args.join(" "));
    eprintln!();

    let status = Command::new("git").args(args).status();

    match status {
        Ok(s) => {
            if !s.success() {
                std::process::exit(s.code().unwrap_or(1));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("git is not installed or not in PATH")
        }
        Err(e) => Err(e.into()),
    }
}

/// Show Diamond help and exit with error code.
fn show_help_and_exit() -> ! {
    // Re-invoke ourselves with --help
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("--help").status();
    }
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_known_git_command_common_commands() {
        assert!(is_known_git_command("status"));
        assert!(is_known_git_command("diff"));
        assert!(is_known_git_command("log"));
        assert!(is_known_git_command("branch"));
        assert!(is_known_git_command("stash"));
        assert!(is_known_git_command("add"));
        assert!(is_known_git_command("commit"));
        assert!(is_known_git_command("push"));
        assert!(is_known_git_command("pull"));
        assert!(is_known_git_command("fetch"));
    }

    #[test]
    fn test_is_known_git_command_ancillary() {
        assert!(is_known_git_command("config"));
        assert!(is_known_git_command("remote"));
        assert!(is_known_git_command("reflog"));
        assert!(is_known_git_command("blame"));
        assert!(is_known_git_command("help"));
        assert!(is_known_git_command("version"));
    }

    #[test]
    fn test_is_known_git_command_unknown() {
        assert!(!is_known_git_command("notarealcommand"));
        assert!(!is_known_git_command("foobar"));
        assert!(!is_known_git_command(""));
        assert!(!is_known_git_command("xyz123"));
    }

    #[test]
    fn test_is_known_git_command_case_sensitive() {
        // Git commands are case-sensitive on most systems
        assert!(!is_known_git_command("STATUS"));
        assert!(!is_known_git_command("Status"));
        assert!(!is_known_git_command("DIFF"));
    }

    #[test]
    fn test_is_known_git_command_no_partial_match() {
        // Should not match partial command names
        assert!(!is_known_git_command("stat"));
        assert!(!is_known_git_command("statu"));
        assert!(!is_known_git_command("lo"));
    }

    #[test]
    fn test_is_git_recognized_command_valid() {
        // These should be recognized by git
        assert!(is_git_recognized_command("status"));
        assert!(is_git_recognized_command("version"));
    }

    #[test]
    fn test_is_git_recognized_command_invalid() {
        // These should not be recognized by git
        assert!(!is_git_recognized_command("notarealcommand"));
        assert!(!is_git_recognized_command("foobar123"));
    }
}
