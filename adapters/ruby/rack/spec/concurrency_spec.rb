# frozen_string_literal: true

require "spec_helper"

RSpec.describe "connection lifecycle and concurrency", :tunnel do
  let(:secret) { "test-secret" }
  let(:server) { Outpunch::Rack::Server.new(secret: secret, timeout: 5) }
  let(:port)   { start_tunnel_server(server) }

  it "does not let a stale disconnect remove a newer connection" do
    old = authed_client(port, service: "svc", token: secret)
    authed_client(port, service: "svc", token: secret)

    old.close
    expect(old.closed?).to be true
    sleep 0.1

    expect(server.connected?("svc")).to be true
  end

  it "fails a pending request promptly when its client disconnects" do
    client = authed_client(port, service: "svc", token: secret)
    thread = request_thread(server, "svc")
    expect(client.next_message&.fetch("type")).to eq("request")

    client.close

    expect(thread.join(1)).not_to be_nil, "request should fail fast, not wait for the timeout"
  ensure
    thread&.kill
  end

  it "does not let one client answer another service's request" do
    victim = authed_client(port, service: "victim", token: secret)
    attacker = authed_client(port, service: "attacker", token: secret)
    thread = request_thread(server, "victim")
    request = victim.next_message

    attacker.respond(request, status: 418)
    sleep 0.05
    victim.respond(request, status: 200)

    expect(thread.value["status"]).to eq(200)
  end
end
