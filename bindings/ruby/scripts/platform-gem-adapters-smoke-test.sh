#!/usr/bin/env bash
# Install the precompiled gem with the Rack and Rails adapters and boot them.
# The adapters' websocket-driver dependency compiles C, so this uses an image with a compiler.
# Usage: platform-gem-adapters-smoke-test.sh <gem dir> <ruby version>
set -euo pipefail

gem_dir=$(cd "$1" && pwd)
ruby_version=$2

docker run --rm --platform linux/amd64 \
  --volume "$gem_dir:/smoke:ro" \
  "ruby:$ruby_version" bash -euo pipefail -c '
    gem install --no-document /smoke/outpunch-ruby-*-x86_64-linux.gem /smoke/outpunch-ruby-rack-*.gem /smoke/outpunch-ruby-rails-*.gem

    ruby -e "
      require \"rails\"
      require \"outpunch-ruby-rack\"
      require \"outpunch-ruby-rails\"
      abort \"Rack adapter did not load\" unless defined?(Outpunch::Rack::Server)
      abort \"Rails adapter did not load\" unless defined?(Outpunch::Rails::Engine)
      puts \"adapter smoke test passed on Ruby #{RUBY_VERSION}\"
    "
  '
