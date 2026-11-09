# frozen_string_literal: true

require "io/wait"
require "websocket/driver"

module Outpunch
  module Rack
    # Bridges a hijacked Rack socket to a native outpunch core connection.
    # Ruby handles socket I/O and WebSocket framing; the core handles auth,
    # routing and request state.
    class Connection
      def initialize(native, write_timeout: nil)
        @native = native
        @write_timeout = write_timeout
        @write_mutex = Mutex.new
        # Serializes socket writes: the reader thread writes pongs while the writer thread may be
        # partway through a frame (write_within can take several write_nonblock calls).
        @io_mutex = Mutex.new
        @io = nil
        @driver = nil
        @env = nil
      end

      # Run the WebSocket loop on the hijacked socket. Blocks until disconnected.
      def run(env, io)
        @env = env
        @io = io
        @driver = WebSocket::Driver.rack(self)
        @driver.on(:message) { |event| @native.push_message(event.data) }
        # A client ping (the driver answers it) counts as a sign of life (R30).
        @driver.on(:ping)    { @native.heartbeat }
        @driver.on(:pong)    { @native.heartbeat }
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

      # Called by websocket-driver to write raw bytes to the socket. A write that cannot
      # finish within the write timeout (the client stopped reading) closes the connection,
      # so the service is unregistered instead of queueing requests for it (F7 in
      # spec/FINDINGS.md).
      def write(data)
        @io_mutex.synchronize do
          next @io.write(data) unless @write_timeout && @io.respond_to?(:write_nonblock)

          write_within(data, monotonic_now + @write_timeout) || stop_stalled
        end
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

      # Writes all of data unless the deadline passes first; returns false if it does.
      def write_within(data, deadline)
        until data.empty?
          written = @io.write_nonblock(data, exception: false)
          if written == :wait_writable
            remaining = deadline - monotonic_now
            return false unless remaining.positive? && @io.wait_writable(remaining)
          else
            data = data.byteslice(written..)
          end
        end
        true
      end

      def stop_stalled
        log_error("client stopped reading; closing the connection")
        @native.close
        close_io
        nil
      end

      def monotonic_now
        Process.clock_gettime(Process::CLOCK_MONOTONIC)
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
