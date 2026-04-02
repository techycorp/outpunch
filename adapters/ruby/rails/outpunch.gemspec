# frozen_string_literal: true

require_relative "lib/outpunch/rails/version"

Gem::Specification.new do |spec|
  spec.name    = "outpunch"
  spec.version = Outpunch::Rails::VERSION
  spec.authors = ["TechyCorp"]
  spec.summary = "Rails Engine adapter for the outpunch reverse WebSocket tunnel"
  spec.license = "MIT"

  spec.files         = ["lib/outpunch.rb"] + Dir["lib/**/*.rb"] + Dir["app/**/*.rb"]
  spec.require_paths = ["lib"]

  spec.required_ruby_version = ">= 3.1"

  spec.add_dependency "outpunch-rack", "~> #{Outpunch::Rails::VERSION.split(".").first(2).join(".")}"
  spec.add_dependency "railties", ">= 7.0"

  spec.add_development_dependency "rspec-rails", "~> 6.0"
  spec.add_development_dependency "rails", "~> 7.0"
end
