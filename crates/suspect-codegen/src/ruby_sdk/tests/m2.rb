# frozen_string_literal: true
# Independent installed-gem consumer of the unchanged canonical M2 contract.
require 'ruby_native_gate'
require 'socket'
require 'timeout'

include RubyNativeGate
def assert(value, message = 'assertion failed')
  raise message unless value
end
def reject(type)
  yield
rescue type => error
  error
else
  raise "expected #{type}"
end

WIDGET = '{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"payload":{"kind":"standard","text":"plain"},"child":{"label":"root"}}'
PAGE = '{"items":[{"id":"w2","amount":1e-400,"payload":{"kind":"secure","vault":"v1"}}]}'
FAILURE = '{"message":"rejected"}'

model = Codecs::Widget.decode_json(WIDGET)
assert(model.instance_of?(Models::Widget))
assert(model.amount.token == '9007199254740993.000000000000000001')
assert(model.meta.nil? && model.child.child.equal?(UNSET))
assert(model.payload.instance_of?(Models::StandardPayload))
assert(Codecs::Widget.encode_json(model).include?('9007199254740993.000000000000000001'))
assert(Codecs::WidgetList.decode_json(PAGE).items.first.meta.equal?(UNSET))
assert(Models::StandardPayload.new(text: 'x').kind == 'standard')
assert(Codecs::WidgetPatch.encode_json(Models::WidgetPatch.new) == '{}')
reject(ArgumentError) { Models::WidgetInput.new }
reject(ArgumentError) { Models::WidgetInput.new(name: 'x', unknown: true) }
reject(CodecError) { Models::WidgetInput.new(name: 42) }
reject(ValidationError) { Models::WidgetInput.new(name: '') }
reject(CodecError) { Models::WidgetPatch.new(amount: nil) }
reject(CodecError) { Models::WidgetPatch.new(amount: false) }
reject(CodecError) { Models::WidgetPatch.new(amount: 0.1) }
model.payload.kind = 'secure'
model.payload.extra_fields['vault'] = 'v1'
reject(ValidationError) { Codecs::Widget.encode_json(model) } # parent alone accepts SecurePayload
model.payload.kind = 'standard'
model.payload.extra_fields.clear
model.child.child = model.child
reject(CodecError) { Codecs::Widget.encode(model) }
model.child.child = UNSET
model.meta = UNSET
assert(!Codecs::Widget.encode_json(model).include?('"meta"'))
model.meta = nil
assert(Codecs::Widget.encode_json(model).include?('"meta":null'))
model.extra_fields['meta'] = 'collision'
reject(CodecError) { Codecs::Widget.encode(model) }
model.extra_fields.clear

seen, workers, errors = [], [], Queue.new
entered = Queue.new
server = TCPServer.new('127.0.0.1', 0)
acceptor = Thread.new do
  loop do
    socket = server.accept
    workers << Thread.new(socket) do |io|
      begin
        request = io.gets("\r\n")
        next unless request
        method, target, = request.split(' ')
        headers = {}
        while (line = io.gets("\r\n")) && line != "\r\n"
          key, value = line.split(':', 2)
          headers[key.downcase] = value.strip
        end
        bytes = io.read(headers.fetch('content-length', '0').to_i)
        seen << [method, target, headers['authorization'], bytes]
        if target.end_with?('/cancelled')
          entered << true
          sleep 0.4
        elsif target.end_with?('/timeout')
          sleep 0.3
        end
        body = bytes == '{"name":"deny"}' ? FAILURE : target.include?('?') ? PAGE : WIDGET
        status = bytes == '{"name":"deny"}' ? 422 : target.end_with?('/redirect') ? 307 : 200
        media = target.end_with?('/badmedia') ? 'text/plain' : 'Application/JSON; charset="utf-8"'
        body = '{"id":true}' if target.end_with?('/invalid')
        extra = target.end_with?('/redirect') ? "Location: /must-not-follow\r\n" : ''
        io.write("HTTP/1.1 #{status} Test\r\nContent-Type: #{media}\r\nContent-Length: #{body.bytesize}\r\n#{extra}Connection: close\r\n\r\n#{body}")
      rescue Errno::EPIPE, Errno::ECONNRESET
        # Expected when a caller interrupts the response.
      rescue StandardError => error
        errors << error
      ensure
        io.close
      end
    end
  end
rescue IOError, Errno::EBADF
  nil
end

base = "http://127.0.0.1:#{server.addr[1]}/api/v1"
begin
  ENV['HTTP_PROXY'] = ENV['http_proxy'] = 'http://127.0.0.1:1'
  Client.open(auth: {'apiKey' => 'test-key'}, server_url: base) do |client|
    created = client.create_widget(body: Models::WidgetInput.new(name: 'alpha'))
    assert(created.instance_of?(CreateWidgetStatus200) && created.status == 200)
    assert(created.data.amount.token == '9007199254740993.000000000000000001')
    listed = client.list_widgets(tag: 'a', tags: ['x', 'y'], labels: ['a,b', 'c'], limit: 2)
    assert(listed.data.items.first.amount.token == '1e-400')
    client.get_widget(widget_id: "a/b 雪!'()*")
    client.update_widget(widget_id: 'w1', body: Models::WidgetPatch.new)
    assert(seen == [
      ['POST', '/api/v1/widgets', 'Bearer test-key', '{"name":"alpha"}'],
      ['GET', '/api/v1/widgets?tag=a&tags=x&tags=y&labels=a%2Cb,c&limit=2', 'Bearer test-key', ''],
      ['GET', '/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A', 'Bearer test-key', ''],
      ['PATCH', '/api/v1/widgets/w1', 'Bearer test-key', '{}']
    ], seen.inspect)
    error = reject(RequestError) { client.list_widgets(limit: 0) }
    assert(error.cause.instance_of?(ValidationError) && error.operation_id == 'listWidgets' && error.source.include?('minimum'))
    assert(seen.length == 4)
    patch = Models::WidgetPatch.new
    patch.amount = false
    reject(RequestError) { client.update_widget(widget_id: 'w1', body: patch) }
    assert(seen.length == 4)
    error = reject(CreateWidgetStatus422) { client.create_widget(body: Models::WidgetInput.new(name: 'deny')) }
    assert(error.is_a?(CreateWidgetApiError) && error.is_a?(ApiError) && error.data.message == 'rejected' && error.status == 422)
    assert(!error.inspect.include?('rejected') && !client.inspect.include?('test-key'))
    error = reject(ResponseError) { client.get_widget(widget_id: 'badmedia') }
    assert(error.kind == :unexpected_media && error.status == 200)
    error = reject(ResponseError) { client.get_widget(widget_id: 'invalid') }
    assert(error.kind == :response_decoding && error.status == 200 && error.cause.is_a?(CodecError))
    error = reject(ResponseError) { client.get_widget(widget_id: 'redirect') }
    assert(error.kind == :unexpected_status && error.status == 307)
    assert(seen.none? { |row| row[1].include?('must-not-follow') })
    error = reject(ResourceLimitError) { client.get_widget(widget_id: 'limited', max_response_bytes: 8, max_capture_bytes: 4) }
    assert(error.status == 200 && error.capture.bytesize <= 4 && error.truncated)
    started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    error = reject(TimeoutError) { client.get_widget(widget_id: 'timeout', timeout: 0.04) }
    assert(error.operation_id == 'getWidget')
    assert(Process.clock_gettime(Process::CLOCK_MONOTONIC) - started < 0.25, 'total deadline not enforced')
    token = CancellationToken.new
    job = Thread.new { reject(CancelledError) { client.get_widget(widget_id: 'cancelled', cancellation: token) } }
    ::Timeout.timeout(2) { entered.pop }
    token.cancel
    assert(job.join(1), 'cancellation did not interrupt blocked transport')
    assert(job.value.operation_id == 'getWidget')
    before = seen.length
    reject(CancelledError) { client.get_widget(widget_id: 'never-sent', cancellation: token) }
    assert(seen.length == before)
  end
  closed = Client.new(auth: {'apiKey' => 'test-key'}, server_url: base)
  closed.close
  reject(RequestError) { closed.get_widget(widget_id: 'never-sent') }
ensure
  server.close
  acceptor.join
  workers.each(&:join)
end
raise errors.pop unless errors.empty?
puts "M2 independent native wire, models, errors, timeout and cancellation passed (#{seen.length} exchanges)"
