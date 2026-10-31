#!/usr/bin/env bash
# Cross-compile the x86_64-linux gem into bindings/ruby/pkg.
# Runs from the repo root so the build container can see the path dependencies in crates/.
# Usage: build-platform-gem.sh <comma-separated Ruby minor versions>
set -euo pipefail

ruby_versions=$1
root=$(cd "$(dirname "$0")/../../.." && pwd)
rb_sys_version=$(awk '$1 == "rb_sys" && $2 ~ /^\([0-9]/ { gsub(/[()]/, "", $2); print $2 }' "$root/bindings/ruby/Gemfile.lock")

gem install rb_sys --version "$rb_sys_version" --no-document

cd "$root"
# The image's gcc 9.4 miscompiles memcmp at -O2 (GCC bug 95189), which the TLS crypto library refuses.
rb-sys-dock --platform x86_64-linux --ruby-versions "$ruby_versions" --directory "$root/bindings/ruby" --build -- \
  "sudo apt-get update -qq && sudo apt-get install -y -qq gcc-10 && sudo ln -sf /usr/bin/x86_64-linux-gnu-gcc-10 /usr/bin/x86_64-linux-gnu-gcc"
