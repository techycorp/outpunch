# frozen_string_literal: true

require "rack"
require "puma"
require "puma/configuration"
require "websocket-client-simple"
require "socket"
require "json"

# Real Puma server with the outpunch middleware, plus a WebSocket tunnel client.
module TunnelHarness
  class TunnelClient
    def initialize(port, service:, token:)
      messages = @messages = Queue.new
      closed = @closed = Queue.new
      # Handlers are registered in the block so they exist before the handshake starts.
      @ws = WebSocket::Client::Simple.connect("ws://127.0.0.1:#{port}/ws") do |ws|
        ws.on(:open) { ws.send(JSON.generate(type: "auth", token: token, service: service)) }
        ws.on(:message) do |msg|
          case msg.type
          when :text then messages << JSON.parse(msg.data)
          when :close then closed << true
          end
        end
        ws.on(:close) { closed << true }
      end
    end

    def next_message(timeout: 2)
      pop_within(@messages, timeout)
    end

    def respond(request, status: 200, body: "ok")
      @ws.send(JSON.generate(type: "response", request_id: request["request_id"],
                             status: status, headers: {}, body: body))
    end

    def close
      @ws.close
    end

    def closed?(timeout: 2)
      !pop_within(@closed, timeout).nil?
    end

    private

    # Queue#pop(timeout:) needs Ruby 3.2; poll so the specs also run on 3.1.
    def pop_within(queue, timeout)
      deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + timeout
      loop do
        return queue.pop(true)
      rescue ThreadError
        return nil if Process.clock_gettime(Process::CLOCK_MONOTONIC) >= deadline

        sleep 0.01
      end
    end
  end

  # Leaves no `rack.hijack_io` in the env, as a Rack 3 server that doesn't set it would: the
  # hijack still returns the socket. Puma sets the key inside the hijack call, so it is removed
  # after the call, not before.
  class WithoutHijackIo
    def initialize(app)
      @app = app
    end

    def call(env)
      hijack = env["rack.hijack"]
      env["rack.hijack"] = lambda do
        socket = hijack.call
        env.delete("rack.hijack_io")
        socket
      end
      @app.call(env)
    end
  end

  def start_tunnel_server(server, without_hijack_io: false)
    app = ::Rack::Builder.new do
      use WithoutHijackIo if without_hijack_io
      use Outpunch::Rack::Middleware, server: server
      run ->(_env) { [200, {}, ["inner app"]] }
    end.to_app
    port = TCPServer.open("127.0.0.1", 0) { |s| s.addr[1] }
    config = Puma::Configuration.new do |c|
      c.bind "tcp://127.0.0.1:#{port}"
      c.app app
      c.threads 4, 16
      c.quiet
    end
    launcher = Puma::Launcher.new(config)
    thread = Thread.new { launcher.run }
    wait_for_port(port)
    @tunnel_servers ||= []
    @tunnel_servers << [launcher, thread]
    port
  end

  def stop_tunnel_servers
    (@tunnel_servers || []).each do |launcher, thread|
      launcher.stop
      thread.join(2)
    end
  end

  def tunnel_client(port, service:, token:)
    TunnelClient.new(port, service: service, token: token)
  end

  def authed_client(port, service:, token:)
    client = tunnel_client(port, service: service, token: token)
    expect(client.next_message&.fetch("type")).to eq("auth_ok")
    client
  end

  # A raw WebSocket client that authenticates, reads auth_ok, and then never reads again.
  def stalled_client(port, service:, token:)
    socket = TCPSocket.new("127.0.0.1", port)
    socket.write("GET /ws HTTP/1.1\r\nHost: 127.0.0.1:#{port}\r\nUpgrade: websocket\r\n" \
                 "Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n" \
                 "Sec-WebSocket-Version: 13\r\n\r\n")
    nil until socket.gets == "\r\n"
    socket.write(masked_text_frame(JSON.generate(type: "auth", token: token, service: service)))
    _header = socket.read(2)
    expect(JSON.parse(socket.readpartial(1024))["type"]).to eq("auth_ok")
    socket
  end

  def request_thread(server, service, **overrides)
    Thread.new do
      server.handle_request(service: service, method: "GET", path: "test",
                            query: {}, headers: {}, body: "", **overrides)
    rescue StandardError, Timeout::Error => e
      e
    end
  end

  private

  def masked_text_frame(text)
    masked_frame(0x81, text)
  end

  def masked_frame(first_byte, text)
    mask = [1, 2, 3, 4]
    payload = text.bytes.each_with_index.map { |b, i| b ^ mask[i % 4] }
    raise ArgumentError, "short frames only" if payload.size > 125

    ([first_byte, 0x80 | payload.size] + mask + payload).pack("C*")
  end

  public

  def send_ping(socket)
    socket.write(masked_frame(0x89, ""))
  end

  # Polls until the block is true or the limit passes; returns whether it became true.
  def eventually(limit = 5)
    deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + limit
    until yield
      return false if Process.clock_gettime(Process::CLOCK_MONOTONIC) >= deadline

      sleep 0.05
    end
    true
  end

  def wait_for_port(port)
    50.times do
      TCPSocket.new("127.0.0.1", port).close
      return
    rescue Errno::ECONNREFUSED
      sleep 0.02
    end
    raise "puma did not start on #{port}"
  end
end

RSpec.configure do |config|
  config.include TunnelHarness, :tunnel
  config.after(:each, :tunnel) { stop_tunnel_servers }
end
