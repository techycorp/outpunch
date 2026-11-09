# frozen_string_literal: true

require "outpunch"

RSpec.configure do |config|
  # Vendor specs check facts about Ruby itself: `just test-vendor`.
  config.filter_run_excluding :vendor unless config.inclusion_filter[:vendor]
end
