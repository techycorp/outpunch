# frozen_string_literal: true

require "spec_helper"

RSpec.describe Outpunch::TunnelController, type: :request do
  let(:service_name) { "testservice" }
  let(:service_path) { "api/test" }
  let(:conn)         { double("connection", send_request: nil) }

  before do
    OutpunchRails.server.reset!
    Outpunch::Rack::Hooks.clear!
    OutpunchRails.configuration.authorize_service = nil
  end

  after do
    OutpunchRails.server.reset!
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
        OutpunchRails.server.register_connection(service_name, conn)
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
        OutpunchRails.server.register_connection(service_name, conn)
        OutpunchRails.configuration.authorize_service = ->(_svc, _req) { true }
      end

      it "proxies the request and returns the response" do
        allow(conn).to receive(:send_request) do |payload|
          Thread.new do
            sleep 0.01
            OutpunchRails.server.complete_request(payload[:request_id], {
              "status" => 200, "body" => "ok", "headers" => {}
            })
          end
        end

        get "/outpunch/#{service_name}/#{service_path}"

        expect(response.status).to eq(200)
        expect(response.body).to eq("ok")
      end
    end

    context "when connected with no authorize_service set" do
      before { OutpunchRails.server.register_connection(service_name, conn) }

      it "proxies without authorization check" do
        allow(conn).to receive(:send_request) do |payload|
          Thread.new do
            sleep 0.01
            OutpunchRails.server.complete_request(payload[:request_id], {
              "status" => 200, "body" => "ok", "headers" => {}
            })
          end
        end

        get "/outpunch/#{service_name}/#{service_path}"

        expect(response.status).to eq(200)
      end
    end
  end

  describe "HTTP method forwarding" do
    before { OutpunchRails.server.register_connection(service_name, conn) }

    %w[GET POST PUT PATCH DELETE].each do |http_method|
      it "forwards #{http_method}" do
        allow(conn).to receive(:send_request) do |payload|
          expect(payload[:method]).to eq(http_method)
          Thread.new do
            sleep 0.01
            OutpunchRails.server.complete_request(payload[:request_id], {
              "status" => 200, "body" => "ok", "headers" => {}
            })
          end
        end

        send(http_method.downcase.to_sym, "/outpunch/#{service_name}/#{service_path}")

        expect(response.status).to eq(200)
      end
    end
  end

  describe "query parameter forwarding" do
    before { OutpunchRails.server.register_connection(service_name, conn) }

    it "forwards query parameters" do
      allow(conn).to receive(:send_request) do |payload|
        expect(payload[:query]).to include("foo" => "bar")
        Thread.new do
          sleep 0.01
          OutpunchRails.server.complete_request(payload[:request_id], {
            "status" => 200, "body" => "ok", "headers" => {}
          })
        end
      end

      get "/outpunch/#{service_name}/#{service_path}?foo=bar"

      expect(response.status).to eq(200)
    end
  end

  describe "error handling" do
    before { OutpunchRails.server.register_connection(service_name, conn) }

    it "returns 504 on timeout" do
      allow(OutpunchRails).to receive(:handle_request).and_raise(Timeout::Error)

      get "/outpunch/#{service_name}/#{service_path}"

      expect(response.status).to eq(504)
      expect(JSON.parse(response.body)["error"]).to include("timeout")
    end

    it "returns 502 on other errors" do
      allow(OutpunchRails).to receive(:handle_request).and_raise(StandardError.new("boom"))

      get "/outpunch/#{service_name}/#{service_path}"

      expect(response.status).to eq(502)
      expect(JSON.parse(response.body)["error"]).to include("boom")
    end
  end

  describe "binary response handling" do
    before { OutpunchRails.server.register_connection(service_name, conn) }

    it "decodes base64 body" do
      pdf_content = "%PDF-1.4 fake"
      encoded     = Base64.encode64(pdf_content)

      allow(conn).to receive(:send_request) do |payload|
        Thread.new do
          sleep 0.01
          OutpunchRails.server.complete_request(payload[:request_id], {
            "status"        => 200,
            "body"          => encoded,
            "body_encoding" => "base64",
            "headers"       => { "content-type" => "application/pdf" }
          })
        end
      end

      get "/outpunch/#{service_name}/#{service_path}"

      expect(response.status).to eq(200)
      expect(response.body).to eq(pdf_content)
    end

    it "passes content-disposition filename through" do
      encoded = Base64.encode64("data")

      allow(conn).to receive(:send_request) do |payload|
        Thread.new do
          sleep 0.01
          OutpunchRails.server.complete_request(payload[:request_id], {
            "status"        => 200,
            "body"          => encoded,
            "body_encoding" => "base64",
            "headers"       => {
              "content-type"        => "application/pdf",
              "content-disposition" => 'attachment; filename="report.pdf"'
            }
          })
        end
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
      OutpunchRails.server.register_connection(service_name, conn)
      OutpunchRails.configuration.hooks = hook_module
      hook_module.reset!
    end

    after { OutpunchRails.configuration.hooks = Outpunch::Rack::Hooks }

    it "calls before_proxy and after_proxy" do
      allow(conn).to receive(:send_request) do |payload|
        Thread.new do
          sleep 0.01
          OutpunchRails.server.complete_request(payload[:request_id], {
            "status" => 200, "body" => "ok", "headers" => {}
          })
        end
      end

      post "/outpunch/#{service_name}/#{service_path}"

      expect(hook_module.calls[:before].length).to eq(1)
      expect(hook_module.calls[:before].first[:path]).to eq(service_path)
      expect(hook_module.calls[:before].first[:payload]).to include(:request_id, :method, :path)

      expect(hook_module.calls[:after].length).to eq(1)
      expect(hook_module.calls[:after].first[:result]).to include(:status, :body)
    end
  end
end
