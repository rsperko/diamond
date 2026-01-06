# 💎 Diamond

**Stop waiting for code reviews. Start shipping.**

Diamond is a lightning-fast CLI for stacked pull requests—the workflow used at Meta, Google, and top engineering teams to ship code faster with higher quality.

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![CI](https://github.com/rsperko/diamond/workflows/CI/badge.svg)](https://github.com/rsperko/diamond/actions)

> **⚠️ Alpha Software:** Diamond is in active development. Works well, but expect rough edges. Please [report issues](https://github.com/rsperko/diamond/issues)!

---

## The Problem

You finish a feature. You open a PR. Then you **wait**.

Hours. Sometimes days. Your next feature depends on this one, so you're stuck. You could branch off your unmerged work, but rebasing that later? Nightmare fuel.

**Meanwhile, your best engineers are context-switching instead of coding.**

---

## The Solution

Diamond lets you build Feature B *on top of* Feature A—before Feature A is even reviewed. When Feature A gets feedback, Diamond automatically rebases your entire stack in milliseconds.

### For Developers

✅ **Stay in Flow** – Never blocked waiting for reviews to continue working
✅ **Smaller, Focused PRs** – Break features into reviewable chunks (200-500 lines each)
✅ **Automatic Rebase** – Your entire stack rebases in milliseconds, not hours
✅ **Standard Git** – Works with existing tools, no vendor lock-in

### For Engineering Leaders

✅ **4x Faster Reviews** – Research shows reviewers process 5 small PRs faster than 1 large one ([Dr. Michaela Greiler](https://www.michaelagreiler.com/stacked-pull-requests/))
✅ **21% More Code Shipped** – Teams using stacked workflows ship measurably more ([Graphite at Asana](https://graphite.dev/use-cases/increasing-developer-productivity))
✅ **7 Hours/Week Saved** – Reduced context switching per developer ([Asana case study](https://www.gitkraken.com/gitkon/stacked-pull-requests-tomas-reimers))
✅ **Zero Reviewer Training** – Creates standard GitHub/GitLab PRs—no new tools required

---

## Installation

### macOS (Homebrew)

```bash
brew install rsperko/tap/diamond
```

### Other Platforms

See [Getting Started Guide](docs/GETTING_STARTED.md#installation) for Cargo, build-from-source, and more.

**Verify installation:**
```bash
dm --help
```

**Next:** Set up GitHub (`gh auth login`) or GitLab (`glab auth login`). See [Getting Started](docs/GETTING_STARTED.md#forge-setup) for details.

---

## Quick Start

```bash
# Initialize Diamond in your repo
dm init

# Create your first branch
dm create add-database-schema -am "Add users table"

# Stack another branch on top (no waiting!)
dm create add-user-service -am "Add service layer"

# Visualize your stack
dm log

# Submit both PRs
dm submit --stack
```

**Full walkthrough:** [Getting Started Guide](docs/GETTING_STARTED.md)

---

## Real-World Example

Let's build a complete authentication system as a stack of small, focused PRs:

```bash
# Start from main
dm init

# Layer 1: Database (150 lines)
dm create auth-schema -am "Add users and sessions tables"

# Layer 2: Business logic (200 lines) - builds on Layer 1
dm create auth-service -am "Add authentication service"

# Layer 3: API endpoints (180 lines) - builds on Layer 2
dm create auth-api -am "Add /login and /logout endpoints"

# Layer 4: Frontend (220 lines) - builds on Layer 3
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

### Why This Works Better

**Before (monolithic PR):**
- 1 PR × 750 lines = **4 hours review time**
- Generic reviewers handle everything
- Merge conflicts likely
- Hard to revert if issues found

**After (stacked PRs):**
- Database PR → DB specialist reviews (**30 min**)
- Service PR → Backend engineer reviews (**30 min**)
- API PR → API team reviews (**30 min**)
- UI PR → Frontend engineer reviews (**30 min**)

**Result:** Reviews happen in parallel. **Total time: 30 minutes** instead of 4 hours sequential.

---

## Why Diamond?

### 🦀 Rust Performance
Built with `libgit2` for sub-millisecond operations. Your stack with 20 branches? Rebased in <100ms.

### 🎯 TUI-First Design
`dm log` gives you a beautiful terminal UI to visualize complex stacks. No more ASCII art in `git log`.

### 🔧 Git-Native
Diamond uses standard Git branches and commits. Your team doesn't need Diamond to review your PRs. You can always drop back to vanilla Git.

### 📦 Zero SaaS Dependencies
Metadata lives in `.git/diamond/` (Git-ignored by default) and git refs. Delete it and you're back to regular Git branches. No remote dependencies.

---

## Essential Commands

| Command | Description |
|---------|-------------|
| `dm create <name>` | Create branch on top of current |
| `dm checkout [name]` | Switch branches (interactive if no name) |
| `dm log` | Visualize stack tree (TUI) |
| `dm submit --stack` | Push and create PRs for entire stack |
| `dm sync` | Fetch trunk and rebase all branches |
| `dm restack` | Rebase stack without fetching |
| `dm up / down` | Navigate to child/parent branch |
| `dm modify -am "msg"` | Amend current branch |
| `dm continue / abort` | Handle conflicts or cancel operation |
| `dm doctor --fix` | Diagnose and repair metadata |

**Full reference:** [Command Reference](docs/COMMANDS.md)

---

## How It Works

Diamond stores parent-child relationships as git refs that travel with push/fetch:

```
refs/diamond/config/trunk    → blob("main")
refs/diamond/parent/auth-schema  → blob("main")
refs/diamond/parent/auth-service → blob("auth-schema")
refs/diamond/parent/auth-api     → blob("auth-service")
```

When you run `dm restack` or `dm sync`, Diamond:
1. Topologically sorts your branches (parents before children)
2. Rebases each branch onto its parent using `git rebase`
3. Handles conflicts interactively with `dm continue` / `dm abort`

**That's it.** No magic. Just smart orchestration of Git primitives.

---

## FAQ

**Q: Do my reviewers need Diamond?**
A: No. Diamond creates standard GitHub PRs and GitLab MRs. Reviewers see normal PRs with clear base branches.

**Q: What if I get conflicts during rebase?**
A: Diamond pauses and lets you resolve conflicts manually. Then run `dm continue` to resume.

**Q: Does this work with GitLab?**
A: Yes! GitLab has full feature parity with GitHub. Diamond creates MRs, handles approvals, waits for pipelines, and supports auto-merge. Requires `glab` CLI. Self-hosted GitLab instances are also supported.

**Q: Can I import stacks from Graphite?**
A: Yes! Use `dm get <PR>` to download any GitHub PR or GitLab MR and its dependencies. Works with Graphite-created stacks.

**More questions?** See [Getting Started](docs/GETTING_STARTED.md) or [Troubleshooting](docs/TROUBLESHOOTING.md)

---

## Next Steps

- 📚 **Get Started:** [Installation & First Stack](docs/GETTING_STARTED.md)
- ⚙️ **Configure:** [Setup squash merge, shell completion, and more](docs/CONFIGURATION.md)
- 🚀 **Advanced:** [Team collaboration, CI/CD, large stacks](docs/WORKFLOWS.md)
- 📖 **Reference:** [All commands with options](docs/COMMANDS.md)

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
```

**Security:**
This project uses [Gitleaks](https://github.com/gitleaks/gitleaks) to prevent secrets from being committed. Run `just setup-hooks` to install the pre-commit hook.

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
- 📈 [Increasing Developer Productivity](https://graphite.dev/use-cases/increasing-developer-productivity) – Graphite Case Study
- 🎥 [Stacked Pull Requests Talk](https://www.gitkraken.com/gitkon/stacked-pull-requests-tomas-reimers) – GitKon 2022
