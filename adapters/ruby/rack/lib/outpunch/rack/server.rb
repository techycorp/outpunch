# frozen_string_literal: true

require "concurrent"
require "securerandom"
require "timeout"
require "openssl"
require "base64"
require "json"

module Outpunch
  module Rack
    class Server
      HOP_BY_HOP = %w[HOST CONNECTION UPGRADE].freeze

      def initialize(secret:, timeout: 25)
        @secret = secret
        @timeout = timeout
        @connections = Concurrent::Map.new
        @pending_requests = Concurrent::Map.new
      end

      def create_connection
        Connection.new(self)
      end

      # Register a service connection. Called by Connection after successful auth.
      def register_connection(service_name, conn)
        @connections[service_name] = conn
      end

      # Remove a service connection. Called by Connection on close.
      def unregister_connection(service_name)
        @connections.delete(service_name)
      end

      def connected?(service_name)
        @connections.key?(service_name)
      end

      # Send a request through the tunnel and block until response or timeout.
      def handle_request(service:, method:, path:, query:, headers:, body:)
        conn = @connections[service]
        raise "Service '#{service}' not connected" unless conn

        request_id = SecureRandom.uuid
        queue = Queue.new
        @pending_requests[request_id] = queue

        conn.send_request(
          request_id: request_id,
          service: service,
          method: method,
          path: path,
          query: query,
          headers: headers,
          body: body
        )

        begin
          Timeout.timeout(@timeout) { queue.pop }
        ensure
          @pending_requests.delete(request_id)
        end
      end

      # Deliver a response to a waiting handle_request call.
      def complete_request(request_id, response_data)
        @pending_requests[request_id]&.push(response_data)
      end

      # Constant-time token validation.
      def valid_token?(token)
        return false if @secret.nil? || @secret.empty? || token.nil? || token.empty?
        return false if token.bytesize != @secret.bytesize

        OpenSSL.fixed_length_secure_compare(token, @secret)
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

      # For test isolation — resets all state.
      def reset!
        @connections.clear
        @pending_requests.clear
      end
    end
  end
end
