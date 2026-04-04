# frozen_string_literal: true

module Outpunch
  module Rails
    class Engine < ::Rails::Engine
      initializer "outpunch_rails.insert_middleware" do |app|
        app.middleware.insert_before 0, Outpunch::Rack::Middleware, server: OutpunchRails.server
      end

      initializer "outpunch_rails.routes" do
        prefix = OutpunchRails.configuration.route_prefix
        ::Rails.application.routes.prepend do
          match "#{prefix}/:service_name/*service_path", to: "outpunch/tunnel#proxy", via: :all
          match "#{prefix}/:service_name",               to: "outpunch/tunnel#proxy", via: :all
        end
      end
    end
  end
end
