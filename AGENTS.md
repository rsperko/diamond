# Repository Guidelines

> **Core Philosophy**
> We are making an **enterprise-grade tool**. We do not want to overengineer it, but it must be **robust and ready for anything**. Reliability is achieved through simplicity and exhaustive testing, not complexity.

## 🤖 LLM & Agent Guidelines

**These instructions are specifically for AI agents working on this project. Read this section first.**

### 0. 🛑 CRITICAL SAFETY RULE: NO EXPERIMENTING ON DIAMOND REPO
**ABSOLUTE RULE: When fixing bugs, reproducing issues, or testing changes, YOU MUST NOT COMMIT TO OR MODIFY THE DIAMOND REPOSITORY DIRECTLY.**

- **NEVER** run `git commit`, `git rebase`, or `dm` commands on the Diamond repository itself to test things.
- **ALWAYS** use `just playground` or `cd sandbox/` to reproduce bugs and verify fixes.
- **EXCEPTION**: You may modify source code (`src/`) and tests (`tests/`) as part of the fix, but you must NOT use the repository's git history as your testbed.
- Modifying the tool you are building while it is running on itself is dangerous and leads to repository corruption.

#### 🛡️ Safety Mechanisms

Diamond has three layers of protection to prevent unit tests from committing to the repository:

1. **Runtime Safety Checks** - `RefStore::new()` and `GitGateway::new()` panic in test mode if `TestRepoContext` is not set:
   ```rust
   // This will panic with helpful error message:
   let ref_store = RefStore::new()?;  // ❌ Missing TestRepoContext

   // Correct usage:
   let dir = tempdir()?;
   let _ctx = TestRepoContext::new(dir.path());  // ✅ Isolates test
   let ref_store = RefStore::new()?;
   ```

2. **Pre-commit Git Hook** - `.git/hooks/pre-commit` blocks commits with test author signatures:
   - Rejects commits from "Test User <test@example.com>" or similar test patterns
   - Prevents unit test accidents from reaching git history
   - Warns if test signatures appear in code changes

3. **TestRepoContext RAII Guard** - Thread-local test isolation (see `src/test_context.rs`):
   - Sets temporary repo path for the test
   - Automatically cleans up on drop (even on panic)
   - Prevents tests from accessing diamond repository

**If you write a unit test, you MUST use TestRepoContext:**
```rust
#[test]
fn test_something() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());  // REQUIRED

    // Now safe to call RefStore::new(), GitGateway::new(), etc.
    Ok(())
}
```

**Without TestRepoContext, the test will panic immediately with a clear error message.**

### 1. ⚡ Efficiency & Token Usage
- **Don't guess, check**: Read `Cargo.toml` before suggesting new dependencies.
- **Error Analysis**: When you see an error, use `anyhow::Context` to explain *why* it happened, not just *what* failed.
- **Context Awareness**: You don't need to read every file. Trust `src/git_gateway.rs` for git operations and `src/state.rs` for metadata.

### 2. 🦀 Rust Design Patterns
**Stop guessing patterns from other languages (Java/Python/TS). Think in Rust.**

- **Ownership First**: Before writing an API, decide who owns the data.
- **Borrowing**: Prefer passing references (`&str`, `&Path`) over cloning (`String`, `PathBuf`) unless ownership transfer is required.
- **Trait Bounds**: Use clear trait bounds.
    - ❌ *Bad*: `fn process(input: String)` (Forces allocation)
    - ✅ *Good*: `fn process<T: AsRef<str>>(input: T)` (Flexible, zero-cost)
- **Simplicity**: Do not over-engineer traits "just in case". implementation `Into<T>` is often better than a complex builder pattern.

### 3. 🧪 Testing Protocol
**We are burning tokens on full test suite runs. Stop it.**

- **Your Responsibility**: Run **ONLY** the specific unit/integration tests relevant to your changes.
    - `cargo test --lib commands::sync`
    - `cargo test --test integration_tests -- test_sync_basic`
- **Handoff Protocol**: When you are confident, **ask the user** to run the full suite:
    > "I have verified my changes with specific tests. Please run `just test` (or `cargo t`) to ensure no regressions."

### 4. 🔴 TDD Enforcement (Red-Green-Refactor)
**MANDATORY WORKFLOW - Do not skip steps:**

1. **STOP** - Before writing ANY implementation code, write the test first
2. **Show the Red** - Run the test and paste the failure output to prove it fails
3. **Implement** - Write the minimal code to make the test pass
4. **Show the Green** - Run the test again and confirm it passes
5. **Refactor** - Only if needed, keeping tests green

**If you catch yourself writing implementation before a failing test exists, STOP immediately and write the test first.**

### 5. 💎 UX Principles for Enterprise-Grade CLI

**Diamond is a professional tool for engineering teams. UX is not an afterthought.**

For comprehensive guidelines, see `agent_notes/ux_principles/`. These are the critical rules:

#### Error Messages

**Format** (inform, don't prescribe):
```
[WHAT] is [PROBLEM]:
  [SPECIFIC DETAILS]
```

**Do**:
- ✅ Show information you already have (don't make users run extra commands)
- ✅ Use real values, not `<placeholders>`
- ✅ Hide implementation details (no exit codes, internal commands, stack traces)
- ✅ Professional tone (no emoji spam, no "Pwease" language)

**Don't**:
- ❌ Assume user intent (especially destructive actions like deletion)
- ❌ Suggest commands when multiple valid approaches exist
- ❌ Make users run another command for info you already queried
- ❌ Give users numbered lists of 5 options (minimize cognitive load)

**Example** (worktree conflict):
```rust
// ❌ Bad - assumes destructive intent
bail!("Branch '{}' is already checked out at:\n  {}\n\nTo remove that worktree:\n  git worktree remove {}",
    name, path, path);

// ✅ Good - informative without prescribing
bail!("Branch '{}' is already checked out at:\n  {}", name, path);
```

**Why**: Most worktree users have persistent worktrees they reuse. Suggesting deletion assumes the wrong (and rare) intent.

#### Interactive Features - CRITICAL TTY Detection

**ALL interactive code MUST detect TTY before prompting or launching TUI**:

```rust
// Before launching TUI (dm log, dm checkout with no args)
if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
    anyhow::bail!("This command requires a terminal. Use --format=short for scripts.");
}

// Before prompting for confirmation
if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
    anyhow::bail!("This command requires confirmation. Use --force to skip.");
}
```

**Why**: Tests, CI, and scripts run in non-TTY environments. Without this check, they hang forever waiting for input that never comes.

#### Output and Consistency

**Silent success, loud failure**:
- Operations that succeed normally show minimal output
- Only show progress for operations >2 seconds
- Reserve color for errors (red), warnings (yellow), success confirmations (green)

**Consistent vocabulary**:
- **Branch** (not "ref" or "head")
- **Stack** (the tree of branches)
- **Trunk** (main/master branch)
- **--force** for skipping confirmations (not `--yes`, `--skip-prompt`, `--no-interactive`)

#### UX Validation Checklist

Before shipping a feature:

- [ ] Error messages pass "3am test" (can user fix without Googling?)
- [ ] No `<placeholders>` in suggested commands
- [ ] TTY detection for all interactive features
- [ ] Consistent flag usage (`--force`, not `--yes`)
- [ ] Silent success (don't say "Successfully completed!" for normal operations)
- [ ] No assumptions about destructive intent

**For detailed guidance**: See `agent_notes/ux_principles/`
- `error_messages.md` - Comprehensive error message principles
- `cli_design.md` - CLI design patterns and anti-patterns
- `lessons_learned.md` - Real mistakes and how we fixed them

## Project Structure & Module Organization

- `Cargo.toml` / `Cargo.lock`: Rust workspace metadata and dependency locks.
- `src/main.rs`: CLI entrypoint (`clap`) and subcommand routing.
- `src/commands/`: Subcommand implementations (e.g., `create`, `sync`, `restack`, `move`).
- `src/git_gateway.rs`: Unified interface for all `git2` operations.
- `src/state.rs`: Operation state persistence under `.git/diamond/operation_state.json`.
- `src/ref_store.rs`: Stack metadata persistence using git refs (`refs/diamond/parent/*`).
- `src/operation_log.rs`: Transaction history for undo/redo and crash recovery.
- `src/validation.rs`: Integrity checks for stack data and git state.
- `src/forge/`: Hosting provider integrations (e.g., GitHub).
- `agent_notes/`: Design/architecture notes (not part of the shipped binary).
  - `agent_notes/ux_principles/`: Comprehensive UX guidelines for error messages, CLI design, and lessons learned.

## Build, Test, and Development Commands

### Quick Reference
- `just`: Show all available commands
- `just test` or `cargo t`: Run all tests (unit + integration)
- `just check`: Run fmt + clippy + tests (pre-commit validation)
- `just playground`: Create isolated test repo for manual testing (**USE THIS, NOT THE REAL REPO**)
- `cargo build`: Compile a debug build
- `cargo run -- --help`: Run the CLI and show available subcommands
- `cargo run -- log`: Launch the TUI stack view (`ratatui`/`crossterm`)
- `cargo fmt`: Format with `rustfmt` (run before opening a PR)
- `cargo clippy --all-targets --all-features -- -D warnings`: Lint; treat warnings as errors

### ⚠️ CRITICAL: Safe Testing Environments

**NEVER test features directly in the Diamond repository!**

This project provides safe, isolated environments for testing:

#### 1. Automated Integration Tests (`tests/`)
- All integration tests run in temporary directories that auto-cleanup
- Tests spawn the actual `dm` binary for end-to-end validation
- Run with: `just test-integration` or `cargo ti` or `cargo test --test '*'`
- **Structure**:
  - `basic_tests.rs`: Core command functionality
  - `stack_ops_tests.rs`: Complex stack manipulations
  - `sync_restack_tests.rs`: Rebase and sync logic
  - `battle_hardness_tests.rs`: Chaos testing and stress scenarios
  - `edge_cases_tests.rs`: Robustness against bad states

#### 2. Manual Testing Playground (`sandbox/`)
```bash
just playground          # Creates sandbox/test-repo with git initialized
cd sandbox/test-repo    # Enter the test environment
../../target/debug/dm create my-feature  # Test commands safely
```

The playground is git-ignored and disposable. When exploring features or debugging, **always use the playground**, never the main repository.

#### 3. Unit Tests (Automatic Isolation)
- All unit tests use `tempdir()` to create isolated temporary repos
- Tests use `std::env::set_current_dir()` when needed
- No test should ever modify the Diamond repository

## Coding Style & Naming Conventions

- Rust 2021 edition; 4-space indentation (standard `rustfmt` defaults).
- Modules/files: `snake_case.rs` under `src/` and `src/commands/`.
- Types/traits: `CamelCase`; functions/vars: `snake_case`; constants: `SCREAMING_SNAKE_CASE`.
- Prefer small, focused command modules; keep CLI parsing in `src/main.rs` and logic in `src/commands/*`.
- **Dynamic Program Name**: Use `program_name()` instead of hardcoding "dm" in help and error messages.

## 🖥️ Interactive Features & TUI Guidelines

**CRITICAL: All interactive code must handle non-TTY environments.**

### TTY Detection Requirement

Any code that:
- Launches a TUI (`enable_raw_mode`, `Terminal::new`, `EnterAlternateScreen`)
- Reads from stdin (`stdin().read_line()`)
- Waits for user input

**MUST check if running in a TTY before attempting interaction.**

### Implementation Pattern

```rust
use std::io::IsTerminal;

// For TUI commands (log, checkout)
if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
    // Fall back to non-interactive mode or return clear error
    anyhow::bail!("command requires interactive mode. Usage: dm command <args>");
}

// For stdin prompts (cleanup confirmation)
if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
    anyhow::bail!("command requires --force when running non-interactively");
}
```

### Why This Matters

**Without TTY detection:**
- ❌ Tests hang forever waiting for input that never comes
- ❌ CI/CD pipelines timeout or fail mysteriously
- ❌ Scripts/automation break without clear error messages
- ❌ Test suite takes 2+ minutes instead of seconds

**With TTY detection:**
- ✅ Tests complete immediately with clear pass/fail
- ✅ Non-interactive environments get actionable error messages
- ✅ Scripts know to pass required flags (e.g., `--force`)
- ✅ Full test suite runs in seconds

### Testing Interactive Features

When adding interactive features:
1. **Add TTY detection FIRST** before any interactive code
2. **Write integration test** that calls the command from a non-TTY context
3. **Verify timeout protection**: Test should complete in <1 second
4. **Test error messages**: Ensure non-TTY errors are clear and actionable

### Current Interactive Commands

| Command | TTY Check | Fallback Behavior |
|---------|-----------|-------------------|
| `dm log` | ✅ stdout | Falls back to `short` mode |
| `dm checkout` | ✅ stdout | Returns error asking for branch name |
| `dm cleanup` | ✅ stdin | Returns error asking for `--force` |

### The `--force` / `-f` Convention

**All commands that prompt for confirmation MUST accept `--force` (and `-f`) to bypass the prompt.**

This is a Diamond-wide convention. Do not use `--no-interactive`, `--yes`, `-y`, or other variants.

**Rationale:**
- **Unix tradition**: `rm -f`, `git push -f`, `docker rm -f` — developers have muscle memory for `-f`
- **Simplicity**: One flag to remember: "if it prompts, add `--force`"
- **Internal consistency**: All Diamond commands behave the same way
- **Brevity**: `-f` is quick to type in scripts

**Implementation pattern:**
```rust
#[derive(Parser)]
struct Args {
    /// Skip confirmation prompt
    #[arg(short, long)]
    force: bool,
}

// In command logic:
if !args.force && std::io::stdin().is_terminal() {
    // Show interactive prompt
    print!("Are you sure? [y/N] ");
    // ... handle input
} else if !args.force {
    anyhow::bail!("This command requires confirmation. Use --force to skip.");
}
// Proceed with operation
```

**Error messages** should always tell users about `--force`:
- ✅ `"This command requires confirmation. Use --force to skip."`
- ❌ `"Cannot run in non-interactive mode."`

## 🎯 Simplicity Principle

**CRITICAL: Always choose the simplest solution that works.**

This is a CLI tool for managing git branches, not a framework. Avoid:
- ❌ Over-abstraction: Don't create layers "for future flexibility"
- ❌ Premature optimization: Don't optimize code that isn't proven slow
- ❌ Complex patterns: Don't use design patterns unless they solve a real problem
- ❌ Unnecessary generics: Don't make code generic "just in case"
- ❌ Feature creep: Don't add features that weren't requested

Do:
- ✅ **Write straightforward code**: If it's simple and works, ship it
- ✅ **Solve the actual problem**: Address the specific need, nothing more
- ✅ **Delete code when possible**: Less code = fewer bugs
- ✅ **Copy-paste over abstraction**: Three similar functions are better than one confusing abstraction
- ✅ **Use stdlib first**: Prefer standard library over external crates when reasonable

**Rule of thumb:** If you're debating between a simple solution and a "clever" one, choose simple every time. You can always refactor later when complexity is justified by real requirements.

### YAGNI: You Aren't Gonna Need It

**CRITICAL: Do not write code "just in case" or "for future use".**

- ❌ **No `#[allow(dead_code)]`**: If code isn't used, delete it. Don't preserve it for hypothetical future needs.
- ❌ **No speculative features**: Don't add parameters, flags, or methods that aren't needed by current requirements.
- ❌ **No "reserved for future use"**: Comments like "TODO: might need this later" are a code smell. Delete the code.

**Why this matters:**
- Dead code rots: It doesn't get tested, updated, or maintained
- It creates confusion: Future readers wonder if they're missing something
- It's trivially recoverable: Git history preserves everything - you can always bring it back

**When you're tempted to keep unused code:**
1. Delete it
2. If you need it later, `git log -S "function_name"` will find it
3. Rewriting from scratch with fresh context is often better anyway

## Test Driven Development & Testing

**TDD (Red-Green-Refactor) is mandatory for this project.**

1. **Red**: Write a failing test for the desired behavior *before* writing any implementation code.
2. **Green**: Write the minimal code to make the test pass.
3. **Refactor**: Improve code structure while ensuring tests remain green.

### Guidelines
- **Unit Tests**: Co-locate with code in `#[cfg(test)] mod tests { ... }`.
- **Integration Tests**: Place in `tests/` for end-to-end CLI behavior (e.g., `tests/log_command.rs`).
- **Deterministic**: Tests must not rely on the environment (e.g., user's global git config).
- **Bug Fixes**: Always start by writing a test that reproduces the bug.

## ⚠️ Code Quality & Reliability Requirements

**CRITICAL: This tool manipulates git repositories and stack metadata. Bugs can cause data loss, repository corruption, or orphaned branches. The company's codebase depends on this tool's reliability.**

### Mandatory Test Coverage Standards

**Target: 85-95% code coverage for all non-TUI code.**

Every module must have comprehensive test coverage before being considered complete:

1. **All Happy Paths**: Every successful code path must be tested with real git operations
2. **All Error Paths**: Every failure mode must be tested and verified to fail gracefully
3. **All Edge Cases**: Empty inputs, corrupted data, missing files, detached HEAD, orphaned branches
4. **All State Mutations**: Every operation that modifies stack metadata or git state must be verified

### What Must Be Tested

#### git_gateway.rs (Git Operations)
- ✅ Branch creation, checkout, and existence checks
- ✅ Error handling: duplicate branches, non-existent branches, detached HEAD
- ✅ Current branch detection in all states
- ✅ Repository operations on real temp repositories (not mocked)
- ✅ Backup reference management for undo operations

#### state.rs & validation.rs (Metadata & Integrity)
- ✅ Save/load cycles with round-trip verification
- ✅ Corrupted JSON handling (must fail gracefully, not panic)
- ✅ Missing file handling (return empty state, not error)
- ✅ Parent-child relationship integrity (ConsistencyValidator)
- ✅ Cycle detection (CycleValidator)
- ✅ Orphaned branch handling
- ✅ Complex tree structures (multi-level stacks)
- ✅ Branch removal with and without children

#### commands/* (Command Logic)
- ✅ Full command workflows (git + metadata changes)
- ✅ Parent tracking accuracy
- ✅ Error messages for user mistakes
- ✅ Idempotency where appropriate
- ✅ State consistency after operations

### Critical Test Patterns

**Use serial_test for directory-changing tests:**
```rust
#[test]
#[serial]
fn test_with_cwd_change() -> Result<()> {
    let dir = tempdir()?;
    std::env::set_current_dir(dir.path())?;
    // Test logic
}
```

**Use real git repositories:**
```rust
fn init_test_repo(path: &Path) -> Result<Repository> {
    let repo = Repository::init(path)?;
    // Make initial commit so HEAD is valid
    let sig = git2::Signature::now("Test", "test@example.com")?;
    let tree_id = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    repo.commit(Some("HEAD"), &sig, &sig, "Initial", &tree, &[])?;
    drop(tree);
    Ok(repo)
}
```

**Test both success and failure:**
```rust
#[test]
fn test_duplicate_branch_fails() -> Result<()> {
    create_branch("feature")?;
    let result = create_branch("feature");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));
    Ok(())
}
```

### When Adding New Features

**MANDATORY CHECKLIST:**

- [ ] Write tests BEFORE implementation (TDD)
- [ ] Test happy path with real git operations (in unit tests with `tempdir()`)
- [ ] Add integration test for end-to-end workflow (e.g., `tests/basic_tests.rs` or specialized suite)
- [ ] Test all error conditions
- [ ] Test edge cases (empty, null, corrupted, missing)
- [ ] Verify state consistency after operations
- [ ] Manual verification in playground: `just playground` then test the feature
- [ ] Run `just test` or `cargo t` - all tests must pass (unit + integration)
- [ ] Run `cargo clippy -- -D warnings` - zero warnings
- [ ] Coverage: Aim for 90%+ of new code paths

**NEVER manually test features in the Diamond repository - always use `just playground`**

### Consequences of Insufficient Testing

**Without comprehensive tests, this tool could:**
- Corrupt the `refs/diamond/*` refs, losing all stack metadata
- Create orphaned branches that appear "lost" to users
- Fail to detect detached HEAD state, corrupting parent relationships
- Accept invalid state, causing crashes in `dm log`
- Leave the git repository in an inconsistent state
- Cause data loss that affects the entire engineering team

**Rule of thumb:** If you wouldn't trust the code with your company's main repository without the test, the test coverage is insufficient.

### Current Test Coverage Status

**As of late 2025: Comprehensive test suite with >90% coverage**

| Test Type | Location | Purpose |
|-----------|----------|---------|
| Unit Tests | `src/**/*.rs` | Module-level testing (git gateway, state validation, operational log) |
| Basic Integration | `tests/basic_tests.rs` | Core command functionality |
| Stack Operations | `tests/stack_ops_tests.rs` | Complex parent/child manipulations |
| Sync & Restack | `tests/sync_restack_tests.rs` | Rebasing and syncing logic |
| Navigation | `tests/navigation_tests.rs` | Movement between branches |
| Battle Hardness | `tests/battle_hardness_tests.rs` | Stress testing and complex workflows |
| Edge Cases | `tests/edge_cases_tests.rs` | Robustness against invalid states |

**All tests must pass with zero warnings:**
```bash
$ just test  # or: cargo t
$ cargo clippy --all-targets --all-features -- -D warnings
```

## Commit & Pull Request Guidelines

### ⚠️ Protected Branch Workflow

**Main branch is protected on GitHub.** Direct commits are blocked.

All changes must go through pull requests:
1. Create feature branch: `git checkout -b feature/your-feature`
2. Commit using conventional commits (`feat:`, `fix:`, `docs:`, etc.)
3. Push and open PR for review
4. **Never** commit directly to `main` or force-push to protected branches

### General Guidelines

- Git history is not established yet; use Conventional Commits (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`) with an imperative subject.
- PRs should include: what/why, repro or verification steps, and (for TUI changes) a screenshot or short recording.
- Required before merge: `cargo fmt`, `cargo clippy ... -D warnings`, and `cargo test` passing locally.

## Configuration & Safety Notes

- Diamond writes stack state to `refs/diamond/*` (git refs); do not hand-edit unless you know the format.
- `log` uses an alternate screen; if the terminal looks "stuck", run `reset` after exiting.

## 🧪 Testing Reminder

**Before testing any feature manually:**
1. Run `just playground` to create an isolated test environment
2. Navigate to `sandbox/test-repo/`
3. Test your feature there, not in the Diamond repository

**Before committing code:**
1. Run `just check` to validate formatting, linting, and all tests
2. Ensure all 206 tests pass (200 unit + 6 integration)
3. Add integration tests for new user-facing features in `tests/integration_test.rs`

## 🔴 QA Agent Guidelines (Diamond-Specific)

When using the `qa-tester` agent on this project, these are the Diamond-specific testing concerns:

### Environment Setup
- **Always** run `just playground` first to create an isolated test environment
- Work in `sandbox/test-repo/`, **never** the main Diamond repository
- Use `../../target/debug/dm` or `../../target/release/dm` to run the CLI

### State to Monitor
- `refs/diamond/parent/*` - Branch parent relationships (verify consistency)
- `refs/diamond/config/trunk` - Trunk configuration
- `.git/diamond/operation_state.json` - In-progress operation state

### Git State Edge Cases
Test `dm` commands in these challenging git states:
- Detached HEAD
- During merge conflict
- During rebase
- With dirty working tree (uncommitted changes)
- With branches that have diverged from remote
- With orphaned branches (parent deleted)
- With circular parent references (should be impossible, but verify)

### Critical Paths to Stress Test
1. **Stack Creation**: `dm create` with special chars, unicode, spaces, reserved names
2. **State Corruption Recovery**: Corrupt refs/diamond/* refs, then run commands
3. **Concurrent Operations**: Rapid `dm create && dm sync && dm restack`
4. **Navigation**: `dm up`, `dm down`, `dm top`, `dm bottom` at stack boundaries
5. **Cleanup**: `dm cleanup` with PRs in various states (merged, closed, open)

### Expected Error Behaviors
- Commands should fail gracefully with helpful messages, never panic
- State should remain consistent after errors (no partial updates)
- Interactive commands (`dm log`, `dm checkout`) must detect non-TTY and fail fast
