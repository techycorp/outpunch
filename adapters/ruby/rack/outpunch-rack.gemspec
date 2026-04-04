# frozen_string_literal: true

require_relative "lib/outpunch/rack/version"

Gem::Specification.new do |spec|
  spec.name    = "outpunch-rack"
  spec.version = Outpunch::Rack::VERSION
  spec.authors = ["TechyCorp"]
  spec.summary = "Rack adapter for the outpunch reverse WebSocket tunnel"
  spec.license = "MIT"

  spec.files         = Dir["lib/**/*.rb"]
  spec.require_paths = ["lib"]

  spec.required_ruby_version = ">= 3.1"

  spec.add_dependency "websocket-driver", ">= 0.7"
  spec.add_dependency "concurrent-ruby", ">= 1.0"

  spec.add_development_dependency "rspec", "~> 3.12"
  spec.add_development_dependency "rack", "~> 3.0"
  spec.add_development_dependency "puma", "~> 6.0"
  spec.add_development_dependency "websocket-client-simple", "~> 0.6"
end
