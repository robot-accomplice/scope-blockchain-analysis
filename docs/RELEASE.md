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
version = "1.0.0"  # Use semver: patch for fixes, minor for features, major for breaking
```

Edit `CHANGELOG.md`:

- Move `[Unreleased]` content under `## [X.Y.Z] - YYYY-MM-DD`
- Update `[Unreleased]` link: `compare/vX.Y.Z...HEAD`
- Add `[X.Y.Z]`: `compare/vA.B.C...vX.Y.Z`

## Tag and Release

```bash
# Create and push tag
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
```

Optionally create a GitHub Release from the tag with release notes from CHANGELOG.

## crates.io Publish

### First-Time Setup

1. Create account at https://crates.io
2. Get API token from Account Settings → API Tokens
3. `cargo login` and paste token (or set `CARGO_REGISTRY_TOKEN`)

### Publish

Since the v0.5.5 workspace split the project publishes three crates under the
`scope-bca` umbrella:

| Workspace dir         | Published name     | Purpose                                           |
|-----------------------|--------------------|---------------------------------------------------|
| `crates/scope-core/`  | `scope-bca-core`   | Core library (`use scope::*`)                     |
| `crates/scope-cli/`   | `scope-bca-cli`    | CLI handler library                                |
| `crates/scope-web/`   | `scope-bca`        | `scope` binary + web server — this is what users  |
|                       |                    | install with `cargo install scope-bca`            |

Crates must be published in dependency order. The `just publish` recipe and the
GitHub Actions release workflow both handle this automatically:

```bash
# Publish all three via the interactive recipe
just publish

# Or manually, in order, with a sleep to let the sparse index settle:
cargo publish -p scope-bca-core --dry-run
cargo publish -p scope-bca-core && sleep 45 \
  && cargo publish -p scope-bca-cli && sleep 45 \
  && cargo publish -p scope-bca
```

### Post-Publish

- Packages appear at https://crates.io/crates/scope-bca (+ `-cli`, `-core`)
- Install with: `cargo install scope-bca`
- Update README badges if needed

## Troubleshooting

| Issue | Fix |
|-------|-----|
| `edition` invalid | Ensure Rust stable supports it (`rustc --version`) |
| `cargo publish` auth | Re-run `cargo login` or check token |
| Version already exists | Bump version, cannot republish same version |
| Dependency resolution | Run `cargo update` and `cargo publish --dry-run` |
