require 'ruby_native_gate'
client = RubyNativeGate::Client.new
body = RubyNativeGate::Models::ChoicesRequest.new(name: 'native')
client.choices(body: body, content_type: 'application/json')
client.choices(body: RubyNativeGate::Bytes.new("\x00\xff".b), content_type: 'application/octet-stream')
upload = RubyNativeGate::Models::UploadRequest.new(
  file: RubyNativeGate::BytePart.new(bytes: 'file', content_type: 'image/png', headers: {'X-Part-Id' => 'one'}),
  metadata: RubyNativeGate::Models::UploadRequestMetadata.new(title: 'metadata'),
  labels: ['one', 'two']
)
client.upload(body: upload)
client.submit_form(body: RubyNativeGate::Models::SubmitFormRequest.new(id: 'id', enabled: true))
client.positional(body: ['intro', RubyNativeGate::Part.new(data: RubyNativeGate::Models::PositionalRequestItem.new(n: 1), headers: {'X-Order' => 1})])
stream = client.events.data
# @type var data: String
stream.each { |item| data = item.data }
stream.close
client.send_events(body: [RubyNativeGate::Models::Event.new(data: '[DONE]', retry_value: 5)])
client.send_lines(body: [RubyNativeGate::JsonNumber.new('1.0'), 2])
# @type var count: Integer | RubyNativeGate::JsonNumber
count = client.read_headers.typed_headers.x_count
# @type var link: RubyNativeGate::Link?
link = client.read_headers.links['again']
client.security_probe(security: 0)
RubyNativeGate::Client.new(auth: {'basic' => RubyNativeGate::BasicCredential.new(username: 'u', password: 'p')}).security_probe(security: 2)
client.server_probe(server: 'relative', document_url: 'https://example.test/api.json')
client.close
