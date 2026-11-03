# frozen_string_literal: true

module Outpunch
  module Rack
    class Middleware
      WS_PATH = "/ws"

      def initialize(app, server: nil)
        @app = app
        @server = server
      end

      def call(env)
        if env["PATH_INFO"] == WS_PATH && websocket_upgrade?(env)
          # Rack 3: a full hijack returns the client socket; rack.hijack_io is gone from the spec.
          socket = env["rack.hijack"].call
          conn = (@server || Outpunch::Rack.server).create_connection
          Thread.new { conn.run(env, socket) }
          # The server ignores the response after a full hijack; Rack 3 still requires a valid one.
          [200, {}, []]
        else
          @app.call(env)
        end
      end

      private

      def websocket_upgrade?(env)
        env["HTTP_UPGRADE"]&.downcase == "websocket" &&
          env["HTTP_CONNECTION"]&.downcase&.include?("upgrade")
      end
    end
  end
end
