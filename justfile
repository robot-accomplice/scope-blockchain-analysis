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
