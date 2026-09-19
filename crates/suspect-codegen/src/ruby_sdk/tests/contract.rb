# frozen_string_literal: true
require 'ruby_native_gate'
require 'json'
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

fixture = JSON.parse(File.read('runtime-contract-v1.json'))
assert(fixture['version'] == 'suspect.runtime-contract.v1' && fixture['cases'].length == 17)
count = 0
fixture['cases'].each_with_index do |entry, index|
  codec = Codecs.const_get("Case#{index}")
  entry['valid'].each do |text|
    value = codec.decode_json(text)
    codec.decode_json(codec.encode_json(value))
    count += 1
  end
  entry['invalid'].each do |text|
    error = reject(ValidationError) { codec.decode_json(text) }
    assert(error.source.include?('#/components/schemas/'), "unbound #{entry['name']}")
    count += 1
  end
end

%w[- 01 1. 1e 1e+ +1 NaN --1 Infinity 00 1e--1].each { |token| reject(JsonError) { JsonNumber.new(token) } }
['{}true', '[1,]', '{"a":1,"\u0061":2}', '"\ud800"', '"\udc00"', "\"\n\"", 'true false', '/*x*/null', '[true false]', '{"x":}', '\ufeffnull'].each { |text| reject(JsonError) { Json.parse(text) } }
["\xc0\xaf".b, "\xed\xa0\x80".b, "\xf4\x90\x80\x80".b, "\x80".b].each { |text| reject(JsonError) { Json.parse(text) } }
assert(Json.parse('"\uD83D\uDE00"') == '😀')
assert(Json.dump({'😀' => "e\u0301"}).encoding == Encoding::UTF_8)
assert(Json.parse('"\u0000"') == "\0")
assert(Json.parse('1e999999999999999999999999999999999999999999').token == '1e999999999999999999999999999999999999999999')
assert(JsonNumber.new('1e000000000000000000000000000000000002').to_i == 100)
assert(JsonNumber.new('1e-000000000000000000000000000000000002').integer? == false)
assert(JsonNumber.new('100e-2') == JsonNumber.new('1.00'))
assert(JsonNumber.new('100e-2').hash == JsonNumber.new('1.00').hash)
assert(JsonNumber.new('-0').hash == JsonNumber.new('0e999999999999999999999').hash)
huge = JsonNumber.new('1e999999999999999999999999999999999999999999')
assert(huge.integer? && huge > JsonNumber.new('9e999999999999999999999999999999999999999998'))
reject(RangeError) { huge.to_i }
reject(RangeError) { JsonNumber.new('1.1').to_i }
reject(RangeError) { huge.to_f }
reject(JsonError) { JsonNumber.new('1' * 4097) }
reject(JsonError) { Json.dump(1.0) }
reject(JsonError) { Json.dump({a: true}) }
reject(JsonError) { Json.dump(UNSET) }
reject(JsonError) { Json.dump("\xff".b) }
reject(JsonError) { Json.dump('abc'.encode('UTF-16LE')) }
reject(JsonError) { Json.parse('[]', max_bytes: 1) }
reject(JsonError) { Json.dump({'x' => 'a' * 100}, max_bytes: 20) }
reject(JsonError) { Json.parse(' ' * 100 + 'null', max_work: 100) }
reject(JsonError) { Json.dump({'x' => 'a' * 100}, max_work: 50) }
reject(JsonError) { Json.parse('[' * 129 + '0' + ']' * 129) }
cycle = []; cycle << cycle
reject(JsonError) { Json.dump(cycle) }
assert(Json.parse(Json.dump([1, true, false, nil])).map { |v| v.is_a?(JsonNumber) ? v.token : v } == ['1', true, false, nil])

%w[1e0000000000000000000000000000000000000002 100e-0000000000000000000000000000000000000002].each do |text|
  assert(Codecs::Padded.decode_json(text).integer?)
end
reject(ValidationError) { Codecs::Padded.decode_json('1e-0000000000000000000000000000000000000002') }
tree = Models::Tree.new(name: 'root', nullable: nil, optional_null: nil, extra_fields: {'count' => JsonNumber.new('1.0')})
assert(tree.optional.equal?(UNSET) && tree.optional_null.nil?)
assert(tree.to_h.keys.sort == %w[count name nullable optionalNull])
tree.child = Models::Tree.new(name: 'child', nullable: 'present')
tree.child.optional = nil
reject(CodecError) { Codecs::Tree.encode(tree) }
tree.child.optional = UNSET
tree.child.child = tree
reject(CodecError) { Codecs::Tree.encode(tree) }
tree.child = UNSET
tree.extra_fields['count'] = true
reject(CodecError) { Codecs::Tree.encode(tree) }
tree.extra_fields['count'] = 1
assert(Codecs::Tree.decode_json(Codecs::Tree.encode_json(tree)).nullable.nil?)
tree.name = UNSET
reject(CodecError) { Codecs::Tree.encode(tree) }

arm = Models::ArmA.new(a: 'A')
assert(Codecs::Either.decode_json(Codecs::Either.encode_json(arm)).instance_of?(Models::ArmA))
arm.tag = 'b'
arm.extra_fields['b'] = 'B'
reject(ValidationError) { Codecs::Either.encode(arm) }
reject(ValidationError) { Codecs::Sibling.encode('x') }
reject(ValidationError) { Codecs::Sibling.encode(3) }
assert(Codecs::Sibling.encode(2) == 2)
reject(EvaluationFailure) { Codecs::Loop.decode_json('null') }
reject(EvaluationFailure) { Codecs::Loop.encode(nil) }
reject(ValidationError) { Codecs::LiteralMix.decode_json('false') }
assert(Codecs::LiteralMix.decode_json('true').equal?(true))
assert(Codecs::LiteralMix.decode_json('1.00').token == '1.00')
assert(Codecs::Sparse.encode_json(Models::Sparse.new) == '{}')
assert(Codecs::Sparse.encode_json(Models::Sparse.new(empty: nil)) == '{"empty":null}')

reserved_json = '{"class":"x","hash":1,"a-b":"dash","a_b":"underscore","雪":"snow","extra_fields":"extra","schema_index":"schema","validate!":"validate","to_json":"json","#{raise \'injected\'}":"inert"}'
reserved = Codecs::Reserved.decode_json(reserved_json)
assert(reserved.class_value == 'x' && reserved.hash_value.token == '1')
assert(reserved.a_b == 'dash' && reserved.a_b_2 == 'underscore')
assert(reserved.to_json_value == 'json' && reserved.validate_value == 'validate')
assert(Json.parse(Codecs::Reserved.encode_json(reserved)) == Json.parse(reserved_json))
assert(!Object.const_defined?(:Injected))
assert(Client.instance_methods(false).include?(:raise_value))
assert(Client.instance_methods(false).include?(:lambda_value))
assert(!Client.instance_methods(false).include?(:raise))
puts "Shared runtime contract passed: 17 schemas / #{count} values; exact JSON, recursion, literal/union/presence/name adversarial gates passed"
