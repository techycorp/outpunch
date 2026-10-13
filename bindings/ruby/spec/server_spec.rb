# frozen_string_literal: true

require "spec_helper"
require "json"
require "timeout"

RSpec.describe Outpunch::Server do
  let(:server) { described_class.new("test-secret", 5.0, nil) }

  def authed_connection(service)
    conn = server.create_connection
    runner = Thread.new { conn.run }
    conn.push_message(JSON.generate(type: "auth", token: "test-secret", service: service))
    expect(JSON.parse(conn.next_outgoing)["type"]).to eq("auth_ok")
    [conn, runner]
  end

  def request(service, srv = server)
    srv.handle_request(service, "GET", "api/test", { "q" => "1" }, { "x-h" => "v" }, nil)
  end

  it "is not connected before a client authenticates" do
    expect(server.connected?("svc")).to be false
  end

  it "raises NotConnectedError when no client is connected" do
    expect { request("svc") }.to raise_error(Outpunch::NotConnectedError, /not connected/)
  end

  it "relays a request to the client and returns its response" do
    conn, runner = authed_connection("svc")
    expect(server.connected?("svc")).to be true

    client = Thread.new do
      req = JSON.parse(conn.next_outgoing)
      conn.push_message(JSON.generate(type: "response", request_id: req["request_id"],
                                      status: 201, headers: { "a" => "b" }, body: "hi"))
      req
    end

    result = request("svc")
    sent = client.value

    expect(sent).to include("method" => "GET", "path" => "api/test",
                            "query" => { "q" => "1" }, "headers" => { "x-h" => "v" })
    expect(result).to include("status" => 201, "headers" => { "a" => "b" }, "body" => "hi")
  ensure
    conn&.close
    runner&.join
  end

  it "raises a Timeout::Error subclass when the client does not answer" do
    fast = described_class.new("test-secret", 0.1, nil)
    conn = fast.create_connection
    runner = Thread.new { conn.run }
    conn.push_message(JSON.generate(type: "auth", token: "test-secret", service: "svc"))
    conn.next_outgoing

    expect { request("svc", fast) }.to raise_error(Timeout::Error)
  ensure
    conn&.close
    runner&.join
  end

  it "rejects a bad token and ends the connection" do
    conn = server.create_connection
    runner = Thread.new { conn.run }
    conn.push_message(JSON.generate(type: "auth", token: "wrong", service: "svc"))

    expect(JSON.parse(conn.next_outgoing)["type"]).to eq("auth_error")
    expect(runner.join(1)).not_to be_nil
    expect(conn.next_outgoing).to be_nil
  end

  it "lets other Ruby threads run while a request is waiting" do
    conn, runner = authed_connection("svc")
    waiting = Thread.new do
      request("svc")
    rescue StandardError
      nil
    end

    ticks = 0
    5.times do
      sleep 0.01
      ticks += 1
    end

    expect(ticks).to eq(5)
    waiting.kill
    waiting.join
  ensure
    conn&.close
    runner&.join
  end

  it "can be interrupted by Timeout while waiting on a response" do
    conn, runner = authed_connection("svc")

    started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    expect { Timeout.timeout(0.2) { request("svc") } }.to raise_error(Timeout::Error)
    elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - started

    expect(elapsed).to be < 1
  ensure
    conn&.close
    runner&.join
  end

  it "can kill a thread blocked in next_outgoing" do
    conn = server.create_connection
    reader = Thread.new { conn.next_outgoing }
    sleep 0.05

    reader.kill
    expect(reader.join(1)).not_to be_nil
  end

  it "closes the connection when the thread running it is killed" do
    _conn, runner = authed_connection("svc")

    runner.kill
    expect(runner.join(1)).not_to be_nil
    expect(server.connected?("svc")).to be false
  end
end
