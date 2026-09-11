# frozen_string_literal: true
module __NAMESPACE__
  # @api private
  module Internal
    V1_VERSION = 'suspect.validation.experimental.v1'
    V1_PROFILE = 'oas31-jsonschema202012-static-subset'
    V2_VERSION = 'suspect.validation.experimental.v2'
    V2_PROFILE = 'oas31-jsonschema202012-static-applicators'
    V3_VERSION = 'suspect.validation.experimental.v3'
    V3_PROFILE = 'oas31-jsonschema202012-resources-dynamic'
    V2_OPS = %w[if dependentRequired dependentSchemas contains patternProperties additionalPropertiesWithPatterns propertyNames unevaluatedProperties unevaluatedItems].freeze
    V1_OPS = %w[always type ref allOf anyOf oneOf not properties additionalProperties items prefixItems required bound multipleOf count const enum uniqueItems pattern].freeze

    # Checks only portable instructions, never raw JSON Schema. In particular,
    # inactive branches/kinds cannot hide malformed opcodes, edges or operands.
    class ProgramGuard
      def initialize(program)
        @program = program
      end

      def fail_program(message, source = nil)
        location = source && source.instance_of?(Hash) && source['document'].instance_of?(String) && source['pointer'].instance_of?(String) ? source['document'] + '#' + source['pointer'] : nil
        raise EvaluationFailure.new(message, source: location)
      end

      def object(value, at = nil)
        fail_program('compiled metadata must be an object', at) unless value.instance_of?(Hash)
        value
      end

      def array(value, at = nil)
        fail_program('compiled metadata must be an array', at) unless value.instance_of?(Array)
        value
      end

      def fields(value, allowed, at = nil)
        object(value, at)
        fail_program('unknown portable metadata field', at) unless (value.keys - allowed).empty?
      end

      def integer(value, at = nil, cap = 128 * 1024 * 1024)
        value = value.to_i(max_digits: 20) if value.instance_of?(JsonNumber)
        fail_program('metadata requires a bounded nonnegative integer', at) unless value.instance_of?(Integer) && value.between?(0, cap)
        value
      rescue RangeError, ArgumentError
        fail_program('metadata requires a bounded nonnegative integer', at)
      end

      def boolean(value, at)
        fail_program('metadata requires a boolean', at) unless value.equal?(true) || value.equal?(false)
      end

      def source(value)
        object(value)
        fields(value, %w[document pointer])
        document, pointer = value.values_at('document', 'pointer')
        unless document.instance_of?(String) && pointer.instance_of?(String) && /\A[A-Za-z][A-Za-z0-9+.-]*:/.match?(document) && !/[\s\x00-\x1f\x7f#]/.match?(document) && !/%(?![0-9a-fA-F]{2})/.match?(document) && (pointer.empty? || pointer.start_with?('/')) && !/~(?![01])/.match?(pointer)
          fail_program('source needs an absolute fragment-free URI and escaped JSON Pointer', value)
        end
        uri_parts(document, value) if @v3
        value
      end

      def child(at, token)
        {'document' => at.fetch('document'), 'pointer' => at.fetch('pointer') + '/' + token.gsub('~', '~0').gsub('/', '~1')}
      end

      def strings(value, at)
        names = array(value, at)
        fail_program('duplicate or non-string metadata names', at) unless names.all? { |n| n.instance_of?(String) } && names.uniq.length == names.length
        names
      end

      def target(value, at, expected = nil)
        index = integer(value, at)
        fail_program('schema target is outside the compiled graph', at) if index >= @nodes.length
        fail_program('schema target has the wrong source identity', at) if expected && @nodes[index]['source'] != expected
        index
      end

      def number(value, at, positive: false, count: false)
        unless value.instance_of?(String) && value.bytesize <= @limits.fetch('maxNumberBytes') && JsonNumber::TOKEN.match?(value.b)
          fail_program('invalid or oversized exact numeric metadata', at)
        end
        parsed = Exact.new(value)
        fail_program('divisor must be strictly positive', at) if positive && parsed.sign <= 0
        fail_program('count must be a nonnegative mathematical integer', at) if count && (parsed.sign.negative? || !parsed.integral?)
      end

      def pattern(value, at)
        object(value, at)
        fields(value, %w[version start states], at)
        fail_program('unsupported portable pattern version', at) unless value['version'] == 'suspect.pattern.experimental.v1'
        states = array(value['states'], at)
        fail_program('invalid finite pattern graph', at) unless states.length.between?(1, 8192)
        index = ->(v) { i = integer(v, at, 8191); fail_program('pattern edge outside graph', at) if i >= states.length; i }
        value['start'] = index.call(value['start'])
        ranges_total = 0
        states.each do |state|
          object(state, at)
          allowed = {'match' => %w[op], 'split' => %w[op first second], 'jump' => %w[op target], 'start' => %w[op target], 'end' => %w[op target], 'char' => %w[op ranges target]}[state['op']]
          fail_program('unknown portable pattern opcode', at) unless allowed
          fields(state, allowed, at)
          case state['op']
          when 'match'
          when 'split'
            state['first'] = index.call(state['first']); state['second'] = index.call(state['second'])
          when 'jump', 'start', 'end'
            state['target'] = index.call(state['target'])
          when 'char'
            state['target'] = index.call(state['target'])
            ranges = array(state['ranges'], at)
            ranges_total += ranges.length
            fail_program('pattern character range ceiling exceeded', at) if ranges.length > 8192 || ranges_total > 65_536
            previous = -2
            ranges.each do |range|
              fail_program('pattern range must contain two scalar endpoints', at) unless range.instance_of?(Array) && range.length == 2
              low, high = range.map { |n| integer(n, at, 0x10ffff) }
              if low > high || low <= previous + 1 || low <= 0xdfff && high >= 0xd800
                fail_program('pattern ranges must be normalized Unicode scalar ranges', at)
              end
              range[0], range[1], previous = low, high, high
            end
          else fail_program('unknown portable pattern opcode', at)
          end
        end
      end

      def keyword(check)
        case check['op']
        when 'always' then nil
        when 'ref' then '$ref'
        when 'dynamicRef' then '$dynamicRef'
        when 'additionalPropertiesWithPatterns' then 'additionalProperties'
        when 'bound'
          check['maximum'] ? (check['exclusive'] ? 'exclusiveMaximum' : 'maximum') : (check['exclusive'] ? 'exclusiveMinimum' : 'minimum')
        when 'count'
          stem = {'string' => 'Length', 'array' => 'Items', 'object' => 'Properties'}[check['target']]
          fail_program('unknown cardinality target', check['source']) unless stem
          (check['maximum'] ? 'max' : 'min') + stem
        else check['op']
        end
      end

      def check
        object(@program)
        pairs = [[V1_VERSION, V1_PROFILE], [V2_VERSION, V2_PROFILE], [V3_VERSION, V3_PROFILE]]
        fail_program('unsupported compiled validation version/profile') unless pairs.include?([@program['version'], @program['profile']])
        @v3 = @program['version'] == V3_VERSION
        @v2 = @program['version'] != V1_VERSION
        fields(@program, %w[version profile roots nodes limits] + (@v3 ? ['resourceContext'] : []))
        @limits = object(@program['limits'])
        fields(@limits, %w[maxDepth maxErrors maxNumberBytes maxEqualitySteps maxEvaluationSteps])
        %w[maxDepth maxErrors maxNumberBytes maxEqualitySteps maxEvaluationSteps].each { |k| @limits[k] = integer(@limits[k]) }
        fail_program('native program metadata limit exceeded') if @limits['maxDepth'] > 512 || @limits['maxNumberBytes'] > 4096
        @nodes = array(@program['nodes'])
        fail_program('native program node limit exceeded') if @nodes.length > 10_000
        identities = {}
        @nodes.each do |node|
          object(node); at = source(node['source']); fields(node, %w[source checks], at)
          key = [at['document'], at['pointer']]
          fail_program('duplicate schema source identity', at) if identities[key]
          identities[key] = true
        end
        check_resources if @v3
        @nodes.each { |node| check_node(node) }
        roots = {}
        array(@program['roots']).each do |root|
          object(root); at = source(root['source']); fields(root, %w[source target], at)
          root['target'] = target(root['target'], at, at)
          key = [at['document'], at['pointer']]
          fail_program('duplicate selected root identity', at) if roots[key]
          roots[key] = true
        end
        Internal.freeze_tree(@program)
      rescue KeyError, NoMethodError, TypeError, ArgumentError => error
        fail_program('malformed portable program metadata')
      end

      def check_node(node)
        node_source = node['source']
        checks = array(node['checks'], node_source)
        seen, declared, properties, patterns, aware, prefix, items, tail = {}, nil, [], 0, false, 0, nil, false
        checks.each do |check|
          object(check, node_source); at = source(check['source']); op = check['op']
          fail_program('unknown portable validation opcode', at) unless V1_OPS.include?(op) || V2_OPS.include?(op) || op == 'dynamicRef'
          fail_program('dynamic instruction requires v3', at) if op == 'dynamicRef' && !@v3
          operands = case op
                     when 'always', 'const', 'multipleOf' then %w[value]
                     when 'type' then %w[types]
                     when 'ref', 'not', 'propertyNames', 'unevaluatedProperties', 'unevaluatedItems' then %w[target]
                     when 'dynamicRef' then %w[target initialResource anchor]
                     when 'properties' then %w[properties]
                     when 'additionalProperties', 'additionalPropertiesWithPatterns' then %w[declared target]
                     when 'required' then %w[names]
                     when 'items' then %w[target start]
                     when 'prefixItems', 'allOf', 'anyOf', 'oneOf' then %w[targets]
                     when 'bound' then %w[value maximum exclusive]
                     when 'count' then %w[value maximum target]
                     when 'enum' then %w[values]
                     when 'pattern' then %w[program]
                     when 'if' then %w[condition thenTarget elseTarget]
                     when 'dependentRequired', 'dependentSchemas' then %w[dependencies]
                     when 'contains' then %w[target minimum maximum]
                     when 'patternProperties' then %w[patterns]
                     else []
                     end
          fields(check, %w[source op] + operands, at)
          fail_program('v2 instruction in a v1 program', at) if !@v2 && V2_OPS.include?(op)
          last = %w[unevaluatedProperties unevaluatedItems].include?(op)
          fail_program('unevaluated instructions must be last', at) if @v2 && tail && !last
          tail ||= last
          %w[maximum exclusive].each { |k| boolean(check[k], at) } if op == 'bound'
          boolean(check['maximum'], at) if op == 'count'
          key = keyword(check)
          expected = key ? child(node_source, key) : node_source
          normalized = op == 'bound' && check['exclusive'] && at == child(node_source, check['maximum'] ? 'maximum' : 'minimum')
          fail_program('instruction source is inconsistent or duplicated', at) if (at != expected && !normalized) || seen[at['pointer']]
          seen[at['pointer']] = true
          case op
          when 'always'
            boolean(check['value'], at)
            fail_program('boolean schema must contain exactly one check', at) unless checks.length == 1
          when 'type'
            types = strings(check['types'], at)
            fail_program('invalid type set', at) if types.empty? || !(types - %w[null boolean integer number string array object]).empty?
          when 'ref' then check['target'] = target(check['target'], at)
          when 'dynamicRef'
            check['target'] = target(check['target'], at)
            check['initialResource'] = integer(check['initialResource'], at)
            context = @program.fetch('resourceContext')
            initial = check['initialResource']
            if initial >= context['resources'].length || context['nodeScopes'][check['target']][0] != initial
              fail_program('dynamic initial resource does not match target scope', at)
            end
            fail_program('missing optional dynamic anchor operand', at) unless check.key?('anchor')
            unless check['anchor'].nil?
              unless anchor_name?(check['anchor']) && context['resources'][initial]['dynamicAnchors'].any? { |name, _, target| name == check['anchor'] && target == check['target'] }
                fail_program('dynamic name does not bind its initial target', at)
              end
            end
          when 'properties'
            fields = array(check['properties'], at)
            properties = strings(fields.map { |f| object(f, at)['name'] }, at)
            fields.each { |f| self.fields(f, %w[name target], at); f['target'] = target(f['target'], at, child(at, f['name'])) }
          when 'additionalProperties', 'additionalPropertiesWithPatterns'
            declared = strings(check['declared'], at); aware = op == 'additionalPropertiesWithPatterns'
            check['target'] = target(check['target'], at, at)
          when 'required' then strings(check['names'], at)
          when 'items'
            check['target'] = target(check['target'], at, at)
            check['start'] = integer(check['start'], at); items = [at, check['start']]
          when 'not', 'propertyNames', 'unevaluatedProperties', 'unevaluatedItems'
            check['target'] = target(check['target'], at, at)
          when 'allOf', 'anyOf', 'oneOf', 'prefixItems'
            values = array(check['targets'], at)
            fail_program('applicator requires targets', at) if values.empty?
            values.map!.with_index { |i, position| target(i, at, child(at, position.to_s)) }
            prefix = values.length if op == 'prefixItems'
          when 'if'
            check['condition'] = target(check['condition'], at, at)
            %w[thenTarget elseTarget].each do |field|
              fail_program('missing optional branch operand', at) unless check.key?(field)
              check[field] = target(check[field], at, child(node_source, field == 'thenTarget' ? 'then' : 'else')) unless check[field].nil?
            end
          when 'dependentRequired'
            deps = array(check['dependencies'], at)
            fail_program('dependency must be a [trigger,names] pair', at) unless deps.all? { |d| d.instance_of?(Array) && d.length == 2 }
            strings(deps.map(&:first), at)
            deps.each { |trigger, names| strings(names, child(at, trigger)) }
          when 'dependentSchemas'
            deps = array(check['dependencies'], at)
            strings(deps.map { |d| object(d, at)['name'] }, at)
            deps.each { |d| fields(d, %w[name target], at); d['target'] = target(d['target'], at, child(at, d['name'])) }
          when 'contains'
            check['target'] = target(check['target'], at, at)
            %w[minimum maximum].each do |field|
              fail_program('missing optional contains count', at) unless check.key?(field)
              number(check[field], child(node_source, field == 'minimum' ? 'minContains' : 'maxContains'), count: true) unless check[field].nil?
            end
          when 'patternProperties'
            entries = array(check['patterns'], at)
            fail_program('pattern entry must be a [text,program,target] triple', at) unless entries.all? { |e| e.instance_of?(Array) && e.length == 3 }
            strings(entries.map(&:first), at); patterns = entries.length
            entries.each { |e| location = child(at, e[0]); e[2] = target(e[2], at, location); pattern(e[1], location) }
          when 'bound' then number(check['value'], at)
          when 'multipleOf' then number(check['value'], at, positive: true)
          when 'count' then number(check['value'], at, count: true)
          when 'enum' then array(check['values'], at)
          when 'const' then fail_program('missing const operand', at) unless check.key?('value')
          when 'pattern' then pattern(check['program'], at)
          end
        end
        if declared
          fail_program('additional declared names do not match properties', child(node_source, 'additionalProperties')) unless declared.sort == properties.sort
          fail_program('pattern-aware additional opcode must match adjacent patterns', child(node_source, 'additionalProperties')) unless aware == patterns.positive?
        end
        fail_program('items start does not match prefix length', items[0]) if items && items[1] != prefix
      end
    end
  end
end
