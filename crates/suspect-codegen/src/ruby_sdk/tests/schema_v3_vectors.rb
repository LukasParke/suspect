# frozen_string_literal: true
require 'ruby_schema_v3'
include RubySchemaV3
def assert(value, message = 'assertion failed') = (raise message unless value)
def rejects(type)
  yield
rescue type => error
  error
else
  raise "expected #{type}"
end
cases = Json.parse(File.binread('cases.json'), max_bytes: 16 * 1024 * 1024, max_work: 128 * 1024 * 1024)
cases.each do |entry|
  program = Internal.load_program(Json.dump(entry['program'], max_bytes: 16 * 1024 * 1024, max_work: 128 * 1024 * 1024))
  session = Internal::ValidationSession.new(program)
  actual, error = 'Valid', nil
  begin
    session.check(entry['rootTarget'].to_i, Json.parse(entry['instanceJson']))
  rescue EvaluationFailure => finding
    actual, error = 'EvaluationFailure', finding
  rescue ValidationError => finding
    actual, error = 'Invalid', finding
  end
  assert(actual == entry['expected'], "#{entry['id']}: expected #{entry['expected']}, got #{actual}: #{error&.source}")
  assert(error.source.end_with?('#' + entry['source']), "wrong dynamic finding #{error.source}") if entry['source']
  assert(session.instance_variable_get(:@resource_stack).empty?, 'entered resources leaked')
  assert(session.instance_variable_get(:@entered_resources).empty?, 'resource membership leaked')
  assert(session.instance_variable_get(:@resource_identity) == 0, 'resource context was not restored')
  assert(session.instance_variable_get(:@active).empty?, 'cycle identities leaked')
end
def mutable(value) = Json.parse(Json.dump(value, max_bytes: 16 * 1024 * 1024))
def bad_program(value)
  @malformed = (@malformed || 0) + 1
  rejects(EvaluationFailure) { Internal.load_program(Json.dump(value, max_bytes: 16 * 1024 * 1024)) }
  rejects(EvaluationFailure) { Internal::ValidationSession.new(value) }
end
original = cases.find { |e| e['id'] == 'outermost-entered' }['program']
[[Internal::V1_VERSION, Internal::V1_PROFILE], [Internal::V2_VERSION, Internal::V2_PROFILE]].each do |version, profile|
  bad = mutable(original); bad['version'], bad['profile'] = version, profile; bad_program(bad)
  bad.delete('resourceContext'); bad_program(bad)
end
bad = mutable(original); bad.delete('resourceContext'); bad_program(bad)
bad = mutable(original); bad['resourceContext'] = nil; bad_program(bad)
bad = mutable(original); bad['profile'] = Internal::V2_PROFILE; bad_program(bad)
bad = mutable(original); bad['resourceContext']['nodeScopes'].pop; bad_program(bad)
bad = mutable(original); bad['resourceContext']['nodeScopes'][0][0] = 99999; bad_program(bad)
bad = mutable(original); bad['resourceContext']['nodeScopes'][0][2] = 'urn:wrong'; bad_program(bad)
bad = mutable(original); bad['resourceContext']['nodeScopes'][0][1]['document'] = 'https://unrelated.test/physical.json'; bad_program(bad)
bad = mutable(original); bad['resourceContext']['resources'][0]['aliases'] = []; bad_program(bad)
bad = mutable(original); resources = bad['resourceContext']['resources']; resources[1]['aliases'] << resources[0]['canonicalUri']; bad_program(bad)
bad = mutable(original); bad['resourceContext']['resources'][0]['baseUri'] += '#fragment'; bad_program(bad)
bad = mutable(original); bad['resourceContext']['resources'][0]['canonicalUri'] = 'relative'; bad_program(bad)
bad = mutable(original); bad['resourceContext']['resources'][0]['kind'] = 'unknown'; bad_program(bad)
bad = mutable(original); resource = bad['resourceContext']['resources'].find { |r| r['declarationSource'] }; resource['declarationSource']['pointer'] += '/wrong'; bad_program(bad)
bad = mutable(original); resource = bad['resourceContext']['resources'].find { |r| !r['dynamicAnchors'].empty? }; resource['dynamicAnchors'][0][2] = 99999; bad_program(bad)
bad = mutable(original); resource = bad['resourceContext']['resources'].find { |r| !r['dynamicAnchors'].empty? }; resource['dynamicAnchors'][0][0] = '0bad'; bad_program(bad)
bad = mutable(original); dynamic = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'dynamicRef' }; dynamic['initialResource'] = true; bad_program(bad)
bad = mutable(original); dynamic = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'dynamicRef' }; dynamic['initialResource'] = 99999; bad_program(bad)
bad = mutable(original); dynamic = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'dynamicRef' }; dynamic['anchor'] = 'other'; bad_program(bad)
bad = mutable(original); dynamic = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'dynamicRef' }; dynamic.delete('anchor'); bad_program(bad)
bad = mutable(original); dynamic = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'dynamicRef' }; dynamic['source']['pointer'] += '/wrong'; bad_program(bad)
bad = mutable(original); dynamic = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'dynamicRef' }; dynamic['target'] = 99999; bad_program(bad)
bad = mutable(original); bad['resourceContext']['futureRule'] = true; bad_program(bad)
bad = mutable(original); bad['resourceContext']['resources'][0]['futureRule'] = true; bad_program(bad)
bad = mutable(original); bad['resourceContext']['nodeScopes'][0] << 'extra'; bad_program(bad)
bad = mutable(original); resources = bad['resourceContext']['resources']; resources[1]['aliases'] << resources[0]['canonicalUri'].sub(/\A[^:]+:/) { |scheme| scheme.upcase } + '#'; bad_program(bad)
bad = mutable(original); resources = bad['resourceContext']['resources']; resources[0]['aliases'] << 'https://alias.test/doc#x'; resources[1]['aliases'] << 'https://alias.test/doc#%78'; bad_program(bad)
['https://host/%', 'https://[invalid]/x', 'https://host/x#%FF', 'https://host/😀', 'https://host/x#%C0%AF'].each do |uri|
  bad = mutable(original); bad['resourceContext']['resources'][0]['aliases'] << uri; bad_program(bad)
end
bad = mutable(original); resource = bad['resourceContext']['resources'].find { |r| !r['dynamicAnchors'].empty? }; resource['dynamicAnchors'] << mutable(resource['dynamicAnchors'][0]); bad_program(bad)
bad = mutable(original); resource = bad['resourceContext']['resources'].find { |r| !r['dynamicAnchors'].empty? }; resource['dynamicAnchors'][0][1]['pointer'] += '/wrong'; bad_program(bad)
bad = mutable(cases.find { |e| e['id'] == 'every-inspected-binding-costs-one' }['program'])
bad['resourceContext']['resources'].find { |r| r['dynamicAnchors'].length > 1 }['dynamicAnchors'].reverse!
bad_program(bad)
caller_program = mutable(original)
session = Internal::ValidationSession.new(caller_program)
assert(!caller_program.frozen? && !caller_program['resourceContext']['resources'].frozen?, 'checking froze caller containers')
assert(session.program['resourceContext']['resources'].all? { |r| r.frozen? && r['dynamicAnchors'].frozen? }, 'checked metadata is mutable')
File.write('vectors-summary.json', Json.dump({'official_dynamic_ref_cases' => 44, 'official_dynamic_unevaluated_cases' => 4, 'independent_and_compatibility_controls' => cases.length - 48, 'malformed_programs' => @malformed, 'guard_entrypoints' => 2}))
puts "Ruby v3: 44 official dynamicRef + 4 official dynamic/unevaluated + #{cases.length - 48} independent/compatibility controls and #{@malformed} malformed-program checks through two entrypoints passed"
