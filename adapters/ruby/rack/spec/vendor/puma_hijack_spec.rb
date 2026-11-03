# frozen_string_literal: true

require "spec_helper"
require "socket"
require "puma"
require "puma/configuration"

# Facts about Puma's Rack full hijack, observed on a real Puma. They are not tests of our code:
# when one fails, Puma changed or we misunderstood it. Run with `just test-vendor`; needs only
# a free local port.
#
# What depends on them: Outpunch::Rack::Middleware hijacks the WebSocket upgrade and hands the
# socket to Outpunch::Rack::Connection, then returns a 200 that the server must ignore; and the
# fake Rack env in spec/support/fakes/rack_hijack.rb.
RSpec.describe "Puma", :vendor do
  HIJACKED_BYTES = "bytes written by the app on the hijacked socket"

  # Boots Puma with the app on a free port; yields the port, then stops Puma.
  def with_puma(app)
    port = TCPServer.open("127.0.0.1", 0) { |s| s.addr[1] }
    config = Puma::Configuration.new do |c|
      c.bind "tcp://127.0.0.1:#{port}"
      c.app app
      c.threads 1, 4
      c.quiet
    end
    launcher = Puma::Launcher.new(config)
    thread = Thread.new { launcher.run }
    50.times do
      TCPSocket.new("127.0.0.1", port).close
      break
    rescue Errno::ECONNREFUSED
      sleep 0.02
    end
    yield port
  ensure
    launcher&.stop
    thread&.join(2)
  end

  # Everything the client receives for one upgrade request, until the socket closes.
  def raw_exchange(port)
    socket = TCPSocket.new("127.0.0.1", port)
    socket.write("GET /ws HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n")
    socket.read
  ensure
    socket&.close
  end

  # Takes the socket the hijack call returns, writes to it and closes it, then answers the way
  # Outpunch::Rack::Middleware does.
  def hijacking_app(response)
    lambda do |env|
      io = env["rack.hijack"].call
      io.write(HIJACKED_BYTES)
      io.close
      response
    end
  end

  it "returns the client's socket from a full hijack" do
    received = with_puma(hijacking_app([200, {}, []])) { |port| raw_exchange(port) }

    expect(received).to start_with(HIJACKED_BYTES)
  end

  it "writes nothing of its own to a fully hijacked socket, whatever the app returns" do
    response = [200, { "content-type" => "text/plain" }, ["a body the server must not send"]]
    received = with_puma(hijacking_app(response)) { |port| raw_exchange(port) }

    expect(received).to eq(HIJACKED_BYTES)
  end
end
