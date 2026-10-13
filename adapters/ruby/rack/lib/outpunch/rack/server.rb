# frozen_string_literal: true

require "outpunch/outpunch"
require "base64"
require "json"

module Outpunch
  module Rack
    # Ruby-facing wrapper around the native outpunch core server.
    class Server
      HOP_BY_HOP = %w[HOST CONNECTION UPGRADE].freeze

      def initialize(secret:, timeout: 25)
        raise ArgumentError, "outpunch secret must be set" if secret.nil? || secret.to_s.empty?

        @native = Outpunch::Server.new(secret.to_s, timeout.to_f, nil)
      end

      def create_connection
        Connection.new(@native.create_connection)
      end

      def connected?(service_name)
        @native.connected?(service_name.to_s)
      end

      # Send a request through the tunnel and block until response or timeout.
      # Raises Outpunch::NotConnectedError (a RuntimeError) when no client is connected
      # and Outpunch::TunnelTimeoutError (a Timeout::Error) on timeout.
      def handle_request(service:, method:, path:, query:, headers:, body:)
        @native.handle_request(
          service.to_s,
          method.to_s,
          path.to_s,
          stringify(query),
          stringify(headers),
          body&.to_s
        )
      end

      # Build a success result hash from raw tunnel response data.
      def success_response(data)
        body = data["body"]
        headers = (data["headers"] || {}).transform_keys(&:downcase)

        if data["body_encoding"] == "base64" && !body.nil? && !body.empty?
          body = Base64.decode64(body)
          body.force_encoding("BINARY")
        end

        {
          status: data["status"] || 200,
          body: body,
          headers: headers,
          body_encoding: data["body_encoding"]
        }
      end

      # Build an error result hash.
      def error_response(status, message)
        {
          status: status,
          body: JSON.generate(error: message),
          headers: { "Content-Type" => "application/json" }
        }
      end

      # Extract and normalize HTTP headers from a Rack env hash.
      def extract_proxy_headers(headers)
        headers
          .to_h
          .select { |k, _| k.to_s.start_with?("HTTP_") }
          .transform_keys { |k| k.to_s.sub("HTTP_", "").tr("_", "-") }
          .reject { |k, _| HOP_BY_HOP.include?(k) }
      end

      private

      def stringify(hash)
        (hash || {}).to_h.to_h { |k, v| [k.to_s, v.to_s] }
      end
    end
  end
end
