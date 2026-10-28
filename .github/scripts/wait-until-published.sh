#!/usr/bin/env bash
# Waits until every outpunch package is served at <version> by its registry; fails after <timeout> seconds.
# A publish command can succeed before the registry serves the new version, so this is the release's proof.
#   wait-until-published.sh <version> <timeout seconds>
set -euo pipefail

version=$1 timeout=$2
root=$(cd "$(dirname "$0")/../.." && pwd)

packages=(
  "crates outpunch"
  "crates outpunch-axum"
  "crates outpunch-client"
  "pypi outpunch"
  "npm outpunch"
  "rubygems outpunch-ruby"
  "rubygems outpunch-ruby-rack"
  "rubygems outpunch-ruby-rails"
)
for manifest in "$root"/bindings/node/npm/*/package.json; do
  packages+=("npm $(node -p "require('$manifest').name")")
done

deadline=$((SECONDS + timeout))
pending=("${packages[@]}")
while [ ${#pending[@]} -gt 0 ]; do
  still=()
  for entry in "${pending[@]}"; do
    read -r registry name <<<"$entry"
    if [ "$(bash "$root/.github/scripts/registry-status.sh" "$registry" "$name" "$version")" = published ]; then
      echo "live: $registry $name $version"
    else
      still+=("$entry")
    fi
  done
  pending=("${still[@]+"${still[@]}"}")
  [ ${#pending[@]} -eq 0 ] && break
  if [ $SECONDS -ge $deadline ]; then
    printf '::error::not live after %ss: %s\n' "$timeout" "${pending[*]}"
    exit 1
  fi
  sleep 20
done
echo "all ${#packages[@]} packages live at $version"
