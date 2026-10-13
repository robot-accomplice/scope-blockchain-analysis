#!/usr/bin/env bash
# Publish one workspace crate to crates.io (release.yml, publish-crate job).
# Skips the publish when crates.io already has this version, so a rerun after
# a partial release works. Any other publish error fails the step.
set -euo pipefail

crate="${1:?Usage: publish-crate.sh <crate>}"
version="$(cargo metadata --no-deps --format-version 1 \
    | jq -r --arg c "$crate" '.packages[] | select(.name == $c) | .version')"
[ -n "$version" ] || { echo "::error::$crate is not a workspace package"; exit 1; }

# If this request fails for any other reason, cargo publish runs and fails on
# a duplicate. The step goes red. It never goes falsely green.
if curl -sf -A "scope-bca-release (github.com/robot-accomplice/scope-blockchain-analysis)" \
    "https://crates.io/api/v1/crates/$crate/$version" >/dev/null; then
    echo "::notice::$crate $version is already on crates.io. Skipping the publish."
    exit 0
fi

cargo publish -p "$crate"
