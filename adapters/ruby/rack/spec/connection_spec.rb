# frozen_string_literal: true

require "spec_helper"

RSpec.describe Outpunch::Rack::Connection do
  let(:conn) { described_class.new(double("native connection")) }

  describe "#url" do
    it "builds a ws:// URL from the rack env" do
      conn.instance_variable_set(:@env, {
        "rack.url_scheme" => "http", "HTTP_HOST" => "example.com", "REQUEST_URI" => "/ws"
      })
      expect(conn.url).to eq("ws://example.com/ws")
    end

    it "builds a wss:// URL for https" do
      conn.instance_variable_set(:@env, {
        "rack.url_scheme" => "https", "HTTP_HOST" => "example.com", "REQUEST_URI" => "/ws"
      })
      expect(conn.url).to eq("wss://example.com/ws")
    end

    it "falls back to localhost when HTTP_HOST is absent" do
      conn.instance_variable_set(:@env, { "rack.url_scheme" => "http", "REQUEST_URI" => "/ws" })
      expect(conn.url).to include("localhost")
    end
  end

  describe "#env" do
    it "returns empty hash before run is called" do
      expect(conn.env).to eq({})
    end
  end

  describe "over a real socket", :tunnel do
    let(:secret) { "test-secret" }
    let(:server) { Outpunch::Rack::Server.new(secret: secret, timeout: 5) }
    let(:port)   { start_tunnel_server(server) }

    it "registers the service after a valid auth" do
      authed_client(port, service: "svc", token: secret)
      expect(server.connected?("svc")).to be true
    end

    it "sends auth_error and closes the socket for a bad token" do
      client = tunnel_client(port, service: "svc", token: "wrong")

      expect(client.next_message&.fetch("type")).to eq("auth_error")
      expect(client.closed?).to be true
      expect(server.connected?("svc")).to be false
    end

    it "unregisters the service when the client disconnects" do
      client = authed_client(port, service: "svc", token: secret)

      client.close
      expect(client.closed?).to be true
      sleep 0.1

      expect(server.connected?("svc")).to be false
    end

    it "relays requests with string query and header values" do
      client = authed_client(port, service: "svc", token: secret)
      thread = request_thread(server, "svc", method: "POST", path: "api/x",
                                             query: { page: 2 }, headers: { "X-Id" => 7 }, body: "data")
      request = client.next_message
      client.respond(request, status: 201, body: "done")

      expect(request).to include("method" => "POST", "path" => "api/x", "body" => "data",
                                 "query" => { "page" => "2" }, "headers" => { "X-Id" => "7" })
      expect(thread.value).to include("status" => 201, "body" => "done")
    end
  end
end
