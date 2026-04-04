# frozen_string_literal: true

require "spec_helper"

RSpec.describe Outpunch::ClientConfig do
  describe ".new" do
    it "creates a config with required fields" do
      config = Outpunch::ClientConfig.new(
        "wss://example.com/ws",
        "my-secret",
        "my-service",
        nil,
        nil,
        nil
      )
      expect(config.server_url).to eq("wss://example.com/ws")
      expect(config.secret).to eq("my-secret")
      expect(config.service).to eq("my-service")
    end

    it "uses default forward_to when nil" do
      config = Outpunch::ClientConfig.new("wss://example.com/ws", "s", "svc", nil, nil, nil)
      expect(config.forward_to).to eq("http://localhost:8080")
    end

    it "uses default reconnect_delay when nil" do
      config = Outpunch::ClientConfig.new("wss://example.com/ws", "s", "svc", nil, nil, nil)
      expect(config.reconnect_delay).to eq(5.0)
    end

    it "uses default request_timeout when nil" do
      config = Outpunch::ClientConfig.new("wss://example.com/ws", "s", "svc", nil, nil, nil)
      expect(config.request_timeout).to eq(25.0)
    end

    it "accepts custom values" do
      config = Outpunch::ClientConfig.new(
        "wss://example.com/ws",
        "secret",
        "stormsnap",
        "http://localhost:8081",
        10.0,
        30.0
      )
      expect(config.forward_to).to eq("http://localhost:8081")
      expect(config.reconnect_delay).to eq(10.0)
      expect(config.request_timeout).to eq(30.0)
    end
  end

  describe "attribute accessors" do
    let(:config) do
      Outpunch::ClientConfig.new("wss://example.com/ws", "secret", "svc", nil, nil, nil)
    end

    it "allows setting server_url" do
      config.server_url = "wss://other.com/ws"
      expect(config.server_url).to eq("wss://other.com/ws")
    end

    it "allows setting service" do
      config.service = "newservice"
      expect(config.service).to eq("newservice")
    end

    it "allows setting forward_to" do
      config.forward_to = "http://localhost:9090"
      expect(config.forward_to).to eq("http://localhost:9090")
    end

    it "allows setting reconnect_delay" do
      config.reconnect_delay = 15.0
      expect(config.reconnect_delay).to eq(15.0)
    end
  end

  describe "#inspect" do
    it "includes server_url, service, and forward_to" do
      config = Outpunch::ClientConfig.new(
        "wss://example.com/ws", "secret", "stormsnap",
        "http://localhost:8081", nil, nil
      )
      str = config.inspect
      expect(str).to include("wss://example.com/ws")
      expect(str).to include("stormsnap")
      expect(str).to include("http://localhost:8081")
    end

    it "does not include secret" do
      config = Outpunch::ClientConfig.new(
        "wss://example.com/ws", "supersecret", "svc", nil, nil, nil
      )
      expect(config.inspect).not_to include("supersecret")
    end
  end
end
