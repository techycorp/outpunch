#!/usr/bin/env bash
# Runs a publish command unless that exact version is already on the registry.
# The publish command's exit status is returned unchanged, so every publish failure fails the step.
#   publish-if-missing.sh <crates|npm|rubygems|rubygems:<platform>|pypi> <name> <version> <command...>
set -euo pipefail

registry=$1 name=$2 version=$3
shift 3

status=$(bash "$(dirname "$0")/registry-status.sh" "$registry" "$name" "$version")
if [ "$status" = published ]; then
  echo "::warning::$name $version is already on $registry; not publishing it again"
  exit 0
fi
exec "$@"
