# frozen_string_literal: true

require "spec_helper"

RSpec.describe Outpunch::Rack::Server do
  let(:secret) { "test-secret-123" }
  let(:server) { described_class.new(secret: secret, timeout: 1) }
  let(:service_name) { "testservice" }

  describe ".new" do
    it "rejects a missing secret" do
      expect { described_class.new(secret: nil) }.to raise_error(ArgumentError, /secret/)
    end

    it "rejects an empty secret" do
      expect { described_class.new(secret: "") }.to raise_error(ArgumentError, /secret/)
    end
  end

  describe "#connected?" do
    it "starts disconnected" do
      expect(server.connected?(service_name)).to be false
    end
  end

  describe "#handle_request" do
    it "raises a RuntimeError when service not connected" do
      expect {
        server.handle_request(service: service_name, method: "GET", path: "test",
                              query: {}, headers: {}, body: "")
      }.to raise_error(RuntimeError, /not connected/)
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
