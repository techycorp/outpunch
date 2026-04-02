# frozen_string_literal: true

require "websocket/driver"
require "json"

module Outpunch
  module Rack
    class Connection
      def initialize(server)
        @server = server
        @service_name = nil
        @write_mutex = Mutex.new
        @io = nil
        @driver = nil
        @env = nil
      end

      # Hijack the socket and run the WebSocket loop. Blocks until disconnected.
      def run(env)
        @env = env
        @io = env.fetch("rack.hijack_io")
        @driver = WebSocket::Driver.rack(self)
        @driver.on(:message) { |event| on_message(event.data) }
        @driver.on(:close)   { on_close }
        @driver.on(:error)   { |e| log_error(e.message) }
        @driver.start

        while (chunk = @io.readpartial(4096))
          @driver.parse(chunk)
        end
      rescue EOFError, IOError, Errno::ECONNRESET
        # normal close
      ensure
        on_close
      end

      # Called by Server#handle_request to send a request over the wire.
      def send_request(payload)
        msg = payload
          .merge(type: "request")
          .transform_keys(&:to_s)
        transmit(msg)
      end

      # Called by websocket-driver to write raw bytes to the socket.
      def write(data)
        @io.write(data)
      rescue IOError, Errno::EPIPE, Errno::ECONNRESET
        nil
      end

      # websocket-driver requires #env and #url to build the handshake.
      def env
        @env || {}
      end

      def url
        scheme = env["rack.url_scheme"] == "https" ? "wss" : "ws"
        host   = env["HTTP_HOST"] || env["SERVER_NAME"] || "localhost"
        "#{scheme}://#{host}#{env["REQUEST_URI"]}"
      end

      private

      def on_message(raw)
        data = JSON.parse(raw)
        case data["type"]
        when "auth"     then handle_auth(data)
        when "response" then @server.complete_request(data["request_id"], data)
        else
          # unknown message type — ignore
        end
      rescue JSON::ParserError
        nil
      end

      def handle_auth(data)
        if @server.valid_token?(data["token"]) && !data["service"].to_s.empty?
          @service_name = data["service"]
          @server.register_connection(@service_name, self)
          transmit(type: "auth_ok")
        else
          transmit(type: "auth_error", message: "invalid token")
          @driver.close
        end
      end

      def on_close
        return unless @service_name

        @server.unregister_connection(@service_name)
        @service_name = nil
      end

      def transmit(data)
        @write_mutex.synchronize { @driver.text(JSON.generate(data)) }
      end

      def log_error(message)
        warn "[Outpunch::Rack] WebSocket error: #{message}"
      end
    end
  end
end
