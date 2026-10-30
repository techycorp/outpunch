# frozen_string_literal: true

module Outpunch
  # Raised by a tunnel hook (e.g. before_proxy) to short-circuit proxying with a
  # deliberate HTTP status + body, instead of letting the request forward upstream.
  #
  # The tunnel controller rescues this *before* its catch-all, so the client gets the
  # hook's chosen status (e.g. 402 for a billing gate) rather than a generic 502. This
  # keeps the gem product-agnostic: the gem provides the halt mechanism; what a halt
  # means (billing, quota, …) lives in the application's hook.
  class HookHalt < StandardError
    attr_reader :status, :body

    def initialize(status, body)
      @status = status
      @body = body
      super("HookHalt #{status}")
    end
  end
end
