# frozen_string_literal: true

require "spec_helper"
require "rack/mock"

RSpec.describe Outpunch::Rack::Middleware do
  let(:inner_app) { ->(_env) { [200, {}, ["ok"]] } }
  let(:server)    { instance_double(Outpunch::Rack::Server) }
  let(:middleware) { described_class.new(inner_app, server: server) }

  def ws_env(path = "/ws")
    ::Rack::MockRequest.env_for(path).merge(
      "HTTP_UPGRADE"    => "websocket",
      "HTTP_CONNECTION" => "Upgrade",
      "rack.hijack"     => -> { },
      "rack.hijack_io"  => StringIO.new
    )
  end

  describe "#call" do
    context "with a WebSocket upgrade request to /ws" do
      it "hijacks the connection and returns -1" do
        conn = instance_double(Outpunch::Rack::Connection)
        allow(server).to receive(:create_connection).and_return(conn)
        allow(conn).to receive(:run)

        env = ws_env
        hijacked = false
        env["rack.hijack"] = -> { hijacked = true }

        status, = middleware.call(env)

        expect(hijacked).to be true
        expect(status).to eq(-1)
      end

      it "spawns a thread that calls run on the connection" do
        conn = instance_double(Outpunch::Rack::Connection)
        allow(server).to receive(:create_connection).and_return(conn)
        started = Queue.new
        allow(conn).to receive(:run) { started.push(:started) }

        middleware.call(ws_env)
        started.pop

        expect(conn).to have_received(:run)
      end
    end

    context "with a non-WebSocket request" do
      it "passes through to the inner app" do
        env = ::Rack::MockRequest.env_for("/api/health")
        status, _, body = middleware.call(env)

        expect(status).to eq(200)
        expect(body).to eq(["ok"])
      end
    end

    context "with a WebSocket request to a different path" do
      it "passes through to the inner app" do
        env = ws_env("/other-ws")
        status, _, body = middleware.call(env)

        expect(status).to eq(200)
        expect(body).to eq(["ok"])
      end
    end
  end
end
