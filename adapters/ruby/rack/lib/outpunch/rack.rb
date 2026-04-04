# frozen_string_literal: true

require_relative "rack/version"
require_relative "rack/hooks"
require_relative "rack/server"
require_relative "rack/connection"
require_relative "rack/middleware"

module Outpunch
  module Rack
    class Configuration
      attr_accessor :secret, :timeout, :base_controller, :authorize_service, :hooks, :route_prefix

      def initialize
        @timeout = 25
        @base_controller = "ActionController::API"
        @hooks = Outpunch::Rack::Hooks
        @route_prefix = "/outpunch"
      end
    end

    class << self
      def configure
        yield configuration
        @server = nil # reset server so it picks up new config
      end

      def configuration
        @configuration ||= Configuration.new
      end

      def server
        @server ||= Server.new(
          secret: configuration.secret,
          timeout: configuration.timeout
        )
      end

      # Delegates — convenience access for application code.

      def connected?(service_name)
        server.connected?(service_name)
      end

      def handle_request(**kwargs)
        server.handle_request(**kwargs)
      end

      def success_response(data)
        server.success_response(data)
      end

      def error_response(status, message)
        server.error_response(status, message)
      end

      def extract_proxy_headers(headers)
        server.extract_proxy_headers(headers)
      end
    end
  end
end
