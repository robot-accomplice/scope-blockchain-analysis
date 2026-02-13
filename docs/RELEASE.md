# Release Checklist

Steps for cutting a new release and publishing to crates.io.

## Pre-Release

- [ ] All changes merged to `develop` (or target branch)
- [ ] `cargo test` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo fmt --all -- --check` passes
- [ ] CHANGELOG.md updated with version and release date
- [ ] Cargo.toml version bumped
- [ ] No `[Unreleased]` changes that belong in this release

## Version Bump

Edit `Cargo.toml`:

```toml
version = "0.3.1"  # Use semver: patch for fixes, minor for features
```

Edit `CHANGELOG.md`:

- Move `[Unreleased]` content under `## [X.Y.Z] - YYYY-MM-DD`
- Update `[Unreleased]` link: `compare/vX.Y.Z...HEAD`
- Add `[X.Y.Z]`: `compare/vA.B.C...vX.Y.Z`

## Tag and Release

```bash
# Create and push tag
git tag -a v0.3.1 -m "Release v0.3.1"
git push origin v0.3.1
```

Optionally create a GitHub Release from the tag with release notes from CHANGELOG.

## crates.io Publish

### First-Time Setup

1. Create account at https://crates.io
2. Get API token from Account Settings → API Tokens
3. `cargo login` and paste token (or set `CARGO_REGISTRY_TOKEN`)

### Publish

```bash
# Dry run to verify package metadata and dependencies
cargo publish --dry-run

# Actual publish (requires crates.io account)
cargo publish
```

### Post-Publish

- Package appears at https://crates.io/crates/scope-bca
- Install with: `cargo install scope-bca`
- Update README badges if needed

## Troubleshooting

| Issue | Fix |
|-------|-----|
| `edition` invalid | Ensure Rust stable supports it (`rustc --version`) |
| `cargo publish` auth | Re-run `cargo login` or check token |
| Version already exists | Bump version, cannot republish same version |
| Dependency resolution | Run `cargo update` and `cargo publish --dry-run` |
