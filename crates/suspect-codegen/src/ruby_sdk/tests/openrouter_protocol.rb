# frozen_string_literal: true
require 'ruby_native_gate'
require 'socket'
include RubyNativeGate
def assert(value, message = 'assertion failed') = (raise message unless value)
octets = (0..255).to_a.pack('C*')
seen = []
server = TCPServer.new('127.0.0.1', 0)
worker = Thread.new do
  3.times do
    io = server.accept
    begin
      method, path, = io.gets("\r\n").split(' ')
      headers = {}
      while (line = io.gets("\r\n")) && line != "\r\n"
        k, v = line.split(':', 2); headers[k.downcase] = v.strip
      end
      body = io.read(headers.fetch('content-length', '0').to_i)
      seen << [method, path, headers['authorization'], body]
      reply = method == 'DELETE' ? '{"deleted":true}' : octets
      media = method == 'DELETE' ? 'application/json' : 'application/octet-stream'
      io.write("HTTP/1.1 200 OK\r\nContent-Type: #{media}\r\nContent-Length: #{reply.bytesize}\r\nConnection: close\r\n\r\n")
      io.write(reply)
    ensure io.close
    end
  end
end
begin
  Client.open(auth: {'apiKey' => 'management-test'}, server_url: "http://127.0.0.1:#{server.addr[1]}/api/v1") do |client|
    container = client.download_container_file_content(container_id: 'sess_abc', file_id: 'a/b 雪')
    assert(container.data.instance_of?(Bytes) && container.data.data == octets)
    file = client.download_file_content(file_id: 'or_file_1', workspace_id: '00000000-0000-4000-8000-000000000000', provider: 'openai')
    assert(file.data.data == octets && file.status == 200)
    deleted = client.delete_keys(hash_value: 'a/b 雪')
    assert(deleted.data.deleted.equal?(true))
  end
  assert(seen == [
    ['GET', '/api/v1/containers/sess_abc/files/a%2Fb%20%E9%9B%AA/content', 'Bearer management-test', ''],
    ['GET', '/api/v1/files/or_file_1/content?workspace_id=00000000-0000-4000-8000-000000000000&provider=openai', 'Bearer management-test', ''],
    ['DELETE', '/api/v1/keys/a%2Fb%20%E9%9B%AA', 'Bearer management-test', '']
  ], seen.inspect)
ensure
  server.close; worker.join(2) || worker.kill
end
puts 'Three additional actual OpenRouter operations passed: two explicit-profile binary downloads and DELETE; all 256 byte values preserved'
