# frozen_string_literal: true
require 'ruby_native_gate'
require 'socket'
require 'timeout'
include RubyNativeGate
def assert(value, message = 'assertion failed') = (raise message unless value)
def reject(type)
  yield
rescue type => error
  error
else
  raise "expected #{type}"
end
class StreamFixture
  attr_reader :closed, :reads
  def initialize(chunks, media)
    @chunks, @media, @closed, @reads = chunks, media, 0, 0
  end
  def exchange(request:, context:)
    owner = self
    body = Object.new
    body.define_singleton_method(:each) do |&block|
      owner.instance_variable_get(:@chunks).each do |chunk|
        owner.instance_variable_set(:@reads, owner.reads + 1)
        block.call(chunk)
      end
    end
    yield WireResponse.new(status: 200, headers: {'Content-Type' => @media}, body: body) { @closed += 1 }
  end
end

raw = "\xEF\xBB\xBF: comment\r\nid: 7\rretry: 00020\r\nevent: delta\ndata: {\"x\":1}\r\ndata: 雪\r\n\r\ndata: [DONE]\n\nid: bad\x00id\nretry: nope\ndata: after\n\ndata: unfinished".b
t = StreamFixture.new(raw.bytes.map { |b| b.chr.b }, 'text/event-stream')
client = Client.new(transport: t)
response = client.events
assert(response.data.is_a?(Enumerator) && response.data.is_a?(ItemStream))
assert(t.reads == 0, 'stream eagerly consumed before iteration')
values = response.data.to_a
assert(values.length == 3)
assert(values[0].data == "{\"x\":1}\n雪" && values[0].id == '7' && values[0].retry_value.token == '20' && values[0].event == 'delta')
assert(values[1].data == '[DONE]' && values[2].data == 'after' && values[2].id == '7')
assert(values[1].event.equal?(UNSET))
assert(t.closed == 1 && response.data.closed?)

jsonl = "{\"value\":9007199254740993.000000000000000001,\"label\":\"雪\"}\r\n{\"value\":1e-400}".b
t = StreamFixture.new(jsonl.bytes.map { |b| b.chr.b }, 'application/x-ndjson')
values = Client.new(transport: t).lines.data.to_a
assert(values.map { |v| v.value.token } == ['9007199254740993.000000000000000001', '1e-400'] && values.first.label == '雪')
assert(t.closed == 1)

t = StreamFixture.new(["data: too-long\n\n"], 'text/event-stream')
reject(ResourceLimitError) { Client.new(transport: t).events(max_response_bytes: 8, max_capture_bytes: 4).data.to_a }
assert(t.closed == 1)
t = StreamFixture.new(['data: ' + 'x' * (1024 * 1024 + 1) + "\n\n"], 'text/event-stream')
reject(ResourceLimitError) { Client.new(transport: t).events.data.to_a }
assert(t.closed == 1)
failure = RuntimeError.new('consumer-owned failure')
t = StreamFixture.new(["data: value\n\n"], 'text/event-stream')
error = reject(RuntimeError) { Client.new(transport: t).events.data.each { raise failure } }
assert(error.equal?(failure) && t.closed == 1)
['{"value":true}\n', "\n", "{\"value\":1,\"value\":2}\n", "\xff\n".b].each do |bad|
  t = StreamFixture.new([bad], 'application/x-ndjson')
  reject(ResponseError) { Client.new(transport: t).lines.data.to_a }
  assert(t.closed == 1)
end
t = StreamFixture.new(["data: \xff\n\n".b], 'text/event-stream')
assert(Client.new(transport: t).events.data.to_a.first.data == "\uFFFD")
t = StreamFixture.new(["data: first\n\n", "data: never-read\n\n"], 'text/event-stream')
Client.new(transport: t).events.data.each { |event| assert(event.data == 'first'); break }
assert(t.closed == 1 && t.reads == 1, 'early break leaked/read ahead')
t = StreamFixture.new(["data: first\n\n", "data: later\n\n"], 'text/event-stream')
stream = Client.new(transport: t).events.data
assert(stream.next.data == 'first')
stream.close
assert(t.closed == 1)

# Real blocked sockets establish that close/cancel/total deadline release the
# native response while no more chunks arrive, including paused consumers.
server = TCPServer.new('127.0.0.1', 0)
arrived, released = Queue.new, Queue.new
threads = []
acceptor = Thread.new do
  loop do
    io = server.accept
    threads << Thread.new(io) do |socket|
      begin
        socket.gets("\r\n")
        while (line = socket.gets("\r\n")) && line != "\r\n"; end
        socket.write("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
        first = "data: first\n\n"
        socket.write("#{first.bytesize.to_s(16)}\r\n#{first}\r\n")
        arrived << true
        # The client should close the socket, not wait for server completion.
        socket.read
        released << true
      rescue IOError, Errno::ECONNRESET, Errno::EPIPE
        released << true
      ensure socket.close
      end
    end
  end
rescue IOError, Errno::EBADF
  nil
end
begin
  client = Client.new(server_url: "http://127.0.0.1:#{server.addr[1]}")
  stream = client.events.data
  ::Timeout.timeout(2) { arrived.pop }
  assert(stream.next.data == 'first')
  stream.close
  ::Timeout.timeout(2) { released.pop }
  token = CancellationToken.new
  stream = client.events(cancellation: token).data
  ::Timeout.timeout(2) { arrived.pop }
  assert(stream.next.data == 'first')
  token.cancel
  reject(CancelledError) { stream.next }
  stream.close
  ::Timeout.timeout(2) { released.pop }
  stream = client.events(timeout: 0.1).data
  ::Timeout.timeout(2) { arrived.pop }
  assert(stream.next.data == 'first')
  ::Timeout.timeout(2) { released.pop }
  reject(TimeoutError) { stream.next }
  stream.close
ensure
  server.close; acceptor.join
  threads.each { |t| t.join(2) || t.kill }
end
puts 'Native SSE/JSON-lines Enumerator framing, exact items, backpressure, close, cancellation and active paused deadlines passed'
