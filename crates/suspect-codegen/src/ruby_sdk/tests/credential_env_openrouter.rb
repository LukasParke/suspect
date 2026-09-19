# frozen_string_literal: true
def assert(value, message = 'OpenRouter credential-env assertion failed') = (raise message unless value)
def rejects(type)
  yield
rescue type => error
  error
else
  raise "expected #{type}"
end
module EnvProbe
  class << self
    attr_accessor :reads, :unavailable
  end
  self.reads = 0
end
ENV.singleton_class.prepend(Module.new do
  def [](key)
    if key == 'OPENROUTER_API_KEY'
      EnvProbe.reads += 1
      raise SecurityError, 'unavailable-reader-secret' if EnvProbe.unavailable
    end
    super
  end
end)
require 'openrouter'
assert(EnvProbe.reads.zero?, 'import read OpenRouter key')
assert(Gem.loaded_specs.fetch('openrouter').full_gem_path.start_with?(File.expand_path('installed')))

class SourceHttpsTransport
  attr_reader :requests
  def initialize = (@requests = [])
  def exchange(request:, context:)
    context.check!
    raise 'not a source-default HTTPS GET' unless request.method == 'GET' && ['https://openrouter.ai/api/v1/key', 'https://openrouter.ai/api/v1/credits'].include?(request.url)
    @requests << request
    text = if request.operation_id == 'getCurrentKey'
      '{"data":{"label":"controlled-key","limit":null,"usage":1.25,"usage_daily":0,"usage_weekly":0,"usage_monthly":0,"byok_usage":0,"byok_usage_daily":0,"byok_usage_weekly":0,"byok_usage_monthly":0,"is_free_tier":false,"is_management_key":false,"is_provisioning_key":false,"limit_remaining":null,"limit_reset":null,"include_byok_in_limit":false,"creator_user_id":null,"rate_limit":{"requests":-1,"interval":"n/a","note":"controlled"}}}'
    else
      '{"data":{"total_credits":100,"total_usage":2.5}}'
    end
    yield OpenRouter::WireResponse.new(status: 200, headers: {'Content-Type' => 'application/json'}, body: text)
  end
end

ENV['OPENROUTER_API_KEY'] = 'controlled-initial-token'
transport = SourceHttpsTransport.new
client = OpenRouter::Client.new(transport: transport, timeout: 2)
assert(EnvProbe.reads == 1, 'shared scheme should be read once at creation')
result = client.get_current_key
assert(result.status == 200 && result.data.data.label == 'controlled-key')
assert(result.data.data.usage.token == '1.25' && result.data.data.limit.nil?)
assert(result.data.data.rate_limit.requests.token == '-1')
assert(transport.requests.last.url == 'https://openrouter.ai/api/v1/key')
assert(transport.requests.last.headers['Authorization'] == 'Bearer controlled-initial-token')
ENV['OPENROUTER_API_KEY'] = 'controlled-new-token'
credits = client.get_credits
assert(credits.data.data.total_credits.token == '100')
assert(transport.requests.last.url == 'https://openrouter.ai/api/v1/credits')
assert(transport.requests.last.headers['Authorization'] == 'Bearer controlled-initial-token')
assert(EnvProbe.reads == 1, 'per-request environment read')
client.close
OpenRouter::Client.open(transport: transport) do |open|
  open.get_current_key
  assert(transport.requests.last.headers['Authorization'] == 'Bearer controlled-new-token')
end
assert(EnvProbe.reads == 2)

reads = EnvProbe.reads
OpenRouter::Client.open(auth: {'apiKey' => 'explicit-token'}, transport: transport) do |open|
  open.get_current_key
  assert(transport.requests.last.headers['Authorization'] == 'Bearer explicit-token')
end
[{}, {'unmapped' => 'explicit-other'}].each do |auth|
  OpenRouter::Client.open(auth: auth, transport: transport) do |open|
    size = transport.requests.length
    error = rejects(OpenRouter::RequestError) { open.get_current_key }
    assert(error.source.end_with?('/security/0/apiKey') && transport.requests.length == size)
  end
end
[nil, {'apiKey' => ''}, {'apiKey' => nil}].each do |auth|
  rejects(ArgumentError) { OpenRouter::Client.new(auth: auth, transport: transport) }
end
assert(EnvProbe.reads == reads, 'explicit argument was supplemented')
[:missing, :empty, :unavailable].each do |state|
  ENV.delete('OPENROUTER_API_KEY')
  ENV['OPENROUTER_API_KEY'] = '' if state == :empty
  EnvProbe.unavailable = state == :unavailable
  OpenRouter::Client.open(transport: transport) do |open|
    size = transport.requests.length
    error = rejects(OpenRouter::RequestError) { open.get_current_key }
    assert(transport.requests.length == size)
    while error
      assert(!error.message.include?('unavailable-reader-secret') && !error.message.include?('controlled-initial-token'))
      error = error.cause
    end
  end
end
File.write('native-summary.json', OpenRouter::Json.dump({'status' => 'passed', 'source_default_urls' => ['https://openrouter.ai/api/v1/key', 'https://openrouter.ai/api/v1/credits'], 'controlled_requests' => transport.requests.length, 'live_account_requests' => 0}))
puts 'OpenRouter get_current_key and optional get_credits: actual schemas, creation-time env, original HTTPS base, native decoded values, explicit precedence and pre-HTTP missing errors passed'
