#!/usr/bin/env bash
# Fails if a Linux binary needs a newer glibc than the given version.
#   check-glibc-floor.sh <binary> <max version>
set -euo pipefail

binary=$1 floor=$2
needed=$(readelf -V "$binary" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -1)
newest=$(printf '%s\n%s\n' "$needed" "$floor" | sort -V | tail -1)
if [ "$newest" != "$floor" ]; then
  echo "::error::$binary needs glibc $needed, above the $floor floor"
  exit 1
fi
echo "$binary needs glibc $needed (floor $floor)"
