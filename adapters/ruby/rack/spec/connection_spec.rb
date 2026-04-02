# frozen_string_literal: true

require "spec_helper"

RSpec.describe Outpunch::Rack::Connection do
  let(:secret)  { "test-secret" }
  let(:server)  { Outpunch::Rack::Server.new(secret: secret, timeout: 1) }
  let(:conn)    { described_class.new(server) }

  after { server.reset! }

  # Simulate running the WebSocket driver against a StringIO pipe.
  # Returns [client_io, server_io] where client_io is the side we write WS frames to.
  def pipe
    rd, wr = IO.pipe
    [wr, rd]
  end

  describe "#send_request" do
    it "serialises the payload as JSON with type=request" do
      io = StringIO.new
      driver = instance_double(WebSocket::Driver::Server)
      conn.instance_variable_set(:@driver, driver)
      conn.instance_variable_set(:@write_mutex, Mutex.new)

      captured = nil
      allow(driver).to receive(:text) { |msg| captured = JSON.parse(msg) }

      conn.send_request(
        request_id: "req-1",
        service: "svc",
        method: "GET",
        path: "api/test",
        query: {},
        headers: {},
        body: ""
      )

      expect(captured["type"]).to eq("request")
      expect(captured["request_id"]).to eq("req-1")
      expect(captured["method"]).to eq("GET")
      expect(captured["path"]).to eq("api/test")
    end
  end

  describe "#url" do
    it "builds a ws:// URL from the rack env" do
      env = {
        "rack.url_scheme" => "http",
        "HTTP_HOST"       => "example.com",
        "REQUEST_URI"     => "/ws"
      }
      conn.instance_variable_set(:@env, env)
      expect(conn.url).to eq("ws://example.com/ws")
    end

    it "builds a wss:// URL for https" do
      env = {
        "rack.url_scheme" => "https",
        "HTTP_HOST"       => "example.com",
        "REQUEST_URI"     => "/ws"
      }
      conn.instance_variable_set(:@env, env)
      expect(conn.url).to eq("wss://example.com/ws")
    end

    it "falls back to localhost when HTTP_HOST is absent" do
      env = { "rack.url_scheme" => "http", "REQUEST_URI" => "/ws" }
      conn.instance_variable_set(:@env, env)
      expect(conn.url).to include("localhost")
    end
  end

  describe "#env" do
    it "returns empty hash before run is called" do
      expect(conn.env).to eq({})
    end
  end

  describe "auth handling" do
    let(:driver) { instance_double(WebSocket::Driver::Server) }

    before do
      conn.instance_variable_set(:@driver, driver)
      conn.instance_variable_set(:@write_mutex, Mutex.new)
    end

    it "registers connection and sends auth_ok for valid token" do
      sent = []
      allow(driver).to receive(:text) { |msg| sent << JSON.parse(msg) }

      conn.send(:handle_auth, "token" => secret, "service" => "my-service")

      expect(server.connected?("my-service")).to be true
      expect(sent.last["type"]).to eq("auth_ok")
    end

    it "sends auth_error and closes for invalid token" do
      sent = []
      allow(driver).to receive(:text) { |msg| sent << JSON.parse(msg) }
      allow(driver).to receive(:close)

      conn.send(:handle_auth, "token" => "wrong", "service" => "my-service")

      expect(server.connected?("my-service")).to be false
      expect(sent.last["type"]).to eq("auth_error")
      expect(driver).to have_received(:close)
    end

    it "sends auth_error for blank service name" do
      sent = []
      allow(driver).to receive(:text) { |msg| sent << JSON.parse(msg) }
      allow(driver).to receive(:close)

      conn.send(:handle_auth, "token" => secret, "service" => "")

      expect(sent.last["type"]).to eq("auth_error")
    end
  end

  describe "on_message routing" do
    let(:driver) { instance_double(WebSocket::Driver::Server) }

    before do
      conn.instance_variable_set(:@driver, driver)
      conn.instance_variable_set(:@write_mutex, Mutex.new)
    end

    it "routes auth messages to handle_auth" do
      allow(driver).to receive(:text)
      msg = JSON.generate(type: "auth", token: secret, service: "svc")
      conn.send(:on_message, msg)
      expect(server.connected?("svc")).to be true
    end

    it "routes response messages to server.complete_request" do
      expect(server).to receive(:complete_request).with("req-1", hash_including("status" => 200))
      msg = JSON.generate(type: "response", request_id: "req-1", status: 200)
      conn.send(:on_message, msg)
    end

    it "ignores unknown message types" do
      expect { conn.send(:on_message, JSON.generate(type: "unknown")) }.not_to raise_error
    end

    it "ignores invalid JSON" do
      expect { conn.send(:on_message, "not json") }.not_to raise_error
    end
  end

  describe "on_close" do
    it "unregisters the connection" do
      allow(double).to receive(:text)
      driver = instance_double(WebSocket::Driver::Server)
      allow(driver).to receive(:text)
      allow(driver).to receive(:close)
      conn.instance_variable_set(:@driver, driver)
      conn.instance_variable_set(:@write_mutex, Mutex.new)

      conn.send(:handle_auth, "token" => secret, "service" => "svc")
      expect(server.connected?("svc")).to be true

      conn.send(:on_close)
      expect(server.connected?("svc")).to be false
    end

    it "is safe to call when not authenticated" do
      expect { conn.send(:on_close) }.not_to raise_error
    end
  end
end
