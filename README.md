# 💎 Diamond

**Stop waiting for code reviews. Start shipping.**

Diamond is a lightning-fast CLI for stacked pull requests—the workflow used at Meta, Google, and top engineering teams to ship code 4x faster.

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![CI](https://github.com/rsperko/diamond/workflows/CI/badge.svg)](https://github.com/rsperko/diamond/actions)

> **⚠️ Alpha Software:** Diamond is in active development. Works well, but expect rough edges. Please [report issues](https://github.com/rsperko/diamond/issues)!

---

## The Problem

You finish a feature. You open a PR. Then you **wait**.

Hours. Sometimes days. Your next feature depends on this one, so you're stuck. You could branch off your unmerged work, but rebasing that later? Nightmare fuel.

**Meanwhile, your best engineers are context-switching instead of coding.**

## The Solution

Stacked pull requests let you build Feature B *on top of* Feature A—before Feature A is even reviewed. When Feature A gets feedback, Diamond automatically rebases your entire stack in milliseconds.

**This is how the best teams ship faster:**

✅ **Never blocked waiting for reviews** – Keep coding while your team reviews
✅ **4x faster code reviews** – Reviewers prefer 5 files over 50 ([research-backed](https://www.michaelagreiler.com/stacked-pull-requests/))
✅ **Stay in flow** – Stop context-switching between unrelated work
✅ **Ship incrementally** – Small PRs are easier to review, test, and rollback
✅ **Every commit stays green** – CI runs on every change, making `git bisect` actually useful

---

## Installation

**Prerequisites:** Rust 1.82+

**Install from crates.io (recommended):**
```bash
cargo install diamond-cli
```

**Or install latest from GitHub:**
```bash
cargo install --git https://github.com/rsperko/diamond
```

**Or build from source:**
```bash
git clone https://github.com/rsperko/diamond.git
cd diamond
cargo install --path .
```

**Verify installation:**
```bash
dm --help
```

**For GitHub:** Install the [GitHub CLI](https://cli.github.com/) (`gh`) and authenticate:
```bash
gh auth login
```

**For GitLab:** Install the [GitLab CLI](https://gitlab.com/gitlab-org/cli) (`glab`) and authenticate:
```bash
glab auth login
```

Diamond auto-detects your forge from the git remote URL. Self-hosted GitLab instances are fully supported.

> **GitLab Note:** Diamond requires force push for stacked workflows. Feature branches work by default, but if your organization protects all branches, you'll need to enable "Allow force push" in your branch protection settings. See [Troubleshooting](docs/TROUBLESHOOTING.md#you-are-not-allowed-to-force-push--protected-branch) for details.

---

## Repository Setup (Important!)

**⚠️ For the best experience, configure your repository to use squash merging:**

Stacked PRs work best with a clean, linear git history. Configure your GitHub/GitLab repository to squash merge by default:

**GitHub:** Settings → Pull Requests → **Uncheck** "Allow merge commits", **Check** "Allow squash merging"

**GitLab:** Settings → General → Merge requests → Set merge method to "Squash commits"

Without this, your git history will be cluttered with merge commits instead of clean, revertable changes. See [Configuration Guide](docs/CONFIGURATION.md#repository-setup-githubgitlab) for details.

---

## Shell Completion

Diamond supports tab completion for bash, zsh, fish, and more. Completions include all subcommands, options, and even **dynamic branch name suggestions** for commands like `checkout` and `delete`.

**Bash:**
```bash
dm completion bash > ~/.local/share/bash-completion/completions/dm
source ~/.bashrc
```

**Zsh:**
```bash
mkdir -p ~/.zsh/completions
dm completion zsh > ~/.zsh/completions/_dm
echo 'fpath=(~/.zsh/completions $fpath)' >> ~/.zshrc
autoload -Uz compinit && compinit
```

**Fish:**
```bash
dm completion fish > ~/.config/fish/completions/dm.fish
```

**Example usage:**
```bash
dm checkout <TAB>         # Shows: feat/auth  feat/ui  main
dm delete --<TAB>         # Shows: --reparent --help
dm submit --<TAB>         # Shows: --stack --force
```

For more details, see [docs/COMPLETIONS.md](docs/COMPLETIONS.md).

---

## Quick Start

### 1. Initialize Diamond in your repo
```bash
cd your-project
dm init
```

Diamond stores stack relationships as git refs (`refs/diamond/parent/*`), enabling seamless collaboration.

### 2. Create your first stacked feature

Start with the foundation:
```bash
dm create add-database-schema
# Write your migration files...
git add .
git commit -m "Add user table schema"
```

Now build on top of it **immediately**—don't wait for review:
```bash
dm create add-user-service
# Write your business logic...
git commit -am "Add user service layer"
```

Stack another layer:
```bash
dm create add-user-api
# Write your REST endpoints...
git commit -am "Add user API endpoints"
```

### 3. Visualize your stack
```bash
dm log
```

**Output:**
```
● main (trunk)
└── ● add-database-schema (PR #156)
    └── ● add-user-service (PR #157)
        └── ● add-user-api ← (you are here)
```

### 4. Navigate your stack

Jump to any branch:
```bash
dm checkout        # Interactive picker
dm top             # Jump to top of stack
dm bottom          # Jump to bottom (closest to main)
dm up              # Move to child branch
dm down            # Move to parent branch
```

### 5. Submit your stack for review

Push and create PRs for the whole stack:
```bash
dm submit --stack
```

Diamond pushes each branch and creates GitHub/GitLab PRs with the correct base branches. Each PR is small, focused, and reviewable.

---

## When Reviews Come Back

Your reviewer asks for changes on `add-database-schema`. No problem:

```bash
dm checkout add-database-schema
# Make changes...
git commit -am "Add index to email column"
dm restack   # Rebases all dependent branches automatically
```

Your entire stack stays in sync. No merge conflicts. No manual rebasing.

---

## Essential Commands

### Creating & Managing Branches
| Command | Alias | Description |
|---------|-------|-------------|
| `dm create <name>` | `dm c` | Create a new branch on top of current |
| `dm checkout [name]` | `dm co` | Switch branches (interactive if no name) |
| `dm track` | - | Start tracking current branch |
| `dm rename <name>` | - | Rename current branch and update metadata |
| `dm delete <name>` | - | Delete branch (with optional re-parenting) |

### Navigating Stacks
| Command | Alias | Description |
|---------|-------|-------------|
| `dm log` | `dm l` | Visualize your entire stack tree (TUI) |
| `dm top` | `dm t` | Jump to top of current stack |
| `dm bottom` | `dm b` | Jump to bottom (closest to trunk) |
| `dm up` | `dm u` | Move to child branch |
| `dm down` | `dm d` | Move to parent branch |
| `dm info [branch]` | - | Show branch details, commits, and PR status |

### Submitting & Syncing
| Command | Alias | Description |
|---------|-------|-------------|
| `dm submit` | `dm s` | Push current branch and create PR |
| `dm submit --stack` | - | Submit entire stack (current + descendants) |
| `dm sync` | - | Fetch trunk and rebase all branches |
| `dm restack` | - | Rebase stack without fetching |
| `dm pr` | - | Open current branch's PR in browser |

### Modifying Branches
| Command | Alias | Description |
|---------|-------|-------------|
| `dm modify -am "msg"` | `dm m` | Stage all and amend/create commit |
| `dm squash` | `dm sq` | Squash all commits in current branch |
| `dm absorb` | - | Auto-absorb staged changes into earlier commits |
| `dm move --onto <branch>` | - | Move branch to new parent |

### Recovering from Conflicts
| Command | Alias | Description |
|---------|-------|-------------|
| `dm continue` | `dm cont` | Continue after resolving conflicts |
| `dm abort` | - | Abort current operation |

### Collaboration & Maintenance
| Command | Alias | Description |
|---------|-------|-------------|
| `dm get <PR>` | - | Download a teammate's PR stack |
| `dm freeze` | - | Freeze branch to prevent modifications |
| `dm unfreeze` | - | Unfreeze branch to allow modifications |
| `dm doctor --fix` | - | Diagnose and repair stack metadata |
| `dm undo` | - | Restore branch from backup |
| `dm gc` | - | Clean up old backup refs |

---

## Real-World Example

Let's build a complete authentication system as a stack:

```bash
# Start from main
dm init

# Layer 1: Database
dm create auth-schema -am "Add users and sessions tables"

# Layer 2: Business logic (builds on Layer 1)
dm create auth-service -am "Add authentication service"

# Layer 3: API endpoints (builds on Layer 2)
dm create auth-api -am "Add /login and /logout endpoints"

# Layer 4: Frontend integration (builds on Layer 3)
dm create auth-ui -am "Add login form component"

# Visualize the stack
dm log
```

**Your stack:**
```
● main
└── ● auth-schema
    └── ● auth-service
        └── ● auth-api
            └── ● auth-ui ← (current)
```

**Submit for review:**
```bash
dm submit --stack
```

This creates **4 small PRs** instead of 1 massive 2,000-line PR:
- PR #1: Database schema (150 lines) ✅ Easy review
- PR #2: Service layer (200 lines) ✅ Focused scope
- PR #3: API endpoints (180 lines) ✅ Clear purpose
- PR #4: UI components (220 lines) ✅ Reviewable

**Each PR gets reviewed faster because reviewers can focus.**

---

## Why Diamond?

### 🦀 **Rust Performance**
Built with `libgit2` for sub-millisecond operations. Your stack with 20 branches? Rebased in <100ms.

### 🎯 **TUI-First Design**
`dm log` gives you a beautiful terminal UI to visualize complex stacks. No more ASCII art in `git log`.

### 🔧 **Git-Native**
Diamond uses standard Git branches and commits. Your team doesn't need Diamond to review your PRs. You can always drop back to vanilla Git.

### 📦 **Zero Lock-In**
Metadata lives in `.git/diamond/` (Git-ignored by default). Delete it and you're back to regular Git branches. No remote dependencies.

---

## How It Works

Diamond stores parent-child relationships as git refs that travel with push/fetch:

```
refs/diamond/config/trunk    → blob("main")
refs/diamond/parent/auth-schema  → blob("main")
refs/diamond/parent/auth-service → blob("auth-schema")
refs/diamond/parent/auth-api     → blob("auth-service")
```

Children are derived by scanning parent refs (not stored explicitly).

When you run `dm restack` or `dm sync`, Diamond:
1. Topologically sorts your branches (parents before children)
2. Rebases each branch onto its parent using `git rebase`
3. Handles conflicts interactively with `dm continue` / `dm abort`

**That's it.** No magic. Just smart orchestration of Git primitives.

---

## FAQ

**Q: Do my reviewers need Diamond?**
No. Diamond creates standard GitHub PRs and GitLab MRs. Reviewers see normal PRs with clear base branches.

**Q: What if I get conflicts during rebase?**
Diamond pauses and lets you resolve conflicts manually. Then run `dm continue` to resume.

**Q: Can I use this with GitLab?**
Yes! GitLab has **full feature parity** with GitHub. Diamond creates MRs, handles approvals, waits for pipelines, and supports auto-merge. Requires `glab` CLI. Self-hosted GitLab instances are also supported.

**Q: What about Bitbucket / Gitea?**
Not yet, but the forge architecture makes adding new providers straightforward. Contributions welcome!

**Q: What if Diamond breaks?**
Diamond stores metadata as git refs under `refs/diamond/`. Your Git history is never modified unsafely. Worst case: delete the refs and use Git normally.

**Q: How is this different from other tools?**
Diamond is laser-focused on speed (Rust), simplicity (minimal commands), and beautiful UX (TUI). No web dashboards, no SaaS, no lock-in.

**Q: How do I fix corrupted stack metadata?**
Run `dm doctor --fix` to automatically repair common issues like orphaned refs, broken parent links, and cycle detection.

**Q: How do I undo a mistake?**
Diamond automatically creates backups before destructive operations. Run `dm undo --list` to see available backups, then `dm undo <branch>` to restore.

**Q: My operation got interrupted—what do I do?**
If you have an in-progress sync or restack, run `dm continue` after resolving any conflicts, or `dm abort` to cancel and roll back.

---

## Advanced: Stack Surgery

Reorganize your stack on the fly:

**Move a branch to a different parent:**
```bash
dm move --onto main        # Move current branch to trunk
```

**Delete a branch and re-parent its children:**
```bash
dm delete auth-service --reparent
# auth-api now builds directly on auth-schema
```

**Squash commits before submitting:**
```bash
dm checkout auth-schema
dm squash                  # Squash all commits into one
dm restack                 # Update dependent branches
```

---

## Team Collaboration

Diamond makes it easy to collaborate on stacked PRs:

**Download a teammate's stack:**
```bash
dm get 123                 # Download PR #123 and its dependencies
dm get https://github.com/org/repo/pull/123
```

Downloaded branches are frozen by default to prevent accidental modifications.

**Stack on top of a colleague's work:**
```bash
dm get 123                 # Get their stack (frozen)
dm create my-feature       # Build on top of it
```

**Unfreeze to make changes:**
```bash
dm unfreeze                # Allow modifications to current branch
dm unfreeze --upstack      # Also unfreeze all child branches
```

---

## Contributing

Diamond is open source and built with 🦀 Rust. Contributions welcome!

**Development setup:**
```bash
git clone https://github.com/rsperko/diamond.git
cd diamond
cargo build
cargo test                 # 206+ tests, ~90% coverage
cargo clippy -- -D warnings
just setup-hooks           # Configure git hooks (Gitleaks)

**Security:**
This project uses [Gitleaks](https://github.com/gitleaks/gitleaks) to prevent secrets from being committed.
Run `just setup-hooks` to install the pre-commit hook that checks for secrets.

**Architecture:**
- `src/main.rs` – CLI parsing (clap)
- `src/commands/` – Command implementations
- `src/git_gateway.rs` – Git operations (libgit2)
- `src/ref_store.rs` – Stack metadata in git refs
- `src/state.rs` – Operation state management
- `src/forge/` – GitHub/GitLab integration

---

## Inspired By

The stacked diff workflow has been used for years at:
- **Meta** (Phabricator)
- **Google** (Critique)
- **Uber, Airbnb, Stripe** (various internal tools)

Diamond brings this workflow to **any team using GitHub or GitLab**, with the speed of Rust and zero SaaS dependencies.

---

## License

Apache 2.0 © 2025

Built with ❤️ for developers who ship fast.

---

## Learn More

- 📚 [Stacked Diffs Explained](https://newsletter.pragmaticengineer.com/p/stacked-diffs) – The Pragmatic Engineer
- 📊 [Why Stacked PRs Work](https://www.michaelagreiler.com/stacked-pull-requests/) – Dr. Michaela Greiler
- 🎯 [In Praise of Stacked PRs](https://benjamincongdon.me/blog/2022/07/17/In-Praise-of-Stacked-PRs/) – Ben Congdon
