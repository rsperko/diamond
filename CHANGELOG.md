# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Homebrew tap for macOS installation**: Diamond can now be installed via `brew install rsperko/tap/diamond` on macOS, removing the need for a Rust toolchain.

## [0.1.1] - 2026-01-04

### Changed
- **`dm sync` now uses stack-aware conflict handling**: When syncing from trunk, conflicts in other branches are skipped and warned instead of blocking. When syncing from a feature branch, only conflicts in your current stack (ancestors, current, or descendants) will stop the operation. This prevents getting stuck in rebase state for branches you're not working on.

### Fixed
- **PR titles now use commit messages instead of branch names**: When running `dm submit`, PR titles are now generated from the first line of the tip commit message instead of the branch name. This fixes the issue where date-prefixed branch names (e.g., "01-04-add-feature") would become PR titles like "01 04 add feature" instead of using the actual commit message "Add feature".

## [0.1.0] - 2025-01-03
### Added
- Initial public release
- Core stacked PR workflow commands (`create`, `submit`, `sync`, `restack`)
- GitHub and GitLab forge support with full feature parity
- Interactive TUI for stack visualization (`dm log`)
- Stack navigation commands (`up`, `down`, `top`, `bottom`, `checkout`)
- Shell completion for bash/zsh/fish with dynamic branch suggestions
- Comprehensive test suite (1124 tests, ~90% coverage)
- Automated CI/CD with cross-platform testing
- Security scanning with Gitleaks and cargo-audit
- Branch freezing for collaboration (`freeze`, `unfreeze`)
- Stack manipulation (`move`, `delete --reparent`, `squash`)
- Backup and undo system for safety (`undo`, `gc`)
- Stack integrity validation (`doctor --fix`)

### Documentation
- Complete command reference in `docs/COMMANDS.md`
- Workflow tutorials in `docs/WORKFLOWS.md`
- Troubleshooting guide in `docs/TROUBLESHOOTING.md`
- Shell completion guide in `docs/COMPLETIONS.md`
