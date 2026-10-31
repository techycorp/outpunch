#!/usr/bin/env bash
# Build the source gem the way the release does, install it, and connect with it.
# The crates may not be on crates.io yet, so a cargo patch resolves them from this checkout.
set -euo pipefail

root=$(cd "$(dirname "$0")/../../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cd "$root"
git ls-files -z bindings/ruby | xargs -0 tar -cf - | tar -xf - -C "$work"
bash bindings/ruby/scripts/prepare-source-gem.sh "$work/bindings/ruby"

mkdir "$work/.cargo"
cat > "$work/.cargo/config.toml" <<TOML
[patch.crates-io]
outpunch = { path = "$root/crates/outpunch" }
outpunch-client = { path = "$root/crates/outpunch-client" }
TOML

cd "$work/bindings/ruby"
gem build outpunch-ruby.gemspec
export GEM_HOME=$work/gems GEM_PATH=$work/gems
gem install --no-document outpunch-ruby-*.gem
ruby "$root/bindings/ruby/scripts/connect-check.rb"
