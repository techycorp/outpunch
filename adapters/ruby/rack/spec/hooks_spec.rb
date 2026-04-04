# frozen_string_literal: true

require "spec_helper"

RSpec.describe Outpunch::Rack::Hooks do
  let(:service_name) { "testservice" }
  let(:mock_request) { double("request") }
  let(:payload)      { { request_id: "req-123", method: "POST", path: "api/test" } }
  let(:result)       { { status: 200, body: "ok" } }

  let(:handler_class) do
    Class.new do
      attr_reader :before_calls, :after_calls

      def initialize
        @before_calls = []
        @after_calls  = []
      end

      # Use class-level storage so tests can inspect across instances
      def self.calls
        @calls ||= { before: [], after: [] }
      end

      def self.reset!
        @calls = { before: [], after: [] }
      end

      def before_proxy(path:, payload:, request:)
        self.class.calls[:before] << { path: path, payload: payload, request: request }
      end

      def after_proxy(path:, payload:, result:, request:)
        self.class.calls[:after] << { path: path, payload: payload, result: result, request: request }
      end
    end
  end

  before do
    described_class.clear!
    handler_class.reset!
  end

  describe ".register" do
    it "adds handler to the hooks map" do
      described_class.register(service_name, /^api\/test$/, handler_class)

      expect(described_class::HOOKS[service_name].length).to eq(1)
      expect(described_class::HOOKS[service_name].first[:pattern]).to eq(/^api\/test$/)
      expect(described_class::HOOKS[service_name].first[:handler_class]).to eq(handler_class)
    end

    it "allows multiple handlers for the same service" do
      other = Class.new
      described_class.register(service_name, /^api\/test$/, handler_class)
      described_class.register(service_name, /^api\/other$/, other)

      expect(described_class::HOOKS[service_name].length).to eq(2)
    end

    it "allows handlers for different services independently" do
      other = Class.new
      described_class.register("svc1", /^api\/test$/, handler_class)
      described_class.register("svc2", /^api\/test$/, other)

      expect(described_class::HOOKS["svc1"].length).to eq(1)
      expect(described_class::HOOKS["svc2"].length).to eq(1)
    end
  end

  describe ".clear!" do
    it "removes all registered hooks" do
      described_class.register(service_name, /^api\/test$/, handler_class)
      described_class.clear!

      expect(described_class::HOOKS).to be_empty
    end
  end

  describe ".before_proxy" do
    context "with a matching handler" do
      before { described_class.register(service_name, /^api\/test$/, handler_class) }

      it "instantiates handler and calls before_proxy" do
        described_class.before_proxy(
          service_name: service_name, path: "api/test",
          payload: payload, request: mock_request
        )

        expect(handler_class.calls[:before].length).to eq(1)
        expect(handler_class.calls[:before].first[:path]).to eq("api/test")
        expect(handler_class.calls[:before].first[:payload]).to eq(payload)
        expect(handler_class.calls[:before].first[:request]).to eq(mock_request)
      end
    end

    context "with a non-matching path" do
      before { described_class.register(service_name, /^api\/other$/, handler_class) }

      it "does not call the handler" do
        described_class.before_proxy(
          service_name: service_name, path: "api/test",
          payload: payload, request: mock_request
        )
        expect(handler_class.calls[:before]).to be_empty
      end
    end

    context "with a wrong service name" do
      before { described_class.register("other-service", /^api\/test$/, handler_class) }

      it "does not call the handler" do
        described_class.before_proxy(
          service_name: service_name, path: "api/test",
          payload: payload, request: mock_request
        )
        expect(handler_class.calls[:before]).to be_empty
      end
    end

    context "with no registered handlers" do
      it "does nothing" do
        expect {
          described_class.before_proxy(
            service_name: service_name, path: "api/test",
            payload: payload, request: mock_request
          )
        }.not_to raise_error
      end
    end

    context "with multiple matching handlers" do
      let(:other_class) do
        Class.new do
          def self.calls; @calls ||= []; end
          def self.reset!; @calls = []; end
          def before_proxy(path:, payload:, request:) = self.class.calls << :before
          def after_proxy(path:, payload:, result:, request:) = self.class.calls << :after
        end
      end

      before do
        other_class.reset!
        described_class.register(service_name, /^api\/.*$/, handler_class)
        described_class.register(service_name, /^api\/test$/, other_class)
      end

      it "calls all matching handlers" do
        described_class.before_proxy(
          service_name: service_name, path: "api/test",
          payload: payload, request: mock_request
        )

        expect(handler_class.calls[:before].length).to eq(1)
        expect(other_class.calls).to eq([:before])
      end
    end
  end

  describe ".after_proxy" do
    context "with a matching handler" do
      before { described_class.register(service_name, /^api\/test$/, handler_class) }

      it "instantiates handler and calls after_proxy" do
        described_class.after_proxy(
          service_name: service_name, path: "api/test",
          payload: payload, result: result, request: mock_request
        )

        expect(handler_class.calls[:after].length).to eq(1)
        expect(handler_class.calls[:after].first[:path]).to eq("api/test")
        expect(handler_class.calls[:after].first[:result]).to eq(result)
      end
    end

    context "with a non-matching path" do
      before { described_class.register(service_name, /^api\/other$/, handler_class) }

      it "does not call the handler" do
        described_class.after_proxy(
          service_name: service_name, path: "api/test",
          payload: payload, result: result, request: mock_request
        )
        expect(handler_class.calls[:after]).to be_empty
      end
    end
  end

  describe "regex pattern matching" do
    before { described_class.register(service_name, /^api\/generate-report$/, handler_class) }

    it "matches the exact path" do
      described_class.before_proxy(
        service_name: service_name, path: "api/generate-report",
        payload: payload, request: mock_request
      )
      expect(handler_class.calls[:before].length).to eq(1)
    end

    it "does not match a longer path" do
      described_class.before_proxy(
        service_name: service_name, path: "api/generate-report/extra",
        payload: payload, request: mock_request
      )
      expect(handler_class.calls[:before]).to be_empty
    end

    it "does not match a different path" do
      described_class.before_proxy(
        service_name: service_name, path: "api/other-endpoint",
        payload: payload, request: mock_request
      )
      expect(handler_class.calls[:before]).to be_empty
    end
  end
end
