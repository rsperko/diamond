# Configuration Guide

Diamond uses a layered configuration system with TOML format.

---

## Repository Setup (GitHub/GitLab)

**⚠️ IMPORTANT:** For the best stacked PR experience, configure your GitHub/GitLab repository to use **squash merging** by default.

### Why Squash Merging Matters

Stacked PRs work best with a **clean, linear history**. Without squash merging, you'll get messy merge commits:

**❌ Without squash (merge commits):**
```
*   a1b2c3d Merge pull request #3 from user/feature-c
|\
| * e4f5g6h Third commit
| * h7i8j9k Second commit
| * k1l2m3n First commit
|/
*   n4o5p6q Merge pull request #2 from user/feature-b
```

**✅ With squash merging:**
```
* a1b2c3d feature-c: Add authentication
* e4f5g6h feature-b: Update database schema
* h7i8j9k feature-a: Fix bug in parser
```

### GitHub Configuration

1. Go to your repository settings: `https://github.com/YOUR_USERNAME/YOUR_REPO/settings`
2. Scroll to **"Pull Requests"** section
3. Configure merge options:
   - ✅ **Check** "Allow squash merging"
   - ❌ **Uncheck** "Allow merge commits"
   - ❌ **Uncheck** "Allow rebase merging" (optional, but recommended for consistency)
4. Set **default merge method** to "Squash and merge"

### GitLab Configuration

1. Go to Settings → General → Merge requests
2. Set **Merge method** to "Fast-forward merge" or "Squash commits"
3. Enable "Delete source branch" option (recommended)

### GitLab Force Push Requirements

**Why force push is needed:** When you run `dm sync` or `dm restack`, Diamond rebases your branches onto the updated trunk. This changes commit hashes, requiring a force push to update the remote. Diamond uses `--force-with-lease` by default, which safely fails if the remote has commits you haven't fetched.

**Default behavior:** GitLab protects only the default branch (main/master) by default. Feature branches are unprotected, so Diamond works out of the box for most repositories.

**If force push is blocked:**

Your organization may have additional protection rules. To fix this:

1. Enable "Allow force push" in your repository's branch protection settings, or
2. Only protect main/master, leaving feature branches unprotected

See [GitLab's protected branches documentation](https://docs.gitlab.com/user/project/repository/branches/protected/) for current steps.

**For self-hosted GitLab:** Check with your administrator—default settings may differ from GitLab.com.

### What Happens When You Squash

When you merge a PR with squash:
- All commits on the branch are combined into a single commit
- The commit message becomes the PR title/description
- Branch history is simplified
- Easy to revert entire features with one `git revert`

**Example:**

```bash
# Your branch has 3 commits:
* fix typo
* address review feedback
* Add authentication feature

# After squash merge to main:
* Add authentication feature  ← Single clean commit
```

### Manual Squash Per PR

If you don't want to change repo settings, you can squash manually:

1. When merging on GitHub, click the dropdown next to **"Merge pull request"**
2. Select **"Squash and merge"**
3. Edit the commit message if needed
4. Confirm

**Note:** This requires remembering to squash every time. Changing the default is more reliable.

---

## Shell Completion

Diamond provides comprehensive shell completion support for bash, zsh, fish, and other popular shells.

### Features

- ✅ **Static completions**: All 40+ subcommands with their options and flags
- ✅ **Dynamic completions**: Branch names from your repository's tracked branches
- ✅ **Multi-shell support**: bash, zsh, fish, elvish, powershell
- ✅ **Instant updates**: Completions reflect current repository state

### Installation

#### Bash

**Prerequisites:** bash 4.0 or later

```bash
# Create completion directory if it doesn't exist
mkdir -p ~/.local/share/bash-completion/completions

# Generate and install completion script
dm completion bash > ~/.local/share/bash-completion/completions/dm

# Reload your shell configuration
source ~/.bashrc
```

**Alternative locations:**
```bash
# System-wide (requires sudo)
sudo dm completion bash > /usr/share/bash-completion/completions/dm

# macOS Homebrew bash-completion
dm completion bash > $(brew --prefix)/etc/bash_completion.d/dm
```

#### Zsh

**Prerequisites:** zsh 5.0 or later

```bash
# Create completion directory
mkdir -p ~/.zsh/completions

# Generate completion script
dm completion zsh > ~/.zsh/completions/_dm

# Add to fpath (add this line to ~/.zshrc if not already present)
echo 'fpath=(~/.zsh/completions $fpath)' >> ~/.zshrc

# Initialize completion system (add to ~/.zshrc if not present)
echo 'autoload -Uz compinit && compinit' >> ~/.zshrc

# Reload your shell
source ~/.zshrc
```

**Notes for zsh:**
- Completion files must start with `_` (underscore)
- If completions don't work, try running `rm ~/.zcompdump && compinit`
- Some zsh frameworks (oh-my-zsh, prezto) may require additional configuration

#### Fish

**Prerequisites:** fish 3.0 or later

```bash
# Generate and install completion script
dm completion fish > ~/.config/fish/completions/dm.fish

# Completions are loaded automatically in fish
# No need to reload or modify config files
```

Fish completions are typically active immediately. If not, restart your fish shell.

### Usage

**Completing subcommands:**
```bash
dm <TAB>
# Shows: create, checkout, log, track, submit, sync, ...
```

**Completing options:**
```bash
dm create --<TAB>
# Shows: --all --message --help

dm submit -<TAB>
# Shows: --stack --force --help
```

**Dynamic branch name completion:**
```bash
dm checkout <TAB>
# Shows only tracked branches from refs/diamond/parent/*
# Example: feat/auth  feat/ui  fix/bug-123
```

**Other commands with branch completion:**
- `dm delete <TAB>` - tracked branches
- `dm info <TAB>` - tracked branches
- `dm move --onto <TAB>` - tracked branches

### Troubleshooting

#### Completions Not Working

1. **Verify completion script is installed:**
   ```bash
   # Bash
   ls -la ~/.local/share/bash-completion/completions/dm

   # Zsh
   ls -la ~/.zsh/completions/_dm

   # Fish
   ls -la ~/.config/fish/completions/dm.fish
   ```

2. **Check shell version:**
   ```bash
   bash --version    # Need 4.0+
   zsh --version     # Need 5.0+
   fish --version    # Need 3.0+
   ```

3. **Regenerate completion script:**
   ```bash
   dm completion <shell> > /path/to/completion/file
   ```

#### Bash-Specific Issues

**Problem:** Completions don't load

**Solutions:**
```bash
# Check if bash-completion is installed
ls /usr/share/bash-completion/bash_completion

# macOS: install bash-completion via Homebrew
brew install bash-completion

# Add to ~/.bashrc if not present:
if [ -f /usr/share/bash-completion/bash_completion ]; then
    . /usr/share/bash-completion/bash_completion
fi

# Reload
source ~/.bashrc
```

#### Zsh-Specific Issues

**Problem:** Completions not showing up

**Solutions:**
```bash
# Clear completion cache
rm ~/.zcompdump
compinit

# Verify fpath includes completion directory
echo $fpath | grep zsh/completions

# If not, add to ~/.zshrc:
fpath=(~/.zsh/completions $fpath)
autoload -Uz compinit && compinit

# Reload
source ~/.zshrc
```

**Problem:** Completions are outdated

**Solution:**
```bash
# Force rebuild completion cache
rm ~/.zcompdump && exec zsh
```

#### Fish-Specific Issues

**Problem:** Completions not appearing

**Solutions:**
```bash
# Check if completion file exists
ls ~/.config/fish/completions/dm.fish

# Reload fish configuration
source ~/.config/fish/config.fish

# Or restart fish
exec fish
```

#### Dynamic Completions Not Working

**Problem:** Branch names don't appear in completions

**Diagnosis:**
```bash
# Check if you're in a Diamond-initialized repository
git show-ref refs/diamond/config/trunk

# Verify branches are tracked
git for-each-ref --format='%(refname:short)' refs/diamond/parent/
```

**Common causes:**
1. Not in a git repository
2. Diamond not initialized (`dm init` not run)
3. No branches tracked yet
4. Corrupted refs

### Updating Completions

When Diamond is updated with new commands or options, regenerate completion scripts:

```bash
# Bash
dm completion bash > ~/.local/share/bash-completion/completions/dm
source ~/.bashrc

# Zsh
dm completion zsh > ~/.zsh/completions/_dm
rm ~/.zcompdump && compinit

# Fish
dm completion fish > ~/.config/fish/completions/dm.fish
```

---

## Diamond Configuration Layers

Configuration is loaded from multiple sources, with later sources overriding earlier ones:

| Priority | Location | Scope | Committed |
|----------|----------|-------|-----------|
| 1 (lowest) | Defaults | — | — |
| 2 | `.diamond/config.toml` | Repository (shared) | Yes |
| 3 | `~/.config/diamond/config.toml` | User (global) | No |
| 4 (highest) | `.git/diamond/config.toml` | Local (per-repo) | No |

**Use cases:**
- **Repository config** (`.diamond/`) — Team settings like custom remote name
- **User config** (`~/.config/diamond/`) — Personal defaults like branch prefix
- **Local config** (`.git/diamond/`) — Per-repo overrides for personal preferences

## Configuration Options

### repo.remote

Git remote name for push/pull operations.

```toml
# .diamond/config.toml
remote = "upstream"
```

| Property | Value |
|----------|-------|
| Default | `origin` |
| Scope | Repository only |

**Set via CLI:**
```bash
dm config set repo.remote upstream
```

This setting is always stored in `.diamond/config.toml` (repository config) so it can be committed and shared with your team.

---

### branch.format

Template for auto-generated branch names when using `dm create -m "message"`.

```toml
# ~/.config/diamond/config.toml
[branch]
format = "{prefix}{date}-{name}"
```

| Property | Value |
|----------|-------|
| Default | `{date}-{name}` |
| Scope | User or Local |

**Available placeholders:**

| Placeholder | Description | Example |
|-------------|-------------|---------|
| `{name}` | Slugified branch name | `add_login` |
| `{date}` | Current date (MM-DD) | `12-28` |
| `{prefix}` | User-defined prefix | `alice/` |

**Examples:**

```toml
# Default: "12-28-add_login"
format = "{date}-{name}"

# With prefix: "alice/add_login"
format = "{prefix}{name}"

# Full: "alice/12-28-add_login"
format = "{prefix}{date}-{name}"
```

**Set via CLI:**
```bash
dm config set branch.format "{prefix}{name}"
dm config set branch.format "{prefix}{name}" --local  # This repo only
```

---

### branch.prefix

User-defined prefix for branch names. Include your separator (e.g., `/` or `-`).

```toml
# ~/.config/diamond/config.toml
[branch]
prefix = "alice/"
```

| Property | Value |
|----------|-------|
| Default | (none) |
| Scope | User or Local |

**Set via CLI:**
```bash
dm config set branch.prefix "alice/"
dm config set branch.prefix "alice/" --local  # This repo only
```

**Note:** The prefix is only used when the format template includes `{prefix}`.

---

## CLI Commands

### dm config show

Display merged configuration from all sources.

```bash
$ dm config show
Repository Configuration:
  remote: origin

Branch Configuration:
  format: {date}-{name}
  prefix: alice/

Config file locations:
  repo:  /path/to/repo/.diamond/config.toml (not found)
  user:  /home/user/.config/diamond/config.toml (exists)
  local: /path/to/repo/.git/diamond/config.toml (not found)
```

### dm config get

Get a specific configuration value.

```bash
dm config get repo.remote      # → origin
dm config get branch.format    # → {date}-{name}
dm config get branch.prefix    # → alice/
```

### dm config set

Set a configuration value.

```bash
# Set in user config (default)
dm config set branch.prefix "alice/"

# Set in local config (this repo only)
dm config set branch.prefix "alice/" --local

# repo.remote always goes to repository config
dm config set repo.remote upstream
```

### dm config unset

Remove a configuration value.

```bash
dm config unset branch.prefix
dm config unset branch.prefix --local
```

---

## Example Configurations

### Personal Developer Setup

`~/.config/diamond/config.toml`:
```toml
[branch]
format = "{prefix}{name}"
prefix = "alice/"
```

Creates branches like `alice/add_login` when using `dm create -m "Add login"`.

### Team Repository

`.diamond/config.toml` (committed):
```toml
remote = "upstream"
```

All team members will push to/pull from `upstream` instead of `origin`.

### Date-Prefixed Branches

`~/.config/diamond/config.toml`:
```toml
[branch]
format = "{date}-{name}"
```

Creates branches like `12-28-add_login` (default behavior).

### Combined Prefix and Date

`~/.config/diamond/config.toml`:
```toml
[branch]
format = "{prefix}{date}-{name}"
prefix = "alice/"
```

Creates branches like `alice/12-28-add_login`.

---

## Behavior Notes

### Explicit Names Bypass Formatting

When you provide an explicit branch name, formatting is not applied:

```bash
dm create my-feature           # → my-feature (exact name)
dm create -m "Add feature"     # → 12-28-add_feature (formatted)
```

### Empty Prefix

If `branch.prefix` is not set, `{prefix}` becomes an empty string:

```toml
format = "{prefix}{name}"  # With no prefix set → "add_feature"
```

### Config File Creation

Config files are created automatically when you run `dm config set`. Parent directories are created as needed.

---

## File Format Reference

All configuration files use TOML format.

### Repository Config (`.diamond/config.toml`)

```toml
remote = "origin"
```

### User/Local Config

```toml
[branch]
format = "{prefix}{date}-{name}"
prefix = "alice/"
```

---

## Troubleshooting

### Config not taking effect

1. Check which config file is being used:
   ```bash
   dm config show
   ```

2. Verify the file exists and has correct syntax:
   ```bash
   cat ~/.config/diamond/config.toml
   ```

3. Check for typos in key names (e.g., `format` not `Format`)

### Wrong remote being used

The remote is loaded from `.diamond/config.toml` only. Check:
```bash
dm config get repo.remote
cat .diamond/config.toml
```

### Branch names not formatted

Formatting only applies to auto-generated names (`-m` flag). Explicit names are used as-is.
