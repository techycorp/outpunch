# frozen_string_literal: true

require "outpunch"

def connection_error(url)
  Outpunch.run_connection(Outpunch::ClientConfig.new(url, "secret", "smoke", nil, nil, nil))
  abort "#{url}: connection unexpectedly completed"
rescue RuntimeError => e
  e.message
end

trusted = connection_error("wss://sha256.badssl.com/")
abort "public CA chain failed: #{trusted}" unless trusted.start_with?("HTTP error")

untrusted = connection_error("wss://untrusted-root.badssl.com/")
abort "untrusted root not rejected: #{untrusted}" unless untrusted.include?("invalid peer certificate")

puts "smoke test passed on Ruby #{RUBY_VERSION}"
