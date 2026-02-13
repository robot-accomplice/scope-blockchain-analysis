# =============================================================================
# Scope Blockchain Analysis
# Development Task Runner
# =============================================================================
#
# Run `just --list` to see all available recipes.
#
# =============================================================================

# Market summary: use Scope CLI (override pair: just summary pair_symbol=XYZ_USDT)
pair_symbol := "PUSD_USDT"

# Default recipe - show available commands
default:
    @just --list

# -----------------------------------------------------------------------------
# Code Quality
# -----------------------------------------------------------------------------

# Format all Rust code
format:
    cargo fmt --all

# Check formatting without making changes
format-check:
    cargo fmt --all -- --check

# Run Clippy lints
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Run all code quality checks (format + lint)
check: format-check lint

# -----------------------------------------------------------------------------
# Testing
# -----------------------------------------------------------------------------

# Run all tests with nextest
test:
    cargo nextest run --all-features

# Run tests with CI profile (stricter)
test-ci:
    cargo nextest run --all-features --profile ci

# Run doc tests only
test-doc:
    cargo test --doc --all-features

# Run all tests including doc tests
test-all: test test-doc

# Run tests with coverage report
coverage:
    cargo tarpaulin --out Html --all-features
    @echo "Coverage report: tarpaulin-report.html"

# Check coverage (80% min + no regression); used by pre-push hook
coverage-check:
    ./scripts/check-coverage.sh

# Install git hooks (pre-push runs coverage check before push)
install-hooks:
    ./scripts/install-hooks.sh

# -----------------------------------------------------------------------------
# Building
# -----------------------------------------------------------------------------

# Build debug binary
build:
    cargo build

# Build release binary
build-release:
    cargo build --release

# Quick compilation check
check-compile:
    cargo check --all-targets --all-features

# -----------------------------------------------------------------------------
# Documentation
# -----------------------------------------------------------------------------

# Build and open documentation
docs:
    cargo doc --no-deps --all-features --open

# Build documentation without opening
docs-build:
    cargo doc --no-deps --all-features

# -----------------------------------------------------------------------------
# Security
# -----------------------------------------------------------------------------

# Run security audit
audit:
    cargo audit

# Update advisory database and audit
audit-update:
    cargo audit fetch
    cargo audit

# -----------------------------------------------------------------------------
# Utilities
# -----------------------------------------------------------------------------

# Clean build artifacts
clean:
    cargo clean

# Update dependencies
update:
    cargo update

# Run the CLI with arguments
run *ARGS:
    cargo run -- {{ARGS}}

# Install locally
install:
    cargo install --path .

# One-screen market summary: peg price, spread, orderbook depth, book health.
# Uses Scope's built-in market command.
# Usage: just summary  (or just summary pair_symbol=XYZ_USDT)
summary pair_symbol='PUSD_USDT':
    cargo run -q -- market summary {{pair_symbol}}

# Pre-commit: format, lint, and test
pre-commit: format lint test

# -----------------------------------------------------------------------------
# CI Simulation
# -----------------------------------------------------------------------------

# Run full CI workflow locally (mimics GitHub Actions)
ci-test:
    @echo "═══════════════════════════════════════════════════════════════════"
    @echo "Step 1/7: Check (cargo check)"
    @echo "═══════════════════════════════════════════════════════════════════"
    cargo check --all-targets --all-features
    @echo ""
    @echo "═══════════════════════════════════════════════════════════════════"
    @echo "Step 2/7: Format (cargo fmt --check)"
    @echo "═══════════════════════════════════════════════════════════════════"
    cargo fmt --all -- --check
    @echo ""
    @echo "═══════════════════════════════════════════════════════════════════"
    @echo "Step 3/7: Lint (cargo clippy)"
    @echo "═══════════════════════════════════════════════════════════════════"
    cargo clippy --all-targets --all-features -- -D warnings
    @echo ""
    @echo "═══════════════════════════════════════════════════════════════════"
    @echo "Step 4/7: Test (cargo nextest)"
    @echo "═══════════════════════════════════════════════════════════════════"
    cargo nextest run --all-features --profile ci
    @echo ""
    @echo "═══════════════════════════════════════════════════════════════════"
    @echo "Step 5/7: Doc Tests (cargo test --doc)"
    @echo "═══════════════════════════════════════════════════════════════════"
    cargo test --doc --all-features
    @echo ""
    @echo "═══════════════════════════════════════════════════════════════════"
    @echo "Step 6/7: Docs (cargo doc)"
    @echo "═══════════════════════════════════════════════════════════════════"
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
    @echo ""
    @echo "═══════════════════════════════════════════════════════════════════"
    @echo "Step 7/7: Build Release (cargo build --release)"
    @echo "═══════════════════════════════════════════════════════════════════"
    cargo build --release
    ./target/release/scope --version
    @echo ""
    @echo "═══════════════════════════════════════════════════════════════════"
    @echo "✓ All CI checks passed!"
    @echo "═══════════════════════════════════════════════════════════════════"

# -----------------------------------------------------------------------------
# Release
# -----------------------------------------------------------------------------

# Publish current version to crates.io (dry-run first, then publish)
publish:
    #!/usr/bin/env bash
    set -euo pipefail

    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
    echo "═══════════════════════════════════════════════════════════════════"
    echo "Publishing scope-bca v$VERSION to crates.io"
    echo "═══════════════════════════════════════════════════════════════════"
    echo ""

    # Step 1: Verify clean working tree
    echo "Step 1/4: Checking working tree..."
    if [ -n "$(git status --porcelain)" ]; then
        echo "❌ Working tree is dirty. Commit or stash changes first."
        exit 1
    fi
    echo "✓ Working tree is clean"
    echo ""

    # Step 2: Run tests
    echo "Step 2/4: Running tests..."
    cargo test --quiet --all-features
    echo "✓ All tests passed"
    echo ""

    # Step 3: Dry-run to verify packaging
    echo "Step 3/4: Verifying package (dry-run)..."
    cargo publish --dry-run
    echo "✓ Package verified"
    echo ""

    # Step 4: Publish
    echo "Step 4/4: Publishing..."
    read -p "Publish scope-bca v$VERSION to crates.io? (y/N): " CONFIRM
    if [ "$CONFIRM" = "y" ] || [ "$CONFIRM" = "Y" ]; then
        cargo publish
        echo ""
        echo "═══════════════════════════════════════════════════════════════════"
        echo "✓ scope-bca v$VERSION published to crates.io!"
        echo "═══════════════════════════════════════════════════════════════════"
        echo ""
        echo "Install with: cargo install scope-bca"
        echo "Crate page:   https://crates.io/crates/scope-bca"
    else
        echo "Aborted. To publish manually: cargo publish"
    fi

# Dry-run crates.io publish (verify packaging without uploading)
publish-dry-run:
    cargo publish --dry-run

# Create a new release (interactive)
release:
    #!/usr/bin/env bash
    set -euo pipefail

    # Get current version
    CURRENT_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
    echo "Current version: $CURRENT_VERSION"
    echo ""
    
    # Prompt for new version
    read -p "Enter new version (e.g., 0.2.0): " NEW_VERSION
    
    if [ -z "$NEW_VERSION" ]; then
        echo "Error: Version cannot be empty"
        exit 1
    fi
    
    # Validate version format (semver)
    if ! echo "$NEW_VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
        echo "Error: Version must be in semver format (X.Y.Z)"
        exit 1
    fi
    
    echo ""
    echo "═══════════════════════════════════════════════════════════════════"
    echo "Preparing release v$NEW_VERSION"
    echo "═══════════════════════════════════════════════════════════════════"
    echo ""
    
    # Update version in Cargo.toml
    echo "Step 1/6: Updating Cargo.toml..."
    sed -i '' "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" Cargo.toml
    echo "✓ Cargo.toml updated"
    
    # Update CHANGELOG.md
    echo ""
    echo "Step 2/6: Updating CHANGELOG.md..."
    TODAY=$(date +%Y-%m-%d)
    
    # Check if CHANGELOG has Unreleased section
    if ! grep -q "## \[Unreleased\]" CHANGELOG.md; then
        echo "Error: CHANGELOG.md missing [Unreleased] section"
        exit 1
    fi
    
    # Add new version section after Unreleased using awk (avoids sed backslash issues)
    awk -v ver="$NEW_VERSION" -v dt="$TODAY" '{print} /^## \[Unreleased\]/ {print ""; print "## [" ver "] - " dt}' CHANGELOG.md > CHANGELOG.tmp && mv CHANGELOG.tmp CHANGELOG.md
    
    # Update comparison links at bottom
    OLD_LINK="[Unreleased]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v$CURRENT_VERSION...HEAD"
    NEW_LINK="[Unreleased]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v$NEW_VERSION...HEAD"
    VER_LINK="[$NEW_VERSION]: https://github.com/robot-accomplice/scope-blockchain-analysis/compare/v$CURRENT_VERSION...v$NEW_VERSION"
    sed -i '' "s#$OLD_LINK#$NEW_LINK#" CHANGELOG.md
    echo "$VER_LINK" >> CHANGELOG.md
    
    echo "✓ CHANGELOG.md updated"
    
    # Run tests to ensure everything works
    echo ""
    echo "Step 3/7: Running tests..."
    cargo test --quiet
    echo "✓ All tests passed"
    
    # Check code coverage
    echo ""
    echo "Step 4/7: Checking code coverage (minimum 80%)..."
    
    # Check if cargo-tarpaulin is installed
    if ! command -v cargo-tarpaulin &> /dev/null; then
        echo "⚠️  cargo-tarpaulin not found. Installing..."
        cargo install cargo-tarpaulin
    fi
    
    # Generate coverage and extract percentage
    COVERAGE_OUTPUT=$(cargo tarpaulin --out Stdout 2>&1)
    COVERAGE_PCT=$(echo "$COVERAGE_OUTPUT" | grep -o '[0-9]\+\.[0-9]\+% coverage' | head -1 | sed 's/% coverage//')
    
    echo "Current coverage: $COVERAGE_PCT%"
    
    # Compare coverage (using bc for float comparison)
    if command -v bc &> /dev/null; then
        COVERAGE_OK=$(echo "$COVERAGE_PCT >= 80.0" | bc)
        if [ "$COVERAGE_OK" -eq 0 ]; then
            echo ""
            echo "❌ ERROR: Coverage $COVERAGE_PCT% is below 80% threshold"
            echo "Release blocked. Add more tests to reach 80% coverage."
            echo ""
            echo "Top uncovered modules:"
            echo "$COVERAGE_OUTPUT" | grep -E '^\|\| src/' | head -10
            echo ""
            read -p "Continue anyway? (y/N): " FORCE_CONTINUE
            if [ "$FORCE_CONTINUE" != "y" ] && [ "$FORCE_CONTINUE" != "Y" ]; then
                echo "Release aborted."
                exit 1
            fi
            echo "⚠️  Continuing with insufficient coverage (override)"
        else
            echo "✓ Coverage check passed ($COVERAGE_PCT% >= 80%)"
        fi
    else
        echo "⚠️  bc not installed, skipping coverage check"
        echo "   Install with: brew install bc (macOS) or apt-get install bc (Linux)"
    fi
    
    # Commit changes
    echo ""
    echo "Step 5/7: Committing version bump..."
    git add Cargo.toml Cargo.lock CHANGELOG.md
    git commit -m "Release v$NEW_VERSION"
    echo "✓ Changes committed"
    
    # Create and push tag
    echo ""
    echo "Step 6/7: Creating and pushing git tag..."
    
    # Check if tag already exists
    if git rev-parse "v$NEW_VERSION" >/dev/null 2>&1; then
        echo "⚠️  Tag v$NEW_VERSION already exists locally"
        read -p "Delete and recreate? (y/N): " RECREATE
        if [ "$RECREATE" = "y" ] || [ "$RECREATE" = "Y" ]; then
            git tag -d "v$NEW_VERSION"
            git tag "v$NEW_VERSION"
        fi
    else
        git tag "v$NEW_VERSION"
    fi
    
    # Check if tag exists on remote
    if git ls-remote --tags origin "refs/tags/v$NEW_VERSION" | grep -q "v$NEW_VERSION"; then
        echo "⚠️  Tag v$NEW_VERSION already exists on remote"
        echo "   Skipping tag push"
    else
        git push origin "v$NEW_VERSION"
        echo "✓ Tag v$NEW_VERSION pushed to GitHub"
    fi
    
    git push origin main
    
    echo ""
    echo "═══════════════════════════════════════════════════════════════════"
    echo "✓ Release v$NEW_VERSION initiated!"
    echo "═══════════════════════════════════════════════════════════════════"
    echo ""
    echo "GitHub Actions workflow is now building:"
    echo "  - Linux x64 binary"
    echo "  - Linux ARM64 binary"
    echo "  - macOS x64 binary"
    echo "  - macOS ARM64 binary"
    echo ""
    echo "Check progress at: https://github.com/robot-accomplice/scope-blockchain-analysis/actions"
    echo ""
    read -p "Wait for GitHub Actions to complete before publishing? (y/n): " WAIT
    
    if [ "$WAIT" = "y" ] || [ "$WAIT" = "Y" ]; then
        echo ""
        echo "Waiting for GitHub Actions..."
        echo "(Press Ctrl+C to skip and publish manually later)"
        echo ""
        
        # Poll for workflow completion (simplified)
        sleep 5
        echo "Workflow started. Check status:"
        echo "https://github.com/robot-accomplice/scope-blockchain-analysis/actions"
        echo ""
        read -p "Press Enter when GitHub Actions workflow has completed..."
    fi
    
    # Publish to crates.io
    echo ""
    echo "Step 7/7: Publishing to crates.io..."
    echo ""
    read -p "Publish v$NEW_VERSION to crates.io? (y/n): " PUBLISH
    
    if [ "$PUBLISH" = "y" ] || [ "$PUBLISH" = "Y" ]; then
        cargo publish
        echo ""
        echo "═══════════════════════════════════════════════════════════════════"
        echo "✓ v$NEW_VERSION published to crates.io!"
        echo "═══════════════════════════════════════════════════════════════════"
        echo ""
        echo "Users can now install with: cargo install scope-bca"
    else
        echo ""
        echo "Skipped publishing. To publish manually:"
        echo "  cargo publish"
    fi
    
    echo ""
    echo "Next steps:"
    echo "  1. Check the GitHub release: https://github.com/robot-accomplice/scope-blockchain-analysis/releases"
    echo "  2. Verify binaries are attached"
    echo "  3. Update release notes if needed"
    echo ""
    echo "Release complete!"
