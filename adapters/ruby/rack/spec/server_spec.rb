# frozen_string_literal: true

require "spec_helper"

RSpec.describe Outpunch::Rack::Server do
  let(:secret) { "test-secret-123" }
  let(:server) { described_class.new(secret: secret, timeout: 1) }
  let(:service_name) { "testservice" }
  let(:conn) { double("connection", send_request: nil) }

  after { server.reset! }

  describe "#valid_token?" do
    it "returns true for matching token" do
      expect(server.valid_token?(secret)).to be true
    end

    it "returns false for non-matching token" do
      expect(server.valid_token?("wrong")).to be false
    end

    it "returns false for nil" do
      expect(server.valid_token?(nil)).to be false
    end

    it "returns false for empty string" do
      expect(server.valid_token?("")).to be false
    end

    it "returns false when secret is nil" do
      s = described_class.new(secret: nil, timeout: 1)
      expect(s.valid_token?("anything")).to be false
    end

    it "returns false when lengths differ" do
      expect(server.valid_token?("short")).to be false
    end
  end

  describe "connection management" do
    it "starts disconnected" do
      expect(server.connected?(service_name)).to be false
    end

    it "is connected after registration" do
      server.register_connection(service_name, conn)
      expect(server.connected?(service_name)).to be true
    end

    it "is disconnected after unregistration" do
      server.register_connection(service_name, conn)
      server.unregister_connection(service_name)
      expect(server.connected?(service_name)).to be false
    end

    it "handles multiple services independently" do
      other = double("other_conn")
      server.register_connection("svc1", conn)
      server.register_connection("svc2", other)

      expect(server.connected?("svc1")).to be true
      expect(server.connected?("svc2")).to be true

      server.unregister_connection("svc1")

      expect(server.connected?("svc1")).to be false
      expect(server.connected?("svc2")).to be true
    end
  end

  describe "#handle_request" do
    it "raises when service not connected" do
      expect {
        server.handle_request(service: service_name, method: "GET", path: "test",
                              query: {}, headers: {}, body: "")
      }.to raise_error(RuntimeError, /not connected/)
    end

    it "sends request through connection and returns response" do
      server.register_connection(service_name, conn)

      expect(conn).to receive(:send_request) do |payload|
        Thread.new do
          sleep 0.01
          server.complete_request(payload[:request_id], { "status" => 200, "body" => "ok" })
        end
      end

      result = server.handle_request(
        service: service_name, method: "POST", path: "api/test",
        query: {}, headers: {}, body: ""
      )
      expect(result).to eq({ "status" => 200, "body" => "ok" })
    end

    it "times out if no response arrives" do
      server.register_connection(service_name, conn)
      allow(conn).to receive(:send_request)

      expect {
        server.handle_request(service: service_name, method: "GET", path: "test",
                              query: {}, headers: {}, body: "")
      }.to raise_error(Timeout::Error)
    end

    it "cleans up pending request after timeout" do
      server.register_connection(service_name, conn)
      allow(conn).to receive(:send_request)

      begin
        server.handle_request(service: service_name, method: "GET", path: "test",
                              query: {}, headers: {}, body: "")
      rescue Timeout::Error
        nil
      end

      # Internal pending map should be empty
      expect(server.instance_variable_get(:@pending_requests).size).to eq(0)
    end
  end

  describe "#complete_request" do
    it "does nothing for unknown request_id" do
      expect { server.complete_request("unknown", { "status" => 200 }) }.not_to raise_error
    end
  end

  describe "#success_response" do
    it "builds response with defaults" do
      result = server.success_response({})
      expect(result[:status]).to eq(200)
      expect(result[:body]).to be_nil
      expect(result[:headers]).to eq({})
    end

    it "normalizes header keys to lowercase" do
      result = server.success_response(
        "status" => 200,
        "headers" => { "Content-Type" => "application/pdf" }
      )
      expect(result[:headers]).to have_key("content-type")
      expect(result[:headers]).not_to have_key("Content-Type")
    end

    it "decodes base64 body" do
      original = "Hello, World!"
      encoded = Base64.encode64(original)

      result = server.success_response(
        "status" => 200,
        "body" => encoded,
        "body_encoding" => "base64"
      )

      expect(result[:body]).to eq(original)
      expect(result[:body].encoding.name).to eq("ASCII-8BIT")
    end
  end

  describe "#error_response" do
    it "builds JSON error body" do
      result = server.error_response(502, "Gateway Error")
      expect(result[:status]).to eq(502)
      expect(JSON.parse(result[:body])).to eq({ "error" => "Gateway Error" })
      expect(result[:headers]["Content-Type"]).to eq("application/json")
    end
  end

  describe "#extract_proxy_headers" do
    it "extracts and transforms HTTP_ headers" do
      headers = {
        "HTTP_AUTHORIZATION" => "Bearer token",
        "HTTP_CONTENT_TYPE"  => "application/json",
        "HTTP_HOST"          => "example.com",
        "HTTP_CONNECTION"    => "keep-alive",
        "HTTP_UPGRADE"       => "websocket",
        "OTHER"              => "ignored"
      }

      result = server.extract_proxy_headers(headers)

      expect(result).to eq(
        "AUTHORIZATION" => "Bearer token",
        "CONTENT-TYPE"  => "application/json"
      )
    end
  end
end
