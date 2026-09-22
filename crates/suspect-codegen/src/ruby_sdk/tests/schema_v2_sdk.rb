# frozen_string_literal: true
require 'ruby_schema_v2'
require 'socket'
include RubySchemaV2
def assert(value, message = 'assertion failed') = (raise message unless value)
def rejects(type)
  yield
rescue type => error
  error
else
  raise "expected #{type}"
end

settings = Models::Settings.new(mode: 'advanced', name: 'native', note: nil,
  extra_fields: {'x-count' => JsonNumber.new('1.0'), 'x-rate' => JsonNumber.new('9007199254740993.000000000000000001')})
wire = Codecs::Settings.encode_json(settings)
assert(wire == '{"mode":"advanced","name":"native","note":null,"x-count":1.0,"x-rate":9007199254740993.000000000000000001}')
decoded = Codecs::Settings.decode_json(wire)
assert(decoded.extra_fields['x-count'].token == '1.0' && decoded.note.nil? && decoded.peer.equal?(UNSET))
decoded.note = UNSET
assert(!Codecs::Settings.encode_json(decoded).include?('"note"'))
decoded.extra_fields['x-count'] = JsonNumber.new('1.5')
error = rejects(ValidationError) { Codecs::Settings.encode_json(decoded) }
assert(error.source.end_with?('/patternProperties/count$/type') && error.instance_path == '/x-count')
decoded.extra_fields['x-count'] = 2
decoded.extra_fields['plain'] = 1
rejects(ValidationError) { Codecs::Settings.encode(decoded) }
decoded.extra_fields.delete('plain')
decoded.extra_fields['x-count'] = true
rejects(ValidationError) { Codecs::Settings.encode(decoded) }
decoded.extra_fields['x-count'] = 2
decoded.trigger = nil
rejects(ValidationError) { Codecs::Settings.encode(decoded) }
decoded.peer = 'present'
Codecs::Settings.encode(decoded)
decoded.mode = 'simple'; decoded.extra_fields.delete('x-count')
Codecs::Settings.encode(decoded)
decoded.mode = 'advanced'
rejects(ValidationError) { Codecs::Settings.encode(decoded) }
rejects(ValidationError) { Codecs::Settings.decode_json('{"mode":"simple","name":"n","x-😀":1}') }

batch = Codecs::Batch.decode_json('["head",1.0,"tail"]')
assert(batch[1].token == '1.0')
batch << false
rejects(ValidationError) { Codecs::Batch.encode(batch) }
rejects(ValidationError) { Codecs::Batch.decode_json('["head",1,2,3]') }
nested = Codecs::Nested.decode_json('{"outer":{"inner":1e-400}}')
assert(nested.outer.inner.token == '1e-400')
nested.extra_fields['inner'] = 1
rejects(ValidationError) { Codecs::Nested.encode(nested) }
nested.extra_fields.clear
nested.outer.extra_fields['unknown'] = nil
rejects(ValidationError) { Codecs::Nested.encode(nested) }
either = Codecs::Either.decode_json('{"a":"first","b":1}')
assert(Codecs::Either.encode(either).keys.sort == %w[a b])
either.extra_fields['unknown'] = true
rejects(ValidationError) { Codecs::Either.encode(either) }

# A strict independent loopback echoes actual request bytes, never generated
# example response values. Mutations must fail before this server sees a call.
server = TCPServer.new('127.0.0.1', 0)
seen = Queue.new
worker = Thread.new do
  loop do
    io = server.accept
    begin
      verb, path, = io.gets("\r\n").split(' ')
      headers = {}
      while (line = io.gets("\r\n")) && line != "\r\n"
        key, value = line.split(':', 2); headers[key.downcase] = value.strip
      end
      bytes = io.read(headers.fetch('content-length').to_i)
      seen << [verb, path, bytes]
      io.write("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: #{bytes.bytesize}\r\nConnection: close\r\n\r\n")
      io.write(bytes)
    ensure io.close
    end
  end
rescue IOError, Errno::EBADF
  nil
end
begin
  Client.open(server_url: "http://127.0.0.1:#{server.addr[1]}/v1") do |client|
    returned = client.save_settings(body: settings).data
    assert(returned.name == 'native' && returned.extra_fields['x-rate'].token == '9007199254740993.000000000000000001')
    assert(seen.pop == ['POST', '/v1/settings', wire])
    assert(client.check_batch(body: ['head', 1, 'tail']).data[1].token == '1')
    assert(seen.pop == ['POST', '/v1/batch', '["head",1,"tail"]'])
    settings.extra_fields['x-count'] = JsonNumber.new('0.5')
    rejects(RequestError) { client.save_settings(body: settings) }
    assert(seen.empty?)
  end
ensure
  server.close; worker.join
end
puts 'Scoped SDK keyword models, pattern extras, mutable codecs, exact values, presence and real HTTP passed'
