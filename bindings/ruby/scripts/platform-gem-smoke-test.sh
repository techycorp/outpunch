#!/usr/bin/env bash
# Install a precompiled gem in a container with no compiler and exercise it.
# Usage: platform-gem-smoke-test.sh <path to .gem> <ruby version>
set -euo pipefail

gem_file=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
ruby_version=$2
image=heroku/heroku:24
ruby_url=https://heroku-buildpack-ruby.s3.us-east-1.amazonaws.com/heroku-24/amd64/ruby-$ruby_version.tgz

docker run --rm --platform linux/amd64 --user root \
  --volume "$gem_file:/smoke/$(basename "$gem_file"):ro" \
  --env GEM_FILE="/smoke/$(basename "$gem_file")" \
  --env RUBY_URL="$ruby_url" \
  "$image" bash -euo pipefail -c '
    if command -v cc gcc cargo rustc; then
      echo "smoke image must have no compiler" >&2
      exit 1
    fi

    mkdir /ruby
    curl --fail --silent --show-error --location "$RUBY_URL" | tar -xz -C /ruby
    export PATH=/ruby/bin:$PATH
    export GEM_HOME=/gems GEM_PATH=/gems HOME=/tmp

    gem install --local --no-document "$GEM_FILE"
    if [ -n "$(find /gems/extensions -type f 2>/dev/null)" ]; then
      echo "the gem compiled on install instead of using its precompiled binary" >&2
      exit 1
    fi

    cd "$(mktemp -d)"
    gem list --local --exact outpunch-ruby | grep -q "x86_64-linux"
    printf "source \"https://rubygems.org\"\ngem \"outpunch-ruby\"\n" > Gemfile
    bundle lock --local --add-platform x86_64-linux
    grep -q "outpunch-ruby (.*-x86_64-linux)" Gemfile.lock

    bundle exec ruby -e "
      require \"outpunch\"

      def connection_error(url)
        Outpunch.run_connection(Outpunch::ClientConfig.new(url, \"secret\", \"smoke\", nil, nil, nil))
        abort \"#{url}: connection unexpectedly completed\"
      rescue RuntimeError => e
        e.message
      end

      trusted = connection_error(\"wss://sha256.badssl.com/\")
      abort \"public CA chain failed: #{trusted}\" unless trusted.start_with?(\"HTTP error\")

      untrusted = connection_error(\"wss://untrusted-root.badssl.com/\")
      abort \"untrusted root not rejected: #{untrusted}\" unless untrusted.include?(\"invalid peer certificate\")

      puts \"smoke test passed on Ruby #{RUBY_VERSION}\"
    "
  '
