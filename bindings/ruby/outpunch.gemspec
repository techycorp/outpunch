# frozen_string_literal: true

Gem::Specification.new do |spec|
  spec.name    = "outpunch"
  spec.version = "0.1.0"
  spec.authors = ["TechyCorp"]
  spec.summary = "Ruby bindings for the outpunch tunnel client"
  spec.license = "MIT"

  spec.files         = Dir["lib/**/*.rb", "ext/**/*"]
  spec.require_paths = ["lib"]
  spec.extensions    = ["ext/outpunch/extconf.rb"]

  spec.required_ruby_version = ">= 3.1"

  spec.add_dependency "rb_sys", "~> 0.9"

  spec.add_development_dependency "rake-compiler", "~> 1.2"
  spec.add_development_dependency "rspec", "~> 3.12"
end
