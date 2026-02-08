# =============================================================================
# BCA - Blockchain Analysis CLI
# Development Task Runner
# =============================================================================
#
# Run `just --list` to see all available recipes.
#
# =============================================================================

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
    ./target/release/bcc --version
    @echo ""
    @echo "═══════════════════════════════════════════════════════════════════"
    @echo "✓ All CI checks passed!"
    @echo "═══════════════════════════════════════════════════════════════════"

# -----------------------------------------------------------------------------
# Release
# -----------------------------------------------------------------------------

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
    
    # Add new version section after Unreleased
    sed -i '' "/## \[Unreleased\]/a\\
\\
## [$NEW_VERSION] - $TODAY" CHANGELOG.md
    
    # Update comparison links at bottom
    sed -i '' "s#\[Unreleased\]: https://github.com/robot-accomplice/bcc/compare/v$CURRENT_VERSION...HEAD#[Unreleased]: https://github.com/robot-accomplice/bcc/compare/v$NEW_VERSION...HEAD\n[$NEW_VERSION]: https://github.com/robot-accomplice/bcc/compare/v$CURRENT_VERSION...v$NEW_VERSION#" CHANGELOG.md
    
    echo "✓ CHANGELOG.md updated"
    
    # Run tests to ensure everything works
    echo ""
    echo "Step 3/6: Running tests..."
    cargo test --quiet
    echo "✓ All tests passed"
    
    # Commit changes
    echo ""
    echo "Step 4/6: Committing version bump..."
    git add Cargo.toml Cargo.lock CHANGELOG.md
    git commit -m "Release v$NEW_VERSION"
    echo "✓ Changes committed"
    
    # Create and push tag
    echo ""
    echo "Step 5/6: Creating and pushing git tag..."
    git tag -a "v$NEW_VERSION" -m "Release version $NEW_VERSION"
    git push origin main
    git push origin "v$NEW_VERSION"
    echo "✓ Tag v$NEW_VERSION pushed to GitHub"
    
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
    echo "Check progress at: https://github.com/robot-accomplice/bcc/actions"
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
        echo "https://github.com/robot-accomplice/bcc/actions"
        echo ""
        read -p "Press Enter when GitHub Actions workflow has completed..."
    fi
    
    # Publish to crates.io
    echo ""
    echo "Step 6/6: Publishing to crates.io..."
    echo ""
    read -p "Publish v$NEW_VERSION to crates.io? (y/n): " PUBLISH
    
    if [ "$PUBLISH" = "y" ] || [ "$PUBLISH" = "Y" ]; then
        cargo publish
        echo ""
        echo "═══════════════════════════════════════════════════════════════════"
        echo "✓ v$NEW_VERSION published to crates.io!"
        echo "═══════════════════════════════════════════════════════════════════"
        echo ""
        echo "Users can now install with: cargo install bcc"
    else
        echo ""
        echo "Skipped publishing. To publish manually:"
        echo "  cargo publish"
    fi
    
    echo ""
    echo "Next steps:"
    echo "  1. Check the GitHub release: https://github.com/robot-accomplice/bcc/releases"
    echo "  2. Verify binaries are attached"
    echo "  3. Update release notes if needed"
    echo ""
    echo "Release complete!"
