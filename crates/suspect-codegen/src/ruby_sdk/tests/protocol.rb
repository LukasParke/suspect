# frozen_string_literal: true
require 'ruby_native_gate'
require 'socket'
require 'json'
include RubyNativeGate
def assert(value, message = 'assertion failed') = (raise message unless value)
def reject(type)
  yield
rescue type => error
  error
else
  raise "expected #{type}"
end

# A real socket recording fixture supplies hand-authored empty responses. The
# expected parameter bytes are the independently maintained OAS vectors.
seen = Queue.new
server = TCPServer.new('127.0.0.1', 0)
worker = Thread.new do
  loop do
    io = server.accept
    begin
      verb, target, = io.gets("\r\n").split(' ')
      headers = {}
      while (line = io.gets("\r\n")) && line != "\r\n"
        k, v = line.split(':', 2); headers[k.downcase] = v.strip
      end
      bytes = io.read(headers.fetch('content-length', '0').to_i)
      seen << [verb, target, headers, bytes]
      io.write("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    ensure
      io.close
    end
  end
rescue IOError, Errno::EBADF
  nil
end
begin
  Client.open(server_url: "http://127.0.0.1:#{server.addr[1]}/v1") do |client|
    JSON.parse(File.read('vectors.json')).each_with_index do |binding, index|
      v = binding.fetch('vector')
      value = Codecs.const_get(binding.fetch('codec')).decode(v.fetch('value'))
      client.public_send(binding.fetch('method'), **{binding.fetch('keyword').to_sym => value})
      verb, target, headers, body = seen.pop
      assert(verb == 'GET' && body.empty?)
      case v['parameter']['in']
      when 'path' then assert(target == "/v1/vectors/#{index}/#{v['wire']}", target)
      when 'query' then assert(target == "/v1/vectors/#{index}?#{v['wire']}", target)
      when 'header' then assert(headers[v['parameter']['name'].downcase] == v['wire'], headers.inspect)
      when 'cookie' then assert(headers['cookie'] == v['wire'], headers.inspect)
      end
    end
    JSON.parse(File.read('methods.json')).each do |entry|
      response = client.public_send(entry['call'])
      verb, = seen.pop
      assert(verb == entry['method'], "method was normalized: #{verb}")
      assert(entry['method'] == 'HEAD' ? response.data.equal?(NO_CONTENT) : response.data.is_a?(Bytes))
    end
  end
ensure
  server.close; worker.join
end

class FixtureTransport
  attr_accessor :status, :headers, :body
  attr_reader :requests, :closes
  def initialize(status: 200, headers: {'Content-Type' => 'application/json'}, body: '{"ok":true}')
    @status, @headers, @body, @requests, @closes = status, headers, body, [], 0
  end
  def exchange(request:, context:)
    context.check!; @requests << request
    yield WireResponse.new(status: @status, headers: @headers, body: @body) { @closes += 1 }
  end
end
t = FixtureTransport.new
auth = {'token' => 'caller-token', 'headerKey' => 'header-value', 'queryKey' => 'q +/雪', 'cookieKey' => 'c +/雪', 'basic' => BasicCredential.new(username: 'user', password: 'pass')}
client = Client.new(auth: auth, transport: t)
assert(client.anonymous.data.ok)
assert(!t.requests.last.headers.keys.any? { |k| k.casecmp?('authorization') })
reject(RequestError) { client.security_probe }
client.security_probe(security: 0)
assert(!t.requests.last.headers.key?('Authorization'))
client.security_probe(security: 1)
r = t.requests.last
assert(r.headers['Authorization'] == 'Bearer caller-token' && r.headers['X-Key'] == 'header-value')
assert(r.url == 'https://api.example.test/v1/security?api_key=q%20%2B%2F%E9%9B%AA', r.url)
assert(r.headers['Cookie'] == 'session_key=c%20%2B%2F%E9%9B%AA')
client.security_probe(security: 2)
assert(t.requests.last.headers['Authorization'] == 'Basic dXNlcjpwYXNz')
contexts = []
provider = lambda do |context|
  contexts << context
  AuthorizationCredential.new(scheme: 'CallerScheme', token: 'opaque-value')
end
hooked = Client.new(credential_provider: provider, transport: t)
hooked.security_probe(security: 3)
assert(contexts.last.name == 'oauth' && contexts.last.permissions['kind'] == 'scopes')
assert(contexts.last.metadata['flows'][0]['token_url']['value'] == 'https://auth.example.test/token')
assert(contexts.last.metadata['metadata_url']['value'] == 'https://auth.example.test/metadata')
assert(contexts.last.scopes == ['read:items'] && contexts.last.roles.empty?)
assert(contexts.last.flows.first.token_url == 'https://auth.example.test/token')
hooked.security_probe(security: 4)
assert(contexts.last.metadata['discovery_url']['value'].end_with?('openid-configuration'))
assert(t.requests.last.headers['Authorization'] == 'CallerScheme opaque-value')
assert(contexts.length == 2 && t.requests.none? { |r| r.url.start_with?('https://auth.') }, 'implicit auth request')
reject(RequestError) { Client.new(auth: {'oauth' => 'no-token-type'}, transport: t).security_probe(security: 3) }

reject(RequestError) { client.server_probe }
client.server_probe(server: 'tenant', server_variables: {'tenant' => 'customer', 'port' => '8443', 'basePath' => 'v3'})
assert(t.requests.last.url == 'https://customer.example.test:8443/v3/servers')
client.server_probe(server: 'relative', document_url: 'https://docs.example.test/spec/openapi.json')
assert(t.requests.last.url == 'https://docs.example.test/relative/servers', t.requests.last.url)
reject(RequestError) { client.server_probe(server: 0, server_variables: {'port' => '123'}) }
reject(RequestError) { client.server_probe(server: 0, server_variables: {'typo' => 'x'}) }
reject(RequestError) { client.server_probe(server: 1) }

# Concrete selection cannot choose a wildcard byte path to bypass JSON schema.
json_body = Models::ChoicesRequest.new(name: 'native')
client.choices(body: json_body, content_type: 'Application/JSON; charset=UTF-8')
assert(t.requests.last.body == '{"name":"native"}')
reject(RequestError) { client.choices(body: Bytes.new('not JSON'), content_type: 'application/json') }
reject(RequestError) { client.choices(body: Bytes.new('raw'), content_type: '*/*') }
reject(RequestError) { client.choices(body: Bytes.new('raw')) }
client.choices(body: Bytes.new("\x00\xff\x01".b), content_type: 'image/png')
assert(t.requests.last.body == "\x00\xff\x01".b)
client.choices(body: 'text 雪', content_type: 'text/plain')
assert(t.requests.last.body == 'text 雪'.b)
t.headers = {'Content-Type' => 'application/json;profile=v2; charset=UTF-8'}
assert(client.choices(body: json_body, content_type: 'application/json').data.ok)
t.headers = {'Content-Type' => 'application/problem+json'}; t.body = '{"message":"problem"}'
assert(client.choices(body: json_body, content_type: 'application/json').data.message == 'problem')
t.headers = {'Content-Type' => 'application/pdf'}; t.body = "\x00\xffPDF".b
assert(client.choices(body: json_body, content_type: 'application/json').data.data == t.body)
t.headers = {'Content-Type' => 'text/plain'}; t.body = 'plain'
assert(client.choices(body: json_body, content_type: 'application/json').data == 'plain')
t.status = 201
assert(client.choices(body: json_body, content_type: 'application/json').status == 201)
t.status = 204; t.headers = {}; t.body = 'not JSON and must not be decoded'
assert(client.choices(body: json_body, content_type: 'application/json').data.equal?(NO_CONTENT))
t.status = 500; t.body = "\x00\xff".b
error = reject(ChoicesApiError) { client.choices(body: json_body, content_type: 'application/json') }
assert(error.status == 500 && error.data.data == t.body)
t.status = 201; t.headers = {'Content-Type' => 'application/json'}; t.body = '{"ok":true}'
assert(client.default_probe.status == 201)
t.status = 404
assert(reject(DefaultProbeApiError) { client.default_probe }.status == 404)
t.status = 200; t.headers = {'Content-Type' => 'text/plain'}
reject(ResponseError) { client.default_probe }
t.headers = {}; reject(ResponseError) { client.default_probe }

t.headers = {'Content-Type' => 'application/json', 'X-Count' => '1.0', 'X-List' => ['1,2', '3'], 'X-Map' => 'a=1,b=hello', 'Set-Cookie' => ['a=one; Expires=Wed, 21 Oct 2030 07:28:00 GMT', 'b=two']}
response = client.read_headers
assert(response.typed_headers.x_count.token == '1.0')
assert(response.typed_headers.x_list.map(&:token) == %w[1 2 3])
assert(response.typed_headers.x_map.a.token == '1' && response.typed_headers.x_map.b == 'hello')
assert(response.headers['set-cookie'].length == 2 && response.headers['set-cookie'][0].include?(','))
assert(response.typed_headers.x_optional.equal?(UNSET))
assert(response.links['again'].request_body['$ref'] == 'literal-instance-value')
assert(response.links['again'].parameters['query'] == '$response.header.X-Count')
before = t.requests.length; response.links['again']; assert(t.requests.length == before)
t.headers.delete('X-Count'); reject(ResponseError) { client.read_headers }
t.headers = {'X-Count' => '2'}; t.body = 'forbidden body'
response = client.head_probe; assert(response.data.equal?(NO_CONTENT) && response.typed_headers.x_count.token == '2')
t.headers = {}; t.body = "\x00\xff".b
assert(client.raw_probe.data.data == t.body)

t.status = 204
upload = Models::UploadRequest.new(
  file: BytePart.new(bytes: "\x00\xff".b, filename: 'snow 雪.png', content_type: 'image/png', headers: {'X-Part-Id' => 'p1'}),
  metadata: Models::UploadRequestMetadata.new(title: 'native'), labels: ['one', '雪'],
  files: [BytePart.new(bytes: 'first'), Bytes.new('second')]
)
client.upload(body: upload)
r = t.requests.last
boundary = r.headers.fetch('Content-Type').split('boundary=', 2).last
assert(boundary && r.body.end_with?("--#{boundary}--\r\n"))
assert(r.body.include?("Content-Type: image/png\r\nX-Part-Id: p1\r\n\r\n\x00\xff\r\n".b))
assert(r.body.include?('filename="snow%20%E9%9B%AA.png"'))
assert(r.body.scan('name="files"').length == 2 && r.body.scan('name="labels"').length == 2)
assert(r.body.include?('{"title":"native"}') && !r.body.include?('null'))
reject(ResourceLimitError) { client.choices(body: Bytes.new('x' * 65), content_type: 'application/octet-stream') }
reject(ArgumentError) { Models::UploadRequest.new(metadata: Models::UploadRequestMetadata.new(title: 'x')) }
reject(CodecError) { Models::UploadRequest.new(file: Bytes.new('x'), metadata: Models::UploadRequestMetadata.new(title: 'x')) }
upload.labels = []
reject(RequestError) { client.upload(body: upload) }

# Independently authored multipart response, not the generated encoder's output.
t.status = 200; t.headers = {'Content-Type' => 'multipart/form-data; boundary=independent'}
t.body = "--independent\r\nContent-Disposition: form-data; name=\"file\"; filename=\"out.bin\"\r\nContent-Type: image/png\r\nX-Part-Id: received\r\n\r\n\x00\xff\r\n--independent\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{\"title\":\"received\"}\r\n--independent--\r\n".b
parts = client.read_parts.data
assert(parts.file.data.data == "\x00\xff".b && parts.file.filename == 'out.bin')
assert(parts.file.headers.x_part_id == 'received' && parts.metadata.data.title == 'received')
valid_mime = t.body
t.body = valid_mime.sub('X-Part-Id: received', 'X-Other: absent')
reject(ResponseError) { client.read_parts }
t.body = valid_mime.sub('--independent--', '--independent--garbage')
reject(ResponseError) { client.read_parts }
t.status = 204; t.headers = {}; t.body = ''
upload.labels = ['x']
upload.file = BytePart.new(bytes: 'x', content_type: 'image/gif', headers: {'X-Part-Id' => 'p'})
reject(RequestError) { client.upload(body: upload) }

form = Models::SubmitFormRequest.new(id: 'a + b', enabled: true, address: Models::SubmitFormRequestAddress.new(city: '雪'), tags: ['x', 'y'], codes: [1, 2])
client.submit_form(body: form)
assert(t.requests.last.body == 'address=%7B%22city%22%3A%22%E9%9B%AA%22%7D&codes=1&codes=2&enabled=true&id=a+%2B+b&tags=x&tags=y', t.requests.last.body)
t.status = 200; t.headers = {'Content-Type' => 'application/x-www-form-urlencoded'}; t.body = t.requests.last.body
response = client.read_form
assert(response.data.id == 'a + b' && response.data.address.city == '雪' && response.data.codes.map(&:token) == %w[1 2])
t.headers = {}; t.body = ''
client.whole_query(complete: Models::WholeQueryCompleteParameter.new(a: 1))
assert(t.requests.last.url.end_with?('/whole-query?%7B%22a%22%3A1%7D'))
client.whole_form(complete: Models::WholeFormCompleteParameter_2.new(foo: 'a + b', bar: true, items: [1, 2]))
assert(t.requests.last.url.end_with?('/whole-form?bar=true&foo=a+%2B+b&items=1&items=2'), t.requests.last.url)

t.status = 204
client.send_lines(body: [JsonNumber.new('1.0'), 2])
assert(t.requests.last.body == "1.0\n2\n")
client.send_events(body: [Models::Event.new(data: "one\ntwo", id: 'event1', retry_value: 20)])
assert(t.requests.last.body == "id: event1\nretry: 20\ndata: one\ndata: two\n\n")
client.positional(body: ['intro', Part.new(data: Models::PositionalRequestItem.new(n: 2), headers: {'X-Order' => 1})])
assert(!t.requests.last.body.include?('Content-Disposition') && t.requests.last.body.include?("X-Order: 1\r\n"))
assert(t.requests.length == t.closes)
reject(RequestError) { client.vector21(q: 'one&admin=true') }
reject(RequestError) { client.vector23(x_tag: "ok\r\nInjected: yes") }
puts 'Expanded protocol: 30 literal parameter vectors, exact method tokens, security/servers, status/media, headers/links, forms, byte parts and request streams passed'
