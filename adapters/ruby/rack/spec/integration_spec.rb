# frozen_string_literal: true

require "spec_helper"
require "rack"
require "puma"
require "puma/configuration"
require "websocket-client-simple"
require "socket"
require "json"

# Full round-trip integration test: Rack server + WebSocket client + HTTP request.
#
# Starts a real Puma server with the outpunch-rack middleware, connects a real
# WebSocket client, authenticates, then sends a fake tunnel request and verifies
# the response comes back correctly.

RSpec.describe "integration: full tunnel round-trip", :integration do
  SECRET = "integration-secret"
  PORT   = 19876

  # Minimal Rack app that always returns 200.
  INNER_APP = ->(_env) { [200, { "Content-Type" => "text/plain" }, ["inner app"]] }

  let(:server_instance) do
    Outpunch::Rack::Server.new(secret: SECRET, timeout: 5)
  end

  let(:rack_app) do
    srv = server_instance
    ::Rack::Builder.new do
      use Outpunch::Rack::Middleware, server: srv
      run INNER_APP
    end.to_app
  end

  around do |example|
    # Start Puma in a background thread
    puma_config = Puma::Configuration.new do |c|
      c.bind "tcp://127.0.0.1:#{PORT}"
      c.app rack_app
      c.quiet
    end
    launcher = Puma::Launcher.new(puma_config)
    thread = Thread.new { launcher.run }
    sleep 0.3 # wait for server to bind

    example.run
  ensure
    launcher.stop rescue nil
    thread.join(2) rescue nil
  end

  it "authenticates and proxies a tunnel request" do
    received_request = nil
    ws_done = Queue.new

    ws = WebSocket::Client::Simple.connect("ws://127.0.0.1:#{PORT}/ws")

    ws.on(:open) do
      ws.send(JSON.generate(type: "auth", token: SECRET, service: "test-service"))
    end

    ws.on(:message) do |msg|
      data = JSON.parse(msg.data)
      case data["type"]
      when "auth_ok"
        # Auth succeeded — server will now route requests to us
      when "request"
        received_request = data
        ws.send(JSON.generate(
          type: "response",
          request_id: data["request_id"],
          status: 200,
          headers: {},
          body: "hello from tunnel"
        ))
        ws_done.push(:done)
      end
    end

    # Give WS time to connect and auth
    sleep 0.2

    # Make a tunnel request from the server side
    result = server_instance.handle_request(
      service: "test-service",
      method: "GET",
      path: "api/hello",
      query: {},
      headers: {},
      body: ""
    )

    ws_done.pop
    ws.close

    expect(result["status"]).to eq(200)
    expect(result["body"]).to eq("hello from tunnel")
    expect(received_request["method"]).to eq("GET")
    expect(received_request["path"]).to eq("api/hello")
  end

  it "returns 502 when no client connected" do
    expect {
      server_instance.handle_request(
        service: "offline-service",
        method: "GET",
        path: "test",
        query: {},
        headers: {},
        body: ""
      )
    }.to raise_error(RuntimeError, /not connected/)
  end
end
