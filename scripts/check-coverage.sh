#!/usr/bin/env bash
# =============================================================================
# Coverage check script for scope-blockchain-analysis
# - Ensures coverage meets mandatory 80% threshold
# - Tracks last known coverage in .coverage-last to prevent regressions
# - Run via pre-push hook or manually: ./scripts/check-coverage.sh
# =============================================================================

set -e

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

COVERAGE_FILE="$REPO_ROOT/.coverage-last"
MANDATORY_THRESHOLD=80

# -----------------------------------------------------------------------------
# Check prerequisites
# -----------------------------------------------------------------------------
if ! command -v cargo-tarpaulin &>/dev/null; then
    echo "❌ cargo-tarpaulin not found. Install with: cargo install cargo-tarpaulin"
    exit 1
fi

# -----------------------------------------------------------------------------
# Run coverage and extract percentage
# -----------------------------------------------------------------------------
echo "Running coverage (cargo tarpaulin)..."
COVERAGE_OUTPUT=$(cargo tarpaulin --out Stdout 2>&1)

# Prefer "X.XX% coverage" (summary line); fallback to first percentage match
COVERAGE_PCT=$(echo "$COVERAGE_OUTPUT" | grep -oE '[0-9]+\.[0-9]+% coverage' | head -1 | sed 's/% coverage//')
if [ -z "$COVERAGE_PCT" ]; then
    COVERAGE_PCT=$(echo "$COVERAGE_OUTPUT" | grep -oE '[0-9]+\.[0-9]+%' | head -1 | sed 's/%//')
fi

if [ -z "$COVERAGE_PCT" ]; then
    echo "❌ Could not parse coverage percentage from tarpaulin output"
    echo ""
    echo "Last lines of output:"
    echo "$COVERAGE_OUTPUT" | tail -20
    exit 1
fi

echo "Current coverage: ${COVERAGE_PCT}%"

# -----------------------------------------------------------------------------
# Float comparison helper (works without bc)
# -----------------------------------------------------------------------------
ge() {
    awk -v a="$1" -v b="$2" 'BEGIN { exit (a+0 >= b+0) ? 0 : 1 }'
}

# -----------------------------------------------------------------------------
# Check 1: Mandatory 80% threshold
# -----------------------------------------------------------------------------
if ! ge "$COVERAGE_PCT" "$MANDATORY_THRESHOLD"; then
    echo ""
    echo "❌ Coverage ${COVERAGE_PCT}% is below mandatory threshold of ${MANDATORY_THRESHOLD}%"
    echo "   Add more tests to reach the required coverage."
    echo ""
    echo "Top uncovered modules:"
    echo "$COVERAGE_OUTPUT" | grep -E '^\|\| src/' 2>/dev/null | head -10 || true
    exit 1
fi

# -----------------------------------------------------------------------------
# Check 2: No regression below last known coverage
# -----------------------------------------------------------------------------
if [ -f "$COVERAGE_FILE" ]; then
    LAST_COVERAGE=$(cat "$COVERAGE_FILE")
    if ! ge "$COVERAGE_PCT" "$LAST_COVERAGE"; then
        echo ""
        echo "❌ Coverage regressed: ${COVERAGE_PCT}% (was ${LAST_COVERAGE}%)"
        echo "   Do not push code that reduces test coverage."
        echo "   Last known: $COVERAGE_FILE"
        exit 1
    fi
    if [ "$COVERAGE_PCT" != "$LAST_COVERAGE" ]; then
        echo "✓ Coverage improved: ${LAST_COVERAGE}% → ${COVERAGE_PCT}%"
    fi
else
    echo "✓ First run: recording ${COVERAGE_PCT}% as baseline in .coverage-last"
fi

# -----------------------------------------------------------------------------
# Record new baseline
# -----------------------------------------------------------------------------
echo "$COVERAGE_PCT" > "$COVERAGE_FILE"
echo "✓ Coverage check passed (${COVERAGE_PCT}% >= ${MANDATORY_THRESHOLD}%)"
