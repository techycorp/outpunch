# frozen_string_literal: true

require "spec_helper"
require "websocket/driver"

# Facts about websocket-driver, observed on the installed gem. They are not tests of our code.
# Run with `just test-vendor`.
#
# What depends on them: Outpunch::Rack::Connection counts a client ping as a sign of life (R30 in
# spec/REQUIREMENTS.md) through the driver's :ping event, and relies on the driver to answer it.
RSpec.describe "websocket-driver", :vendor do
  # A driver in server mode over an in-memory "socket" that records what the driver writes.
  class RecordingSocket
    attr_reader :written

    def initialize
      @written = +""
    end

    def env
      { "HTTP_UPGRADE" => "websocket", "HTTP_CONNECTION" => "Upgrade", "HTTP_HOST" => "localhost",
        "HTTP_SEC_WEBSOCKET_KEY" => "dGhlIHNhbXBsZSBub25jZQ==", "HTTP_SEC_WEBSOCKET_VERSION" => "13",
        "REQUEST_METHOD" => "GET", "PATH_INFO" => "/ws" }
    end

    def url
      "ws://localhost/ws"
    end

    def write(data)
      @written << data.b
    end
  end

  def masked_ping(payload)
    mask = [1, 2, 3, 4]
    bytes = payload.bytes.each_with_index.map { |b, i| b ^ mask[i % 4] }
    ([0x89, 0x80 | bytes.size] + mask + bytes).pack("C*")
  end

  # Claim:    a server-side driver emits :ping for a client ping and writes a pong frame with the same payload
  # Fails if: the driver answered without emitting :ping (the adapter could not see the ping) or
  #           wrote no pong (clients that wait for one would reconnect)
  # Control:  the pong frame (opcode 0x8A, unmasked) carries the ping's payload
  it "emits :ping for a client ping and answers it with a pong" do
    socket = RecordingSocket.new
    driver = WebSocket::Driver.rack(socket)
    pings = []
    driver.on(:ping) { |event| pings << event.data }
    driver.start
    socket.written.clear

    driver.parse(masked_ping("p"))

    expect(pings).to eq(["p"])
    expect(socket.written.bytes).to eq([0x8A, 0x01, "p".ord])
  end
end
