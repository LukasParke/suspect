require 'ruby_schema_v2'
settings = RubySchemaV2::Models::Settings.new(mode: 'simple', name: 'native', note: nil,
  extra_fields: {'x-count' => RubySchemaV2::JsonNumber.new('1.0')})
settings.note = RubySchemaV2::UNSET
settings.extra_fields['x-rate'] = 100
RubySchemaV2::Codecs::Settings.encode_json(settings)
client = RubySchemaV2::Client.new
# @type var response: RubySchemaV2::Models::Settings
response = client.save_settings(body: settings).data
# @type var name: String
name = response.name
client.check_batch(body: ['head', 1, 'tail'])
client.close
