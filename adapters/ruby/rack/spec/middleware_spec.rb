# frozen_string_literal: true

require "spec_helper"
require "rack/mock"
require "rack/lint"
require "socket"

RSpec.describe Outpunch::Rack::Middleware do
  let(:inner_app) { ->(_env) { [200, {}, ["ok"]] } }
  let(:server)    { instance_double(Outpunch::Rack::Server) }
  let(:middleware) { described_class.new(inner_app, server: server) }

  # A connection double whose #run reports what it was given, from the middleware's thread.
  def connection_receiving_run
    conn = instance_double(Outpunch::Rack::Connection)
    allow(server).to receive(:create_connection).and_return(conn)
    runs = Queue.new
    allow(conn).to receive(:run) { |*args| runs.push(args) }
    [conn, runs]
  end

  describe "#call" do
    context "with a WebSocket upgrade request to /ws" do
      it "hijacks the connection" do
        _conn, runs = connection_receiving_run
        hijacked = false

        middleware.call(websocket_upgrade_env(on_hijack: -> { hijacked = true }))
        runs.pop

        expect(hijacked).to be true
      end

      # The server ignores the response after a full hijack, but Rack 3 still requires a valid one.
      it "answers with a response Rack 3 accepts" do
        _conn, runs = connection_receiving_run
        socket, _peer = UNIXSocket.pair

        expect { ::Rack::Lint.new(middleware).call(websocket_upgrade_env(socket: socket)) }.not_to raise_error
        runs.pop
      end

      it "runs the connection on the socket the hijack returned" do
        _conn, runs = connection_receiving_run
        socket = StringIO.new
        env = websocket_upgrade_env(socket: socket)

        middleware.call(env)

        expect(runs.pop).to eq([env, socket])
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
        status, _, body = middleware.call(websocket_upgrade_env("/other-ws"))

        expect(status).to eq(200)
        expect(body).to eq(["ok"])
      end
    end
  end
end
