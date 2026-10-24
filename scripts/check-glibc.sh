#!/usr/bin/env bash
# Fails when a Linux binary needs a newer glibc than the oldest we support.
#
#   scripts/check-glibc.sh <binary> [max]   (max defaults to 2.35, Ubuntu 22.04)
#
# A binary built on a newer system asks for its glibc's symbol versions, and
# an older system then warns ("weak version `GLIBC_2.39' not found") or
# fails to start. readelf reads any architecture, so a cross-built binary is
# checked too.
set -euo pipefail

bin="${1:?Usage: check-glibc.sh <binary> [max]}"
max="${2:-2.35}"

need="$(readelf -W -V "$bin" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -1)"
[ -n "$need" ] || { echo "No glibc symbol versions in $bin."; exit 0; }

if [ "$(printf '%s\n%s\n' "$need" "$max" | sort -V | tail -1)" != "$max" ]; then
    echo "✗ $bin needs glibc $need; the most allowed is $max." >&2
    readelf -W -V "$bin" | grep -o "GLIBC_[0-9.]*" | sort -Vu | tail -3 >&2
    exit 1
fi
echo "✓ $bin needs glibc $need (≤ $max)."
