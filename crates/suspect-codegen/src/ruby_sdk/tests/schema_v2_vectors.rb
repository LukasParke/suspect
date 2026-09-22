# frozen_string_literal: true
require 'ruby_schema_v2'
include RubySchemaV2
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
  program = Internal.load_program(Json.dump(entry.fetch('program'), max_bytes: 16 * 1024 * 1024, max_work: 128 * 1024 * 1024))
  index = program.fetch('roots').first.fetch('target')
  actual, finding = 'Valid', nil
  begin
    Internal::ValidationSession.new(program).check(index, Json.parse(entry.fetch('instanceJson')))
  rescue EvaluationFailure => error
    actual, finding = 'EvaluationFailure', error
  rescue ValidationError => error
    actual, finding = 'Invalid', error
  end
  assert(actual == entry['expected'], "#{entry['id']}: expected #{entry['expected']}, got #{actual} #{finding&.source} #{finding&.instance_path}")
  assert(finding.source.end_with?('#' + entry['source']), "#{entry['id']}: wrong source #{finding.source}") if entry['source']
  assert(finding.instance_path == entry['instancePath'], "#{entry['id']}: wrong instance path #{finding.instance_path}") if entry['instancePath']
end

def mutable(program) = Json.parse(Json.dump(program, max_bytes: 16 * 1024 * 1024))
def guard_reject(program)
  rejects(EvaluationFailure) { Internal.load_program(Json.dump(program, max_bytes: 16 * 1024 * 1024)) }
  rejects(EvaluationFailure) { Internal::ValidationSession.new(program) }
end
program = cases.first['program']
bad = mutable(program); bad['version'] = Internal::V1_VERSION; bad['profile'] = Internal::V1_PROFILE; guard_reject(bad)
bad = mutable(program); bad['profile'] = Internal::V1_PROFILE; guard_reject(bad)
bad = mutable(program); bad['version'] = 'unrecognized'; guard_reject(bad)
bad = mutable(program); bad['roots'][0]['target'] = true; guard_reject(bad)
bad = mutable(program); bad['nodes'][0]['checks'][0]['op'] = 'unknown-op'; guard_reject(bad)
bad = mutable(program); bad['nodes'][0]['source']['pointer'] = '/broken~2pointer'; guard_reject(bad)
bad = mutable(program); bad['nodes'][0]['source']['document'] = 'relative.json'; guard_reject(bad)
bad = mutable(program); bad['nodes'] << bad['nodes'][0]; guard_reject(bad)
bad = mutable(program)
conditional = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'if' }
conditional['thenTarget'] = bad['roots'][0]['target']; guard_reject(bad)

pattern = cases.find { |c| c['id'] == 'pattern-overlap-accepts-and-excludes-extra' }['program']
bad = mutable(pattern)
check = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'additionalPropertiesWithPatterns' }
check['op'] = 'additionalProperties'; guard_reject(bad)
bad = mutable(pattern)
check = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'patternProperties' }
check['patterns'][0][2] = bad['roots'][0]['target']; guard_reject(bad)
bad = mutable(pattern)
nfa = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'patternProperties' }['patterns'][0][1]
nfa['start'] = 999999; guard_reject(bad)
bad = mutable(pattern)
nfa = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'patternProperties' }['patterns'][0][1]
nfa['states'][0]['op'] = 'hidden-bad-op'; guard_reject(bad)
bad = mutable(pattern)
check = bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'additionalPropertiesWithPatterns' }
check['declared'] = ['invented']; guard_reject(bad)

contains = cases.find { |c| c['id'] == 'contains-zero-does-not-mark-unmatched' }['program']
['-1', '0.5', '1e-400', ' 1', 'NaN', '1' * 4097].each do |token|
  bad = mutable(contains)
  bad['nodes'].flat_map { |n| n['checks'] }.find { |c| c['op'] == 'contains' }['minimum'] = token
  guard_reject(bad)
end
bad = mutable(contains)
bad['nodes'].find { |n| n['checks'].any? { |c| c['op'] == 'unevaluatedItems' } }['checks'].reverse!
guard_reject(bad)

# The loaded metadata is immutable, independent sessions never share annotation
# sets, and checking an explicitly supplied program never freezes caller data.
unfrozen = mutable(program)
session = Internal::ValidationSession.new(unfrozen)
assert(!unfrozen.frozen? && !unfrozen['nodes'].frozen?)
assert(session.program.frozen? && session.program['nodes'].first['checks'].frozen?)
puts "Ruby schema-v2: 32 maintained source cases + #{cases.length - 32} scoped/budget/v1 witnesses and malformed-program guards passed"
