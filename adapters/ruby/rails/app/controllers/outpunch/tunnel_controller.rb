# frozen_string_literal: true

class Outpunch::TunnelController < OutpunchRails.configuration.base_controller.constantize
  skip_before_action :verify_authenticity_token, raise: false

  def proxy
    service_name = params[:service_name]
    service_path = params[:service_path] || ""

    return render_error(400, "Service name required") if service_name.blank?

    if (auth = OutpunchRails.configuration.authorize_service)
      unless auth.call(service_name, request)
        return render_error(403, "Service '#{service_name}' not allowed for this product")
      end
    end

    unless OutpunchRails.connected?(service_name)
      return render_error(502, "Service '#{service_name}' offline")
    end

    request_id = SecureRandom.uuid
    payload = {
      request_id: request_id,
      method: request.method,
      path: service_path,
      query: request.query_parameters,
      headers: OutpunchRails.extract_proxy_headers(request.headers),
      body: request.body&.read || ""
    }

    run_hook(:before_proxy, service_name, service_path, payload)

    response_data = OutpunchRails.handle_request(
      service: service_name,
      method: payload[:method],
      path: payload[:path],
      query: payload[:query],
      headers: payload[:headers],
      body: payload[:body]
    )
    result = OutpunchRails.success_response(response_data)

    run_hook(:after_proxy, service_name, service_path, payload, result: result)

    render_tunnel_result(result)
  rescue Timeout::Error
    render_error(504, "Tunnel timeout for service '#{params[:service_name]}'")
  rescue => e
    render_error(502, e.message)
  end

  private

  def run_hook(type, service_name, path, payload, extra = {})
    hooks = OutpunchRails.configuration.hooks
    return unless hooks

    case type
    when :before_proxy
      hooks.before_proxy(service_name: service_name, path: path, payload: payload, request: request)
    when :after_proxy
      hooks.after_proxy(service_name: service_name, path: path, payload: payload, request: request, **extra)
    end
  end

  def render_tunnel_result(result)
    if result[:body_encoding] == "base64"
      content_disposition = result[:headers]["content-disposition"] || "attachment"
      disposition = content_disposition.start_with?("attachment") ? "attachment" : "inline"
      filename = content_disposition[/filename="?([^";]+)"?/, 1]

      send_data result[:body],
                type: result[:headers]["content-type"] || "application/octet-stream",
                disposition: disposition,
                filename: filename,
                status: result[:status]
    else
      result[:headers]&.each { |key, value| response.set_header(key, value) }
      render body: result[:body], status: result[:status]
    end
  end

  def render_error(status, message)
    result = OutpunchRails.error_response(status, message)
    render json: JSON.parse(result[:body]), status: result[:status]
  end
end
