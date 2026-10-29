# frozen_string_literal: true

require "spec_helper"
require "json"
require "timeout"

# Ruby interrupts a thread for reasons that raise nothing: the VM timer when threads compete
# for the GVL, Thread#wakeup, signal handlers. rb_thread_call_without_gvl2 then returns without
# calling the native function at all (thread.c: "Check interrupt and return if interrupted").
# None of that may cancel, skip or drop work; only an exception (Thread#raise, Thread#kill,
# Timeout) may.
RSpec.describe "native calls interrupted without an exception" do
  let(:server) { Outpunch::Server.new("test-secret", 5.0, nil) }
  let(:auth) { JSON.generate(type: "auth", token: "test-secret", service: "svc") }

  def within(seconds = 2, &block)
    Timeout.timeout(seconds, &block)
  end

  def connect(srv = server, service: "svc")
    conn = srv.create_connection
    runner = Thread.new { conn.run }
    conn.push_message(JSON.generate(type: "auth", token: "test-secret", service: service))
    [conn, runner, within { JSON.parse(conn.next_outgoing)["type"] }]
  end

  # Four threads calling in a tight loop make the VM timer fire between calls.
  def compete_for_the_gvl(seconds: 1.5, threads: 4)
    Array.new(threads) do
      Thread.new do
        deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + seconds
        results = []
        results << yield while Process.clock_gettime(Process::CLOCK_MONOTONIC) < deadline
        results
      end
    end.flat_map(&:value)
  end

  context "when threads compete for the GVL" do
    it "never skips a call" do
      outcomes = compete_for_the_gvl do
        server.handle_request("nobody", "GET", "x", {}, {}, nil)
      rescue StandardError => e
        e.class
      end

      expect(outcomes.tally.keys).to eq([Outpunch::NotConnectedError])
    end

    it "delivers every client's auth" do
      replies = compete_for_the_gvl(seconds: 1) do
        conn, runner, reply = connect(Outpunch::Server.new("test-secret", 5.0, nil))
        conn.close
        runner.join(2)
        reply
      rescue Timeout::Error
        "no reply"
      end

      expect(replies.tally.keys).to eq(["auth_ok"])
    end
  end

  context "when a thread is woken" do
    it "keeps its connection running" do
      conn, runner, = connect

      runner.wakeup
      sleep 0.2

      expect(runner.alive?).to be true
      expect(server.connected?("svc")).to be true
    ensure
      conn&.close
      runner&.join(2)
    end

    it "keeps waiting for its request's response" do
      conn, runner, = connect
      caller = Thread.new do
        server.handle_request("svc", "GET", "x", {}, {}, nil)
      rescue StandardError => e
        e
      end
      request = within { JSON.parse(conn.next_outgoing) }

      caller.wakeup
      sleep 0.1
      conn.push_message(JSON.generate(type: "response", request_id: request["request_id"],
                                      status: 200, headers: {}, body: "ok"))

      expect(within { caller.value }).to include("status" => 200, "body" => "ok")
    ensure
      conn&.close
      runner&.join(2)
    end

    it "keeps a push that is waiting for room in the queue" do
      conn = server.create_connection
      conn.push_message(auth)
      63.times { conn.push_message(JSON.generate(type: "pong")) }
      pusher = Thread.new { conn.push_message(JSON.generate(type: "pong")) }
      sleep 0.1

      pusher.wakeup
      sleep 0.1

      expect(pusher.join(0)).to be_nil
    ensure
      runner = Thread.new { conn.run }
      within { conn.next_outgoing }
      pusher&.join(2)
      conn&.close
      runner&.join(2)
    end
  end

  context "when an exception is deferred by Thread.handle_interrupt" do
    it "still delivers a pushed message" do
      conn = server.create_connection
      runner = Thread.new { conn.run }

      expect do
        Thread.handle_interrupt(RuntimeError => :never) do
          Thread.current.raise(RuntimeError, "deferred")
          within { conn.push_message(auth) }
        end
      end.to raise_error(RuntimeError, "deferred")

      expect(within { JSON.parse(conn.next_outgoing) }["type"]).to eq("auth_ok")
    ensure
      conn&.close
      runner&.join(2)
    end
  end

  context "when a thread is interrupted with an exception" do
    it "stops a push that is waiting for room in the queue" do
      conn = server.create_connection
      64.times { conn.push_message(JSON.generate(type: "pong")) }
      pusher = Thread.new { conn.push_message(JSON.generate(type: "pong")) }
      sleep 0.1

      pusher.raise(RuntimeError, "stop")

      expect { within { pusher.join } }.to raise_error(RuntimeError, "stop")
    ensure
      conn&.close
    end
  end
end
