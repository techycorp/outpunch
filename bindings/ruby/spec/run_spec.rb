# frozen_string_literal: true

require "spec_helper"

RSpec.describe "Outpunch.run_connection" do
  it "raises RuntimeError when server is unreachable" do
    config = Outpunch::ClientConfig.new(
      "wss://127.0.0.1:19999/ws",
      "secret",
      "test-service",
      nil, nil, nil
    )
    expect {
      Outpunch.run_connection(config)
    }.to raise_error(RuntimeError)
  end
end
