use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

mod branch_tree;
mod cache;
mod commands;
mod completion;
mod config;
mod context;
mod forge;
mod git_backend;
mod git_gateway;
mod operation_log;
pub mod platform;
pub mod program_name;
mod ref_store;
mod stack_viz;
mod state;
#[cfg(test)]
mod test_context;
pub mod ui;
mod validation;
mod worktree;

#[derive(Parser)]
#[command(
    about = "Diamond: A CLI for stacked git changes",
    long_about = None,
    version,
    disable_help_subcommand = true,
    help_template = "\
{about}

{usage-heading} {usage}

Get Started:
  init        Initialize Diamond in your repo
  create      Create a new stacked branch                [c]
  log         Visualize your stack                       [l]

Core Workflow:
  modify      Stage changes and commit                   [m]
  submit      Push branches and create PRs               [s]
  sync        Rebase stack onto updated trunk

Navigate:
  checkout    Switch to a branch                         [co]
  up          Move to child branch                       [u]
  down        Move to parent branch                      [d]
  top         Jump to top of stack                       [t]
  bottom      Jump to bottom of stack                    [b]

Manage Stack:
  restack     Rebase branches locally
  move        Move branch to new parent
  fold        Merge branch into parent                   [f]
  split       Split branch into multiple                 [sp]
  squash      Squash commits in branch                   [sq]
  delete      Delete a branch
  reorder     Reorder branches interactively
  rename      Rename current branch
  absorb      Absorb staged changes into earlier commits

Pull Requests:
  get         Download a PR stack
  merge       Merge PRs from command line
  pr          Open PR in browser
  unlink      Unlink branch from PR

Recovery:
  continue    Resume interrupted operation               [cont]
  abort       Cancel and rollback operation
  undo        Restore branch from backup
  doctor      Diagnose and repair metadata

Maintenance:
  cleanup     Remove merged branches
  gc          Clean up old backup refs
  history     View operation history

Collaboration:
  freeze      Prevent modifications to branch
  unfreeze    Allow modifications to branch
  pop         Delete branch, keep changes

Setup:
  track       Start tracking a branch
  untrack     Stop tracking a branch                     [utr]
  trunk       Show or set trunk branch
  config      Configuration settings                     [cfg]
  completion  Generate shell completions

Info:
  info        Show branch details
  parent      Show parent branch
  children    Show child branches

Options:
  -v, --verbose  Show git commands being executed
  -n, --dry-run  Preview without executing
  -h, --help     Print help
  -V, --version  Print version

Run '{bin} <command> --help' for more information on a command.
"
)]
pub struct Cli {
    /// Show git commands being executed
    #[arg(short = 'v', long, global = true)]
    verbose: bool,

    /// Preview destructive operations without executing them
    #[arg(short = 'n', long, global = true)]
    dry_run: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    // ═══════════════════════════════════════════════════════════════════════════
    // GET STARTED
    // ═══════════════════════════════════════════════════════════════════════════
    #[command(next_help_heading = "Get Started")]
    /// Initialize Diamond in your repo
    Init {
        /// Trunk branch name (defaults to main/master if found)
        #[arg(long)]
        trunk: Option<String>,
        /// Reset Diamond (untrack all branches and reinitialize)
        #[arg(long)]
        reset: bool,
    },
    /// Create a new stacked branch
    #[command(
        visible_alias = "c",
        after_help = "\
Examples:
  create feature           Create branch named 'feature'
  create -m \"Add login\"    Create with commit message
  create -am \"Fix bug\"     Stage all changes and commit"
    )]
    Create {
        /// Name of the new branch (auto-generated from message if not provided)
        name: Option<String>,
        /// Stage all changes
        #[arg(short = 'a', long)]
        all: bool,
        /// Stage only updates to already-tracked files (like git add -u)
        #[arg(short = 'u', long)]
        update: bool,
        /// Commit message
        #[arg(short = 'm', long)]
        message: Option<String>,
        /// Insert between current branch and its child (auto-detects if one child, or specify child explicitly)
        #[arg(short = 'i', long, value_name = "CHILD", num_args = 0..=1, default_missing_value = "")]
        insert: Option<String>,
    },
    /// Visualize your stack
    #[command(visible_alias = "l")]
    #[command(visible_alias = "ls")]
    #[command(visible_alias = "ll")]
    Log {
        /// Output mode: 'short' for simple text, 'long' for detailed, omit for TUI
        mode: Option<String>,
    },

    // ═══════════════════════════════════════════════════════════════════════════
    // CORE WORKFLOW
    // ═══════════════════════════════════════════════════════════════════════════
    #[command(next_help_heading = "Core Workflow")]
    /// Stage changes and commit
    #[command(
        visible_alias = "m",
        after_help = "\
Examples:
  modify -a                Stage all and amend last commit
  modify -am \"New msg\"     Amend with new message
  modify -c -m \"New\"       Create new commit (don't amend)
  modify --into feature    Amend into a downstack branch"
    )]
    Modify {
        /// Stage all changes
        #[arg(short = 'a', long)]
        all: bool,
        /// Stage only updates to already-tracked files (like git add -u)
        #[arg(short = 'u', long)]
        update: bool,
        /// Commit message
        #[arg(short = 'm', long)]
        message: Option<String>,
        /// Create new commit instead of amending
        #[arg(short = 'c', long)]
        commit: bool,
        /// Edit commit message in editor
        #[arg(short = 'e', long)]
        edit: bool,
        /// Reset the author of the commit to the current user
        #[arg(long)]
        reset_author: bool,
        /// Open interactive rebase from parent branch
        #[arg(short = 'i', long)]
        interactive_rebase: bool,
        /// Amend changes into a downstack branch instead of current
        #[arg(long, value_name = "BRANCH")]
        into: Option<String>,
    },
    /// Push branches and create PRs
    #[command(
        visible_alias = "s",
        after_help = "\
Examples:
  submit                   Submit current branch
  submit --stack           Submit entire stack
  submit -r @alice -r @bob Add reviewers
  submit -d                Create as draft PR
  submit -m                Enable auto-merge when CI passes"
    )]
    Submit {
        /// Submit entire stack (ancestors and descendants)
        #[arg(long)]
        stack: bool,
        /// Force push (instead of --force-with-lease)
        #[arg(short = 'f', long)]
        force: bool,
        /// Create PR as draft
        #[arg(short = 'd', long, conflicts_with = "publish")]
        draft: bool,
        /// Publish draft PRs (mark as ready for review)
        #[arg(short = 'p', long, conflicts_with = "draft")]
        publish: bool,
        /// Enable auto-merge when CI passes (uses squash by default)
        #[arg(short = 'm', long)]
        merge_when_ready: bool,
        /// Submit a specific branch (defaults to current)
        #[arg(short = 'b', long, value_name = "BRANCH")]
        branch: Option<String>,
        /// Add reviewers (can be specified multiple times)
        #[arg(short = 'r', long = "reviewer", value_name = "USERNAME")]
        reviewers: Vec<String>,
        /// Don't open PR URLs in browser after creation
        #[arg(long)]
        no_open: bool,
        /// Skip stack integrity validation before submitting
        #[arg(long)]
        skip_validation: bool,
        /// Only push branches that already have PRs (don't create new PRs)
        #[arg(long)]
        update_only: bool,
        /// Show what would be submitted and ask for confirmation
        #[arg(long)]
        confirm: bool,
    },
    /// Submit entire stack including descendants (shorthand for submit --stack)
    #[command(hide = true)]
    Ss {
        /// Force push (instead of --force-with-lease)
        #[arg(short = 'f', long)]
        force: bool,
        /// Create PR as draft
        #[arg(short = 'd', long, conflicts_with = "publish")]
        draft: bool,
        /// Publish draft PRs (mark as ready for review)
        #[arg(short = 'p', long, conflicts_with = "draft")]
        publish: bool,
        /// Enable auto-merge when CI passes (uses squash by default)
        #[arg(short = 'm', long)]
        merge_when_ready: bool,
        /// Submit a specific branch (defaults to current)
        #[arg(short = 'b', long, value_name = "BRANCH")]
        branch: Option<String>,
        /// Add reviewers (can be specified multiple times)
        #[arg(short = 'r', long = "reviewer", value_name = "USERNAME")]
        reviewers: Vec<String>,
        /// Don't open PR URLs in browser after creation
        #[arg(long)]
        no_open: bool,
        /// Skip stack integrity validation before submitting
        #[arg(long)]
        skip_validation: bool,
        /// Only push branches that already have PRs (don't create new PRs)
        #[arg(long)]
        update_only: bool,
        /// Show what would be submitted and ask for confirmation
        #[arg(long)]
        confirm: bool,
    },
    /// Rebase stack onto updated trunk
    #[command(after_help = "\
Examples:
  sync                     Fetch trunk and rebase all branches
  sync --continue          Continue after resolving conflicts
  sync --abort             Cancel sync and rollback")]
    Sync {
        /// Continue after resolving conflicts
        #[arg(long, visible_alias = "continue")]
        continue_sync: bool,
        /// Abort the current sync
        #[arg(long)]
        abort: bool,
        /// Proceed even if external changes detected
        #[arg(short = 'f', long)]
        force: bool,
        /// Skip cleanup of merged branches entirely
        #[arg(long)]
        no_cleanup: bool,
        /// Keep merged branches instead of deleting them
        #[arg(long)]
        keep: bool,
        /// Skip automatic restack after sync
        #[arg(long)]
        no_restack: bool,
        /// Show detailed output for all branches (including up-to-date)
        #[arg(short = 'v', long)]
        verbose: bool,
    },

    // ═══════════════════════════════════════════════════════════════════════════
    // NAVIGATE STACK
    // ═══════════════════════════════════════════════════════════════════════════
    #[command(next_help_heading = "Navigate Stack")]
    /// Switch to a branch
    #[command(visible_alias = "co")]
    Checkout {
        /// Name of the branch to checkout
        name: Option<String>,
        /// Go directly to trunk branch
        #[arg(short = 't', long)]
        trunk: bool,
        /// Show only current stack branches in selection
        #[arg(short = 's', long)]
        stack: bool,
        /// Show all trunks in selection
        #[arg(short = 'a', long)]
        all: bool,
        /// Include untracked branches in selection
        #[arg(short = 'u', long)]
        untracked: bool,
    },
    /// Move to child branch
    #[command(visible_alias = "u")]
    Up {
        /// Number of steps to move (default: 1)
        #[arg(default_value = "1")]
        steps: usize,
        /// Navigate directly to a specific upstack branch
        #[arg(long, value_name = "BRANCH")]
        to: Option<String>,
    },
    /// Move to parent branch
    #[command(visible_alias = "d")]
    Down {
        /// Number of steps to move (default: 1)
        #[arg(default_value = "1")]
        steps: usize,
    },
    /// Jump to top of stack
    #[command(visible_alias = "t")]
    Top,
    /// Jump to bottom of stack
    #[command(visible_alias = "b")]
    Bottom,

    // ═══════════════════════════════════════════════════════════════════════════
    // MANAGE STACK
    // ═══════════════════════════════════════════════════════════════════════════
    #[command(next_help_heading = "Manage Stack")]
    /// Rebase branches locally
    Restack {
        /// Branch to start from (default: current branch)
        #[arg(short = 'b', long)]
        branch: Option<String>,
        /// Restack only this branch (no descendants)
        #[arg(long)]
        only: bool,
        /// Restack ancestors down to trunk
        #[arg(long)]
        downstack: bool,
        /// Restack descendants (default behavior when branch is specified)
        #[arg(long)]
        upstack: bool,
        /// Proceed even if external changes detected
        #[arg(long)]
        force: bool,
        /// Skip branches with approved PRs
        #[arg(long)]
        skip_approved: bool,
    },
    /// Move branch to new parent
    Move {
        /// Target parent branch
        #[arg(long)]
        onto: Option<String>,
        /// Branch to move (defaults to current branch)
        #[arg(long)]
        source: Option<String>,
    },
    /// Merge branch into parent
    #[command(visible_alias = "f")]
    Fold {
        /// Keep current branch name instead of parent's name
        #[arg(short = 'k', long)]
        keep: bool,
    },
    /// Split branch into multiple
    #[command(
        visible_alias = "sp",
        after_help = "\
Examples:
  split --by-commit            Each commit becomes a branch
  split --by-file \"*.test.ts\"  Extract test files to parent
  split --by-hunk              Interactive hunk selection"
    )]
    Split {
        /// Name for the new branch (when using legacy mode)
        new_branch: Option<String>,
        /// Commit to split at (when using legacy mode, e.g., HEAD~2, abc123)
        commit: Option<String>,
        /// Split by commit - creates a branch for each commit in the stack
        #[arg(short = 'c', long = "by-commit", conflicts_with_all = ["by_file", "by_hunk"])]
        by_commit: bool,
        /// Split by file - extracts files matching patterns into a new parent branch
        #[arg(short = 'f', long = "by-file", num_args = 1.., conflicts_with_all = ["by_commit", "by_hunk"])]
        by_file: Option<Vec<String>>,
        /// Split by hunk - interactively select hunks for new branches (requires TTY)
        #[arg(short = 'H', long = "by-hunk", conflicts_with_all = ["by_commit", "by_file"])]
        by_hunk: bool,
    },
    /// Squash commits in branch
    #[command(visible_alias = "sq")]
    Squash {
        /// Commit message for the squashed commit
        #[arg(short = 'm', long)]
        message: Option<String>,
    },
    /// Delete a branch
    Delete {
        /// Branch name to delete (interactive if not provided)
        name: Option<String>,
        /// Re-parent children to deleted branch's parent
        #[arg(long, default_value = "true")]
        reparent: bool,
        /// Force delete even if branch is not merged
        #[arg(short = 'f', long)]
        force: bool,
        /// Delete branch and all descendants
        #[arg(long, conflicts_with = "downstack")]
        upstack: bool,
        /// Delete branch and all ancestors (except trunk)
        #[arg(long, conflicts_with = "upstack")]
        downstack: bool,
    },
    /// Reorder branches interactively
    Reorder {
        /// Read new order from file instead of opening editor
        #[arg(long)]
        file: Option<String>,
        /// Show current order without opening editor
        #[arg(long)]
        preview: bool,
    },
    /// Rename current branch
    Rename {
        /// New name for the branch
        name: Option<String>,
        /// Only rename locally (don't update remote)
        #[arg(long)]
        local: bool,
        /// Force rename even when a PR is open
        #[arg(short = 'f', long)]
        force: bool,
    },
    /// Absorb staged changes into earlier commits
    Absorb {
        /// Stage all changes before absorbing
        #[arg(short = 'a', long)]
        all: bool,
        /// Skip confirmation prompts
        #[arg(short = 'f', long)]
        force: bool,
    },

    // ═══════════════════════════════════════════════════════════════════════════
    // PULL REQUESTS
    // ═══════════════════════════════════════════════════════════════════════════
    #[command(next_help_heading = "Pull Requests")]
    /// Download a PR stack
    #[command(after_help = "\
Examples:
  get 123                  Download PR #123
  get https://...pull/123  Download by URL
  get 123 -U               Download without freezing")]
    Get {
        /// PR reference (URL or number)
        pr: String,
        /// Overwrite local branches with remote (discard local changes)
        #[arg(short = 'f', long)]
        force: bool,
        /// Don't freeze downloaded branches (allow immediate editing)
        #[arg(short = 'U', long)]
        unfrozen: bool,
    },
    /// Merge PRs from command line
    Merge {
        /// Use merge commit instead of squash (default is squash)
        #[arg(long, conflicts_with = "rebase")]
        merge: bool,
        /// Use rebase merge instead of squash (default is squash)
        #[arg(long, conflicts_with = "merge")]
        rebase: bool,
        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
        /// Don't sync local branches after merging
        #[arg(long)]
        no_sync: bool,
        /// Skip waiting for CI (still proactively rebase)
        #[arg(long)]
        no_wait: bool,
        /// Fast mode: skip proactive rebase and CI wait (reactive-only behavior)
        #[arg(long)]
        fast: bool,
        /// Keep merged branches instead of deleting them
        #[arg(long)]
        keep: bool,
    },
    /// Open PR in browser
    Pr {
        /// Branch name or PR number (defaults to current branch)
        branch: Option<String>,
    },
    /// Unlink branch from PR
    Unlink,

    // ═══════════════════════════════════════════════════════════════════════════
    // RECOVERY
    // ═══════════════════════════════════════════════════════════════════════════
    #[command(next_help_heading = "Recovery")]
    /// Resume interrupted operation
    #[command(visible_alias = "cont")]
    Continue,
    /// Cancel and rollback operation
    Abort,
    /// Restore branch from backup
    Undo {
        /// Branch to restore (restores last operation if not provided)
        branch: Option<String>,
        /// List all available backups
        #[arg(long)]
        list: bool,
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
    /// Diagnose and repair metadata
    Doctor {
        /// Automatically fix detected issues
        #[arg(long)]
        fix: bool,
        /// Update stack visualization in all PRs
        #[arg(long)]
        fix_viz: bool,
    },

    // ═══════════════════════════════════════════════════════════════════════════
    // MAINTENANCE
    // ═══════════════════════════════════════════════════════════════════════════
    #[command(next_help_heading = "Maintenance")]
    /// Remove merged branches
    Cleanup {
        /// Skip confirmation prompt
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// Clean up old backup refs
    Gc {
        /// Maximum age of backups to keep in days (default: 30)
        #[arg(long, value_name = "DAYS")]
        max_age: Option<u64>,
        /// Maximum number of backups to keep per branch (default: 10)
        #[arg(long, value_name = "COUNT")]
        keep: Option<usize>,
        /// Show what would be deleted without actually deleting
        #[arg(long)]
        dry_run: bool,
    },
    /// View operation history
    History {
        /// Number of entries to show (default: 20, use 0 for all)
        #[arg(long, short = 'c')]
        count: Option<usize>,
        /// Show all entries
        #[arg(long)]
        all: bool,
    },

    // ═══════════════════════════════════════════════════════════════════════════
    // COLLABORATION
    // ═══════════════════════════════════════════════════════════════════════════
    #[command(next_help_heading = "Collaboration")]
    /// Prevent modifications to branch
    Freeze {
        /// Branch to freeze (defaults to current)
        branch: Option<String>,
    },
    /// Allow modifications to branch
    Unfreeze {
        /// Branch to unfreeze (defaults to current)
        branch: Option<String>,
        /// Also unfreeze all upstack branches
        #[arg(long)]
        upstack: bool,
    },
    /// Delete branch, keep changes
    Pop,

    // ═══════════════════════════════════════════════════════════════════════════
    // SETUP
    // ═══════════════════════════════════════════════════════════════════════════
    #[command(next_help_heading = "Setup")]
    /// Start tracking a branch
    Track {
        /// Branch name to track (defaults to current branch)
        branch: Option<String>,
        /// Parent branch for the tracked branch
        #[arg(short = 'p', long)]
        parent: Option<String>,
    },
    /// Stop tracking a branch
    #[command(visible_alias = "utr")]
    Untrack { branch: Option<String> },
    /// Show or set trunk branch
    Trunk {
        /// Set the trunk branch to this value
        #[arg(long, value_name = "BRANCH")]
        set: Option<String>,
    },
    /// Configuration settings
    #[command(visible_alias = "cfg")]
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// Generate shell completions
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::shells::Shell,
    },

    // ═══════════════════════════════════════════════════════════════════════════
    // INFO
    // ═══════════════════════════════════════════════════════════════════════════
    #[command(next_help_heading = "Info")]
    /// Show branch details
    Info {
        /// Branch to show info for (defaults to current)
        branch: Option<String>,
    },
    /// Show parent branch
    Parent,
    /// Show child branches
    Children,

    /// Pass through to git for native git commands
    #[command(external_subcommand, hide = true)]
    External(Vec<String>),
}

/// Config subcommands
#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,
    /// Get a configuration value
    Get {
        /// Config key (e.g., branch.format, branch.prefix)
        key: String,
    },
    /// Set a configuration value
    Set {
        /// Config key (e.g., branch.format, branch.prefix)
        key: String,
        /// Value to set
        value: String,
        /// Set in local config (.git/diamond/) instead of user config
        #[arg(long)]
        local: bool,
    },
    /// Unset a configuration value
    Unset {
        /// Config key to unset
        key: String,
        /// Unset in local config (.git/diamond/) instead of user config
        #[arg(long)]
        local: bool,
    },
}

/// Install signal handler for graceful interruption
fn install_signal_handler() {
    ctrlc::set_handler(|| {
        // OperationState is saved at each checkpoint during sync/restack operations
        // so we can simply inform the user about recovery options
        eprintln!("\n\nOperation interrupted. Run:");
        eprintln!("  {} continue   to continue", program_name::program_name());
        eprintln!("  {} abort      to rollback", program_name::program_name());
        std::process::exit(130);
    })
    .expect("Error setting Ctrl-C handler");
}

#[tokio::main]
async fn main() {
    // Install signal handler for graceful interruption
    install_signal_handler();

    let prog_name = program_name::program_name();
    let matches = Cli::command().name(prog_name).get_matches();
    let cli = Cli::from_arg_matches(&matches).expect("Failed to parse arguments");

    // Initialize global execution context
    // Thread-local for backward compatibility with sync code
    context::ExecutionContext::init(cli.verbose, cli.dry_run);
    // Task-local for proper async context propagation
    let ctx = context::ExecutionContext::new(cli.verbose, cli.dry_run);

    // Require a subcommand
    let command = match &cli.command {
        Some(cmd) => cmd,
        None => {
            eprintln!("No command provided. Run '{} --help' for usage.", prog_name);
            std::process::exit(1);
        }
    };

    // Wrap command execution with async context for proper propagation across await points
    let result = context::with_context(ctx, async {
        match command {
            Commands::Init { trunk, reset } => commands::init::run(trunk.clone(), *reset),
            Commands::Create {
                name,
                all,
                update,
                message,
                insert,
            } => commands::create::run(name.clone(), *all, *update, message.clone(), insert.clone()),
            Commands::Checkout {
                name,
                trunk,
                stack,
                all,
                untracked,
            } => commands::checkout::run(name.clone(), *trunk, *stack, *all, *untracked),
            Commands::Log { mode } => commands::log::run(mode.clone()),
            Commands::Track { branch, parent } => commands::track::run_track(branch.clone(), parent.clone()),
            Commands::Untrack { branch } => commands::track::run_untrack(branch.clone()),
            Commands::Down { steps } => commands::down::run(*steps),
            Commands::Up { steps, to } => commands::up::run(*steps, to.clone()),
            Commands::Delete {
                name,
                reparent,
                force,
                upstack,
                downstack,
            } => commands::delete::run(name.clone(), *reparent, *force, *upstack, *downstack),
            Commands::Fold { keep } => commands::fold::run(*keep),
            Commands::Modify {
                all,
                update,
                message,
                commit,
                edit,
                reset_author,
                interactive_rebase,
                into,
            } => commands::modify::run(
                *all,
                *update,
                message.clone(),
                *commit,
                *edit,
                *reset_author,
                *interactive_rebase,
                into.clone(),
            ),
            Commands::Submit {
                stack,
                force,
                draft,
                publish,
                merge_when_ready,
                branch,
                reviewers,
                no_open,
                skip_validation,
                update_only,
                confirm,
            } => {
                commands::submit::run(
                    *stack,
                    *force,
                    *draft,
                    *publish,
                    *merge_when_ready,
                    branch.clone(),
                    reviewers.clone(),
                    *no_open,
                    *skip_validation,
                    *update_only,
                    *confirm,
                )
                .await
            }
            Commands::Ss {
                force,
                draft,
                publish,
                merge_when_ready,
                branch,
                reviewers,
                no_open,
                skip_validation,
                update_only,
                confirm,
            } => {
                commands::submit::run(
                    true, // stack = true
                    *force,
                    *draft,
                    *publish,
                    *merge_when_ready,
                    branch.clone(),
                    reviewers.clone(),
                    *no_open,
                    *skip_validation,
                    *update_only,
                    *confirm,
                )
                .await
            }
            Commands::Sync {
                continue_sync,
                abort,
                force,
                no_cleanup,
                keep,
                no_restack,
                verbose,
            } => {
                commands::sync::run(
                    *continue_sync,
                    *abort,
                    *force,
                    *no_cleanup,
                    *keep,
                    !*no_restack,
                    *verbose,
                )
                .await
            }
            Commands::Get { pr, force, unfrozen } => commands::get::run(pr.clone(), *force, *unfrozen),
            Commands::Pr { branch } => commands::pr::run(branch.clone()),
            Commands::Pop => commands::pop::run(),
            Commands::Freeze { branch } => commands::freeze::run(branch.clone()),
            Commands::Unfreeze { branch, upstack } => commands::unfreeze::run(branch.clone(), *upstack),
            Commands::Unlink => commands::unlink::run(),
            Commands::Merge {
                merge,
                rebase,
                yes,
                no_sync,
                no_wait,
                fast,
                keep,
            } => {
                let method = if *merge {
                    forge::MergeMethod::Merge
                } else if *rebase {
                    forge::MergeMethod::Rebase
                } else {
                    forge::MergeMethod::Squash // default
                };
                let dry_run = crate::context::ExecutionContext::is_dry_run();
                commands::merge::run(method, dry_run, *yes, *no_sync, *no_wait, *fast, *keep).await
            }
            Commands::Move { onto, source } => commands::move_cmd::run(onto.clone(), source.clone()),
            Commands::Continue => commands::continue_op::run(),
            Commands::Abort => commands::abort::run(),
            Commands::Absorb { all, force } => commands::absorb::run(*all, *force),
            Commands::Top => commands::top::run(),
            Commands::Bottom => commands::bottom::run(),
            Commands::Reorder { file, preview } => commands::reorder::run(file.clone(), *preview),
            Commands::Restack {
                branch,
                only,
                downstack,
                upstack,
                force,
                skip_approved,
            } => {
                commands::restack::run(
                    branch.clone(),
                    *only,
                    *downstack,
                    *upstack,
                    *force,
                    *skip_approved,
                    false,
                )
                .await
            }
            Commands::Squash { message } => commands::squash::run(message.clone()),
            Commands::Split {
                new_branch,
                commit,
                by_commit,
                by_file,
                by_hunk,
            } => commands::split::run(
                new_branch.clone(),
                commit.clone(),
                *by_commit,
                by_file.clone(),
                *by_hunk,
            ),
            Commands::Rename { name, local, force } => commands::rename::run(name.clone(), *local, *force),
            Commands::Info { branch } => commands::info::run(branch.clone()),
            Commands::Parent => commands::info::run_parent(),
            Commands::Children => commands::info::run_children(),
            Commands::Trunk { set } => commands::info::run_trunk(set.clone()),
            Commands::Config { action } => match action {
                Some(ConfigAction::Show) => commands::config_cmd::show(),
                Some(ConfigAction::Get { key }) => commands::config_cmd::get(key),
                Some(ConfigAction::Set { key, value, local }) => commands::config_cmd::set(key, value, *local),
                Some(ConfigAction::Unset { key, local }) => commands::config_cmd::unset(key, *local),
                None => commands::config_cmd::show(), // Default to show
            },
            Commands::Doctor { fix, fix_viz } => commands::doctor::run(*fix, *fix_viz),
            Commands::Gc { max_age, keep, dry_run } => commands::gc::run(*max_age, *keep, *dry_run),
            Commands::Cleanup { force } => commands::cleanup::run(*force),
            Commands::Undo { branch, list, force } => commands::undo::run(branch.clone(), *list, *force),
            Commands::History { count, all } => commands::history::run(if *all { Some(0) } else { *count }),
            Commands::Completion { shell } => commands::completion::run(*shell),
            Commands::External(args) => commands::passthrough::run(args.clone()),
        }
    })
    .await;

    if let Err(e) = result {
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}
