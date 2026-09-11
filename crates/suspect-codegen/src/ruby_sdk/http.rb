# frozen_string_literal: true
require 'net/http'
require 'uri'
require 'openssl'
require 'timeout'

module __NAMESPACE__
  class RequestError < SdkError
    def initialize(message, kind: :request, **context) = super(message, kind: kind, **context)
  end
  class ResponseError < SdkError
    def initialize(message, kind: :response_decoding, **context) = super(message, kind: kind, **context)
  end
  class TransportError < SdkError
    def initialize(message, kind: :transport, **context) = super(message, kind: kind, **context)
  end
  class TimeoutError < SdkError
    def initialize(message, kind: :timeout, **context) = super(message, kind: kind, **context)
  end
  class CancelledError < SdkError
    def initialize(message, kind: :cancelled, **context) = super(message, kind: kind, **context)
  end
  class ResourceLimitError < SdkError
    def initialize(message, kind: :resource_limit, **context) = super(message, kind: kind, **context)
  end

  # Owned decoded response; stream data owns its explicit closeable lifetime.
  class ApiResponse
    attr_reader :data, :status, :headers, :content_type, :source, :operation_id, :typed_headers, :links
    def initialize(data:, status:, headers:, source:, operation_id:, content_type: nil, typed_headers: nil, links: {})
      @data, @status, @headers, @source = data, status, headers, source
      @operation_id, @content_type, @typed_headers, @links = operation_id, content_type, typed_headers, links
      freeze
    end
    def close
      @data.close if @data.is_a?(ItemStream)
      nil
    end
    def inspect = "#<#{self.class.name} status=#{@status}>"
  end
  class ApiError < SdkError
    attr_reader :data, :content_type, :typed_headers, :links
    def initialize(data:, content_type: nil, typed_headers: nil, links: {}, **context)
      @data, @content_type, @typed_headers, @links = data, content_type, typed_headers, links
      super('declared API error response', kind: :api, **context)
    end
  end

  # Thread-safe explicit caller cancellation, with no inferred retry behavior.
  class CancellationToken
    def initialize = (@mutex, @cancelled, @listeners = Mutex.new, false, {})
    def cancel
      callbacks = @mutex.synchronize do
        return nil if @cancelled
        @cancelled = true
        @listeners.values.dup
      end
      callbacks.each(&:call)
      nil
    end
    def cancelled? = @mutex.synchronize { @cancelled }
    # @api private
    def subscribe(&callback)
      identity = Object.new
      cancelled = @mutex.synchronize { @listeners[identity] = callback; @cancelled }
      callback.call if cancelled
      identity
    end
    # @api private
    def unsubscribe(identity) = @mutex.synchronize { @listeners.delete(identity); nil }
  end

  # One total monotonic deadline, shared by request work and response lifetime.
  class ExchangeContext
    attr_reader :deadline, :cancellation, :operation_id, :source
    def initialize(timeout:, cancellation:, operation_id:, source:)
      unless (timeout.instance_of?(Float) || timeout.instance_of?(Integer)) && timeout.finite? && timeout.positive? && timeout <= 86_400
        raise ArgumentError, 'timeout must be finite and in (0, 86400] seconds'
      end
      raise ArgumentError, 'invalid cancellation token' unless cancellation.nil? || cancellation.instance_of?(CancellationToken)
      @deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + timeout
      @cancellation, @operation_id, @source = cancellation, operation_id, source
    end
    def remaining = [@deadline - Process.clock_gettime(Process::CLOCK_MONOTONIC), 0.0].max
    def check!
      raise CancelledError.new('exchange cancelled', source: @source, operation_id: @operation_id) if @cancellation&.cancelled?
      raise TimeoutError.new('exchange deadline exceeded', source: @source, operation_id: @operation_id) if remaining.zero?
      nil
    end
    # @api private
    def run
      check!
      thread, guard, active, subscription = Thread.current, Mutex.new, true, nil
      Thread.handle_interrupt(Internal::CancelledSignal => :never, Internal::DeadlineSignal => :never) do
        begin
          subscription = @cancellation&.subscribe { guard.synchronize { thread.raise(Internal::CancelledSignal) if active && thread.alive? } }
          Thread.handle_interrupt(Internal::CancelledSignal => :immediate, Internal::DeadlineSignal => :immediate) do
            ::Timeout.timeout(remaining, Internal::DeadlineSignal) do
              check!; result = yield; check!; result
            end
          end
        ensure
          guard.synchronize { active = false }
          @cancellation&.unsubscribe(subscription)
        end
      end
    rescue Internal::CancelledSignal
      raise CancelledError.new('exchange cancelled', source: @source, operation_id: @operation_id), cause: nil
    rescue Internal::DeadlineSignal
      raise TimeoutError.new('exchange deadline exceeded', source: @source, operation_id: @operation_id), cause: nil
    end
    def inspect = "#<#{self.class.name}>"
  end

  class PreparedRequest
    attr_reader :method, :url, :headers, :body, :source, :operation_id
    def initialize(method:, url:, headers:, body:, source:, operation_id:)
      @method, @url = method.dup.freeze, url.dup.freeze
      @headers = headers.to_h { |k, v| [k.dup.freeze, v.dup.freeze] }.freeze
      @body, @source, @operation_id = body&.dup&.freeze, source, operation_id
      freeze
    end
    def inspect = "#<#{self.class.name} method=#{@method}>"
  end
  class WireResponse
    attr_reader :status, :headers
    def initialize(status:, headers:, body:, &on_close)
      @status, @headers, @body, @on_close, @closed = status, headers, body, on_close, false
    end
    def each_chunk(&block)
      raise IOError, 'closed response' if @closed
      if @body.instance_of?(String)
        offset = 0
        while offset < @body.bytesize
          block.call(@body.byteslice(offset, 16 * 1024)); offset += 16 * 1024
        end
      else @body.each(&block)
      end
      nil
    end
    def close
      return nil if @closed
      @closed = true
      begin @body.close if @body.respond_to?(:close)
      ensure @on_close&.call
      end
      nil
    end
    def closed? = @closed
    def inspect = "#<#{self.class.name}>"
  end

  # One connection per exchange. Exact method tokens, verified TLS, no proxy,
  # redirect, decompression, auto-auth, or retry behavior.
  class NetHTTPTransport
    def exchange(request:, context:)
      uri = ::URI.parse(request.url)
      http = ::Net::HTTP.new(uri.hostname, uri.port, nil)
      http.use_ssl = uri.scheme == 'https'
      http.verify_mode = ::OpenSSL::SSL::VERIFY_PEER if http.use_ssl?
      http.max_retries = 0
      http.open_timeout = http.read_timeout = http.write_timeout = context.remaining
      native = ::Net::HTTPGenericRequest.new(request.method, !request.body.nil?, request.method != 'HEAD', uri.request_uri, request.headers)
      native.body = request.body unless request.body.nil?
      begin
        http.start do
          socket = http.instance_variable_get(:@socket)
          socket.extend(Internal::BoundedLineReader); socket.sdk_framing_context = context
          context.check!
          http.request(native) do |response|
            wire = WireResponse.new(status: response.code.to_i, headers: response.to_hash, body: Enumerator.new { |out| response.read_body { |chunk| context.check!; out << chunk } })
            begin yield wire
            ensure wire.close
            end
          end
        end
      ensure
        http.finish if http.started?
      end
      nil
    end
    def close = nil
    def inspect = "#<#{self.class.name}>"
  end

  # @api private
  module Internal
    class CancelledSignal < Exception; end
    class DeadlineSignal < Exception; end
    module BoundedLineReader
      attr_writer :sdk_framing_context
      def readuntil(terminator, ignore_eof = false)
        out = +''.b; @sdk_framing_bytes ||= 0
        loop do
          out << read(1); @sdk_framing_bytes += 1
          raise ResourceLimitError.new('HTTP framing byte ceiling exceeded', source: @sdk_framing_context.source, operation_id: @sdk_framing_context.operation_id) if @sdk_framing_bytes > POLICY[:max_header_bytes]
          return out if out.end_with?(terminator)
        end
      rescue EOFError
        raise unless ignore_eof
        out
      end
    end
    module_function
    def bounded_limit(value, maximum, name)
      raise ArgumentError, "invalid finite #{name}" unless value.instance_of?(Integer) && value.between?(1, maximum)
      value
    end
    def valid_percent?(value) = !/%(?![0-9a-fA-F]{2})/.match?(value)
    def dot_segment?(value) = ['.', '..'].include?(value.gsub(/%2e/i, '.'))
    def server_url(value)
      uri = Wire.absolute_url(value)
      raise ArgumentError, 'dot segments are not allowed in an explicit override' if uri.path.split('/').any? { |s| dot_segment?(s) }
      value.dup.freeze
    end
    def response_headers(raw)
      raise TransportError, 'transport headers must be a Hash' unless raw.instance_of?(Hash)
      out, bytes = {}, 0
      raw.each do |key, values|
        raise TransportError, 'invalid response header name' unless key.instance_of?(String) && Wire.token?(key)
        values = [values] if values.instance_of?(String)
        raise TransportError, 'invalid response header values' unless values.instance_of?(Array) && !values.empty?
        values.each do |value|
          raise TransportError, 'invalid response header value' unless value.instance_of?(String) && !/[\x00-\x08\x0a-\x1f\x7f]/n.match?(value.b)
          bytes += key.bytesize + value.bytesize + 4
          raise ResourceLimitError, 'response header ceiling exceeded' if bytes > POLICY[:max_header_bytes]
          (out[key.downcase] ||= []) << value.dup.freeze
        end
      end
      out.each_value(&:freeze); out.freeze
    end
  end

  # Native keyword client; every wire choice comes from its retained protocol.
  class Client
    def initialize(auth: {}, credential_provider: nil, security: UNSET, transport: nil, server_url: nil,
                   server: UNSET, server_variables: {}, document_url: nil, timeout: 30.0,
                   max_response_bytes: Internal::POLICY[:max_response_bytes], max_capture_bytes: Internal::POLICY[:max_capture_bytes])
      raise ArgumentError, 'auth must be a string-keyed Hash' unless auth.instance_of?(Hash) && auth.keys.all? { |k| k.instance_of?(String) }
      @auth = auth.to_h { |key, value| [key.dup.freeze, value.instance_of?(String) ? value.dup.freeze : value] }.freeze
      bearer_names = Internal::PROTOCOL['operations'].flat_map { |o| o['security']['alternatives'] || [] }.flat_map { |a| a['requirements'] }.select { |r| r['credential']['kind'] == 'bearer' }.map { |r| r['name'] }
      @auth.each do |name, value|
        if bearer_names.include?(name) && (!value.instance_of?(String) || value.bytesize > Internal::POLICY[:max_header_bytes] || !/\A[A-Za-z0-9._~+\/-]+=*\z/n.match?(value.b))
          raise ArgumentError, 'invalid RFC6750 bearer credential'
        end
      end
      @credential_provider, @security = credential_provider, security
      raise ArgumentError, 'credential_provider must implement call(context)' if credential_provider && !credential_provider.respond_to?(:call)
      @server_url = server_url && Internal.server_url(server_url)
      @server, @server_variables, @document_url = server, server_variables.dup.freeze, document_url&.dup&.freeze
      @timeout = timeout; ExchangeContext.new(timeout: timeout, cancellation: nil, operation_id: '', source: '')
      @max_response_bytes = Internal.bounded_limit(max_response_bytes, Internal::POLICY[:max_response_bytes], 'max_response_bytes')
      @max_capture_bytes = Internal.bounded_limit(max_capture_bytes, [Internal::POLICY[:max_capture_bytes], @max_response_bytes].min, 'max_capture_bytes')
      @transport, @owned, @closed = transport || NetHTTPTransport.new, transport.nil?, false
      raise ArgumentError, 'transport must implement exchange(request:, context:)' unless @transport.respond_to?(:exchange)
    end
    def self.open(**options)
      client = new(**options)
      begin yield client
      ensure client.close
      end
    end
    def close
      return nil if @closed
      @closed = true
      @transport.close if @owned
      nil
    end
    def closed? = @closed
    def inspect = "#<#{self.class.name} closed=#{@closed}>"
    private

    def call_operation(index, parameters, body, timeout:, cancellation:, max_response_bytes:, max_capture_bytes:,
                       content_type:, accept:, security:, server:, server_variables:, document_url:)
      op = Internal::OPERATIONS.fetch(index)
      raise RequestError.new('client is closed', source: op[:source], operation_id: op[:id]) if @closed
      limit = Internal.bounded_limit(max_response_bytes.equal?(UNSET) ? @max_response_bytes : max_response_bytes, @max_response_bytes, 'max_response_bytes')
      capture = Internal.bounded_limit(max_capture_bytes.equal?(UNSET) ? [@max_capture_bytes, limit].min : max_capture_bytes, [@max_capture_bytes, limit].min, 'max_capture_bytes')
      context = ExchangeContext.new(timeout: timeout.equal?(UNSET) ? @timeout : timeout, cancellation: cancellation, operation_id: op[:id], source: op[:source])
      lease = nil; streaming = false
      context.run do
        request = prepare_request(op, parameters, body, content_type, accept, security, server, server_variables, document_url)
        if op[:streaming]
          lease = Internal::ExchangeLease.new(@transport, request, context)
          response = consume_response(op, lease, context, limit, capture)
          streaming = response.data.is_a?(ItemStream)
          response
        else
          yielded = false; result = nil
          @transport.exchange(request: request, context: context) do |wire|
            begin
              raise TransportError, 'transport yielded multiple responses' if yielded
              yielded = true
              result = consume_response(op, wire, context, limit, capture)
            ensure
              primary = $!
              begin wire.close
              rescue StandardError
                raise unless primary
              end
            end
          end
          raise TransportError, 'transport did not yield a response' unless yielded
          result
        end
      end
    rescue SdkError
      raise
    rescue ::Net::OpenTimeout, ::Net::ReadTimeout, ::Net::WriteTimeout => error
      raise TimeoutError.new('HTTP transport timed out', source: op[:source], operation_id: op[:id]), cause: error
    rescue StandardError => error
      raise if error.instance_of?(ArgumentError) && context.nil?
      raise TransportError.new('HTTP transport failed', source: op[:source], operation_id: op[:id]), cause: error
    ensure
      lease&.close unless streaming
    end

    def attach(headers, queries, cookies, ownership, location, name, value, source)
      Internal.utf8!(name); Internal.utf8!(value)
      key = [location, location == 'header' ? name.downcase : name]
      raise RequestError.new('conflicting credential/parameter attachments', source: source) if ownership[key]
      ownership[key] = true
      case location
      when 'header'
        raise RequestError.new('invalid header attachment', source: source) unless Internal::Wire.token?(name) && !/[\x00-\x08\x0a-\x1f\x7f]/n.match?(value.b)
        raise RequestError.new('transport framing header is managed', source: source) if %w[host content-length transfer-encoding connection].include?(name.downcase)
        headers[name] = value
      when 'query' then queries << Internal::Wire.percent(name) + '=' + Internal::Wire.percent(value)
      when 'cookie' then cookies << Internal::Wire.percent(name) + '=' + Internal::Wire.percent(value)
      end
    end
    def credentials(op, selection, headers, queries, cookies, ownership, server_url)
      security = op[:wire].fetch('security')
      return if security['kind'] != 'alternatives'
      options = security.fetch('alternatives')
      selection = @security if selection.equal?(UNSET)
      if selection.equal?(UNSET)
        raise RequestError.new('select a declared security alternative explicitly', source: op[:source]) if options.length != 1
        selection = 0
      end
      raise RequestError.new('invalid security alternative', source: op[:source]) unless selection.instance_of?(Integer) && selection.between?(0, options.length - 1)
      options.fetch(selection).fetch('requirements').each do |requirement|
        context = CredentialContext.new(requirement, op[:id], server_url: server_url)
        value = @auth[context.name]
        value = @credential_provider.call(context) if value.nil? && @credential_provider
        raise RequestError.new('missing declared credential', source: context.source, operation_id: op[:id]) if value.nil?
        hook = requirement.fetch('credential')
        case hook.fetch('kind')
        when 'bearer'
          raise RequestError.new('invalid RFC6750 bearer token', source: context.source) unless value.instance_of?(String) && value.bytesize <= Internal::POLICY[:max_header_bytes] && /\A[A-Za-z0-9._~+\/-]+=*\z/n.match?(value.b)
          attach(headers, queries, cookies, ownership, 'header', 'Authorization', 'Bearer ' + value, context.source)
        when 'basic'
          raise RequestError.new('Basic requires BasicCredential', source: context.source) unless value.instance_of?(BasicCredential)
          attach(headers, queries, cookies, ownership, 'header', 'Authorization', 'Basic ' + ::Base64.strict_encode64(value.username + ':' + value.password), context.source)
        when 'api-key'
          raise RequestError.new('API key must be a String', source: context.source) unless value.instance_of?(String)
          attach(headers, queries, cookies, ownership, hook.fetch('location'), hook.fetch('name').fetch('value'), value, context.source)
        when 'o-auth2', 'oauth2', 'open-id-connect'
          raise RequestError.new('OAuth/OIDC requires an explicit AuthorizationCredential', source: context.source) unless value.instance_of?(AuthorizationCredential)
          attach(headers, queries, cookies, ownership, 'header', 'Authorization', value.scheme + ' ' + value.token, context.source)
        else raise RequestError.new('unknown credential hook', source: context.source)
        end
      end
    end
    def prepare_request(op, parameters, body, content_type, accept, security, server, variables, document_url)
      payload = Internal::PayloadSession.new
      headers, queries, cookies, ownership = {'Accept-Encoding' => 'identity'}, [], [], {}
      path = op[:wire].fetch('path').dup
      op[:wire].fetch('parameters').each_with_index do |p, i|
        value = parameters.fetch(i)
        if value.equal?(UNSET)
          raise RequestError.new('required parameter absent', source: Internal::Wire.source(p['source'])) if p['required']
          next
        end
        if p['content_media'] && p['content_media']['representation']['kind'] == 'form'
          serialized = payload.encode_form(p['content_media'], value)
        else
          wire = payload.encode_codec(p['codec'], value)
          serialized = Internal::Wire.serialize(p['name'], p['location'], p['serialization'], wire)
        end
        case p['location']
        when 'path'
          raise RequestError.new('dot path segment is unsupported', source: Internal::Wire.source(p['source'])) if Internal.dot_segment?(serialized)
          path.gsub!('{' + p['name'] + '}') { serialized }
        when 'query', 'querystring'
          raise RequestError.new('multiple whole-query values', source: op[:source]) if p['location'] == 'querystring' && !queries.empty?
          serialized.split('&').each { |part| ownership[['query', Internal::Wire.unpercent(part.split('=', 2).first)]] = true } if p['location'] == 'query'
          queries << serialized
        when 'header' then attach(headers, queries, cookies, ownership, 'header', p['name'], serialized, Internal::Wire.source(p['source']))
        when 'cookie'
          ownership[['cookie', p['name']]] = true; cookies << serialized
        end
      end
      bytes = nil
      if (b = op[:wire]['body'])
        if body.equal?(UNSET)
          raise RequestError.new('required request body absent', source: Internal::Wire.source(b['source'])) if b['required']
        else
          if content_type.equal?(UNSET)
            raise RequestError.new('choose a concrete request content_type', source: Internal::Wire.source(b['source'])) unless b['media'].length == 1 && b['media'][0]['media_type']['range']['kind'] == 'concrete'
            content_type = b['media'][0]['media_type']['declared']
          end
          media, = Internal::Wire.match_media(b['media'], content_type)
          bytes, concrete = payload.encode(media, body, content_type)
          raise RequestError.new('Content-Type conflicts with a managed body choice', source: op[:source]) if headers.keys.any? { |k| k.casecmp?('content-type') }
          headers['Content-Type'] = concrete
        end
      end
      unless accept.equal?(UNSET)
        Internal::Wire.parse_media(accept)
        headers['Accept'] = accept
      else
        candidates = op[:wire]['responses'].flat_map { |r| r['media'].map { |m| m['media_type']['declared'] } }.uniq
        headers['Accept'] = candidates.join(', ') unless candidates.empty?
      end
      base = Internal::Wire.server(op[:wire]['servers'], server.equal?(UNSET) ? @server : server, variables.equal?(UNSET) ? @server_variables : variables, document_url.equal?(UNSET) ? @document_url : document_url, @server_url)
      headers.each_key { |name| ownership[['header', name.downcase]] = true }
      credentials(op, security, headers, queries, cookies, ownership, base)
      unless cookies.empty?
        raise RequestError.new('Cookie header conflicts with cookie parameters', source: op[:source]) if headers.keys.any? { |k| k.casecmp?('cookie') }
        headers['Cookie'] = cookies.join('; ')
      end
      url = base.sub(%r{/\z}, '') + path
      url += '?' + queries.join('&') unless queries.empty?
      header_bytes = headers.sum { |k, v| k.bytesize + v.bytesize + 4 }
      if url.bytesize > Internal::POLICY[:max_url_bytes] || header_bytes > Internal::POLICY[:max_header_bytes] || url.bytesize + header_bytes + (bytes&.bytesize || 0) > Internal::POLICY[:max_request_bytes]
        raise ResourceLimitError.new('request byte ceiling exceeded', source: op[:source], operation_id: op[:id])
      end
      PreparedRequest.new(method: op[:wire]['method'], url: url, headers: headers, body: bytes, source: op[:source], operation_id: op[:id])
    rescue CodecError, JsonError, ArgumentError => error
      raise RequestError.new('request does not satisfy its source-bound wire/codec plan', source: error.respond_to?(:source) && error.source || op[:source], instance_path: error.respond_to?(:instance_path) ? error.instance_path : '', operation_id: op[:id]), cause: error
    rescue ResourceLimitError => error
      raise ResourceLimitError.new(error.message, source: error.source || op[:source], operation_id: op[:id]), cause: error
    end

    def consume_response(op, response, context, limit, capture_limit)
      status = response.status
      raise TransportError.new('invalid transport status', source: op[:source], operation_id: op[:id]) unless status.instance_of?(Integer) && status.between?(100, 599)
      headers = Internal.response_headers(response.headers)
      selected = Internal::Wire.status_match(op[:wire]['responses'], status)
      native = selected && op[:responses].fetch(selected['status_key'])
      metadata = {source: selected ? Internal::Wire.source(selected['source']) : op[:source], operation_id: op[:id], status: status, headers: headers}
      forbidden = op[:wire]['method'] == 'HEAD' || status.between?(100, 199) || [204, 205, 304].include?(status)
      payload = Internal::PayloadSession.new
      typed_headers = selected && payload.decode_headers(selected['headers'], headers, 'headers:' + Internal::Wire.binding_source(selected['source']))
      links = selected ? selected['links'].to_h { |l| [l['name'], Link.new(l)] }.freeze : {}.freeze
      media, actual, media_error = nil, nil, nil
      if selected && !forbidden && !selected['media'].empty?
        begin
          raise CodecError, 'missing/repeated Content-Type' unless headers['content-type']&.length == 1
          media, actual = Internal::Wire.match_media(selected['media'], headers['content-type'].first)
          encoding = headers['content-encoding']
          raise CodecError, 'encoded structured response is unsupported' if media['representation']['kind'] != 'binary' && encoding && encoding != ['identity']
        rescue CodecError => error
          media_error = error
        end
      end
      limit = [limit, Internal::Wire.integer(selected['max_body_bytes'])].min if selected
      length, transfer = headers['content-length'], headers['transfer-encoding']
      if !forbidden && (length && (length.length != 1 || !/\A[0-9]+\z/n.match?(length[0].b) || transfer) || transfer && (transfer.length != 1 || transfer[0].downcase != 'chunked'))
        raise TransportError.new('ambiguous response framing', **metadata)
      end
      if !forbidden && length && (length[0].length > 16 || length[0].to_i > limit)
        raise ResourceLimitError.new('declared response byte ceiling exceeded', truncated: true, **metadata)
      end
      chunks = Internal::LimitedChunks.new(response, context, limit, capture_limit, metadata)
      if media && media['representation']['kind'] == 'stream' && !forbidden
        stream = ItemStream.new(chunks, media['representation']['stream'], context)
        if status.between?(200, 299)
          return native[:success].new(data: stream, content_type: headers['content-type'].first, typed_headers: typed_headers, links: links, **metadata)
        end
        # Error streams are bounded and validated before becoming API errors.
        values = stream.to_a
        raise native[:error].new(data: ItemStream.completed(values, metadata[:source]), content_type: headers['content-type'].first, typed_headers: typed_headers, links: links, **chunks.details)
      end
      bytes = +''.b
      unless forbidden
        chunks.each_chunk do |chunk|
          break if (!selected || media_error) && chunks.truncated
          bytes << chunk.b
        end
      end
      raise ResponseError.new('undeclared HTTP status', kind: :unexpected_status, **chunks.details) unless selected
      raise ResponseError.new('unexpected response media', kind: :unexpected_media, **chunks.details), cause: media_error if media_error
      if !forbidden && length && bytes.bytesize != length[0].to_i
        raise TransportError.new('response content length was not satisfied', **chunks.details)
      end
      begin
        data = forbidden ? NO_CONTENT : media ? payload.decode(media, bytes, actual) : Bytes.new(bytes)
      rescue JsonError, CodecError => error
        raise ResponseError.new('response does not satisfy its source codec', **chunks.details), cause: error
      end
      if status.between?(200, 299)
        native[:success].new(data: data, content_type: headers['content-type']&.first, typed_headers: typed_headers, links: links, **metadata)
      else
        raise native[:error].new(data: data, content_type: headers['content-type']&.first, typed_headers: typed_headers, links: links, **chunks.details)
      end
    rescue CodecError => error
      raise ResponseError.new('response header does not satisfy its source codec', **(metadata || {source: op[:source], operation_id: op[:id]})), cause: error
    end
  end
end
