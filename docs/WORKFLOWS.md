# Advanced Workflows

This guide covers advanced Diamond workflows for teams, CI/CD integration, and managing complex stacks.

---

## Team Collaboration

### Downloading a Teammate's Stack

When a colleague has a PR stack you need to review or build upon:

```bash
# Download by PR number
dm get 123

# Download by URL
dm get https://github.com/org/repo/pull/123

# Force overwrite local branches
dm get 123 --force
```

**By default, downloaded branches are frozen** to prevent accidental modifications to someone else's work.

---

### Freezing Branches

Frozen branches cannot be modified by Diamond operations. This prevents accidental changes to:
- Teammate's branches you're building on
- Approved PRs waiting to merge
- Stable base branches

```bash
# Freeze a branch
dm freeze feature-branch

# Check if a branch is frozen
dm info feature-branch

# Unfreeze when ready to modify
dm unfreeze feature-branch

# Unfreeze an entire stack
dm unfreeze feature-branch --upstack
```

---

### Stacking on Teammate's Work

To build on top of a colleague's PR:

```bash
# 1. Download their stack (frozen by default)
dm get 123

# 2. Create your work on top
dm checkout their-feature
dm create my-enhancement

# 3. Work normally on your branch
# ... make changes ...
dm modify -am "Add my enhancement"

# 4. Submit your PR
dm submit
```

When their PR is merged, run `dm sync` to rebase your work onto the updated trunk.

---

### Handoff Workflow

When handing off work to a colleague:

**Original author:**
```bash
# Submit your stack for review
dm submit --stack

# Share the PR link with your teammate
```

**Receiving colleague:**
```bash
# Download the stack
dm get 123

# If you need to make changes, unfreeze first
dm unfreeze --upstack

# Make your changes
dm modify -am "Address review feedback"

# Push updates
dm submit --stack
```

---

## CI/CD Integration

### GitLab Repository Setup

Diamond's stacked workflow requires force pushing after rebasing branches. Before using Diamond with GitLab, ensure your repository allows force push on feature branches.

**Default behavior:** GitLab protects only the default branch (main/master) by default. Feature branches are unprotected, so Diamond works out of the box for most repositories.

**If force push is blocked:** Your organization may have additional protection rules. To fix this:

1. Enable "Allow force push" in your repository's branch protection settings, or
2. Only protect main/master, leaving feature branches unprotected

See [GitLab's protected branches documentation](https://docs.gitlab.com/user/project/repository/branches/protected/) for current steps.

**For self-hosted GitLab:** Check with your administrator—default settings may differ from GitLab.com.

**Why force push is required:** When you run `dm sync` or `dm restack`, Diamond rebases your branches onto the updated trunk. This changes commit hashes, requiring a force push to update the remote. Diamond uses `--force-with-lease` by default, which is safer and prevents overwriting commits you haven't fetched.

---

### Merge Strategies

Diamond supports three merge strategies when merging PRs:

| Strategy | Command | Description |
|----------|---------|-------------|
| Squash | `dm merge` (default) | Combines all commits into one |
| Merge | `dm merge --merge` | Creates a merge commit |
| Rebase | `dm merge --rebase` | Rebases commits onto target |

**Recommendation:** Use squash merge for stacked PRs to keep history clean.

---

### Auto-Merge on CI Pass

Enable auto-merge when submitting PRs:

```bash
# Submit with auto-merge enabled
dm submit --merge-when-ready

# Submit entire stack with auto-merge
dm submit --stack --merge-when-ready
```

When all CI checks pass and the PR/MR is approved, it will automatically merge.

---

### CI Waiting with dm merge

By default, `dm merge` waits for CI before each merge:

```bash
# Full merge with CI wait (default)
dm merge

# Skip CI wait but still rebase proactively
dm merge --no-wait

# Fast mode: no proactive rebase, no CI wait
dm merge --fast
```

**Proactive rebase:** Diamond rebases your PR onto the latest trunk before merging, reducing post-merge CI failures.

---

### Keeping CI Green

Stacked PRs help keep CI green by:

1. **Smaller changes:** Each PR is focused and easier to test
2. **Incremental validation:** CI runs on each layer
3. **Easy bisect:** `git bisect` works because each commit is green

```bash
# Ensure your entire stack is up-to-date
dm sync

# Submit all branches to trigger CI
dm submit --stack

# Check PR status
dm info --stack
```

---

## Large Stack Management

### When to Split vs Fold

**Split a branch when:**
- A PR has grown too large for effective review
- Different parts should be reviewed by different people
- You want to merge some changes earlier than others

```bash
# Split by commit (each commit becomes a branch)
dm split --by-commit

# Split specific files to a new parent branch
dm split --by-file "*.test.ts"
```

**Fold branches when:**
- PRs are too small and would be better combined
- A branch became obsolete
- Simplifying a complex stack

```bash
# Fold current branch into parent
dm fold

# Keep current branch name (delete parent instead)
dm fold --keep
```

---

### Keeping Stacks Manageable

**Best practices:**
- Keep stacks under 10 branches
- Each branch should be 200-500 lines of code
- Review and merge bottom-up

```bash
# Visualize your stack regularly
dm log

# Submit incrementally as branches get approved
dm checkout bottom-branch
dm submit
# ... wait for approval ...
dm merge
dm cleanup
```

---

### Partial Stack Submission

You don't have to submit the entire stack at once:

```bash
# Submit just the current branch
dm submit

# Submit from trunk to current branch (downstack)
dm submit --stack

# Update only existing PRs
dm submit --stack --update-only
```

---

### Reordering Branches

If you need to change the order of branches:

```bash
# Open interactive reorder editor
dm reorder

# Preview current order
dm reorder --preview
```

---

## Migration from Other Workflows

### Adopting Stacked Diffs in an Existing Repo

```bash
# 1. Initialize Diamond
dm init

# 2. Track existing feature branches
git checkout feature-1
dm track --parent main

git checkout feature-2
dm track --parent feature-1

# 3. Visualize your new stack
dm log
```

---

### Working Alongside Non-Diamond Users

Diamond creates standard Git branches and PRs—your teammates don't need Diamond:

**You (using Diamond):**
```bash
dm create feature
dm modify -am "Add feature"
dm submit
```

**Teammate (using vanilla Git):**
```bash
# They see a normal PR/MR in the web UI
# Review and approve normally
# Merge via the web UI
```

**You (after their merge):**
```bash
# Sync to pick up their changes
dm sync

# Cleanup merged branches
dm cleanup
```

---

### From Long-Lived Feature Branches

If your team uses long-lived feature branches:

```bash
# 1. Start from trunk
git checkout main
dm init

# 2. Create a stack for your feature
dm create feature/auth-schema
dm create feature/auth-service
dm create feature/auth-api

# 3. Work incrementally
# Each branch can be merged independently

# 4. Sync regularly
dm sync
```

---

## Tips and Best Practices

### Daily Workflow

```bash
# Start of day: sync with trunk
dm sync

# Work on your stack
dm checkout my-feature
# ... make changes ...
dm modify -am "Progress"

# End of day: submit updates
dm submit --stack
```

### Before Code Review

```bash
# Ensure stack is up-to-date
dm sync

# Squash commits if needed
dm squash -m "Feature: Add authentication"

# Submit for review
dm submit --stack
```

### After Approval

```bash
# Merge the stack (bottom-up)
dm merge

# Clean up merged branches
dm cleanup

# Sync to update local state
dm sync
```

---

## See Also

- [Command Reference](COMMANDS.md)
- [Configuration Guide](CONFIGURATION.md)
- [Troubleshooting](TROUBLESHOOTING.md)
