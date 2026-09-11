# frozen_string_literal: true
require 'ruby_native_gate'
require 'socket'
require 'openssl'
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

class ProbeBody
  attr_reader :closed, :reads
  def initialize(chunks, failure = nil)
    @chunks, @failure, @closed, @reads = chunks, failure, 0, 0
  end
  def each
    @chunks.each { |chunk| @reads += 1; yield chunk }
    raise @failure if @failure
  end
  def close
    @closed += 1
  end
end
class ProbeTransport
  attr_reader :calls, :closes, :request, :context, :response
  def initialize(status: 200, headers: {'Content-Type' => 'application/json'}, body:, failure: nil)
    @status, @headers, @body, @failure = status, headers, body, failure
    @calls = @closes = 0
  end
  def exchange(request:, context:)
    @request, @context = request, context
    @calls += 1
    raise @failure if @failure
    @response = WireResponse.new(status: @status, headers: @headers, body: @body)
    yield @response
    nil
  end
  def close = @closes += 1
end
WIDGET = '{"id":"w1","amount":1,"payload":{"kind":"standard","text":"plain"}}'
def call(transport, **options)
  Client.open(auth: {'apiKey' => 'opaque-token'}, transport: transport) { |client| client.get_widget(widget_id: 'one', **options) }
end

body = ProbeBody.new([WIDGET])
transport = ProbeTransport.new(body: body)
assert(call(transport).data.id == 'w1')
assert(body.closed == 1 && transport.response.closed? && transport.closes.zero?, 'borrowed transport lifetime')
assert(transport.request.frozen? && transport.request.headers.frozen? && transport.request.headers.values.all?(&:frozen?))
assert(!transport.request.inspect.include?('opaque-token'))
assert(transport.context.deadline.is_a?(Float) && transport.context.operation_id == 'getWidget')

[
  [{status: 404, body: '{"message":"missing"}'}, GetWidgetStatus404],
  [{status: 307, body: 'redirect'}, ResponseError],
  [{headers: {'Content-Type' => 'text/plain'}, body: 'text'}, ResponseError],
  [{headers: {'Content-Type' => ['application/json', 'application/json']}, body: WIDGET}, ResponseError],
  [{headers: {'Content-Type' => 'application/json', 'Content-Encoding' => 'gzip'}, body: WIDGET}, ResponseError],
  [{body: '{"a":1,"\u0061":2}'}, ResponseError],
  [{body: "\xff".b}, ResponseError],
  [{body: '{}'}, ResponseError],
  [{status: true, body: WIDGET}, TransportError],
  [{headers: {'Bad\r\nHeader' => 'value'}, body: WIDGET}, TransportError],
  [{headers: {'Content-Type' => "application/json\r\nInjected: yes"}, body: WIDGET}, TransportError]
].each do |options, type|
  body = ProbeBody.new([options.delete(:body)])
  transport = ProbeTransport.new(**options, body: body)
  reject(type) { call(transport) }
  assert(body.closed == 1 && transport.closes.zero?, 'response was not released exactly once')
end

body = ProbeBody.new(['x' * 32, 'never read'])
transport = ProbeTransport.new(status: 500, body: body)
error = reject(ResponseError) { call(transport, max_response_bytes: 100, max_capture_bytes: 4) }
assert(error.status == 500 && error.capture == 'xxxx' && error.truncated && body.reads == 1 && body.closed == 1)
body = ProbeBody.new(['x' * 32])
error = reject(ResourceLimitError) { call(ProbeTransport.new(body: body), max_response_bytes: 16, max_capture_bytes: 4) }
assert(error.capture.bytesize == 4 && body.closed == 1)
body = ProbeBody.new(Array.new(65_537, ''))
reject(ResourceLimitError) { call(ProbeTransport.new(body: body)) }
assert(body.closed == 1)
failure = IOError.new('original hook cause')
body = ProbeBody.new(['{'], failure)
error = reject(TransportError) { call(ProbeTransport.new(body: body)) }
assert(error.cause.equal?(failure) && !error.message.include?(failure.message) && body.closed == 1)
error = reject(TransportError) { call(ProbeTransport.new(body: '', failure: failure)) }
assert(error.cause.equal?(failure))
body = ProbeBody.new([WIDGET])
reject(ResourceLimitError) { call(ProbeTransport.new(headers: {'X-Large' => 'x' * 70_000}, body: body)) }
assert(body.closed == 1)

['', "a\nb", 'Bearer key', "a\rb", "a\0b", 'a=b=c', '雪'].each do |token|
  reject(ArgumentError) { Client.new(auth: {'apiKey' => token}) }
end
[nil, false, '30', 0, -1, Float::NAN, Float::INFINITY, 86_401].each do |timeout|
  reject(ArgumentError) { Client.new(auth: {'apiKey' => 'token'}, timeout: timeout) }
end
['https://user:secret@example.com', 'https://example.com?x=1', 'https://example.com#x', 'https://example.com/../a', 'https://example.com/%2e%2E/a', "https://example.com\n", 'https://example.com/invalid%'].each do |url|
  reject(ArgumentError) { Client.new(auth: {'apiKey' => 'token'}, server_url: url) }
end
transport = ProbeTransport.new(body: WIDGET)
client = Client.new(auth: {}, transport: transport)
error = reject(RequestError) { client.get_widget(widget_id: 'x') }
assert(transport.calls.zero? && error.source.end_with?('/security/0/apiKey'))
client = Client.new(auth: {'apiKey' => 'token'}, transport: transport)
reject(ArgumentError) { client.get_widget(widget_id: 'x', max_response_bytes: 0) }
reject(ArgumentError) { client.get_widget(widget_id: 'x', max_capture_bytes: false) }
reject(RequestError) { client.get_widget(widget_id: '..') }
reject(RequestError) { client.list_widgets(tag: "\xff".b) }
reject(ResourceLimitError) { client.get_widget(widget_id: 'x' * 100_000) }
reject(ResourceLimitError) { client.list_widgets(tags: Array.new(100, 'x' * 1024)) }
reject(ResourceLimitError) { client.list_widgets(labels: Array.new(100, 'x' * 1024)) }
assert(transport.calls.zero?)

# Actual socket framing, not only an injected pre-buffered response.
def raw_exchange(raw)
  server = TCPServer.new('127.0.0.1', 0)
  worker = Thread.new do
    io = server.accept
    begin
      io.gets("\r\n")
      while (line = io.gets("\r\n")) && line != "\r\n"; end
      io.write(raw)
    rescue Errno::EPIPE, Errno::ECONNRESET
      nil
    ensure
      io.close
    end
  end
  begin
    Client.open(auth: {'apiKey' => 'token'}, server_url: "http://127.0.0.1:#{server.addr[1]}", timeout: 2) { |client| yield client }
  ensure
    server.close
    worker.join(2) || worker.kill
  end
end

raw_exchange("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 999\r\nConnection: close\r\n\r\n#{WIDGET}") do |client|
  reject(TransportError) { client.get_widget(widget_id: 'x') }
end
raw_exchange("HTTP/1.1 200 OK\r\nX-Huge: #{'x' * 70_000}\r\nContent-Type: application/json\r\nContent-Length: #{WIDGET.bytesize}\r\n\r\n#{WIDGET}") do |client|
  error = reject(ResourceLimitError) { client.get_widget(widget_id: 'x') }
  assert(error.source.include?('/paths/') && error.operation_id == 'getWidget')
end
raw_exchange("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n#{WIDGET.bytesize.to_s(16)}\r\n#{WIDGET}\r\n0\r\n\r\n") do |client|
  assert(client.get_widget(widget_id: 'x').data.id == 'w1')
end
raw_exchange("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nNOT-A-SIZE\r\n") do |client|
  reject(TransportError) { client.get_widget(widget_id: 'x') }
end

# A dropped connection does not trigger Net::HTTP's ordinary GET retry.
attempts = 0
server = TCPServer.new('127.0.0.1', 0)
worker = Thread.new do
  loop do
    io = server.accept
    attempts += 1
    io.gets("\r\n")
    io.close
  end
rescue IOError, Errno::EBADF
  nil
end
begin
  Client.open(auth: {'apiKey' => 'token'}, server_url: "http://127.0.0.1:#{server.addr[1]}") do |client|
    reject(TransportError) { client.get_widget(widget_id: 'drop') }
  end
  assert(attempts == 1, 'automatic transport retry')
ensure
  server.close
  worker.join
end

# Self-signed TLS is rejected using the real default adapter.
key = OpenSSL::PKey::RSA.new(2048)
cert = OpenSSL::X509::Certificate.new
cert.version, cert.serial = 2, 1
cert.subject = cert.issuer = OpenSSL::X509::Name.parse('/CN=localhost')
cert.public_key = key.public_key
cert.not_before, cert.not_after = Time.now - 60, Time.now + 600
cert.sign(key, OpenSSL::Digest.new('SHA256'))
context = OpenSSL::SSL::SSLContext.new
context.cert, context.key = cert, key
server = TCPServer.new('127.0.0.1', 0)
ssl = OpenSSL::SSL::SSLServer.new(server, context)
worker = Thread.new do
  begin
    socket = ssl.accept
    socket.close
  rescue OpenSSL::SSL::SSLError
    nil
  end
end
begin
  Client.open(auth: {'apiKey' => 'token'}, server_url: "https://127.0.0.1:#{server.addr[1]}", timeout: 2) do |client|
    error = reject(TransportError) { client.get_widget(widget_id: 'tls') }
    assert(error.cause.is_a?(OpenSSL::SSL::SSLError), 'TLS verification was bypassed')
  end
ensure
  server.close
  worker.join(2) || worker.kill
end
puts 'Injected transport cleanup, security, captures, malformed responses and resource gates passed'
