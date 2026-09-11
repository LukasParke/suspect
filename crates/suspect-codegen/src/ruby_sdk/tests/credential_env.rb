# frozen_string_literal: true
def assert(value, message = 'credential-env assertion failed') = (raise message unless value)
def rejects(type)
  yield
rescue type => error
  error
else
  raise "expected #{type}"
end

module EnvProbe
  VARIABLES = %w[RUBY_ENV_BEARER RUBY_ENV_OTHER RUBY_ENV_HEADER RUBY_ENV_QUERY RUBY_ENV_COOKIE].freeze
  class << self
    attr_accessor :mode, :reads, :failing_variable
  end
  self.mode, self.reads = :normal, []
end
ENV.singleton_class.prepend(Module.new do
  def [](key)
    if EnvProbe::VARIABLES.include?(key)
      EnvProbe.reads << key
      if EnvProbe.mode == :unavailable || EnvProbe.mode == :partial && key == EnvProbe.failing_variable
        raise SecurityError, 'reader-message-secret-canary'
      end
    end
    super
  end
end)

require 'openrouter'
assert(EnvProbe.reads.empty?, 'import read credential values')
assert(Gem.loaded_specs.fetch('openrouter').full_gem_path.start_with?(File.expand_path('installed')), 'loaded another package')

class ControlledTransport
  attr_reader :requests
  def initialize = (@requests = [])
  def exchange(request:, context:)
    context.check!
    raise 'unexpected server or method' unless request.method == 'GET' && request.url.start_with?('https://credentials.ruby.test/api/v1/')
    @requests << request
    yield OpenRouter::WireResponse.new(status: 204, headers: {}, body: '')
  end
end

def reset_env
  EnvProbe.mode = :normal
  EnvProbe::VARIABLES.each { |variable| ENV.delete(variable) }
  EnvProbe.reads.clear
end
def secret_free(error)
  while error
    assert(!error.message.include?('reader-message-secret-canary'))
    assert(!error.message.include?('env-first-token'))
    assert(!error.message.include?('explicit-token'))
    error = error.cause
  end
end
def missing(client, transport, method = :use_bearer, **options)
  count = transport.requests.length
  error = rejects(OpenRouter::RequestError) { client.public_send(method, **options) }
  secret_free(error)
  assert(transport.requests.length == count, 'missing credential reached HTTP')
  error
end

reset_env
ENV['RUBY_ENV_BEARER'] = 'env-first-token'
ENV['RUBY_ENV_OTHER'] = 'unused invalid bearer'
ENV['RUBY_ENV_HEADER'] = 'env-header-key'
ENV['RUBY_ENV_QUERY'] = 'query key/+%'
ENV['RUBY_ENV_COOKIE'] = 'cookie/key'
transport = ControlledTransport.new
client = OpenRouter::Client.new(transport: transport, timeout: 2)
assert(EnvProbe.reads.sort == EnvProbe::VARIABLES.sort, 'creation must read each mapped variable exactly once')
creation_reads = EnvProbe.reads.dup
assert(client.use_bearer.status == 204)
assert(transport.requests.last.url == 'https://credentials.ruby.test/api/v1/bearer')
assert(transport.requests.last.headers['Authorization'] == 'Bearer env-first-token')
missing(client, transport, :use_other_bearer)
client.use_header_key
assert(transport.requests.last.headers['X-API-Key'] == 'env-header-key')
client.use_query_key
assert(transport.requests.last.url == 'https://credentials.ruby.test/api/v1/query?api_key=query%20key%2F%2B%25')
client.use_cookie_key
assert(transport.requests.last.headers['Cookie'] == 'session=cookie%2Fkey')
client.use_both
assert(transport.requests.last.headers['Authorization'] == 'Bearer env-first-token' && transport.requests.last.headers['X-API-Key'] == 'env-header-key')
missing(client, transport, :choose_credential)
client.choose_credential(security: 1)
assert(transport.requests.last.headers['X-API-Key'] == 'env-header-key' && !transport.requests.last.headers.key?('Authorization'))
client.choose_credential(security: 2)
assert(!transport.requests.last.headers.key?('Authorization') && !transport.requests.last.headers.key?('X-API-Key'))
client.snapshot
client.env_bindings
assert(EnvProbe.reads == creation_reads, 'request-time environment lookup')
ENV['RUBY_ENV_BEARER'] = 'env-new-token'
client.use_bearer
assert(transport.requests.last.headers['Authorization'] == 'Bearer env-first-token', 'existing client did not snapshot')
new_client = OpenRouter::Client.new(transport: transport)
new_client.use_bearer
assert(transport.requests.last.headers['Authorization'] == 'Bearer env-new-token')
client.close; new_client.close

# Entire explicit argument precedence: all cases must bypass the environment,
# including values rejected by the established explicit constructor.
EnvProbe.reads.clear
explicit = OpenRouter::Client.new(auth: {'apiKey' => 'explicit-token'}, transport: transport)
explicit.use_bearer
assert(transport.requests.last.headers['Authorization'] == 'Bearer explicit-token')
explicit.close
empty = OpenRouter::Client.new(auth: {}, transport: transport)
empty.snapshot
missing(empty, transport)
empty.close
partial = OpenRouter::Client.new(auth: {'headerKey' => 'explicit-header'}, transport: transport)
partial.use_header_key
missing(partial, transport, :use_both)
partial.close
OpenRouter::Client.open(auth: {'headerKey' => ''}, transport: transport) do |open|
  open.use_header_key
  assert(transport.requests.last.headers['X-API-Key'] == '')
end
OpenRouter::Client.open(auth: {'headerKey' => nil}, transport: transport) do |open|
  missing(open, transport, :use_header_key)
end
[nil, OpenRouter::UNSET, {'apiKey' => nil}, {'apiKey' => ''}].each do |auth|
  secret_free(rejects(ArgumentError) { OpenRouter::Client.new(auth: auth, transport: transport) })
end
OpenRouter::Client.open(auth: {}, transport: transport) { |open| missing(open, transport) }
secret_free(rejects(ArgumentError) { OpenRouter::Client.open(auth: nil, transport: transport) {} })
assert(EnvProbe.reads.empty?, 'explicit auth read environment values')

# Client.open uses the same omitted-auth creation snapshot and normal lifetime.
opened = nil
value = OpenRouter::Client.open(transport: transport) do |open|
  opened = open
  ENV['RUBY_ENV_BEARER'] = 'env-later-token'
  open.use_bearer
  assert(transport.requests.last.headers['Authorization'] == 'Bearer env-new-token')
  :block_result
end
assert(value == :block_result && opened.closed?)

[:missing, :empty, :unavailable, :invalid, :oversized].each do |state|
  reset_env
  if state == :empty
    EnvProbe::VARIABLES.each { |variable| ENV[variable] = '' }
  elsif state == :unavailable
    EnvProbe.mode = :unavailable
  elsif state == :invalid
    ENV['RUBY_ENV_BEARER'] = "bad token\nreader-message-secret-canary"
    ENV['RUBY_ENV_HEADER'] = "bad\r\nheader"
    ENV['RUBY_ENV_QUERY'] = "\xff".b
  elsif state == :oversized
    ENV['RUBY_ENV_BEARER'] = 'x' * (OpenRouter::Internal::POLICY[:max_header_bytes] + 1)
  end
  candidate = OpenRouter::Client.new(transport: transport)
  candidate.snapshot
  candidate.choose_credential(security: 2)
  missing(candidate, transport)
  missing(candidate, transport, :use_both)
  missing(candidate, transport, :use_query_key) if state == :invalid
  candidate.close
end

reset_env
ENV['RUBY_ENV_BEARER'] = 'available-alternative'
EnvProbe.mode, EnvProbe.failing_variable = :partial, 'RUBY_ENV_HEADER'
partial = OpenRouter::Client.new(transport: transport)
partial.choose_credential(security: 0)
assert(transport.requests.last.headers['Authorization'] == 'Bearer available-alternative')
missing(partial, transport, :use_both)
partial.close

# Ruby's environment object itself may be absent in a controlled host. The SDK
# still constructs a mixed client and resolves anonymous operations normally.
environment = Object.send(:remove_const, :ENV)
begin
  portable = OpenRouter::Client.new(transport: transport)
  portable.snapshot
  missing(portable, transport)
  portable.close
ensure
  Object.const_set(:ENV, environment)
end
reset_env

# Existing explicit provider behavior and real constructor names remain usable.
provider = ->(context) { context.name == 'apiKey' ? 'provider-token' : nil }
OpenRouter::Client.open(auth: {}, credential_provider: provider, transport: transport) do |open|
  open.use_bearer
  assert(transport.requests.last.headers['Authorization'] == 'Bearer provider-token')
end
OpenRouter::Client.new.close
OpenRouter::Client.open { |open| assert(!open.closed?) }
File.write('native-summary.json', OpenRouter::Json.dump({'controlled_requests' => transport.requests.length, 'states' => %w[positive missing empty unavailable invalid oversized explicit-value explicit-empty explicit-null explicit-missing-member choices conjunction anonymous snapshot import-no-read portable-unavailable]}))
puts 'Ruby credential-env: omitted new/open, explicit whole-argument precedence, snapshot, OR/AND/anonymous, unavailable values and default HTTPS transport capture passed'
