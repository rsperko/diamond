# Diamond Agent Guidelines

> Enterprise-grade CLI for stacked branches. Reliability through simplicity and exhaustive testing.

<critical_rules>

## Non-Negotiable Rules

1. **Test Isolation**: All testing happens in `sandbox/` (via `just playground`) or `tempdir()`. Never run `git commit`, `dm` commands, or modify git history on the Diamond repository itself.

2. **TDD Workflow**: Write failing test → Run and show failure → Implement minimal fix → Run and show green → Refactor if needed.

3. **Targeted Testing**: Run only tests relevant to your changes (`cargo test --lib module::name`). Ask the user to run `just test` at handoff.

4. **TTY Detection**: All interactive features must check `is_terminal()` before prompting or launching TUI.

5. **Minimal Solutions**: Implement what's requested, nothing more. Delete unused code. No speculative features.

</critical_rules>

<testing>

## Testing Protocol

### Unit Test Pattern (Required)

```rust
#[test]
fn test_feature() -> Result<()> {
    let dir = tempdir()?;
    let _repo = init_test_repo(dir.path())?;
    let _ctx = TestRepoContext::new(dir.path());  // REQUIRED - panics without this

    // Test logic here
    Ok(())
}
```

`TestRepoContext` isolates tests from the real repository. Without it, `RefStore::new()` and `GitGateway::new()` panic immediately.

### Test Locations

| Type | Location | Run Command |
|------|----------|-------------|
| Unit | `src/**/*.rs` | `cargo test --lib module::name` |
| Integration | `tests/*.rs` | `cargo test --test test_name` |

### Handoff Protocol

When confident in your changes:

> "I've verified with targeted tests. Please run `just test` to check for regressions."

</testing>

<patterns>

## Code Patterns

### Rust Idioms

```rust
// Prefer references over owned types
fn process(input: &str, path: &Path) -> Result<()>  // Good
fn process(input: String, path: PathBuf) -> Result<()>  // Avoid unless ownership needed

// Use anyhow::Context for error messages
file.read_to_string(&mut buf)
    .context("Failed to read config file")?;  // Explains why, not just what
```

### TTY Detection (Required for Interactive Features)

```rust
use std::io::IsTerminal;

// Before TUI or prompts
if !std::io::stdout().is_terminal() {
    anyhow::bail!("Requires terminal. Use --format=short for scripts.");
}

// Before confirmation prompts
if !std::io::stdin().is_terminal() {
    anyhow::bail!("Requires confirmation. Use --force to skip.");
}
```

**Verification**: Command completes in <1 second with actionable error in non-TTY environment.

### Error Messages

Format: `[WHAT] is [PROBLEM]:\n  [SPECIFIC DETAILS]`

```rust
// Good - informative, no assumptions
bail!("Branch '{}' is already checked out at:\n  {}", name, path);

// Avoid - assumes destructive intent
bail!("Branch '{}' exists. To delete: git branch -D {}", name, name);
```

Rules:

- Show values you already have (no placeholders)
- Hide implementation details (no exit codes, stack traces)
- Don't assume user intent for destructive actions

</patterns>

<conventions>

## Conventions

### CLI Flags

| Purpose | Use | Avoid |
|---------|-----|-------|
| Skip confirmation | `--force` / `-f` | `--yes`, `-y`, `--no-interactive` |
| Non-TTY output | `--format=short` | `--quiet`, `--no-tui` |

### Output Style

- **Silent success**: Normal operations produce minimal output
- **Loud failure**: Errors are clear and actionable
- Use `program_name()` not hardcoded "dm" in messages

### Vocabulary

- **Branch** (not "ref" or "head")
- **Stack** (the tree of branches)
- **Trunk** (main/master branch)

</conventions>

<simplicity>

## Simplicity Principles

**Implement the minimal solution that works:**

- Delete unused code (git history preserves it)
- No `#[allow(dead_code)]` - if it's unused, remove it
- Three similar functions beat one confusing abstraction
- Prefer stdlib over external crates when reasonable

**Avoid:**

- Layers "for future flexibility"
- Parameters/flags for hypothetical needs
- Design patterns without real problems to solve

</simplicity>

<structure>

## Project Structure

### Key Files

| File | Purpose |
|------|---------|
| `src/git_gateway/` | Git operations (branch, commit, rebase, refs) |
| `src/ref_store/` | Stack metadata (`refs/diamond/parent/*`) |
| `src/state.rs` | Operation state (`.git/diamond/operation_state.json`) |
| `src/validation.rs` | Integrity checks for stack data |
| `src/test_context.rs` | Test isolation (required for unit tests) |
| `src/config.rs` | Layered TOML configuration |
| `src/forge/` | GitHub/GitLab PR integrations |
| `src/commands/*.rs` | Subcommand implementations |

### Commands

```bash
just playground    # Create isolated test environment in sandbox/
just test          # Run all tests (unit + integration)
just check         # fmt + clippy + tests (pre-commit validation)
cargo test --lib   # Unit tests only
cargo clippy --all-targets -- -D warnings  # Zero warnings required
```

</structure>

<workflow>

## Git Workflow

### Commits

Use conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`

Main branch is protected. All changes via pull request:

1. Create feature branch
2. Implement with TDD
3. Run `just check`
4. Open PR for review

### State Storage

- Stack metadata: `refs/diamond/parent/*`
- Config: `refs/diamond/config/trunk`
- Operation state: `.git/diamond/operation_state.json`

</workflow>

<extended>

## Extended Documentation

For detailed guidance, see `agent_notes/`:

- `ux_principles/error_messages.md` - Error message patterns
- `ux_principles/cli_design.md` - CLI design anti-patterns
- `ux_principles/lessons_learned.md` - Real mistakes and fixes
- `qa_guidelines.md` - QA agent testing protocols

</extended>
