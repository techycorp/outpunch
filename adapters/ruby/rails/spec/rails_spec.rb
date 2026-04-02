# frozen_string_literal: true

require "spec_helper"

RSpec.describe OutpunchRails do
  after do
    Outpunch::Rack.instance_variable_set(:@configuration, nil)
    Outpunch::Rack.instance_variable_set(:@server, nil)
  end

  describe ".configure" do
    it "sets secret and timeout on the underlying rack configuration" do
      OutpunchRails.configure do |c|
        c.secret  = "my-secret"
        c.timeout = 45
      end

      expect(Outpunch::Rack.configuration.secret).to eq("my-secret")
      expect(Outpunch::Rack.configuration.timeout).to eq(45)
    end

    it "sets base_controller" do
      OutpunchRails.configure { |c| c.base_controller = "ActionController::API" }
      expect(OutpunchRails.configuration.base_controller).to eq("ActionController::API")
    end

    it "sets authorize_service" do
      auth = ->(svc, req) { svc == "allowed" }
      OutpunchRails.configure { |c| c.authorize_service = auth }
      expect(OutpunchRails.configuration.authorize_service).to be(auth)
    end

    it "sets hooks" do
      custom_hooks = Module.new
      OutpunchRails.configure { |c| c.hooks = custom_hooks }
      expect(OutpunchRails.configuration.hooks).to be(custom_hooks)
    end
  end

  describe ".configuration" do
    it "returns the underlying rack configuration" do
      expect(OutpunchRails.configuration).to be_a(Outpunch::Rack::Configuration)
    end
  end

  describe ".hooks" do
    it "defaults to Outpunch::Rack::Hooks" do
      expect(OutpunchRails.hooks).to be(Outpunch::Rack::Hooks)
    end

    it "returns whatever hooks is configured to" do
      custom = Module.new
      OutpunchRails.configure { |c| c.hooks = custom }
      expect(OutpunchRails.hooks).to be(custom)
    end
  end

  describe ".server" do
    it "returns an Outpunch::Rack::Server" do
      OutpunchRails.configure { |c| c.secret = "s" }
      expect(OutpunchRails.server).to be_a(Outpunch::Rack::Server)
    end

    it "returns the same server instance each call" do
      OutpunchRails.configure { |c| c.secret = "s" }
      expect(OutpunchRails.server).to be(OutpunchRails.server)
    end
  end

  describe ".connected?" do
    it "delegates to the server" do
      OutpunchRails.configure { |c| c.secret = "s" }
      expect(OutpunchRails.connected?("svc")).to be false
    end
  end

  describe ".handle_request" do
    it "raises when service not connected" do
      OutpunchRails.configure { |c| c.secret = "s" }
      expect {
        OutpunchRails.handle_request(service: "svc", method: "GET", path: "test",
                                     query: {}, headers: {}, body: "")
      }.to raise_error(RuntimeError, /not connected/)
    end
  end

  describe ".success_response" do
    it "delegates to the server and normalizes headers" do
      OutpunchRails.configure { |c| c.secret = "s" }
      result = OutpunchRails.success_response("status" => 201, "headers" => { "Content-Type" => "text/plain" })
      expect(result[:status]).to eq(201)
      expect(result[:headers]).to have_key("content-type")
    end
  end

  describe ".error_response" do
    it "delegates to the server" do
      OutpunchRails.configure { |c| c.secret = "s" }
      result = OutpunchRails.error_response(502, "offline")
      expect(result[:status]).to eq(502)
    end
  end

  describe ".extract_proxy_headers" do
    it "delegates to the server and strips hop-by-hop headers" do
      OutpunchRails.configure { |c| c.secret = "s" }
      headers = { "HTTP_AUTHORIZATION" => "Bearer t", "HTTP_HOST" => "x" }
      result = OutpunchRails.extract_proxy_headers(headers)
      expect(result).to have_key("AUTHORIZATION")
      expect(result).not_to have_key("HOST")
    end
  end

  describe "Configuration defaults" do
    it "defaults timeout to 25" do
      expect(Outpunch::Rack::Configuration.new.timeout).to eq(25)
    end

    it "defaults base_controller to ActionController::API" do
      expect(Outpunch::Rack::Configuration.new.base_controller).to eq("ActionController::API")
    end

    it "defaults hooks to Outpunch::Rack::Hooks" do
      expect(Outpunch::Rack::Configuration.new.hooks).to be(Outpunch::Rack::Hooks)
    end

    it "defaults authorize_service to nil" do
      expect(Outpunch::Rack::Configuration.new.authorize_service).to be_nil
    end
  end
end
