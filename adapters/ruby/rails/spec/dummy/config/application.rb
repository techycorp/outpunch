require "rails"
require "action_controller/railtie"

module Dummy
  class Application < Rails::Application
    config.load_defaults 7.0
    config.eager_load = false
    config.logger = Logger.new(nil)
    config.autoload_paths << File.expand_path("../app/controllers", __dir__)
  end
end
