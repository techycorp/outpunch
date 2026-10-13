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
      ws = @ws = WebSocket::Client::Simple.connect("ws://127.0.0.1:#{port}/ws")
      ws.on(:open) { ws.send(JSON.generate(type: "auth", token: token, service: service)) }
      ws.on(:message) do |msg|
        case msg.type
        when :text then messages << JSON.parse(msg.data)
        when :close then closed << true
        end
      end
      ws.on(:close) { closed << true }
    end

    def next_message(timeout: 2)
      @messages.pop(timeout: timeout)
    end

    def respond(request, status: 200, body: "ok")
      @ws.send(JSON.generate(type: "response", request_id: request["request_id"],
                             status: status, headers: {}, body: body))
    end

    def close
      @ws.close
    end

    def closed?(timeout: 2)
      !@closed.pop(timeout: timeout).nil?
    end
  end

  def start_tunnel_server(server)
    app = ::Rack::Builder.new do
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

  def request_thread(server, service, **overrides)
    Thread.new do
      server.handle_request(service: service, method: "GET", path: "test",
                            query: {}, headers: {}, body: "", **overrides)
    rescue StandardError, Timeout::Error => e
      e
    end
  end

  private

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
