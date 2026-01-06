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
# RELEASE WORKFLOW (for protected main branch)
# ============================================================================
# TL;DR - The complete workflow:
#   1. Update CHANGELOG.md on your feature branch (under [Unreleased])
#   2. Merge your PR to main
#   3. Run: just release-patch  (creates release PR)
#   4. Merge the release PR
#   5. Run: just publish-release 0.1.1  (tags & publishes)
#   Done! 🎉
# ============================================================================
#
# DETAILED STEPS:
#
# Step 1: While working on your feature branch
#   - Add entries to CHANGELOG.md under the [Unreleased] section
#   - DO NOT update version in Cargo.toml (the script does this)
#
# Step 2: Merge to main
#   - Get your feature PR merged to main
#   - The changelog entries come along with the merge
#
# Step 3: Create release PR
#   - git checkout main && git pull
#   - Run: just release-patch   (0.1.0 → 0.1.1)
#      OR: just release-minor   (0.1.x → 0.2.0)
#      OR: just release-major   (1.x.x → 2.0.0)
#
#   The script will:
#   ✓ Auto-calculate the new version number
#   ✓ Create a release branch (release/vX.Y.Z)
#   ✓ Update Cargo.toml with the new version
#   ✓ Move [Unreleased] to [X.Y.Z] - DATE in CHANGELOG.md
#   ✓ Create a new empty [Unreleased] section
#   ✓ Run tests to verify everything works
#   ✓ Commit and push the release branch
#   ✓ Create a PR to main (via gh CLI)
#
# Step 4: Merge the release PR on GitHub
#   - Review and merge the release/vX.Y.Z PR
#
# Step 5: Publish the release
#   - git checkout main && git pull
#   - Run: just publish-release X.Y.Z
#   - This creates the git tag and publishes to crates.io
#   - GitHub Actions automatically:
#     * Creates GitHub release with CHANGELOG notes
#     * Updates Homebrew tap formula
#
# If anything goes wrong, the scripts abort and provide clear error messages.
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

# Internal release helper
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
        echo "  1. Merge your feature branch to main first"
        echo "  2. Switch to main: git checkout main"
        echo "  3. Pull latest: git pull"
        echo "  4. Run: just release-{{bump_type}}"
        exit 1
    fi

    # 2. Verify working tree is clean
    if ! git diff-index --quiet HEAD --; then
        echo "❌ ERROR: Working tree has uncommitted changes"
        echo ""
        echo "Please commit or stash changes first:"
        echo "  git status"
        exit 1
    fi

    # 3. Get current version from Cargo.toml
    CURRENT_VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
    echo "📦 Current version: $CURRENT_VERSION"

    # 4. Calculate next version
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

    # 5. Verify CHANGELOG has [Unreleased] section with content
    if ! grep -q "## \[Unreleased\]" CHANGELOG.md; then
        echo "❌ ERROR: CHANGELOG.md missing [Unreleased] section"
        echo ""
        echo "Add changes to CHANGELOG.md under [Unreleased] section first"
        exit 1
    fi

    # Check if there's actual content under [Unreleased]
    UNRELEASED_CONTENT=$(sed -n '/## \[Unreleased\]/,/## \[/p' CHANGELOG.md | grep -v "^## " | grep -E "^-|^###" | wc -l)
    if [ "$UNRELEASED_CONTENT" -eq 0 ]; then
        echo "⚠️  WARNING: [Unreleased] section appears empty"
        echo ""
        read -p "Continue anyway? [y/N]: " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            echo "Aborted."
            exit 1
        fi
    fi

    # 6. Show what will happen
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📋 Release Plan"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "Will perform these steps:"
    echo "  1. Update Cargo.toml: $CURRENT_VERSION → $NEW_VERSION"
    echo "  2. Update CHANGELOG.md: [Unreleased] → [$NEW_VERSION] - $(date +%Y-%m-%d)"
    echo "  3. Commit changes: 'Release v$NEW_VERSION'"
    echo "  4. Create git tag: v$NEW_VERSION"
    echo "  5. Push to origin with tags"
    echo "  6. Publish to crates.io"
    echo ""
    read -p "Proceed with release? [y/N]: " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Aborted."
        exit 1
    fi
    echo ""

    # 7. Update Cargo.toml version
    echo "📝 Updating Cargo.toml..."
    sed -i '' "s/^version = \".*\"/version = \"$NEW_VERSION\"/" Cargo.toml

    # 8. Update CHANGELOG.md - replace [Unreleased] with version and date
    echo "📝 Updating CHANGELOG.md..."
    TODAY=$(date +%Y-%m-%d)
    sed -i '' "s/## \[Unreleased\]/## [$NEW_VERSION] - $TODAY/" CHANGELOG.md

    # Add new empty [Unreleased] section at the top
    awk "/^## \[$NEW_VERSION\]/ {print \"\"; print \"## [Unreleased]\"; print \"\";} {print}" CHANGELOG.md > CHANGELOG.tmp && mv CHANGELOG.tmp CHANGELOG.md

    # 9. Run tests one final time
    echo ""
    echo "🧪 Running final test suite..."
    if ! cargo test --quiet; then
        echo "❌ Tests failed! Aborting release."
        git checkout Cargo.toml CHANGELOG.md
        exit 1
    fi

    # 10. Create release branch
    echo ""
    echo "🌿 Creating release branch..."
    git checkout -b "release/v$NEW_VERSION"

    # 11. Commit changes
    echo "💾 Committing version bump..."
    git add Cargo.toml CHANGELOG.md
    git commit -m "Release v$NEW_VERSION"

    # 12. Push release branch
    echo "📤 Pushing release branch..."
    git push -u origin "release/v$NEW_VERSION"

    # 13. Create PR
    echo ""
    echo "📝 Creating release PR..."
    gh pr create \
        --base main \
        --head "release/v$NEW_VERSION" \
        --title "Release v$NEW_VERSION" \
        --body "Automated release PR for v$NEW_VERSION

    This PR updates:
    - Cargo.toml version: $CURRENT_VERSION → $NEW_VERSION
    - CHANGELOG.md: Moves [Unreleased] entries to [$NEW_VERSION]

    After merging, run:
    \`\`\`bash
    git checkout main && git pull
    just publish-release $NEW_VERSION
    \`\`\`
    "

    # Done with first phase!
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "✅ Release PR created for v$NEW_VERSION"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "📋 Next steps:"
    echo "  1. Review and merge the PR: https://github.com/rsperko/diamond/pulls"
    echo "  2. After merge, run:"
    echo "     git checkout main && git pull"
    echo "     just publish-release $NEW_VERSION"
    echo ""

# Publish a release after the PR is merged
# Usage: just publish-release 0.1.1
publish-release version:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📦 Publishing Release v{{version}}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""

    # 1. Verify we're on main
    BRANCH=$(git branch --show-current)
    if [ "$BRANCH" != "main" ]; then
        echo "❌ ERROR: Must be on 'main' branch (currently on '$BRANCH')"
        echo ""
        echo "Run: git checkout main && git pull"
        exit 1
    fi

    # 2. Verify working tree is clean
    if ! git diff-index --quiet HEAD --; then
        echo "❌ ERROR: Working tree has uncommitted changes"
        echo ""
        echo "Run: git status"
        exit 1
    fi

    # 3. Verify version in Cargo.toml matches
    CURRENT_VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
    if [ "$CURRENT_VERSION" != "{{version}}" ]; then
        echo "❌ ERROR: Version mismatch!"
        echo "   Cargo.toml version: $CURRENT_VERSION"
        echo "   Requested version:  {{version}}"
        echo ""
        echo "Did you forget to merge the release PR?"
        echo "Run: git pull"
        exit 1
    fi

    # 4. Verify this version doesn't already exist as a tag
    if git rev-parse "v{{version}}" >/dev/null 2>&1; then
        echo "❌ ERROR: Tag v{{version}} already exists!"
        echo ""
        echo "If you need to re-release, delete the tag first:"
        echo "  git tag -d v{{version}}"
        echo "  git push origin :refs/tags/v{{version}}"
        exit 1
    fi

    # 5. Show what will happen
    echo "Will perform these steps:"
    echo "  1. Create git tag v{{version}}"
    echo "  2. Push tag to GitHub"
    echo "  3. Publish to crates.io"
    echo ""
    read -p "Proceed with publishing? [y/N]: " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Aborted."
        exit 1
    fi
    echo ""

    # 6. Create and push tag
    echo "🏷️  Creating git tag v{{version}}..."
    git tag "v{{version}}"

    echo "📤 Pushing tag to GitHub..."
    git push origin "v{{version}}"

    # 7. Publish to crates.io
    echo ""
    echo "📦 Publishing to crates.io..."
    cargo publish

    # Done!
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "✅ Release v{{version}} published!"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "🔗 View release:"
    echo "   https://github.com/rsperko/diamond/releases/tag/v{{version}}"
    echo "   https://crates.io/crates/diamond-cli"
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
