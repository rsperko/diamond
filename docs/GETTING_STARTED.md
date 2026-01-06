# Getting Started with Diamond

Get your first stacked pull request working in 10 minutes.

## Installation

### macOS (Homebrew)

```bash
brew tap rsperko/tap
brew install diamond
```

Or in one command:
```bash
brew install rsperko/tap/diamond
```

### All Platforms (Cargo)

**Prerequisites:** Rust 1.82+

**Install from crates.io:**
```bash
cargo install diamond-cli
```

**Or install latest from GitHub:**
```bash
cargo install --git https://github.com/rsperko/diamond
```

### Build from Source

```bash
git clone https://github.com/rsperko/diamond.git
cd diamond
cargo install --path .
```

### Verify Installation

```bash
dm --help
```

You should see Diamond's command list. If not, ensure your Cargo bin directory is in your PATH.

---

## Forge Setup

Diamond works with GitHub and GitLab. You'll need their CLIs for creating pull requests.

### GitHub Setup

Install the GitHub CLI and authenticate:

```bash
# Install (if not already installed)
# macOS: brew install gh
# Other: https://cli.github.com/

# Authenticate
gh auth login
```

### GitLab Setup

Install the GitLab CLI and authenticate:

```bash
# Install (if not already installed)
# macOS: brew install glab
# Other: https://gitlab.com/gitlab-org/cli

# Authenticate
glab auth login
```

Diamond auto-detects your forge from the git remote URL. Self-hosted GitLab instances are fully supported.

**Note:** GitLab's stacked workflow requires force push on feature branches. Most repos work by default, but if you encounter issues, see [CONFIGURATION.md](CONFIGURATION.md#repository-setup-githubgitlab) for setup details.

---

## Your First Stack

Let's build a complete authentication system as a stack of small, reviewable PRs.

### 1. Initialize Diamond

Navigate to your project and initialize:

```bash
cd your-project
dm init
```

Diamond will detect your trunk branch (main or master) and set up stack tracking.

### 2. Create Your First Branch

Start with the database layer:

```bash
dm create auth-schema -am "Add users table schema"
```

**What just happened:**
- Created a new branch `auth-schema` from trunk
- Staged all changes (`-a`)
- Created a commit with the message (`-m`)
- Diamond tracked this branch as building on trunk

### 3. Stack the Service Layer

Without waiting for review, build on top:

```bash
dm create auth-service -am "Add authentication service"
```

Now `auth-service` builds on `auth-schema`. You're stacking!

### 4. Stack the API Layer

Keep going:

```bash
dm create auth-api -am "Add login and logout endpoints"
```

### 5. Visualize Your Stack

```bash
dm log
```

You'll see a beautiful tree structure:

```
● main (trunk)
└── ● auth-schema
    └── ● auth-service
        └── ● auth-api ← (you are here)
```

Press `q` to exit the TUI.

### 6. Submit for Review

Push all branches and create PRs:

```bash
dm submit --stack
```

**What just happened:**
- Pushed all 3 branches to remote
- Created 3 pull requests:
  - PR #1: `main` ← `auth-schema` (150 lines)
  - PR #2: `auth-schema` ← `auth-service` (200 lines)
  - PR #3: `auth-service` ← `auth-api` (180 lines)
- Each PR is small, focused, and ready for review

Your teammates can now review 3 small PRs in parallel instead of one massive 500-line PR.

---

## Essential Daily Commands

Here are the commands you'll use every day:

### Morning: Sync with Trunk

```bash
dm sync
```

Fetches the latest changes from `main` and rebases all your stacks automatically.

### Working: Create and Modify

```bash
# Create new branch on current
dm create feature-name -am "Add feature"

# Modify current branch
dm modify -am "Update implementation"
```

### Sharing: Submit Work

```bash
# Submit current branch
dm submit

# Submit entire stack
dm submit --stack
```

### Navigation: Move Around

```bash
# Interactive branch picker
dm checkout

# Move up/down the stack
dm up     # to child branch
dm down   # to parent branch

# Jump to top or bottom
dm top    # deepest descendant
dm bottom # closest to trunk
```

### Recovering: When Stuck

```bash
# After resolving conflicts
dm continue

# Cancel current operation
dm abort

# View your stack
dm log
```

---

## When Things Go Wrong

### "Working tree is not clean"

You have uncommitted changes. Either commit them or stash them:

```bash
git add .
git commit -m "WIP"
# or
git stash
```

### "Conflict during rebase"

Diamond paused because of merge conflicts:

1. Resolve conflicts in your editor
2. Stage resolved files: `git add <file>`
3. Continue: `dm continue`

Or abort and try later: `dm abort`

### "Corrupted metadata"

Run diagnostics and auto-fix:

```bash
dm doctor --fix
```

For more help, see [TROUBLESHOOTING.md](TROUBLESHOOTING.md).

---

## Next Steps

You're ready to use Diamond! Here's where to go from here:

### Configuration
- **Repository Setup:** Configure squash merging for clean history → [CONFIGURATION.md](CONFIGURATION.md#repository-setup-githubgitlab)
- **Shell Completion:** Tab completion for all commands → [CONFIGURATION.md](CONFIGURATION.md#shell-completion)
- **Branch Formatting:** Customize auto-generated branch names → [CONFIGURATION.md](CONFIGURATION.md#branchformat)

### Advanced Workflows
- **Team Collaboration:** Working with teammates' stacks → [WORKFLOWS.md](WORKFLOWS.md#team-collaboration)
- **Large Stack Management:** Splitting, folding, and reordering → [WORKFLOWS.md](WORKFLOWS.md#large-stack-management)
- **CI/CD Integration:** Auto-merge and merge strategies → [WORKFLOWS.md](WORKFLOWS.md#cicd-integration)

### Command Reference
- **Full Command List:** Every command with all options → [COMMANDS.md](COMMANDS.md)
- **Troubleshooting:** Common issues and solutions → [TROUBLESHOOTING.md](TROUBLESHOOTING.md)
