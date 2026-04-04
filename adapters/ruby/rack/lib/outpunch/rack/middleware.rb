# frozen_string_literal: true

module Outpunch
  module Rack
    class Middleware
      WS_PATH = "/ws"

      def initialize(app, server:)
        @app    = app
        @server = server
      end

      def call(env)
        if env["PATH_INFO"] == WS_PATH && websocket_upgrade?(env)
          env["rack.hijack"].call
          conn = @server.create_connection
          Thread.new { conn.run(env) }
          [-1, {}, []]
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
