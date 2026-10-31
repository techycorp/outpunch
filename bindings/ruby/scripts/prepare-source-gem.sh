#!/usr/bin/env bash
# Point the native gem's crates at crates.io: a gem compiles outside this repo, so the local path
# dependencies can't resolve.
# Usage: prepare-source-gem.sh <bindings/ruby directory>
set -euo pipefail

manifest=$1/ext/outpunch/Cargo.toml
sed -i.bak -E 's/, path = "[^"]*"//' "$manifest"
rm "$manifest.bak"
if grep -q 'path = ' "$manifest"; then
  echo "::error::path dependency left in $manifest"
  exit 1
fi
