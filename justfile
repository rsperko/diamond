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

# ============================================================================
# RELEASE WORKFLOW (PR-based with auto-merge)
# ============================================================================
# TL;DR - The complete workflow:
#   1. Update CHANGELOG.md on your feature branch (under [Unreleased])
#   2. Merge your PR to main
#   3. Run: just release-patch  (creates release PR with auto-merge)
#   4. Wait ~1-2 minutes for CI to pass and PR to auto-merge
#   Done! 🎉
# ============================================================================
#
# PREREQUISITES (one-time setup):
#   1. Create a crates.io API token:
#      - Go to: https://crates.io/settings/tokens
#      - Click "New Token"
#      - Name: "diamond-releases" (or similar)
#      - Copy the token
#
#   2. Add token to GitHub secrets:
#      - Go to: https://github.com/rsperko/diamond/settings/secrets/actions
#      - Click "New repository secret"
#      - Name: CARGO_REGISTRY_TOKEN
#      - Value: [paste token]
#      - Click "Add secret"
#
#   3. Install GitHub CLI (if not already installed):
#      - macOS: brew install gh
#      - Already authenticated if you can run: gh repo view
#
# HOW IT WORKS:
#   - Local script creates a release PR with version bumps
#   - PR auto-merges when CI passes (respects branch protection)
#   - On merge to main, GitHub Actions detects release and publishes
#   - Main branch stays fully protected (no bypass needed)
#
# DETAILED WORKFLOW:
#
# Step 1: While working on your feature branch
#   - Add entries to CHANGELOG.md under the [Unreleased] section
#   - DO NOT update version in Cargo.toml (release script does this)
#
# Step 2: Merge to main
#   - Get your feature PR merged to main (CI must pass)
#   - The changelog entries come along with the merge
#
# Step 3: Run the release command
#   - git checkout main && git pull
#   - Run: just release-patch   (0.1.0 → 0.1.1)
#      OR: just release-minor   (0.1.x → 0.2.0)
#      OR: just release-major   (1.x.x → 2.0.0)
#
#   The script will:
#   ✓ Calculate the new version number
#   ✓ Create release branch (release/vX.Y.Z)
#   ✓ Update Cargo.toml with the new version
#   ✓ Update Cargo.lock to match
#   ✓ Update CHANGELOG.md: [Unreleased] → [X.Y.Z]
#   ✓ Create a new empty [Unreleased] section
#   ✓ Commit and push the release branch
#   ✓ Create PR with auto-merge enabled
#   ✓ Exit (you wait for CI to pass)
#
# Step 4: Wait for automation
#   - CI runs on the release PR (~30 seconds)
#   - PR auto-merges when CI passes
#   - On merge, GitHub Actions:
#     * Creates git tag vX.Y.Z
#     * Publishes to crates.io
#     * Creates GitHub release with CHANGELOG notes
#     * Updates Homebrew tap formula
#
# Step 5: Pull the changes
#   - After ~1-2 minutes, run: git pull
#   - You'll see the release commit and tag
#
# MONITORING:
#   - Watch PR: https://github.com/rsperko/diamond/pulls
#   - Watch Actions: https://github.com/rsperko/diamond/actions
#
# If anything goes wrong, GitHub provides detailed logs and the PR can be closed.
# ============================================================================

# Create a new patch release (0.1.0 -> 0.1.1)
release-patch:
    @just _release patch

# Create a new minor release (0.1.x -> 0.2.0)
release-minor:
    @just _release minor

# Create a new major release (1.x.x -> 2.0.0)
release-major:
    @just _release major

# Internal release helper - creates release PR with auto-merge
_release bump_type:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🚀 Diamond Release Process ({{bump_type}})"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""

    # 1. Verify we're on main branch
    BRANCH=$(git branch --show-current)
    if [ "$BRANCH" != "main" ]; then
        echo "❌ ERROR: Must be on 'main' branch (currently on '$BRANCH')"
        echo ""
        echo "Workflow:"
        echo "  1. Switch to main: git checkout main"
        echo "  2. Pull latest: git pull"
        echo "  3. Run: just release-{{bump_type}}"
        exit 1
    fi

    # 2. Verify working tree is clean
    echo "🔍 Checking git status..."
    if ! git diff-index --quiet HEAD --; then
        echo "❌ ERROR: Working tree has uncommitted changes"
        echo ""
        echo "Please commit or stash changes first:"
        echo "  git status"
        exit 1
    fi

    # 3. Verify in sync with remote
    git fetch origin main --quiet
    LOCAL=$(git rev-parse main)
    REMOTE=$(git rev-parse origin/main)
    if [ "$LOCAL" != "$REMOTE" ]; then
        echo "⚠️  WARNING: Local main is not in sync with remote"
        echo ""
        echo "Run: git pull"
        exit 1
    fi

    # 4. Get current version and calculate next
    CURRENT_VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
    echo "📦 Current version: $CURRENT_VERSION"

    IFS='.' read -r -a VERSION_PARTS <<< "$CURRENT_VERSION"
    MAJOR="${VERSION_PARTS[0]}"
    MINOR="${VERSION_PARTS[1]}"
    PATCH="${VERSION_PARTS[2]}"

    if [ "{{bump_type}}" = "major" ]; then
        MAJOR=$((MAJOR + 1))
        MINOR=0
        PATCH=0
    elif [ "{{bump_type}}" = "minor" ]; then
        MINOR=$((MINOR + 1))
        PATCH=0
    else
        PATCH=$((PATCH + 1))
    fi

    NEW_VERSION="$MAJOR.$MINOR.$PATCH"
    echo "📦 New version: $NEW_VERSION"
    echo ""

    # 5. Verify CHANGELOG has content
    if ! grep -q "## \[Unreleased\]" CHANGELOG.md; then
        echo "❌ ERROR: CHANGELOG.md missing [Unreleased] section"
        echo ""
        echo "Add changes to CHANGELOG.md under [Unreleased] section first"
        exit 1
    fi

    UNRELEASED_CONTENT=$(sed -n '/## \[Unreleased\]/,/## \[/p' CHANGELOG.md | grep -v "^## " | grep -E "^-|^###" | wc -l)
    if [ "$UNRELEASED_CONTENT" -eq 0 ]; then
        echo "⚠️  WARNING: [Unreleased] section appears empty"
        echo ""
    fi

    # 6. Verify gh CLI is installed and authenticated
    if ! command -v gh &> /dev/null; then
        echo "❌ ERROR: GitHub CLI (gh) is not installed"
        echo ""
        echo "Install with: brew install gh"
        echo "Then authenticate: gh auth login"
        exit 1
    fi

    if ! gh auth status &> /dev/null; then
        echo "❌ ERROR: GitHub CLI is not authenticated"
        echo ""
        echo "Run: gh auth login"
        exit 1
    fi

    # 7. Show what will happen
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📋 Release Plan"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "This will create a release PR that:"
    echo "  1. Updates Cargo.toml: $CURRENT_VERSION → $NEW_VERSION"
    echo "  2. Updates Cargo.lock to match"
    echo "  3. Updates CHANGELOG.md: [Unreleased] → [$NEW_VERSION]"
    echo "  4. Auto-merges when CI passes"
    echo ""
    echo "After merge, GitHub Actions will:"
    echo "  5. Create and push git tag: v$NEW_VERSION"
    echo "  6. Publish to crates.io"
    echo "  7. Create GitHub release"
    echo "  8. Update Homebrew tap"
    echo ""
    read -p "Create release PR? [y/N]: " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Aborted."
        exit 1
    fi
    echo ""

    # 8. Create release branch
    echo "🌿 Creating release branch..."
    git checkout -b "release/v$NEW_VERSION"

    # 9. Update Cargo.toml
    echo "📝 Updating Cargo.toml..."
    sed -i '' "s/^version = \".*\"/version = \"$NEW_VERSION\"/" Cargo.toml

    # 10. Update Cargo.lock
    echo "📝 Updating Cargo.lock..."
    cargo update --workspace --quiet

    # 11. Update CHANGELOG.md
    echo "📝 Updating CHANGELOG.md..."
    TODAY=$(date +%Y-%m-%d)
    sed -i '' "s/## \[Unreleased\]/## [$NEW_VERSION] - $TODAY/" CHANGELOG.md

    # Add new empty [Unreleased] section
    awk "/^## \[$NEW_VERSION\]/ {print \"\"; print \"## [Unreleased]\"; print \"\";} {print}" CHANGELOG.md > CHANGELOG.tmp
    mv CHANGELOG.tmp CHANGELOG.md

    # 12. Commit changes
    echo "💾 Committing changes..."
    git add Cargo.toml Cargo.lock CHANGELOG.md
    git commit -m "Release v$NEW_VERSION"

    # 13. Push release branch
    echo "📤 Pushing release branch..."
    git push -u origin "release/v$NEW_VERSION"

    # 14. Create PR with auto-merge
    echo "📝 Creating PR with auto-merge..."
    PR_URL=$(gh pr create \
        --base main \
        --head "release/v$NEW_VERSION" \
        --title "Release v$NEW_VERSION" \
        --body "Automated release PR for v$NEW_VERSION

This PR updates:
- Cargo.toml version: $CURRENT_VERSION → $NEW_VERSION
- Cargo.lock to match
- CHANGELOG.md: Moves [Unreleased] entries to [$NEW_VERSION]

After merge, GitHub Actions will:
- Create git tag v$NEW_VERSION
- Publish to crates.io
- Create GitHub release
- Update Homebrew tap" \
        --auto-merge --squash)

    # 15. Return to main
    git checkout main

    # Done!
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "✅ Release PR created!"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "🔗 PR URL: $PR_URL"
    echo ""
    echo "📊 What happens next:"
    echo "  1. CI runs on the PR (~30 seconds)"
    echo "  2. PR auto-merges when CI passes"
    echo "  3. GitHub Actions publishes the release (~1 minute)"
    echo ""
    echo "Monitor progress:"
    echo "  • PR: $PR_URL"
    echo "  • Actions: https://github.com/rsperko/diamond/actions"
    echo ""
    echo "After ~1-2 minutes, pull the changes:"
    echo "  git pull"
    echo ""

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
