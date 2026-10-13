#!/usr/bin/env bash
# =============================================================================
# Release helper for scope-blockchain-analysis (gitflow).
#
#   scripts/release.sh prep X.Y.Z   On develop: make release/vX.Y.Z with the
#                                   version bump, changelog and checks.
#   scripts/release.sh tag          On main, after the release PR merged:
#                                   tag the merge commit and push the tag.
#
# Run through `just release-prep X.Y.Z` and `just release-tag`.
# =============================================================================

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

REPO_URL="https://github.com/robot-accomplice/scope-blockchain-analysis"
CHANGELOG="CHANGELOG.md"
# Crate manifests that pin internal path dependencies by version.
MANIFESTS=(crates/*/Cargo.toml)

die() {
    echo "❌ $*" >&2
    exit 1
}

workspace_version() {
    grep -m1 '^version = ' Cargo.toml | sed 's/.*"\(.*\)".*/\1/'
}

# Internal path dependencies whose version pin is not $1.
stale_pins() {
    grep -nE 'path = "\.\./[^"]+".*version = "' "${MANIFESTS[@]}" | grep -v "version = \"$1\"" || true
}

# Refuse unless on branch $1, with a clean tree, at the same commit as origin/$1.
require_branch() {
    local branch="$1" current
    current="$(git rev-parse --abbrev-ref HEAD)"
    [ "$current" = "$branch" ] || die "Run this on '$branch' (you are on '$current')."
    [ -z "$(git status --porcelain)" ] || die "The working tree is not clean."
    git fetch --quiet origin "$branch" --tags
    [ "$(git rev-parse HEAD)" = "$(git rev-parse "origin/$branch")" ] \
        || die "'$branch' does not match 'origin/$branch'. Pull or push first."
}

require_tag_absent() {
    local tag="$1"
    git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "Tag $tag exists locally."
    [ -z "$(git ls-remote --tags origin "refs/tags/$tag")" ] || die "Tag $tag exists on origin."
}

prep() {
    local new="${1:-}" old today
    [[ "$new" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "Usage: release.sh prep X.Y.Z"
    require_branch develop
    old="$(workspace_version)"
    [ "$new" != "$old" ] || die "The version is already $new."
    require_tag_absent "v$new"
    grep -q '^## \[Unreleased\]' "$CHANGELOG" || die "$CHANGELOG has no [Unreleased] section."

    git switch --quiet -c "release/v$new"

    echo "Step 1/5: version $old → $new (workspace and internal pins)"
    sed -i.bak "s/^version = \"$old\"/version = \"$new\"/" Cargo.toml
    for f in "${MANIFESTS[@]}"; do
        sed -i.bak -E "/path = \"\.\.\//s/version = \"$old\"/version = \"$new\"/" "$f"
    done
    rm -f Cargo.toml.bak crates/*/Cargo.toml.bak
    [ "$(workspace_version)" = "$new" ] || die "Cargo.toml version was not updated."
    local stale
    stale="$(stale_pins "$new")"
    [ -z "$stale" ] || die "Internal pins not at $new:
$stale"
    cargo update --workspace --quiet

    echo "Step 2/5: $CHANGELOG"
    today="$(date +%Y-%m-%d)"
    awk -v ver="$new" -v dt="$today" -v url="$REPO_URL" -v prev="$old" '
        /^## \[Unreleased\]/ { print; print ""; print "## [" ver "] - " dt; next }
        /^\[Unreleased\]: / {
            print "[Unreleased]: " url "/compare/v" ver "...HEAD"
            print "[" ver "]: " url "/compare/v" prev "...v" ver
            next
        }
        { print }
    ' "$CHANGELOG" > "$CHANGELOG.tmp"
    mv "$CHANGELOG.tmp" "$CHANGELOG"
    grep -qF "[Unreleased]: $REPO_URL/compare/v$new...HEAD" "$CHANGELOG" \
        || die "$CHANGELOG has no [Unreleased] link to rewrite."

    echo "Step 3/5: tests"
    cargo test --workspace --quiet

    echo "Step 4/5: coverage"
    "$REPO_ROOT/scripts/check-coverage.sh"

    echo "Step 5/5: commit"
    git add Cargo.toml Cargo.lock "${MANIFESTS[@]}" "$CHANGELOG" .coverage-last
    git commit --quiet -m "Release v$new"

    cat <<EOF

✓ release/v$new is ready. Next:
  1. Push release/v$new and open a PR into main.
  2. When CI is green, merge the PR.
  3. On main, run: just release-tag
  4. Back-merge main into develop.
EOF
}

tag() {
    require_branch main
    local version stale
    version="$(workspace_version)"
    stale="$(stale_pins "$version")"
    [ -z "$stale" ] || die "Internal pins not at $version:
$stale"
    grep -q "^## \[$version\]" "$CHANGELOG" || die "$CHANGELOG has no [$version] section."
    require_tag_absent "v$version"

    git tag "v$version"
    git push origin "v$version"
    echo "✓ Tagged $(git rev-parse --short HEAD) as v$version. The Release workflow builds it:"
    echo "  $REPO_URL/actions"
}

case "${1:-}" in
    prep) prep "${2:-}" ;;
    tag) tag ;;
    *) die "Usage: release.sh prep X.Y.Z | release.sh tag" ;;
esac
