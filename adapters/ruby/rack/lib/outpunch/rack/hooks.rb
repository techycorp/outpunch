# frozen_string_literal: true

module Outpunch
  module Rack
    module Hooks
      HOOKS = {}

      def self.register(service_name, path_pattern, handler_class)
        HOOKS[service_name] ||= []
        HOOKS[service_name] << { pattern: path_pattern, handler_class: handler_class }
      end

      def self.before_proxy(service_name:, path:, payload:, request:)
        find_handlers(service_name, path).each do |h|
          h[:handler_class].new.before_proxy(path: path, payload: payload, request: request)
        end
      end

      def self.after_proxy(service_name:, path:, payload:, result:, request:)
        find_handlers(service_name, path).each do |h|
          h[:handler_class].new.after_proxy(path: path, payload: payload, result: result, request: request)
        end
      end

      def self.clear!
        HOOKS.clear
      end

      def self.find_handlers(service_name, path)
        (HOOKS[service_name] || []).select { |h| path.match?(h[:pattern]) }
      end
      private_class_method :find_handlers
    end
  end
end
