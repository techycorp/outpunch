# frozen_string_literal: true

require "spec_helper"

RSpec.describe Outpunch::TunnelController, type: :request do
  let(:service_name) { "testservice" }
  let(:service_path) { "api/test" }

  # Stubs the tunnel as connected; the block receives handle_request's kwargs and
  # returns the client's response data.
  def connect_tunnel(&responder)
    allow(OutpunchRails).to receive(:connected?).with(service_name).and_return(true)
    responder ||= ->(_kwargs) { { "status" => 200, "body" => "ok", "headers" => {} } }
    allow(OutpunchRails).to receive(:handle_request) { |**kwargs| responder.call(kwargs) }
  end

  before do
    Outpunch::Rack::Hooks.clear!
    OutpunchRails.configuration.authorize_service = nil
  end

  after do
    Outpunch::Rack::Hooks.clear!
    OutpunchRails.configuration.authorize_service = nil
  end

  describe "GET /outpunch/:service_name/*service_path" do
    context "when tunnel is not connected" do
      it "returns 502" do
        get "/outpunch/#{service_name}/#{service_path}"

        expect(response.status).to eq(502)
        expect(JSON.parse(response.body)["error"]).to include("offline")
      end
    end

    context "when authorize_service blocks the request" do
      before do
        connect_tunnel
        OutpunchRails.configuration.authorize_service = ->(_svc, _req) { false }
      end

      it "returns 403" do
        get "/outpunch/#{service_name}/#{service_path}"

        expect(response.status).to eq(403)
        expect(JSON.parse(response.body)["error"]).to include("not allowed")
      end
    end

    context "when authorize_service allows the request" do
      before do
        connect_tunnel
        OutpunchRails.configuration.authorize_service = ->(_svc, _req) { true }
      end

      it "proxies the request and returns the response" do
        get "/outpunch/#{service_name}/#{service_path}"

        expect(response.status).to eq(200)
        expect(response.body).to eq("ok")
      end
    end

    context "when connected with no authorize_service set" do
      before { connect_tunnel }

      it "proxies without authorization check" do
        get "/outpunch/#{service_name}/#{service_path}"

        expect(response.status).to eq(200)
      end
    end
  end

  describe "HTTP method forwarding" do
    %w[GET POST PUT PATCH DELETE].each do |http_method|
      it "forwards #{http_method}" do
        forwarded = nil
        connect_tunnel do |kwargs|
          forwarded = kwargs[:method]
          { "status" => 200, "body" => "ok", "headers" => {} }
        end

        send(http_method.downcase.to_sym, "/outpunch/#{service_name}/#{service_path}")

        expect(response.status).to eq(200)
        expect(forwarded).to eq(http_method)
      end
    end
  end

  describe "query parameter forwarding" do
    it "forwards query parameters" do
      forwarded = nil
      connect_tunnel do |kwargs|
        forwarded = kwargs[:query]
        { "status" => 200, "body" => "ok", "headers" => {} }
      end

      get "/outpunch/#{service_name}/#{service_path}?foo=bar"

      expect(response.status).to eq(200)
      expect(forwarded).to include("foo" => "bar")
    end
  end

  describe "error handling" do
    before { connect_tunnel }

    it "returns 504 on timeout" do
      allow(OutpunchRails).to receive(:handle_request).and_raise(Timeout::Error)

      get "/outpunch/#{service_name}/#{service_path}"

      expect(response.status).to eq(504)
      expect(JSON.parse(response.body)["error"]).to include("timeout")
    end

    it "returns 504 on a native tunnel timeout" do
      allow(OutpunchRails).to receive(:handle_request).and_raise(Outpunch::TunnelTimeoutError)

      get "/outpunch/#{service_name}/#{service_path}"

      expect(response.status).to eq(504)
    end

    it "returns 502 on other errors" do
      allow(OutpunchRails).to receive(:handle_request).and_raise(StandardError.new("boom"))

      get "/outpunch/#{service_name}/#{service_path}"

      expect(response.status).to eq(502)
      expect(JSON.parse(response.body)["error"]).to include("boom")
    end
  end

  describe "binary response handling" do
    it "decodes base64 body" do
      pdf_content = "%PDF-1.4 fake"
      encoded     = Base64.encode64(pdf_content)
      connect_tunnel do |_kwargs|
        {
          "status"        => 200,
          "body"          => encoded,
          "body_encoding" => "base64",
          "headers"       => { "content-type" => "application/pdf" }
        }
      end

      get "/outpunch/#{service_name}/#{service_path}"

      expect(response.status).to eq(200)
      expect(response.body).to eq(pdf_content)
    end

    it "passes content-disposition filename through" do
      encoded = Base64.encode64("data")
      connect_tunnel do |_kwargs|
        {
          "status"        => 200,
          "body"          => encoded,
          "body_encoding" => "base64",
          "headers"       => {
            "content-type"        => "application/pdf",
            "content-disposition" => 'attachment; filename="report.pdf"'
          }
        }
      end

      get "/outpunch/#{service_name}/#{service_path}"

      expect(response.status).to eq(200)
      expect(response.headers["Content-Disposition"]).to include("report.pdf")
    end
  end

  describe "hooks" do
    let(:hook_module) do
      Module.new do
        def self.calls; @calls ||= { before: [], after: [] }; end
        def self.reset!; @calls = { before: [], after: [] }; end

        def self.before_proxy(service_name:, path:, payload:, request:)
          calls[:before] << { path: path, payload: payload }
        end

        def self.after_proxy(service_name:, path:, payload:, result:, request:)
          calls[:after] << { path: path, result: result }
        end
      end
    end

    before do
      connect_tunnel
      OutpunchRails.configuration.hooks = hook_module
      hook_module.reset!
    end

    after { OutpunchRails.configuration.hooks = Outpunch::Rack::Hooks }

    it "calls before_proxy and after_proxy" do
      post "/outpunch/#{service_name}/#{service_path}"

      expect(hook_module.calls[:before].length).to eq(1)
      expect(hook_module.calls[:before].first[:path]).to eq(service_path)
      expect(hook_module.calls[:before].first[:payload]).to include(:request_id, :method, :path)

      expect(hook_module.calls[:after].length).to eq(1)
      expect(hook_module.calls[:after].first[:result]).to include(:status, :body)
    end
  end

  describe "a before_proxy hook that raises HookHalt" do
    before { connect_tunnel }
    after  { OutpunchRails.configuration.hooks = Outpunch::Rack::Hooks }

    it "short-circuits with the hook's status + body and never forwards upstream" do
      OutpunchRails.configuration.hooks = Module.new do
        def self.before_proxy(service_name:, path:, payload:, request:)
          raise Outpunch::HookHalt.new(402, { error: "payment required" })
        end

        def self.after_proxy(**); end
      end

      get "/outpunch/#{service_name}/#{service_path}"

      expect(response.status).to eq(402)
      expect(JSON.parse(response.body)).to eq("error" => "payment required")
      expect(OutpunchRails).not_to have_received(:handle_request) # upstream never reached
    end

    it "preserves catch-all precedence: a plain StandardError from a hook still yields 502" do
      OutpunchRails.configuration.hooks = Module.new do
        def self.before_proxy(service_name:, path:, payload:, request:)
          raise StandardError, "hook boom"
        end

        def self.after_proxy(**); end
      end

      get "/outpunch/#{service_name}/#{service_path}"

      expect(response.status).to eq(502)
      expect(JSON.parse(response.body)["error"]).to include("hook boom")
    end
  end
end
