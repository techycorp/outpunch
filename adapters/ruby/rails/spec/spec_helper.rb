# frozen_string_literal: true

ENV["RAILS_ENV"] = "test"

require_relative "dummy/config/application"
require "outpunch/rails"

OutpunchRails.configure do |c|
  c.secret          = "test-secret"
  c.base_controller = "ApplicationController"
end

Dummy::Application.initialize!

require "rspec/rails"

RSpec.configure do |config|
  config.include Rails.application.routes.url_helpers
end
