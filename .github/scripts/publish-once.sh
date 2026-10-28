#!/usr/bin/env bash
# Runs a publish command. Succeeds if it publishes or if this version is already on the registry;
# any other failure (auth, permissions, network) fails the step.
set -uo pipefail

out=$("$@" 2>&1)
status=$?
echo "$out"
[ "$status" -eq 0 ] && exit 0

if echo "$out" | grep -qiE "already (uploaded|exists)|Repushing of gem versions is not allowed|cannot publish over the previously published"; then
  echo "Already published, skipping"
  exit 0
fi
exit "$status"
