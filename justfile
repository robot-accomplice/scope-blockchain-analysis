# =============================================================================
# Scope Blockchain Analysis
# Development Task Runner
# =============================================================================
#
# Run `just --list` to see all available recipes.
#
# =============================================================================

# Market summary: use Scope CLI (override pair: just summary pair_symbol=XYZ_USDT)
pair_symbol := "USDC_USDT"

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
summary pair_symbol='USDC_USDT':
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

# Load .env if present (provides CARGO_REGISTRY_TOKEN)
set dotenv-load

# Publish current version to crates.io (dry-run first, then publish)
publish:
    #!/usr/bin/env bash
    set -euo pipefail

    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
    echo "═══════════════════════════════════════════════════════════════════"
    echo "Publishing scope-bca v$VERSION to crates.io"
    echo "═══════════════════════════════════════════════════════════════════"
    echo ""

    if [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
        echo "⚠️  CARGO_REGISTRY_TOKEN not set. Add it to .env or run: cargo login"
    fi

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

    # Step 3: Dry-run to verify packaging (scope-bca-core only — downstream
    # crates can only dry-run against already-published core).
    echo "Step 3/4: Verifying package (dry-run)..."
    cargo publish -p scope-bca-core --dry-run
    echo "✓ scope-bca-core packaging verified"
    echo ""

    # Step 4: Publish the three crates in dependency order.
    #   scope-bca-core  →  scope-bca-cli  →  scope-bca
    # The sleep gives the crates.io sparse index time to pick up the newly
    # published version before the next verify step runs.
    echo "Step 4/4: Publishing..."
    read -p "Publish scope-bca v$VERSION to crates.io? (y/N): " CONFIRM
    if [ "$CONFIRM" = "y" ] || [ "$CONFIRM" = "Y" ]; then
        cargo publish -p scope-bca-core
        echo "  → scope-bca-core uploaded; waiting 45s for index..."
        sleep 45
        cargo publish -p scope-bca-cli
        echo "  → scope-bca-cli uploaded; waiting 45s for index..."
        sleep 45
        cargo publish -p scope-bca
        echo ""
        echo "═══════════════════════════════════════════════════════════════════"
        echo "✓ scope-bca v$VERSION published to crates.io!"
        echo "═══════════════════════════════════════════════════════════════════"
        echo ""
        echo "Install with: cargo install scope-bca"
        echo "Crate page:   https://crates.io/crates/scope-bca"
    else
        echo "Aborted. To publish manually:"
        echo "  cargo publish -p scope-bca-core && sleep 45 \\"
        echo "    && cargo publish -p scope-bca-cli && sleep 45 \\"
        echo "    && cargo publish -p scope-bca"
    fi

# Dry-run crates.io publish for scope-bca-core (downstream crates need core
# to be on crates.io before they can dry-run).
publish-dry-run:
    cargo publish -p scope-bca-core --dry-run

# Yank a version from crates.io (prevents new deps, existing builds unaffected).
# Yanks all three published crates at the given version.
yank VERSION:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
        echo "❌ CARGO_REGISTRY_TOKEN not set. Add it to .env or export it."
        exit 1
    fi
    echo "Yanking scope-bca{,-cli,-core}@{{VERSION}} from crates.io..."
    read -p "Are you sure? This cannot be undone. (y/N): " CONFIRM
    if [ "$CONFIRM" = "y" ] || [ "$CONFIRM" = "Y" ]; then
        cargo yank --version {{VERSION}} scope-bca
        cargo yank --version {{VERSION}} scope-bca-cli
        cargo yank --version {{VERSION}} scope-bca-core
        echo "✓ scope-bca{,-cli,-core}@{{VERSION}} yanked"
    else
        echo "Aborted."
    fi

# Un-yank a previously yanked version (all three crates)
unyank VERSION:
    cargo yank --version {{VERSION}} --undo scope-bca
    cargo yank --version {{VERSION}} --undo scope-bca-cli
    cargo yank --version {{VERSION}} --undo scope-bca-core

# Measure coverage and record it as the new baseline in .coverage-last (commit the file)
coverage-baseline:
    scripts/check-coverage.sh --record

# Release step 1, on develop: make release/vX.Y.Z with the version bump, changelog and checks
release-prep VERSION:
    scripts/release.sh prep {{VERSION}}

# Release step 2, on main after the release PR merged: tag the merge commit and push the tag
release-tag:
    scripts/release.sh tag

# -----------------------------------------------------------------------------
# TUI harness
# -----------------------------------------------------------------------------

# TUI release gate: build scope, run the harness PTY tests, run the offline scenarios
tui-test *ARGS:
    cargo build --bin scope
    cargo test -p scope-tui-harness -- --ignored
    cargo run -q -p scope-tui-harness -- run --tier offline {{ARGS}}

# Record one offline scenario's API replies from the live APIs into tests/tui/fixtures/<NAME>/ (review the diff)
tui-record NAME:
    cargo build --bin scope
    rm -rf tests/tui/fixtures/{{NAME}}
    cargo run -q -p scope-tui-harness -- run --record {{NAME}}
