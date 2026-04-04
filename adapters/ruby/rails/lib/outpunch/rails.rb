# frozen_string_literal: true

require "outpunch/rack"
require "outpunch/rails/version"
require "outpunch/rails/engine"

# OutpunchRails — thin Rails wrapper around OutpunchRack.
#
# Usage in config/initializers/outpunch.rb:
#
#   OutpunchRails.configure do |config|
#     config.secret  = ENV['MORDOR_TUNNEL_SECRET']
#     config.timeout = 60
#   end
#
module OutpunchRails
  class << self
    def configure(&block)
      Outpunch::Rack.configure(&block)
    end

    def configuration
      Outpunch::Rack.configuration
    end

    def hooks
      Outpunch::Rack.configuration.hooks
    end

    def server
      Outpunch::Rack.server
    end

    # Convenience delegates so application code only needs to reference OutpunchRails.

    def connected?(service_name)
      Outpunch::Rack.connected?(service_name)
    end

    def handle_request(**kwargs)
      Outpunch::Rack.handle_request(**kwargs)
    end

    def success_response(data)
      Outpunch::Rack.success_response(data)
    end

    def error_response(status, message)
      Outpunch::Rack.error_response(status, message)
    end

    def extract_proxy_headers(headers)
      Outpunch::Rack.extract_proxy_headers(headers)
    end
  end
end
