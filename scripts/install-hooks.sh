#!/usr/bin/env bash
# Install git hooks into .git/hooks
# Run from repo root: ./scripts/install-hooks.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOOKS_DIR="$REPO_ROOT/.git/hooks"

cd "$REPO_ROOT"

if [ ! -d ".git" ]; then
    echo "❌ Not a git repository"
    exit 1
fi

mkdir -p "$HOOKS_DIR"
cp "$SCRIPT_DIR/pre-push" "$HOOKS_DIR/pre-push"
chmod +x "$HOOKS_DIR/pre-push"

echo "✓ Installed pre-push hook (coverage check before push)"
