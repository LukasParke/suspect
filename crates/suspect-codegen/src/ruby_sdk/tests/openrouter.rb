# frozen_string_literal: true
require 'ruby_native_gate'
require 'socket'
require 'json'
include RubyNativeGate
def assert(value, message = 'assertion failed')
  raise message unless value
end

fixtures = JSON.parse(File.read('responses.json')) # independently hand-authored response bytes
seen, errors = [], Queue.new
server = TCPServer.new('127.0.0.1', 0)
worker = Thread.new do
  5.times do
    io = server.accept
    begin
      method, target, = io.gets("\r\n").split(' ')
      headers = {}
      while (line = io.gets("\r\n")) && line != "\r\n"
        key, value = line.split(':', 2)
        headers[key.downcase] = value.strip
      end
      body = io.read(headers.fetch('content-length', '0').to_i)
      seen << [method, target, headers['authorization'], body]
      key = method == 'POST' ? 'create' : method == 'PATCH' ? 'update' : target.end_with?('/credits') ? 'credits' : target.include?('?') ? 'list' : 'file'
      raw = fixtures.fetch(key)
      status = key == 'create' ? 201 : 200
      io.write("HTTP/1.1 #{status} OK\r\nContent-Type: application/json\r\nContent-Length: #{raw.bytesize}\r\nConnection: close\r\n\r\n#{raw}")
    rescue StandardError => error
      errors << error
    ensure
      io.close
    end
  end
end

begin
  Client.open(auth: {'apiKey' => 'test-key'}, server_url: "http://127.0.0.1:#{server.addr[1]}/api/v1") do |client|
    credits = client.get_credits
    assert(credits.data.data.total_credits.token == '100.50000000000000001')
    created = client.create_keys(body: Models::__CREATE__.new(name: 'Native Test Key', limit: JsonNumber.new('50.25'), limit_reset: nil))
    assert(created.status == 201 && created.data.data.limit.token == '50.250')
    assert(created.data.key == 'fixture-secret' && !created.inspect.include?('fixture-secret'))
    updated = client.update_keys(hash_value: 'fixture-hash', body: Models::__UPDATE__.new(name: 'Updated Native Key', limit: JsonNumber.new('75.50'), limit_reset: nil, disabled: true))
    assert(updated.data.data.limit.token == '75.50' && updated.data.data.disabled.equal?(true))
    listed = client.list_container_files(container_id: 'sess_abc123', limit: 2, after: 'a/b 雪')
    assert(listed.data.has_more.equal?(false))
    file = client.get_container_file(container_id: 'sess_abc123', file_id: 'a/b 雪')
    assert(file.data.bytes.token == '123' && file.data.object == 'container.file')
  end
  assert(seen == [
    ['GET', '/api/v1/credits', 'Bearer test-key', ''],
    ['POST', '/api/v1/keys', 'Bearer test-key', '{"limit":50.25,"limit_reset":null,"name":"Native Test Key"}'],
    ['PATCH', '/api/v1/keys/fixture-hash', 'Bearer test-key', '{"disabled":true,"limit":75.50,"limit_reset":null,"name":"Updated Native Key"}'],
    ['GET', '/api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%E9%9B%AA', 'Bearer test-key', ''],
    ['GET', '/api/v1/containers/sess_abc123/files/a%2Fb%20%E9%9B%AA', 'Bearer test-key', '']
  ], seen.inspect)
ensure
  server.close
  worker.join(2) || worker.kill
end
raise errors.pop unless errors.empty?
puts 'Five actual OpenRouter operations passed independent installed-gem wire and exact-response checks'
