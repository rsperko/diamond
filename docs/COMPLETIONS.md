# Shell Completion Guide

Diamond provides comprehensive shell completion support for bash, zsh, fish, and other popular shells. This guide covers installation, usage, troubleshooting, and technical details.

## Overview

Diamond's completion system provides:

- ✅ **Static completions**: All 40 subcommands with their options and flags
- ✅ **Dynamic completions**: Branch names from your repository's tracked branches
- ✅ **Multi-shell support**: bash, zsh, fish, elvish, powershell
- ✅ **Instant updates**: Completions reflect current repository state

## Installation

### Bash

**Prerequisites:** bash 4.0 or later

**Installation:**
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

### Zsh

**Prerequisites:** zsh 5.0 or later

**Installation:**
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

### Fish

**Prerequisites:** fish 3.0 or later

**Installation:**
```bash
# Generate and install completion script
dm completion fish > ~/.config/fish/completions/dm.fish

# Completions are loaded automatically in fish
# No need to reload or modify config files
```

Fish completions are typically active immediately. If not, restart your fish shell.

### Elvish

**Installation:**
```bash
dm completion elvish > ~/.config/elvish/completions/dm.elv
# Add to ~/.config/elvish/rc.elv: use completions/dm
```

### PowerShell

**Installation:**
```powershell
dm completion powershell | Out-File -FilePath $PROFILE.CurrentUserAllHosts -Append
```

## Usage

### Basic Completion

**Completing subcommands:**
```bash
dm <TAB>
# Shows: create, checkout, log, track, submit, sync, ...
```

**Completing command aliases:**
```bash
dm co<TAB>
# Completes to: dm checkout
```

**Completing options:**
```bash
dm create --<TAB>
# Shows: --all --message --help

dm submit -<TAB>
# Shows: --stack --force --help
```

### Dynamic Branch Name Completion

Diamond provides **intelligent branch name suggestions** based on your repository's state:

**Checkout tracked branches:**
```bash
dm checkout <TAB>
# Shows only tracked branches from refs/diamond/parent/*
# Example: feat/auth  feat/ui  fix/bug-123
```

**Track untracked branches:**
```bash
dm track <TAB>
# Shows git branches that aren't already tracked
# Example: feature/new-feature  hotfix/critical
```

**Delete branches:**
```bash
dm delete <TAB>
# Shows tracked branches
# Example: old-feature  completed-task
```

**Other commands with branch completion:**
- `dm untrack <TAB>` - tracked branches
- `dm info <TAB>` - tracked branches
- `dm undo <TAB>` - tracked branches
- `dm move --onto <TAB>` - tracked branches

## How It Works

### Static Completions

Diamond uses `clap_complete` to generate shell-specific completion scripts from the CLI definition. When you run `dm completion <shell>`, it:

1. Introspects the CLI structure defined in `src/main.rs`
2. Extracts all subcommands, aliases, options, and flags
3. Generates a native completion script for your shell
4. Outputs to stdout for you to save

### Dynamic Completions

**Branch name completion flow:**

1. You type `dm checkout <TAB>`
2. Your shell calls the completion function
3. Diamond reads `refs/diamond/parent/*` refs to get tracked branches
4. Returns sorted list of branch names
5. Shell displays matching branches

**Performance:**
- Completion queries complete in < 10ms for typical repositories
- No file locking (read-only access)
- Gracefully handles missing or corrupted metadata

**Fallback behavior:**
- If not in a git repository: returns empty list
- If no `refs/diamond/parent/*` refs exist: returns empty list
- If refs are corrupted: returns empty list (logs warning to stderr)

## Troubleshooting

### Completions Not Working

**General debugging:**

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

### Bash-Specific Issues

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

### Zsh-Specific Issues

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

### Fish-Specific Issues

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

### Dynamic Completions Not Working

**Problem:** Branch names don't appear in completions

**Diagnosis:**
```bash
# Check if you're in a Diamond-initialized repository
git show-ref refs/diamond/config/trunk

# Verify branches are tracked
git for-each-ref --format='%(refname:short)' refs/diamond/parent/

# Test completion function directly (bash)
_dm
echo "${COMPREPLY[@]}"
```

**Common causes:**
1. Not in a git repository
2. Diamond not initialized (`dm init` not run)
3. No branches tracked yet
4. Corrupted refs

## Updating Completions

When Diamond is updated with new commands or options:

**Regenerate completion scripts:**
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

Consider adding this to your update workflow (e.g., post-install hook).

## Shell-Specific Features

### Bash

- Supports `COMP_WORDBREAKS` for advanced completion
- Compatible with bash 4.0+ (released 2009)
- Works with bash-completion 2.x framework

### Zsh

- Rich completion descriptions (shown with `setopt list_descriptions`)
- Context-sensitive help text
- Integration with zsh's completion menu

### Fish

- Automatic completion loading (no manual sourcing required)
- Rich descriptions in completion menu
- Fuzzy matching support (if enabled in fish)

## Security Considerations

Diamond's completion system is designed with security in mind:

- **Read-only operations**: Completions never modify repository state
- **Input validation**: Branch names are validated before display
- **No command execution**: Never executes user-provided input during completion
- **Graceful degradation**: Errors result in empty completions, not crashes

**Branch name sanitization:**
- Only displays valid git branch names matching `[a-zA-Z0-9/_-]+`
- No special characters that could be misinterpreted by shells
- No shell metacharacters (`;`, `|`, `&`, etc.)

## Performance

**Typical performance:**
- Static completions (subcommands, options): < 1ms
- Dynamic completions (branch names): < 10ms
- Repositories with 1000+ branches: < 50ms

**Optimization tips:**
- Completions use read-only access (no file locking overhead)
- Branch list is sorted once at load time
- JSON parsing is efficient (serde_json)

## Architecture Notes (for Contributors)

Diamond's completion system has three layers:

### 1. Static Completion Generation (`src/commands/completion.rs`)
- Uses `clap_complete` to generate shell-specific scripts
- Command: `dm completion <shell>`
- Output: Shell-specific completion script to stdout

### 2. Dynamic Completion Engine (`src/completion.rs`)
- Functions:
  - `complete_tracked_branches()` - reads from `refs/diamond/parent/*`
  - `complete_git_branches()` - reads from git repository
  - `complete_for_command()` - routes to appropriate completer
- Error handling: all errors return empty Vec, never panic

### 3. Shell Integration
- Each shell has different completion protocols
- Diamond provides native completion scripts for each shell
- Scripts call back into Diamond for dynamic data when needed

**Testing:**
- Unit tests: `src/completion.rs` (90%+ coverage)
- Integration tests: `tests/completion_test.rs`
- Manual tests: Install and test in bash, zsh, fish

## Supported Shells Summary

| Shell | Min Version | Status | Notes |
|-------|-------------|--------|-------|
| **bash** | 4.0 | ✅ Full support | Most widely used |
| **zsh** | 5.0 | ✅ Full support | macOS default since Catalina |
| **fish** | 3.0 | ✅ Full support | Modern, user-friendly completions |
| **elvish** | 0.18 | ⚠️ Basic support | Static completions only |
| **powershell** | 5.0 | ⚠️ Basic support | Windows users |

## FAQ

**Q: Do completions work when not in a Diamond repository?**
A: Yes. Static completions (subcommands, options) work everywhere. Dynamic completions (branch names) are only available inside Diamond-initialized repositories.

**Q: Will completions slow down my shell?**
A: No. Completion functions are only invoked when you press TAB, and they complete in < 10ms.

**Q: Can I customize the completion behavior?**
A: Yes. The completion scripts are regular shell scripts you can modify after generation. However, regenerating will overwrite your changes.

**Q: What if Diamond metadata is out of sync with git branches?**
A: Completions reflect the state in `refs/diamond/parent/*`. Run `dm doctor --fix` to repair metadata issues.

**Q: Do I need to update completions when Diamond updates?**
A: Only if new commands or options are added. Otherwise, completions continue to work with older versions of the script.

## Getting Help

If completions aren't working:

1. Check this document's troubleshooting section
2. Verify your shell version meets minimum requirements
3. Regenerate the completion script
4. File an issue at [github.com/rsperko/diamond/issues](https://github.com/rsperko/diamond/issues)

Include in your issue:
- Shell name and version (`bash --version`, `zsh --version`, etc.)
- How you installed the completion script
- Output of `dm completion <shell> | head -20`
- Any error messages
