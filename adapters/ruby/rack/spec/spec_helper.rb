# frozen_string_literal: true

require "outpunch/rack"

RSpec.configure do |config|
  config.expect_with :rspec do |expectations|
    expectations.include_chain_clauses_in_custom_matcher_descriptions = true
  end
  config.mock_with :rspec do |mocks|
    mocks.verify_partial_doubles = true
  end
  config.shared_context_metadata_behavior = :apply_to_host_groups
  # Vendor specs check facts about real third-party systems: `just test-vendor`.
  config.filter_run_excluding :vendor unless config.inclusion_filter[:vendor]
end

Dir[File.join(__dir__, "support", "**", "*.rb")].each { |f| require f }
