# frozen_string_literal: true

require "websocket/driver"

module Outpunch
  module Rack
    # Bridges a hijacked Rack socket to a native outpunch core connection.
    # Ruby handles socket I/O and WebSocket framing; the core handles auth,
    # routing and request state.
    class Connection
      def initialize(native)
        @native = native
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
        @driver.on(:message) { |event| @native.push_message(event.data) }
        @driver.on(:close)   { @native.close }
        @driver.on(:error)   { |e| log_error(e.message) }
        @driver.start

        core = Thread.new { @native.run }
        writer = Thread.new { write_loop }

        while (chunk = @io.readpartial(4096))
          @driver.parse(chunk)
        end
      rescue EOFError, IOError, Errno::ECONNRESET, Errno::EPIPE
        # normal close
      ensure
        @native.close
        core&.join
        writer&.join
        close_io
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

      # Deliver core messages to the client until the core connection ends, then
      # close the socket so the read loop in #run returns.
      def write_loop
        while (msg = @native.next_outgoing)
          @write_mutex.synchronize { @driver.text(msg) }
        end
        @write_mutex.synchronize { @driver.close }
        close_io
      end

      def close_io
        @io.close unless @io.closed?
      rescue IOError
        nil
      end

      def log_error(message)
        warn "[Outpunch::Rack] WebSocket error: #{message}"
      end
    end
  end
end
