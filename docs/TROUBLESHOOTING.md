# Troubleshooting Guide

This guide covers common issues and how to resolve them.

## Quick Recovery Commands

| Problem | Command |
|---------|---------|
| Corrupted metadata | `dm doctor --fix` |
| See available backups | `dm undo --list` |
| Restore a branch | `dm undo <branch>` |
| Operation stuck | `dm abort` |
| Continue after conflict | `dm continue` |

---

## Common Error Messages

### "Branch 'X' is not tracked by Diamond"

**What it means:** You're trying to use a Diamond command on a branch that isn't part of a stack.

**Solutions:**
```bash
# Track the branch with a parent
dm track --parent main

# Or track without specifying parent (Diamond will detect)
dm track
```

---

### "Branch 'X' already exists"

**What it means:** You're trying to create or rename a branch to a name that's already in use.

**Solutions:**
```bash
# Check existing branches
git branch

# Delete the existing branch first
dm delete old-branch

# Or choose a different name
dm create different-name
```

---

### "Working tree is not clean"

**What it means:** You have uncommitted changes that could be lost during a rebase operation.

**Solutions:**
```bash
# Option 1: Commit your changes first
git add .
git commit -m "WIP"

# Option 2: Stash your changes
git stash
dm sync
git stash pop

# Option 3: Discard changes (careful!)
git checkout -- .
```

---

### "External changes detected"

**What it means:** The branch has been modified outside of Diamond (e.g., by a teammate or from another machine).

**Solutions:**
```bash
# Option 1: Force the operation (overwrites local changes)
dm restack --force

# Option 2: Pull the latest changes first
git fetch origin
git checkout my-branch
git reset --hard origin/my-branch
```

---

### "You are not allowed to force push" / "Protected branch"

**What it means:** The remote repository (GitLab or GitHub) is rejecting force pushes to your branch.

**Why this happens:** Diamond's stacked workflow requires force pushing after rebasing branches. When you run `dm sync`, `dm restack`, or `dm submit` after amending commits, the branch history changes and requires a force push.

**GitLab users:** GitLab protects the default branch (main/master) by default, which blocks force push. Feature branches are NOT protected by default, so normal Diamond workflow should work. However, force push may be blocked if:
- Your organization has wildcard branch protection rules (e.g., `*` protecting all branches)
- Group-level protection rules are inherited from a parent group
- Your self-hosted GitLab instance has custom default settings

**Solutions:**

1. **Enable force push for feature branches:** Look for "Allow force push" in your repository's branch protection settings
2. **Only protect main:** Configure protection rules for main/master only, leaving feature branches unprotected

For exact steps, see:
- [GitLab: Protected branches](https://docs.gitlab.com/user/project/repository/branches/protected/)
- [GitHub: Protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)

**Note:** Diamond uses `--force-with-lease` by default, which safely fails if the remote has commits you haven't fetched.

---

### "Conflict during rebase"

**What it means:** Git found conflicting changes while rebasing your stack.

**Solutions:**
```bash
# 1. Resolve conflicts in your editor
#    Look for conflict markers: <<<<<<< ======= >>>>>>>

# 2. Stage the resolved files
git add <resolved-files>

# 3. Continue the operation
dm continue

# OR abort and try again later
dm abort
```

---

### "Operation already in progress"

**What it means:** A previous sync, restack, or move operation was interrupted.

**Solutions:**
```bash
# Continue the operation
dm continue

# Or abort it
dm abort
```

---

### "No 'origin' remote configured"

**What it means:** Diamond can't find a remote to push to.

**Solutions:**
```bash
# Check your remotes
git remote -v

# Add a remote
git remote add origin https://github.com/your/repo.git

# Or configure a different remote in Diamond
dm config set repo.remote upstream
```

---

### "Interactive rebase requires a terminal"

**What it means:** You're running an interactive command in a non-TTY environment (e.g., script, CI).

**Solutions:**
```bash
# For scripts, use non-interactive alternatives
dm squash -m "Message"  # Instead of interactive squash

# For split, use file patterns instead of interactive hunk selection
dm split --by-file "*.test.ts"  # Instead of --by-hunk
```

---

### "Cannot split trunk branch"

**What it means:** You're trying to split the main/master branch, which isn't part of a stack.

**Solutions:**
```bash
# Create a branch first
dm create new-feature

# Then split that branch
dm split --by-commit
```

---

## Recovery Procedures

### Recovering from a Failed Sync/Restack

If sync or restack fails partway through:

```bash
# Check current state
dm log short

# Option 1: Continue from where it stopped
dm continue

# Option 2: Abort and rollback
dm abort

# Option 3: If abort fails, manually reset
dm undo --list                    # Find backup refs
dm undo my-branch                 # Restore each branch
```

---

### Restoring Deleted Branches

Diamond creates backups before destructive operations:

```bash
# List all available backups
dm undo --list

# Restore a specific branch
dm undo feature-branch

# If the branch backup doesn't exist, check git reflog
git reflog | grep feature-branch
git checkout -b feature-branch <sha>
```

---

### Fixing Corrupted Metadata

If you see strange behavior or missing branches:

```bash
# Run diagnostics
dm doctor

# Auto-fix detected issues
dm doctor --fix

# Manual inspection of Diamond refs
git for-each-ref refs/diamond/

# Nuclear option: reinitialize (preserves git branches)
dm init --reset
```

---

### Handling Orphaned Branches

If a branch's parent was deleted:

```bash
# Check for orphaned branches
dm doctor

# Reparent to trunk
dm track my-orphan --parent main

# Or delete the orphaned branch
dm delete my-orphan
```

---

## Conflict Resolution

### Step-by-Step Conflict Resolution

1. **Identify the conflict:**
   ```bash
   git status
   # Shows files with conflicts
   ```

2. **Open conflicting files:**
   Look for conflict markers:
   ```
   <<<<<<< HEAD
   Your changes
   =======
   Incoming changes
   >>>>>>> feature-branch
   ```

3. **Resolve the conflict:**
   - Keep your changes, their changes, or combine both
   - Remove the conflict markers

4. **Stage the resolved files:**
   ```bash
   git add <resolved-file>
   ```

5. **Continue the operation:**
   ```bash
   dm continue
   ```

### Multiple Conflicts in a Stack

When rebasing a stack, you may hit conflicts at each level:

```bash
# Diamond pauses at each conflict
# 1. Resolve conflicts
# 2. Stage files
# 3. Continue
dm continue

# If you get stuck, you can always abort
dm abort
```

### When to Abort vs Continue

**Abort when:**
- Conflicts are too complex to resolve now
- You realize you need to make changes to parent branches first
- Something unexpected happened

**Continue when:**
- You've resolved all conflicts in the current file
- You want to proceed to the next branch in the stack

---

## Diagnostic Commands

### Check Stack Health

```bash
# Visualize your stack
dm log short

# Get detailed info about a branch
dm info feature-branch

# Run full diagnostics
dm doctor
```

### Inspect Diamond Refs

```bash
# List all parent relationships
git for-each-ref --format='%(refname:short) → %(contents)' refs/diamond/parent/

# Check trunk setting
git show refs/diamond/config/trunk

# View operation history
dm history
```

### Debug Mode

For detailed output during operations:

```bash
# Show git commands being executed
dm sync --verbose

# Preview without making changes
dm restack --dry-run
```

---

## Getting Help

If you're still stuck:

1. **Check the command reference:**
   ```bash
   dm <command> --help
   ```

2. **View operation history:**
   ```bash
   dm history
   ```

3. **File an issue:**
   https://github.com/rsperko/diamond/issues

Include in your issue:
- Diamond version (`dm --version`)
- The command you ran
- The full error message
- Output of `dm doctor`
