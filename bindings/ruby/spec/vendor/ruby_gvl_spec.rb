# frozen_string_literal: true

require "spec_helper"
require "rbconfig"
require "tmpdir"

# Facts about Ruby's C API for calls made without the GVL, observed on the running Ruby through a
# small C probe (spec/vendor/support/gvl_probe.c), not through the outpunch binding. They are not
# tests of our code: when one fails, Ruby changed or we misunderstood it. Run with
# `just test-vendor`; needs a C compiler.
#
# What depends on them: without_gvl in ext/outpunch/src/gvl.rs (retry when the function did not
# run, cancel through the unblocking function, then rb_thread_check_ints) and the model of it in
# spec/RubyGvl.tla (assumptions A1-A3).
RSpec.describe "Ruby", :vendor do
  before(:all) do
    @probe_dir = Dir.mktmpdir("gvl_probe")
    FileUtils.cp(File.expand_path("support/gvl_probe.c", __dir__), @probe_dir)
    File.write(File.join(@probe_dir, "extconf.rb"), %(require "mkmf"\ncreate_makefile("gvl_probe")\n))
    Dir.chdir(@probe_dir) do
      system(RbConfig.ruby, "extconf.rb", out: File::NULL, exception: true)
      system("make", out: File::NULL, exception: true)
    end
    require File.join(@probe_dir, "gvl_probe")
  end

  after(:all) { FileUtils.rm_rf(@probe_dir) }

  # Starts a thread blocked in rb_thread_call_without_gvl2, interrupts it with `interrupt`
  # (or not), and returns what the thread ended with and the probe's record of the call.
  def interrupt_blocked_call(&interrupt)
    thread = Thread.new do
      GvlProbe.block_then_check_ints(1000)
      :returned
    rescue StandardError => e
      e.message
    end
    sleep 0.1
    interrupt&.call(thread)
    [thread.value, GvlProbe.last_block]
  end

  # Claim:    rb_thread_call_without_gvl2 returns without calling its function when the thread has an interrupt pending
  # Fails if: the pending interrupt were ignored or handled first; the function would run and the probe return true
  # Control:  with nothing pending, the same call runs the function
  it "skips the function of rb_thread_call_without_gvl2 when an interrupt is pending" do
    expect(GvlProbe.call_after_wakeup(false)).to be(true)
    expect(GvlProbe.call_after_wakeup(true)).to be(false)
  end

  # Claim:    a thread that holds the GVL while another thread waits for it gets an interrupt pending from the VM timer
  # Fails if: the timer did not interrupt the holder; the call after holding the GVL would run its function
  # Control:  holding the GVL with no other thread waiting leaves nothing pending
  it "leaves a timer interrupt pending on a thread that holds the GVL while another waits" do
    expect(GvlProbe.call_after_holding_gvl(300)).to be(true)
    stop = false
    rival = Thread.new { nil until stop }
    sleep 0.05
    expect(GvlProbe.call_after_holding_gvl(300)).to be(false)
  ensure
    stop = true
    rival&.join
  end

  # Claim:    Thread#raise on a thread blocked in rb_thread_call_without_gvl2 calls its unblocking function, lets the function return, and the exception is raised by rb_thread_check_ints
  # Fails if: the exception interrupted the function or skipped the unblocking function; the probe would record unblocked=false or returned=false, or check_ints would return
  # Control:  with no interrupt, the unblocking function is not called and check_ints returns
  it "calls the unblocking function on Thread#raise and raises the exception from rb_thread_check_ints" do
    expect(interrupt_blocked_call).to eq([:returned, [false, true, true]])
    expect(interrupt_blocked_call { |t| t.raise(RuntimeError, "stop") }).to eq(["stop", [true, true, false]])
  end

  # Claim:    Thread#wakeup on a thread blocked in rb_thread_call_without_gvl2 calls its unblocking function, and rb_thread_check_ints then raises nothing
  # Fails if: wakeup skipped the unblocking function (unblocked=false) or made check_ints raise (it would not return)
  # Control:  with no interrupt, the unblocking function is not called
  it "calls the unblocking function on Thread#wakeup and raises nothing" do
    expect(interrupt_blocked_call).to eq([:returned, [false, true, true]])
    expect(interrupt_blocked_call(&:wakeup)).to eq([:returned, [true, true, true]])
  end
end
