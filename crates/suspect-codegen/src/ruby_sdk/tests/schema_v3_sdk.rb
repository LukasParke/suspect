# frozen_string_literal: true
require 'ruby_schema_v3'
require 'socket'
include RubySchemaV3
def assert(value, message = 'assertion failed') = (raise message unless value)
def rejects(type)
  yield
rescue type => error
  error
else
  raise "expected #{type}"
end

tree_json = '{"amount":9007199254740993.000000000000000001,"children":[{"amount":1e-400,"label":"leaf"}],"label":"root","note":null}'
tree = Codecs::Strict.decode_json(tree_json)
assert(tree.instance_of?(Models::Tree) && tree.children.first.instance_of?(Hash))
assert(tree.amount.token == '9007199254740993.000000000000000001' && tree.note.nil?)
assert(tree.children.first['amount'].token == '1e-400')
assert(Codecs::Strict.encode_json(tree) == tree_json)
tree.note = UNSET
assert(!Codecs::Strict.encode_json(tree).include?('"note"'))
tree.note = nil
tree.children.first['unclaimed'] = true
error = rejects(ValidationError) { Codecs::Strict.encode(tree) }
assert(error.source == 'https://physical.ruby.test/api.json#/components/schemas/Strict/unevaluatedProperties', error.source)
assert(error.instance_path == '/children/0/unclaimed')
Codecs::Tree.encode(tree) # Same native carrier, different rooted resource scope.
tree.children.first.delete('unclaimed')
Codecs::Strict.encode(tree)
assert(Internal::PROGRAM['resourceContext']['resources'].any? { |r| r['canonicalUri'] == 'urn:ruby:inert' })
assert(!Codecs.const_defined?(:Inert, false), 'unentered candidates became standalone native carriers')

# The nested model, evaluated alone, uses the string fallback. Its enclosing
# source root enters an integer override. Codec conversion must not re-trial the
# nested fallback or union without the caller's resource environment.
rejects(ValidationError) { Models::Choice.new(value: 7) }
choice = Models::Choice.new(value: 'standalone')
choice.value = 7
outer = Models::Outer.new(choice: choice, union: 8)
outer_json = '{"choice":{"value":7},"union":8}'
assert(Codecs::Outer.encode_json(outer) == outer_json)
decoded = Codecs::Outer.decode_json(outer_json)
assert(decoded.choice.instance_of?(Models::Choice) && decoded.choice.value.token == '7')
assert(decoded.union.token == '8')
decoded.union = true
Codecs::Outer.encode(decoded)
decoded.choice.value = 'wrong fallback'
error = rejects(ValidationError) { Codecs::Outer.encode(decoded) }
assert(error.source.end_with?('/Outer/$defs/IntegerBinding/type') && error.instance_path == '/choice/value')
rejects(CodecError) { Codecs::Outer.decode({'choice' => {'value' => 0.1}}) }
decoded.choice.value = 7
decoded.choice.value = UNSET
rejects(CodecError) { Codecs::Outer.encode(decoded) }
decoded.choice.value = 7

named = Codecs::Named.decode_json('{"name":"native","a/b~😀%":"escaped"}')
assert(named.instance_of?(Models::Named))
assert(Codecs::Named.encode(named)['a/b~😀%'] == 'escaped')
assert(Internal::PROGRAM['nodes'].all? { |node| node['source']['document'] == 'https://physical.ruby.test/api.json' })
assert(Internal::PROGRAM['resourceContext']['nodeScopes'].any? { |scope| scope[2].include?('a~1b~0%F0%9F%98%80%25') })

# A bounded independent loopback echoes real bytes, with explicit malformed
# responses. It never evaluates schemas or fetches logical resource identifiers.
server = TCPServer.new('127.0.0.1', 0)
seen, replies = Queue.new, Queue.new
worker = Thread.new do
  loop do
    io = server.accept
    begin
      line = io.gets("\r\n")
      headers = {}
      while (header = io.gets("\r\n")) && header != "\r\n"
        key, value = header.split(':', 2); headers[key.downcase] = value.strip
      end
      bytes = io.read(headers.fetch('content-length', '0').to_i)
      seen << [line.strip, bytes]
      response = replies.empty? ? bytes : replies.pop
      io.write("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: #{response.bytesize}\r\nConnection: close\r\n\r\n")
      io.write(response)
    ensure
      io.close
    end
  end
rescue IOError, Errno::EBADF
  nil
end
begin
  Client.open(server_url: "http://127.0.0.1:#{server.addr[1]}/v3") do |client|
    returned = client.save_tree(body: tree).data
    assert(returned.instance_of?(Models::Tree) && returned.children.first['amount'].token == '1e-400')
    assert(seen.pop == ['POST /v3/tree HTTP/1.1', tree_json])
    assert(client.choose(body: outer).data.choice.value.token == '7')
    assert(seen.pop == ['POST /v3/choice HTTP/1.1', outer_json])
    assert(client.identify(body: named).data.name == 'native')
    assert(seen.pop == ['POST /v3/named HTTP/1.1', '{"a/b~😀%":"escaped","name":"native"}'.b])
    tree.children.first['unclaimed'] = nil
    rejects(RequestError) { client.save_tree(body: tree) }
    assert(seen.empty?, 'invalid dynamic request reached transport')
    tree.children.first.delete('unclaimed')
    replies << '{"label":"root","children":[{"label":"leaf","unclaimed":true}]}'
    error = rejects(ResponseError) { client.save_tree(body: tree) }
    assert(error.cause.instance_of?(ValidationError) && error.cause.instance_path == '/children/0/unclaimed')
    seen.pop
    replies << 'null'
    error = rejects(ResponseError) { client.loop_failure }
    assert(error.cause.instance_of?(EvaluationFailure), 'dynamic cycle was treated as invalidity')
    assert(error.cause.source.end_with?('/components/schemas/Loop'))
    assert(seen.pop == ['GET /v3/loop HTTP/1.1', ''])
  end
ensure
  server.close
  worker.join
end
puts 'Resource-scoped SDK: named keyword models, exact JSON dynamic sites, outer overrides, restored codec scope, physical findings, and real request/response controls passed'
