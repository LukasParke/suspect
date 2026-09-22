require 'ruby_native_gate'

input = RubyNativeGate::Models::WidgetInput.new(name: 'alpha')
input.amount = RubyNativeGate::JsonNumber.new('9007199254740993.000000000000000001')
patch = RubyNativeGate::Models::WidgetPatch.new
patch.amount = RubyNativeGate::UNSET
payload = RubyNativeGate::Models::StandardPayload.new(text: 'plain')
widget = RubyNativeGate::Models::Widget.new(id: 'w1', amount: 1, payload: payload, meta: nil)
widget.meta = RubyNativeGate::UNSET
RubyNativeGate::Codecs::Widget.encode_json(widget)
decoded = RubyNativeGate::Codecs::Widget.decode_json('{"id":"w1","amount":1,"payload":{"kind":"standard","text":"plain"}}')
# @type var id: String
id = decoded.id

client = RubyNativeGate::Client.new(auth: {'apiKey' => 'supplied-token'})
response = client.create_widget(body: input)
# @type var data: RubyNativeGate::Models::Widget
data = response.data
client.update_widget(widget_id: 'w1', body: patch)
client.list_widgets(limit: 2, tags: ['one', 'two'])
client.get_widget(widget_id: 'w1', cancellation: RubyNativeGate::CancellationToken.new, timeout: 10)
client.close
