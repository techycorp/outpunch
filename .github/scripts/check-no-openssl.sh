#!/usr/bin/env bash
# Fails if a lockfile contains OpenSSL or native-tls. See "TLS" in AGENTS.md.
#   check-no-openssl.sh <Cargo.lock>...
set -euo pipefail

status=0
for lockfile in "$@"; do
  if grep -E '^name = "(openssl|openssl-sys|native-tls)"$' "$lockfile"; then
    echo "::error::$lockfile depends on OpenSSL or native-tls"
    status=1
  fi
done
exit "$status"
