# frozen_string_literal: true

require "rack/mock"

# A Rack env for a WebSocket upgrade, standing in for the server's: `rack.hijack` returns the
# client socket, and there is no `rack.hijack_io`, which Rack 3 dropped from the spec.
#
# Claims about the server (vendor-claims.toml; verified on real Puma by
# spec/vendor/puma_hijack_spec.rb):
# - a full hijack returns the client's socket;
# - after a full hijack the server writes nothing of its own, whatever the app returns.
module FakeRackHijack
  def websocket_upgrade_env(path = "/ws", socket: StringIO.new, on_hijack: nil)
    ::Rack::MockRequest.env_for(path).merge(
      "HTTP_UPGRADE"    => "websocket",
      "HTTP_CONNECTION" => "Upgrade",
      "rack.hijack?"    => true,
      "rack.hijack"     => lambda {
        on_hijack&.call
        socket
      }
    )
  end
end

RSpec.configure { |config| config.include FakeRackHijack }
