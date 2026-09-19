# frozen_string_literal: true
require 'ruby_schema_v3'
tree = RubySchemaV3::Models::Tree.new(label: 'root', note: nil, children: [{'label' => 'leaf'}], amount: RubySchemaV3::JsonNumber.new('1e-400'))
tree.note = RubySchemaV3::UNSET
tree.note = 'present'
RubySchemaV3::Codecs::Strict.encode_json(tree)
choice = RubySchemaV3::Models::Choice.new(value: 'standalone')
choice.value = 7
outer = RubySchemaV3::Models::Outer.new(choice: choice, union: true)
RubySchemaV3::Codecs::Outer.encode_json(outer)
RubySchemaV3::Client.new.save_tree(body: tree)
RubySchemaV3::Client.new.choose(body: outer)
RubySchemaV3::Client.new.identify(body: RubySchemaV3::Codecs::Named.decode_json('{"name":"native","a/b~😀%":"escaped"}'))
