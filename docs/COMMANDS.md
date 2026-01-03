# Diamond Command Reference

Complete reference for all Diamond commands.

## Table of Contents

- [Global Options](#global-options)
- [Core Workflow](#core-workflow)
- [Navigation](#navigation)
- [Stack Management](#stack-management)
- [Conflict Resolution](#conflict-resolution)
- [Maintenance](#maintenance)
- [Utility Commands](#utility-commands)
- [Configuration](#configuration-commands)
- [Aliases](#aliases)

---

## Global Options

These flags work with most Diamond commands:

| Flag | Short | Description |
|------|-------|-------------|
| `--verbose` | `-v` | Show git commands being executed |
| `--dry-run` | `-n` | Preview destructive operations without executing them |
| `--help` | `-h` | Print help for command |

---

## Core Workflow

### dm init
Initialize Diamond in this repository.

```bash
dm init
dm init --trunk develop       # Specify custom trunk branch
dm init --reset               # Reset all tracking and reinitialize
```

**Options:**

| Flag | Description |
|------|-------------|
| `--trunk <BRANCH>` | Trunk branch name (defaults to main/master if found) |
| `--reset` | Reset Diamond (untrack all branches and reinitialize) |

**What it does:**
- Creates Diamond metadata in git refs
- Sets up stack tracking for the repository
- Detects trunk branch (main/master) or uses `--trunk` value

---

### dm create (alias: c)
Create a new branch in the stack.

```bash
dm create feature-name                    # Create branch with name
dm create -am "Add login"                 # Stage all and commit with message
dm create -um "Fix bug"                   # Stage tracked files and commit
dm create --insert                        # Insert between current and child
dm create --insert=child-branch           # Insert before specific child
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[NAME]` | Name of new branch (auto-generated from message if not provided) |

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--all` | `-a` | Stage all changes |
| `--update` | `-u` | Stage only updates to already-tracked files (like `git add -u`) |
| `--message <MSG>` | `-m` | Commit message |
| `--insert [CHILD]` | `-i` | Insert between current branch and its child (auto-detects if one child) |

**What it does:**
- Creates new branch from current HEAD
- Automatically tracks it in Diamond
- Records current branch as parent
- Optionally stages and commits changes

---

### dm modify (alias: m)
Modify the current branch by amending or creating a new commit.

```bash
dm modify -a                              # Stage all and amend
dm modify -am "Updated message"           # Stage all and amend with new message
dm modify -c -m "New commit"              # Create new commit (not amend)
dm modify --into feature-1                # Amend changes into downstack branch
dm modify -e                              # Edit commit message in editor
dm modify --interactive-rebase            # Open interactive rebase
dm modify --reset-author                  # Reset author to current user
```

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--all` | `-a` | Stage all changes |
| `--update` | `-u` | Stage only updates to already-tracked files (like `git add -u`) |
| `--message <MSG>` | `-m` | Commit message |
| `--commit` | `-c` | Create new commit instead of amending |
| `--edit` | `-e` | Edit commit message in editor |
| `--reset-author` | | Reset the author of the commit to the current user |
| `--interactive-rebase` | `-i` | Open interactive rebase from parent branch |
| `--into <BRANCH>` | | Amend changes into a downstack branch instead of current |

**What it does:**
- Stages changes if requested
- Amends current commit or creates new commit
- Updates branch metadata
- Automatically restacks children if needed

---

### dm submit (alias: s)
Submit branch(es) by pushing and creating pull requests.

```bash
dm submit                     # Submit current branch
dm submit --stack             # Submit entire stack (ancestors + descendants, alias: ss)
dm submit --force             # Force push
dm submit -d                  # Create as draft
dm submit -p                  # Publish draft (mark ready for review)
dm submit -m                  # Enable auto-merge when CI passes
dm submit -r @user1 -r @user2 # Add reviewers
dm submit --update-only       # Only update existing PRs
dm submit --confirm           # Ask for confirmation before submitting
```

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--stack` | | Submit entire stack (ancestors and descendants) |
| `--force` | `-f` | Force push (instead of `--force-with-lease`) |
| `--draft` | `-d` | Create PR as draft |
| `--publish` | `-p` | Publish draft PRs (mark as ready for review) |
| `--merge-when-ready` | `-m` | Enable auto-merge when CI passes (uses squash) |
| `--branch <BRANCH>` | `-b` | Submit a specific branch (defaults to current) |
| `--reviewer <USERNAME>` | `-r` | Add reviewers (can be specified multiple times) |
| `--no-open` | | Don't open PR URLs in browser after creation |
| `--skip-validation` | | Skip stack integrity validation before submitting |
| `--update-only` | | Only push branches that already have PRs |
| `--confirm` | | Show what would be submitted and ask for confirmation |

**What it does:**
- By default, submits only the current branch
- With `--stack`, submits entire stack (ancestors and descendants)
- Creates PRs (GitHub) or MRs (GitLab) with proper base branches
- Adds stack visualization to PR descriptions
- Updates PR URLs in metadata

---

### dm absorb
Absorb staged changes into earlier commits in the current stack.

```bash
dm absorb                     # Absorb staged changes
dm absorb -a                  # Stage all, then absorb
dm absorb --dry-run           # Preview absorption targets
dm absorb -f                  # Absorb without confirmation
```

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--all` | `-a` | Stage all changes before absorbing |
| `--force` | `-f` | Skip confirmation prompts |

**What it does:**
- Analyzes staged changes and matches them to relevant commits
- Amends changes into the appropriate commits
- Automatically restacks affected branches

---

## Navigation

### dm log (alias: l, ls, ll)
Visualize the stack hierarchy.

```bash
dm log                        # Open TUI (interactive)
dm log short                  # Simple text output (alias: dm ls)
dm log long                   # Detailed output (alias: dm ll)
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[MODE]` | Output mode: `short` for simple text, `long` for detailed, omit for TUI |

**What it does:**
- Opens TUI (text user interface) showing stack tree
- Displays parent-child relationships
- Shows PR status and metadata
- Press `q` to exit TUI mode

---

### dm checkout (alias: co)
Switch to a branch.

```bash
dm checkout                   # Interactive branch selection
dm checkout feature-1         # Checkout specific branch
dm checkout -t                # Checkout trunk branch
dm checkout -s                # Select from current stack only
dm checkout -u                # Include untracked branches
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[NAME]` | Name of branch to checkout |

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--trunk` | `-t` | Go directly to trunk branch |
| `--stack` | `-s` | Show only current stack branches in selection |
| `--all` | `-a` | Show all trunks in selection |
| `--untracked` | `-u` | Include untracked branches in selection |

**What it does:**
- Switches to specified branch
- Opens interactive picker if no branch specified

---

### dm up (alias: u)
Move to child branch (up the stack).

```bash
dm up                         # Move up one branch
dm up 2                       # Move up two branches
dm up --to feature-3          # Navigate to specific upstack branch
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[STEPS]` | Number of steps to move (default: 1) |

**Options:**

| Flag | Description |
|------|-------------|
| `--to <BRANCH>` | Navigate directly to a specific upstack branch |

**What it does:**
- Switches to child branch
- Alphabetically sorted if multiple children
- Can navigate multiple levels or to a specific branch

---

### dm down (alias: d)
Move to parent branch (down the stack).

```bash
dm down                       # Move down one branch
dm down 3                     # Move down three branches
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[STEPS]` | Number of steps to move (default: 1) |

**What it does:**
- Switches to parent branch
- Fails if current branch has no tracked parent

---

### dm top (alias: t)
Jump to the tip of the current stack.

```bash
dm top
```

**What it does:**
- Finds deepest descendant in current stack
- Switches to that branch

---

### dm bottom (alias: b)
Jump to the bottom of the current stack.

```bash
dm bottom
```

**What it does:**
- Finds root of current stack (branch parented by trunk)
- Switches to that branch

---

## Stack Management

### dm sync
Sync stacks by rebasing onto updated trunk and restacking all branches.

```bash
dm sync                       # Start sync (restacks automatically)
dm sync --no-restack          # Sync without restacking
dm sync --continue            # Continue after resolving conflicts
dm sync --abort               # Abort sync operation
dm sync --no-cleanup          # Skip cleanup prompt for merged branches
```

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--continue-sync` | | Continue after resolving merge conflicts (alias: `--continue`) |
| `--abort` | | Abort the current sync |
| `--force` | `-f` | Proceed even if external changes detected |
| `--no-cleanup` | | Skip cleanup prompt for merged branches |
| `--no-restack` | | Skip automatic restack after sync |

**What it does:**
- Fetches from origin
- Fast-forwards trunk branch
- Creates backup refs for all affected branches
- Rebases all stack branches onto updated trunk
- Automatically restacks all branches after sync (use `--no-restack` to skip)
- Records operation in history log

**Requires clean working tree**

---

### dm restack
Restack branches without fetching from remote.

```bash
dm restack                    # Restack entire stack
dm restack --only             # Restack only current branch
dm restack --upstack          # Restack current + descendants
dm restack --downstack        # Restack current + ancestors
dm restack --skip-approved    # Skip branches with approved PRs
```

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--branch <BRANCH>` | `-b` | Branch to start from (default: current branch) |
| `--only` | | Restack only this branch (no descendants) |
| `--downstack` | | Restack ancestors down to trunk |
| `--upstack` | | Restack descendants (default when branch is specified) |
| `--force` | | Proceed even if external changes detected |
| `--skip-approved` | | Skip branches with approved PRs |

**What it does:**
- Creates backup refs for all affected branches
- Rebases stack branches onto their parents
- Useful after amending commits in parent branches
- Records operation in history log

**Requires clean working tree**

---

### dm move
Move branch to a new parent.

```bash
dm move                       # Interactive target selection
dm move --onto main           # Move current branch onto main
dm move --source feat-1 --onto feat-2  # Move feat-1 onto feat-2
```

**Options:**

| Flag | Description |
|------|-------------|
| `--onto <BRANCH>` | Target parent branch |
| `--source <BRANCH>` | Branch to move (defaults to current branch) |

**What it does:**
- Creates backup refs for branch and all descendants
- Moves branch and all descendants to new parent
- Updates parent-child relationships in metadata
- Records operation in history log

**Requires clean working tree**

---

### dm reorder
Interactively reorder branches in the downstack.

```bash
dm reorder                    # Open editor to reorder
dm reorder --preview          # Show current order without editing
dm reorder --file order.txt   # Read new order from file
```

**Options:**

| Flag | Description |
|------|-------------|
| `--file <FILE>` | Read new order from file instead of opening editor |
| `--preview` | Show current order without opening editor |

**What it does:**
- Opens an interactive editor to reorder branches
- Rebases branches in new order
- Maintains stack integrity

---

### dm fold (alias: f)
Fold a branch's changes into its parent.

```bash
dm fold                       # Fold into parent (use parent's name)
dm fold --keep                # Fold but keep current branch name
```

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--keep` | `-k` | Keep current branch name instead of parent's name |

**What it does:**
- Merges current branch's commits into parent
- Reparents children to the surviving branch
- Deletes the folded branch
- Automatically restacks descendants

---

### dm split (alias: sp)
Split branch into multiple branches.

```bash
dm split --by-commit          # Each commit becomes a branch
dm split --by-file '*.test.ts'  # Extract test files to parent branch
dm split --by-hunk            # Interactive hunk selection
dm split new-branch HEAD~2    # Legacy: split at specific commit
```

**Arguments (legacy mode):**

| Argument | Description |
|----------|-------------|
| `[NEW_BRANCH]` | Name for the new branch |
| `[COMMIT]` | Commit to split at (e.g., HEAD~2, abc123) |

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--by-commit` | `-c` | Split by commit - creates a branch for each commit |
| `--by-file <PATTERNS>` | `-f` | Split by file - extracts files matching patterns into new parent branch |
| `--by-hunk` | `-H` | Split by hunk - interactively select hunks for new branches (requires TTY) |

**What it does:**
- Creates new branches from portions of current branch
- Maintains stack integrity
- Automatically restacks as needed

---

### dm squash (alias: sq)
Squash all commits in current branch into a single commit.

```bash
dm squash                     # Squash with default message
dm squash -m "Combined changes"  # Squash with custom message
```

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--message <MSG>` | `-m` | Commit message for the squashed commit |

**What it does:**
- Combines all commits since parent into one
- Preserves changes in working tree
- Automatically restacks children if needed

---

### dm pop
Delete current branch but retain file state.

```bash
dm pop
```

**What it does:**
- Deletes current branch
- Keeps changes in working tree
- Reparents children to current branch's parent

---

### dm delete
Delete a branch.

```bash
dm delete                     # Interactive deletion
dm delete feature-name        # Delete specific branch
dm delete -f my-branch        # Force delete unmerged branch
dm delete --upstack           # Delete branch and all descendants
dm delete --downstack         # Delete branch and all ancestors
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[NAME]` | Branch name to delete (interactive if not provided) |

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--reparent` | | Re-parent children to deleted branch's parent |
| `--force` | `-f` | Force delete even if branch is not merged |
| `--upstack` | | Delete branch and all descendants |
| `--downstack` | | Delete branch and all ancestors (except trunk) |

**What it does:**
- Deletes git branch
- Removes from Diamond metadata
- Optionally reparents children

---

### dm rename
Rename current branch.

```bash
dm rename new-name            # Rename current branch
dm rename new-name --local    # Only rename locally
dm rename new-name -f         # Force rename even with open PR
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[NAME]` | New name for the branch |

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--local` | | Only rename locally (don't update remote) |
| `--force` | `-f` | Force rename even when a PR is open |

**What it does:**
- Renames git branch
- Updates all metadata references
- Preserves parent-child relationships
- Records operation in history log

---

### dm track
Track an existing git branch in Diamond.

```bash
dm track                      # Track current branch
dm track feature-name         # Track specific branch
dm track feature --parent main  # Track with explicit parent
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[BRANCH]` | Branch name to track (defaults to current branch) |

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--parent <BRANCH>` | `-p` | Parent branch for the tracked branch |

**What it does:**
- Adds branch to Diamond metadata
- Uses specified parent or detects automatically
- Enables stack operations on this branch

---

### dm untrack (alias: utr)
Remove branch from Diamond tracking.

```bash
dm untrack                    # Untrack current branch
dm untrack feature-name       # Untrack specific branch
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[BRANCH]` | Branch to untrack (defaults to current) |

**What it does:**
- Removes branch from Diamond metadata
- Updates parent's children list
- Git branch remains unchanged

---

### dm freeze
Freeze a branch to prevent local modifications.

```bash
dm freeze                     # Freeze current branch
dm freeze feature-name        # Freeze specific branch
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[BRANCH]` | Branch to freeze (defaults to current) |

**What it does:**
- Prevents local modifications including restacks
- Useful for stacking on teammate's PRs without modifying them

---

### dm unfreeze
Unfreeze a branch to allow modifications.

```bash
dm unfreeze                   # Unfreeze current branch
dm unfreeze --upstack         # Also unfreeze all upstack branches
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[BRANCH]` | Branch to unfreeze (defaults to current) |

**Options:**

| Flag | Description |
|------|-------------|
| `--upstack` | Also unfreeze all upstack branches |

**What it does:**
- Re-enables local modifications on frozen branches

---

### dm get
Download a PR/MR stack from GitHub or GitLab.

```bash
dm get 123                    # By PR/MR number
dm get https://github.com/org/repo/pull/123  # GitHub URL
dm get https://gitlab.com/org/repo/-/merge_requests/123  # GitLab URL
dm get -f feature             # Force overwrite local branches
dm get -U 123                 # Download without freezing
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<PR>` | PR reference (URL or number) |

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--force` | `-f` | Overwrite local branches with remote (discard local changes) |
| `--unfrozen` | `-U` | Don't freeze downloaded branches (allow immediate editing) |

**What it does:**
- Fetches PR and its dependencies
- Creates local branches
- Tracks them in Diamond
- Sets up parent-child relationships
- Freezes branches by default (use `-U` to allow edits)

---

### dm merge
Merge PR(s) from command line with CI integration.

```bash
dm merge                      # Merge entire downstack (trunk→current)
dm merge --squash             # Use squash merge (default)
dm merge --rebase             # Use rebase merge
dm merge -y                   # Skip confirmation
dm merge --fast               # Skip CI wait and proactive rebase
dm merge --no-wait            # Skip CI wait but still rebase proactively
dm merge --no-sync            # Don't sync local branches after merge
```

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--squash` | | Use squash merge (default) |
| `--merge` | | Use merge commit |
| `--rebase` | | Use rebase merge |
| `--yes` | `-y` | Skip confirmation prompt |
| `--no-sync` | | Don't sync local branches after merging |
| `--no-wait` | | Skip waiting for CI (still proactively rebase) |
| `--fast` | | Fast mode: skip proactive rebase and CI wait |

**What it does:**
- Merges all PRs from trunk to current branch (downstack order)
- Proactively rebases PRs onto latest trunk before merging
- Waits for CI to pass before each merge (unless `--no-wait` or `--fast`)
- Syncs local branches after merge (unless `--no-sync`)
- Merges PRs in correct order (parent before child)

**CI Integration:**
- By default, Diamond waits for CI checks to pass before merging each PR
- Use `--no-wait` to skip CI waiting but still rebase proactively
- Use `--fast` for quick merge without any proactive behavior

---

### dm unlink
Unlink current branch from its associated PR.

```bash
dm unlink
```

**What it does:**
- Disassociates the current branch from its PR/MR

---

## Conflict Resolution

### dm continue (alias: cont)
Continue interrupted operation.

```bash
dm continue
```

**What it does:**
- Resumes sync, restack, or move operation
- Processes remaining branches
- Used after resolving rebase conflicts

---

### dm abort
Abort interrupted operation.

```bash
dm abort
```

**What it does:**
- Aborts current sync, restack, or move
- Returns to original branch
- Clears operation state

---

## Maintenance

### dm doctor
Diagnose and repair stack metadata issues.

```bash
dm doctor                     # Check for issues
dm doctor --fix               # Attempt automatic repair
dm doctor --fix-viz           # Update stack visualization in all PRs
```

**Options:**

| Flag | Description |
|------|-------------|
| `--fix` | Automatically fix detected issues |
| `--fix-viz` | Update stack visualization in all PRs |

**What it checks:**
- Circular dependencies in branch relationships
- Orphaned branch references
- Parent-child relationship consistency
- Trunk branch existence

**What it fixes:**
- Inconsistent parent-child relationships
- Broken bidirectional links

---

### dm undo
Restore branch from backup refs.

```bash
dm undo --list                # List all backups
dm undo feature-name          # Restore latest backup for branch
dm undo                       # Restore last operation
dm undo -f feature-name       # Skip confirmation
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[BRANCH]` | Branch to restore (restores last operation if not provided) |

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--list` | | Show all available backup refs |
| `--force` | `-f` | Skip confirmation prompt |

**What it does:**
- Lists backup refs grouped by branch
- Restores branch to backed-up commit
- Backup refs are created automatically before:
  - `dm sync`
  - `dm restack`
  - `dm move`

---

### dm history
View operation history.

```bash
dm history                    # Show last 20 operations
dm history -c 50              # Show last 50 operations
dm history --all              # Show all operations
```

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--count <N>` | `-c` | Number of entries to show (default: 20, use 0 for all) |
| `--all` | | Show all entries |

**What it shows:**
- Timestamp of each operation
- Operation type (sync, restack, move, create, delete, etc.)
- Branches affected
- Success/failure status
- Backup ref creations

---

### dm cleanup
Clean up branches that have been merged.

```bash
dm cleanup                    # Interactive cleanup
dm cleanup -f                 # Skip confirmation
```

**Options:**

| Flag | Short | Description |
|------|-------|-------------|
| `--force` | `-f` | Skip confirmation prompt |

**What it does:**
- Finds branches with merged PRs
- Removes them from tracking
- Deletes local branches
- Reparents any children

---

### dm gc
Garbage collect old backup refs.

```bash
dm gc                         # Clean old backups (default settings)
dm gc --max-age 60            # Keep backups less than 60 days old
dm gc --keep 5                # Keep at most 5 backups per branch
dm gc --dry-run               # Preview what would be deleted
```

**Options:**

| Flag | Description |
|------|-------------|
| `--max-age <DAYS>` | Maximum age of backups to keep (default: 30 days) |
| `--keep <COUNT>` | Maximum number of backups per branch (default: 10) |
| `--dry-run` | Show what would be deleted without deleting |

**What it does:**
- Removes backup refs older than `--max-age` days
- Keeps only the most recent `--keep` backups per branch
- Useful for cleaning up large repositories with many stacks
- Safe: only affects backup refs, never your actual branches

**Defaults:**
- Backups older than 30 days are deleted
- At most 10 backups per branch are retained

---

## Utility Commands

### dm info
Show branch details and PR status.

```bash
dm info                       # Show current branch info
dm info feature-name          # Show specific branch info
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[BRANCH]` | Branch to show info for (defaults to current) |

**What it shows:**
- Branch name and status
- Parent branch
- Children branches
- PR URL (if submitted)
- Commit count ahead of parent

---

### dm pr
Open PR in browser.

```bash
dm pr                         # Open current branch's PR
dm pr feature-name            # Open specific branch's PR
dm pr 123                     # Open PR by number
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `[BRANCH]` | Branch name or PR number (defaults to current branch) |

**What it does:**
- Opens PR URL for branch in browser
- Requires branch to have been submitted

---

### dm parent
Show parent branch of current branch.

```bash
dm parent
```

**What it shows:**
- Name of the parent branch

---

### dm children
Show children branches of current branch.

```bash
dm children
```

**What it shows:**
- Names of all child branches

---

### dm trunk
Show or set trunk branch.

```bash
dm trunk                      # Show current trunk
dm trunk --set develop        # Set trunk to develop
```

**Options:**

| Flag | Description |
|------|-------------|
| `--set <BRANCH>` | Set the trunk branch to this value |

---

### dm completion
Generate shell completion scripts.

```bash
dm completion bash > ~/.local/share/bash-completion/completions/dm
dm completion zsh > ~/.zsh/completions/_dm
dm completion fish > ~/.config/fish/completions/dm.fish
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<SHELL>` | Shell to generate completions for: `bash`, `zsh`, `fish`, `elvish`, `powershell` |

See [COMPLETIONS.md](COMPLETIONS.md) for detailed installation instructions.

---

## Aliases

Quick reference for command aliases:

| Alias | Command | Description |
|-------|---------|-------------|
| `c` | `create` | Create new branch |
| `co` | `checkout` | Checkout branch |
| `l` | `log` | Visualize stack |
| `ls` | `log short` | Visualize stack (short) |
| `ll` | `log long` | Visualize stack (long) |
| `m` | `modify` | Modify branch |
| `s` | `submit` | Submit current branch PR |
| `ss` | `submit --stack` | Submit entire stack |
| `d` | `down` | Move down stack |
| `u` | `up` | Move up stack |
| `t` | `top` | Jump to top |
| `b` | `bottom` | Jump to bottom |
| `f` | `fold` | Fold branch into parent |
| `utr` | `untrack` | Untrack branch |
| `sq` | `squash` | Squash commits |
| `sp` | `split` | Split branch |
| `cont` | `continue` | Continue operation |

---

## Backup & Recovery

Diamond automatically creates backup refs before destructive operations:

- **Before sync**: Backs up all branches being rebased
- **Before restack**: Backs up all branches being rebased
- **Before move**: Backs up branch and all descendants

View backups:
```bash
dm undo --list
```

Restore from backup:
```bash
dm undo branch-name
```

---

## Operation Log

All significant operations are logged to `.git/diamond/operations.jsonl`:

- Branch creation/deletion/rename
- Sync/restack/move operations
- Backup creation/restoration
- Success/failure status

View log:
```bash
dm history
```

---

## Configuration Commands

### dm config show

Display current configuration from all sources.

```bash
dm config show
```

**What it shows:**
- Merged configuration values
- Config file locations and status

---

### dm config get

Get a specific configuration value.

```bash
dm config get repo.remote
dm config get branch.format
dm config get branch.prefix
```

---

### dm config set

Set a configuration value.

```bash
dm config set branch.prefix "alice/"           # User config (default)
dm config set branch.prefix "alice/" --local   # Local config (this repo)
dm config set repo.remote upstream           # Always repo config
```

**Options:**

| Flag | Description |
|------|-------------|
| `--local` | Store in local config (`.git/diamond/`) instead of user config |

---

### dm config unset

Remove a configuration value.

```bash
dm config unset branch.prefix
dm config unset branch.prefix --local
```

**Options:**

| Flag | Description |
|------|-------------|
| `--local` | Remove from local config instead of user config |

---

For detailed configuration options and examples, see [CONFIGURATION.md](CONFIGURATION.md).

---

## Internal Storage

Diamond stores data in:
- `refs/diamond/parent/*` — Branch parent relationships
- `refs/diamond/config/trunk` — Trunk branch setting
- `.diamond/config.toml` — Repository configuration (committed)
- `.git/diamond/config.toml` — Local configuration (not committed)
- `.git/diamond/operations.jsonl` — Operation history
- `.git/diamond/operation_state.json` — In-progress operation state

**Do not manually edit these files** (use `dm config` for configuration).

---

## Getting Help

```bash
dm --help            # List all commands
dm <command> --help  # Help for specific command
```

For issues and feature requests:
https://github.com/rsperko/diamond/issues
