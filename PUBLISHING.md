# Publishing Guide

This document describes how to publish Scope releases and the crate to crates.io.

## Prerequisites

- GitHub account with access to `robot-accomplice/scope-blockchain-analysis` repository
- crates.io account with publish permissions (for crate publishing)
- `cargo` and `rust` installed locally

## Version Numbering

This project follows [Semantic Versioning](https://semver.org/):

- **MAJOR** (X.y.z) - Breaking changes to CLI or library API
- **MINOR** (x.Y.z) - New features, backwards compatible
- **PATCH** (x.y.Z) - Bug fixes, backwards compatible

## Release Checklist

Before publishing a new release:

- [ ] Update `CHANGELOG.md` with new version and date
- [ ] Update version in `Cargo.toml`
- [ ] Ensure all tests pass: `cargo test`
- [ ] Verify clippy is clean: `cargo clippy -- -D warnings`
- [ ] Check formatting: `cargo fmt -- --check`
- [ ] Verify package builds: `cargo build --release`
- [ ] Test package dry-run: `cargo publish --dry-run`
- [ ] Commit all changes
- [ ] Create git tag: `git tag -a v0.1.0 -m "Release version 0.1.0"`
- [ ] Push tag: `git push origin v0.1.0`

## Automated Release Process

Pushing a tag triggers the release workflow:

1. **GitHub Release** is created as a draft
2. **Binaries** are built for:
   - Linux x64 (`x86_64-unknown-linux-gnu`)
   - Linux ARM64 (`aarch64-unknown-linux-gnu`)
   - macOS x64 (`x86_64-apple-darwin`)
   - macOS ARM64 (`aarch64-apple-darwin`)
3. **Crate dry-run** is performed (publish disabled by default)

## Manual Steps

### 1. Update CHANGELOG

```bash
# Edit CHANGELOG.md to move Unreleased changes to a new version section
# Update the comparison links at the bottom
```

### 2. Update Version

```bash
# Edit Cargo.toml and update the version field
version = "0.2.0"  # Example
```

### 3. Commit and Tag

```bash
git add CHANGELOG.md Cargo.toml Cargo.lock
git commit -m "Prepare release v0.2.0"
git tag -a v0.2.0 -m "Release version 0.2.0"
git push origin main
git push origin v0.2.0
```

### 4. Review GitHub Release

The release workflow will create a draft release. Review it at:
https://github.com/robot-accomplice/scope-blockchain-analysis/releases

- Edit release notes
- Verify all binary assets are attached
- Publish the release

### 5. Publish to crates.io (Manual)

**Important**: The GitHub workflow only performs a dry-run. To actually publish:

```bash
# Ensure you're on the release tag
git checkout v0.2.0

# Verify one more time
cargo publish --dry-run

# Publish (requires crates.io API token)
cargo login
cargo publish
```

Or enable the automatic publish step in `.github/workflows/release.yml` by uncommenting:

```yaml
- name: Publish to crates.io
  run: cargo publish --token ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

And adding `CARGO_REGISTRY_TOKEN` to GitHub repository secrets.

## Post-Release

- [ ] Verify crate is available: https://crates.io/crates/scope
- [ ] Verify installation works: `cargo install scope`
- [ ] Update documentation if needed
- [ ] Announce release (if applicable)

## Troubleshooting

### Dry-run fails

```bash
cargo publish --dry-run
```

Common issues:
- Missing fields in `Cargo.toml`
- Files referenced but not included
- Dependencies with incompatible versions

### Binary build fails

Check the Actions logs at:
https://github.com/robot-accomplice/scope-blockchain-analysis/actions

### Crate name taken

If `scope` is already taken on crates.io, consider:
- `scope-blockchain`
- `scope-analysis`
- `blockchain-scope`

Update `Cargo.toml`:
```toml
name = "new-name"
```

## Code Coverage Setup

Scope uses Codecov for coverage reporting. Target: **80%+ coverage** before publishing.

### Setup Steps (OIDC - No Token Required)

Scope uses **OIDC authentication** with Codecov — no static tokens needed.

1. **Sign up for Codecov**: https://about.codecov.io/
2. **Add repository**: Connect `robot-accomplice/scope-blockchain-analysis` GitHub repo
3. **Enable OIDC**: Already configured in `.github/workflows/coverage.yml`
4. **No secrets required** — OIDC handles authentication automatically

### Alternative: Token-Based (If OIDC Fails)

If OIDC doesn't work:
1. Get token from: https://app.codecov.io/gh/robot-accomplice/scope-blockchain-analysis/settings
2. Add GitHub Secret: `CODECOV_TOKEN`
3. Edit workflow: Remove `use_oidc: true` and uncomment token line

### Coverage Badge

Add to README.md after setup:
```markdown
[![codecov](https://codecov.io/gh/robot-accomplice/scope-blockchain-analysis/branch/main/graph/badge.svg)](https://codecov.io/gh/robot-accomplice/scope-blockchain-analysis)
```

### Running Coverage Locally

```bash
# Install tarpaulin
cargo install cargo-tarpaulin

# Generate coverage report
cargo tarpaulin --out html

# View report
open tarpaulin-report.html
```

### Coverage Requirements

Before publishing to crates.io:
- [ ] Overall coverage ≥ 80%
- [ ] Critical modules (compliance, chains) ≥ 75%
- [ ] No uncovered critical paths
- [ ] All new code has tests

### Improving Coverage

Focus areas for test additions:
1. **Compliance module** - Risk scoring, pattern detection
2. **CLI commands** - Error handling, edge cases
3. **Chain clients** - Mock API responses
4. **Display formatting** - All output formats
