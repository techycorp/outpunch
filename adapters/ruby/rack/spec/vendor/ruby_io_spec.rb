# frozen_string_literal: true

require "spec_helper"
require "socket"
require "io/wait"

# Facts about Ruby's socket IO, observed on the running Ruby. They are not tests of our code.
# Run with `just test-vendor`.
#
# What depends on them: Outpunch::Rack::Connection#write, which writes with write_nonblock and
# waits with wait_writable so a client that stops reading cannot block a write past the
# write timeout (F7 in spec/FINDINGS.md).
RSpec.describe "Ruby socket IO", :vendor do
  # A connected pair over loopback TCP; the peer never reads.
  def with_stalled_peer
    server = TCPServer.new("127.0.0.1", 0)
    client = TCPSocket.new("127.0.0.1", server.addr[1])
    peer = server.accept
    yield client
  ensure
    [client, peer, server].each { |s| s&.close }
  end

  # Writes until the socket stays full: the kernel moves data on to the peer's receive buffer
  # for a moment after the first :wait_writable, so one refusal is not enough.
  def fill(socket)
    chunk = "x" * 65_536
    loop do
      nil until socket.write_nonblock(chunk, exception: false) == :wait_writable
      sleep 0.2
      return if socket.write_nonblock(chunk, exception: false) == :wait_writable
    end
  end

  # Claim:    write_nonblock with exception: false returns :wait_writable once a socket's buffers are full because the peer is not reading
  # Fails if: write_nonblock blocked (the spec would hang until RSpec's timeout) or kept accepting
  #           data without limit (fill would never return)
  # Control:  the first write is accepted (returns a byte count)
  it "returns :wait_writable from write_nonblock when the peer stops reading" do
    with_stalled_peer do |socket|
      expect(socket.write_nonblock("x", exception: false)).to eq(1)
      Timeout.timeout(5) { fill(socket) }
      expect(socket.write_nonblock("x" * 65_536, exception: false)).to eq(:wait_writable)
    end
  end

  # Claim:    wait_writable with a timeout returns nil after about that long when the socket stays unwritable
  # Fails if: wait_writable returned the socket (claiming it writable) or waited much longer than
  #           its timeout
  # Control:  on a fresh socket with room it returns the socket at once
  it "returns nil from wait_writable after its timeout when the socket stays full" do
    with_stalled_peer do |socket|
      expect(socket.wait_writable(1)).to eq(socket)
      Timeout.timeout(5) { fill(socket) }

      started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      expect(socket.wait_writable(0.3)).to be_nil
      expect(Process.clock_gettime(Process::CLOCK_MONOTONIC) - started).to be_between(0.25, 1.0)
    end
  end
end
