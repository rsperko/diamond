# Diamond CLI - Development Tasks
# Run `just` or `just --list` to see available commands

# Default recipe shows help
default:
    @just --list

# Build debug binary
build:
    cargo build

# Build release binary
release:
    cargo build --release

# Run all tests in parallel (nextest)
test:
    cargo nextest run

# Run only unit tests (nextest)
test-unit:
    cargo nextest run --bin dm

# Run only integration tests (nextest)
test-integration:
    cargo nextest run --test '*'

# Run tests with cargo test (fallback)
test-cargo:
    cargo test

# Format code with rustfmt
fmt:
    cargo fmt

# Run clippy linter
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Run fmt + clippy + tests (pre-commit validation)
check:
    cargo fmt
    cargo clippy --all-targets --all-features -- -D warnings
    cargo nextest run

# Install dm binary to ~/.cargo/bin
install:
    cargo install --path .

# Clean build artifacts and sandbox
clean:
    cargo clean
    rm -rf sandbox/
    rm -rf target/nextest/

# Setup git hooks for quality checks
setup-hooks:
    @echo "Setting up git hooks..."
    git config core.hooksPath .githooks
    chmod +x .githooks/pre-commit
    @echo "✓ Git hooks configured"
    @echo ""
    @echo "Pre-commit checks enabled:"
    @echo "  • Gitleaks (secret detection)"
    @echo "  • Test signature blocking"

# Publish to crates.io (dry-run first)
publish-check:
    cargo publish --dry-run

# Publish to crates.io (for real)
publish:
    @echo "Publishing to crates.io..."
    cargo publish

# Create a new release (updates version, changelog, tags)
# Usage: just release-patch  (0.1.0 -> 0.1.1)
#        just release-minor  (0.1.x -> 0.2.0)
release-patch version:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Creating patch release: {{version}}"
    echo "Remember to update CHANGELOG.md manually before running this!"
    read -p "Press enter to continue or Ctrl+C to cancel..."
    # Update version in Cargo.toml
    sed -i '' 's/^version = ".*"/version = "{{version}}"/' Cargo.toml
    # Commit and tag
    git add Cargo.toml CHANGELOG.md
    git commit -m "chore: release v{{version}}"
    git tag v{{version}}
    echo "✓ Created tag v{{version}}"
    echo ""
    echo "Next steps:"
    echo "  git push origin main --tags"
    echo "  cargo publish"

# Create isolated test repo for manual testing
playground:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Creating playground test repository..."
    mkdir -p sandbox
    cd sandbox
    rm -rf test-repo
    mkdir test-repo
    cd test-repo
    git init
    git config user.name "Test User"
    git config user.email "test@example.com"
    echo "# Test Repository" > README.md
    git add .
    git commit -m "Initial commit"
    ../../target/debug/dm init
    echo ""
    echo "Playground created at: sandbox/test-repo"
    echo ""
    echo "To use:"
    echo "  cd sandbox/test-repo"
    echo "  ../../target/debug/dm <command>"
    echo ""
    echo "Or add an alias:"
    echo "  alias dm='../../target/debug/dm'"
    echo ""
