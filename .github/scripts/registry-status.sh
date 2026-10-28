#!/usr/bin/env bash
# Prints "published" or "missing" for one package version on a registry.
# Any other HTTP status or a network error exits non-zero, so callers never mistake an outage for "missing".
#   registry-status.sh <crates|npm|rubygems|pypi> <name> <version>
set -euo pipefail

registry=$1 name=$2 version=$3
case "$registry" in
  crates)   url="https://crates.io/api/v1/crates/$name/$version" ;;
  npm)      url="https://registry.npmjs.org/$name/$version" ;;
  rubygems) url="https://rubygems.org/api/v2/rubygems/$name/versions/$version.json" ;;
  pypi)     url="https://pypi.org/pypi/$name/$version/json" ;;
  *)        echo "unknown registry: $registry" >&2; exit 2 ;;
esac

# crates.io rejects requests without a User-Agent.
code=$(curl -sS -o /dev/null -w '%{http_code}' --retry 3 -A "outpunch-release (github.com/techycorp/outpunch)" "$url")
case "$code" in
  200) echo published ;;
  404) echo missing ;;
  *)   echo "$registry $name $version: unexpected HTTP $code from $url" >&2; exit 1 ;;
esac
