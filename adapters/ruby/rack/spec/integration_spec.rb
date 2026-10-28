# frozen_string_literal: true

require "spec_helper"

# Full round-trip: real Puma server with the outpunch-ruby-rack middleware and a real WebSocket client.
RSpec.describe "integration: full tunnel round-trip", :integration, :tunnel do
  let(:server_instance) { Outpunch::Rack::Server.new(secret: "integration-secret", timeout: 5) }
  let!(:port) { start_tunnel_server(server_instance) }

  it "authenticates and proxies a tunnel request" do
    client = authed_client(port, service: "test-service", token: "integration-secret")
    caller = request_thread(server_instance, "test-service", path: "api/hello")

    request = client.next_message
    expect(request["type"]).to eq("request")
    expect(request["method"]).to eq("GET")
    expect(request["path"]).to eq("api/hello")

    client.respond(request, status: 200, body: "hello from tunnel")
    result = caller.value
    client.close

    expect(result["status"]).to eq(200)
    expect(result["body"]).to eq("hello from tunnel")
  end

  it "returns 502 when no client connected" do
    expect {
      server_instance.handle_request(service: "offline-service", method: "GET", path: "test",
                                     query: {}, headers: {}, body: "")
    }.to raise_error(RuntimeError, /not connected/)
  end
end
