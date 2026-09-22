# frozen_string_literal: true
require 'ruby_document_server'
require 'json'
include RubyDocumentServer
def assert(value, message = 'assertion failed') = (raise message unless value)
origins = JSON.parse(File.read('origins.json'))
contexts = []
provider = lambda do |context|
  contexts << context
  assert(context.url_base == 'effective-server')
  assert(context.server_url == origins['parts'] + '/nested/defs/service/')
  assert(context.source.start_with?(origins['parts'] + '/nested/defs/parts.json#'))
  assert(context.scheme['terminal_resource']['base_uri'] == 'https://logical.example.test/catalog/parts.json')
  if context.name == 'flow'
    flow = context.flows.first
    assert(flow.url_base == 'effective-server' && flow.server_url == context.server_url)
    assert(flow.token_url == '../token' && flow.authorization_url == '../authorize')
    assert(context.metadata_url == './.well-known/authorization-server')
  else
    assert(context.discovery_url == '../.well-known/openid-configuration')
  end
  AuthorizationCredential.new(scheme: 'Fixture', token: 'explicit-token')
end
Client.open(timeout: 3, credential_provider: provider) do |client|
  assert(client.inherited_document.data.data == 'ok')
  assert(client.empty_override.data.data == 'ok')
  assert(client.relative_document(server: 'relative').data.data == 'ok')
  assert(client.relative_document(server: 'relative', document_url: origins['explicit'] + '/other/specs/api.json').data.data == 'ok')
  assert(client.relative_document(server: 'variable', server_variables: {'endpoint' => '../Dir//%2E%2E/%2f/KeEp'}).data.data == 'ok')
  assert(client.oauth_metadata.data.data == 'ok')
  assert(client.oidc_metadata.data.data == 'ok')
end
assert(contexts.length == 2, 'authentication metadata caused additional requests')
puts 'Native physical document bases, redirect identities, explicit overrides, encoded path spelling and effective-server auth metadata passed'
