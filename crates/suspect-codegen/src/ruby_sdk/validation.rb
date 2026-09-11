# frozen_string_literal: true

module __NAMESPACE__
  # @api private
  module Internal
    module_function

    def location(source) = source.fetch('document') + '#' + source.fetch('pointer')
    def child_path(path, key) = path + '/' + key.gsub('~', '~0').gsub('/', '~1')

    def metadata_integer(value)
      return value if value.instance_of?(Integer)
      raise EvaluationFailure, 'invalid integer metadata' unless value.instance_of?(JsonNumber)
      value.to_i(max_digits: 16)
    end

    # Numbers in literal enum/const operands stay JsonNumber. Only instruction
    # indices/cardinalities of the compiled metadata become Ruby Integers.
    def load_program(text)
      program = Json.parse(text, max_bytes: 16 * 1024 * 1024, max_depth: 128, max_work: 128 * 1024 * 1024)
      ProgramGuard.new(program).check
    rescue JsonError => error
      raise EvaluationFailure.new('invalid exact JSON program metadata', source: error.source), cause: error
    end

    def json_kind(value)
      return 'null' if value.nil?
      return 'boolean' if value.equal?(true) || value.equal?(false)
      return 'string' if value.instance_of?(String)
      return 'number' if value.instance_of?(Integer) || value.instance_of?(JsonNumber)
      return 'array' if value.instance_of?(Array)
      return 'object' if value.instance_of?(Hash)
      'invalid'
    end

    # One validation budget survives every logical trial and codec union dispatch.
    # The generator checked every opcode; an unknown opcode remains a runtime
    # evaluation failure, including when its ordinary instance kind is absent.
    class ValidationSession
      attr_reader :program

      def initialize(program = PROGRAM)
        # The shipped immutable program was checked once at package load. An
        # explicitly supplied program is independently checked without mutating
        # or freezing the caller's containers.
        program = Internal.load_program(Json.dump(program, max_bytes: 16 * 1024 * 1024, max_work: 128 * 1024 * 1024)) unless program.equal?(PROGRAM)
        @v2 = program['version'] != V1_VERSION
        @v3 = program['version'] == V3_VERSION
        @program, @nodes, @limits = program, program.fetch('nodes'), program.fetch('limits')
        @roots = program.fetch('roots').to_h { |root| [root.fetch('target'), true] }
        @steps, @equalities, @numeric = @limits.fetch('maxEvaluationSteps'), @limits.fetch('maxEqualitySteps'), @limits.fetch('maxEvaluationSteps')
        @findings, @active, @depth = [], {}, 0
        @numbers = {}
        @resource_context = @v3 ? program.fetch('resourceContext') : nil
        @resource_stack, @entered_resources, @contexts, @resource_identity = [], {}, {}, 0
      rescue JsonError => error
        raise EvaluationFailure.new('invalid exact JSON program metadata', source: error.source), cause: error
      end

      def failure(source, path, message)
        raise EvaluationFailure.new(message, source: Internal.location(source), instance_path: path)
      end

      def spend(source, path, amount = 1)
        @steps -= amount
        failure(source, path, 'schema evaluation work limit exceeded') if @steps.negative?
      end

      def mismatch(source, path, message)
        if @limits.fetch('maxErrors').zero? || @findings.length < @limits.fetch('maxErrors')
          @findings << ValidationError.new(message, source: Internal.location(source), instance_path: path)
        end
        false
      end

      def check(index, value)
        selected!(index)
        @findings = []
        valid = evaluate(index, value, '')
        raise(@findings.first || ValidationError.new('source schema rejected the value', source: Internal.location(@nodes.fetch(index).fetch('source')))) unless valid
        nil
      end

      def matches?(index, value, path = '')
        selected!(index)
        trial(index, value, path)
      end

      def selected!(index)
        raise EvaluationFailure, 'validation root was not selected' unless @roots.key?(index)
      end

      def number(value, source, path)
        token = value.instance_of?(String) ? value : Internal.number_token(value)
        failure(source, path, 'numeric operand byte limit exceeded') if token.bytesize > @limits.fetch('maxNumberBytes')
        return @numbers[token] if @v2 && @numbers.key?(token)
        @numeric -= token.bytesize
        failure(source, path, 'exact numeric work limit exceeded') if @numeric.negative?
        result = Exact.new(token)
        @numbers[token.dup.freeze] = result if @v2
        result
      rescue JsonError
        failure(source, path, 'invalid or oversized exact numeric operand')
      end

      def evaluate(index, value, path)
        return eval_scoped(index, value, path).first if @v2
        node = @nodes.fetch(index)
        source = node.fetch('source')
        spend(source, path)
        failure(source, path, 'schema evaluation depth limit exceeded') if @depth >= @limits.fetch('maxDepth')
        identity = [index, value.object_id]
        failure(source, path, 'nonproductive recursive schema evaluation') if @active[identity]
        @active[identity] = true
        @depth += 1
        begin
          valid = true
          node.fetch('checks').each do |check|
            # Deliberately evaluate even after a mismatch: later incomplete
            # evaluations cannot turn into completed invalidity.
            matched = instruction(check, value, path)
            valid = matched && valid
          end
          valid
        ensure
          @depth -= 1
          @active.delete(identity)
        end
      end

      def trial(index, value, path)
        findings, @findings = @findings, []
        begin
          evaluate(index, value, path)
        ensure
          @findings = findings
        end
      end

      def instruction(check, value, path)
        source, op = check.fetch('source'), check.fetch('op')
        spend(source, path)
        case op
        when 'ref' then evaluate(check.fetch('target'), value, path)
        when 'allOf', 'anyOf', 'oneOf'
          count = 0
          check.fetch('targets').each do |target|
            spend(source, path)
            count += 1 if op == 'allOf' ? evaluate(target, value, path) : trial(target, value, path)
          end
          valid = op == 'allOf' ? count == check['targets'].length : op == 'anyOf' ? count.positive? : count == 1
          valid || mismatch(source, path, "#{op} branch count rejected the value")
        when 'not'
          !trial(check.fetch('target'), value, path) || mismatch(source, path, 'negated schema accepted the value')
        when 'properties'
          valid = true
          if value.instance_of?(Hash)
            check.fetch('properties').each do |property|
              spend(source, path)
              name = property.fetch('name')
              next unless value.key?(name)
              matched = evaluate(property.fetch('target'), value[name], Internal.child_path(path, name))
              valid = matched && valid
            end
          end
          valid
        when 'additionalProperties'
          valid = true
          if value.instance_of?(Hash)
            value.each do |name, child|
              spend(source, path)
              next if check.fetch('declared').include?(name)
              matched = evaluate(check.fetch('target'), child, Internal.child_path(path, name))
              valid = matched && valid
            end
          end
          valid
        when 'items', 'prefixItems'
          valid = true
          if value.instance_of?(Array)
            start = op == 'items' ? check.fetch('start') : 0
            finish = op == 'items' ? value.length : [value.length, check.fetch('targets').length].min
            (start...finish).each do |i|
              spend(source, path)
              target = op == 'items' ? check.fetch('target') : check.fetch('targets').fetch(i)
              matched = evaluate(target, value[i], Internal.child_path(path, i.to_s))
              valid = matched && valid
            end
          end
          valid
        else scalar(check, value, path)
        end
      end

      def equal?(left, right, source, path)
        pending = [[left, right, 0]]
        until pending.empty?
          a, b, depth = pending.pop
          @equalities -= 1
          failure(source, path, 'structural equality work/depth limit exceeded') if @equalities.negative? || depth > @limits.fetch('maxDepth')
          kind = Internal.json_kind(a)
          return false unless kind == Internal.json_kind(b)
          case kind
          when 'number' then return false unless number(a, source, path).compare(number(b, source, path)).zero?
          when 'array'
            return false unless a.length == b.length
            failure(source, path, 'structural equality work limit exceeded') if pending.length + a.length > @equalities
            a.each_with_index { |child, i| pending << [child, b[i], depth + 1] }
          when 'object'
            return false unless a.length == b.length && a.keys.all? { |key| b.key?(key) }
            failure(source, path, 'structural equality work limit exceeded') if pending.length + a.length > @equalities
            a.each { |key, child| pending << [child, b[key], depth + 1] }
          when 'invalid' then failure(source, path, 'non-JSON value reached structural equality')
          else return false unless a == b
          end
        end
        true
      end

      def pattern?(program, text, source, path)
        raise EvaluationFailure, 'unsupported pattern version' unless program['version'] == 'suspect.pattern.experimental.v1'
        spend(source, path, text.length)
        codepoints, states = text.codepoints, program.fetch('states')
        (0..codepoints.length).each do |offset|
          current = [program.fetch('start')]
          (offset..codepoints.length).each do |position|
            pending, seen, consuming = current.dup, {}, []
            until pending.empty?
              spend(source, path)
              index = pending.pop
              next if seen[index]
              seen[index] = true
              state = states.fetch(index)
              case state.fetch('op')
              when 'match' then return true
              when 'split' then pending << state.fetch('first') << state.fetch('second')
              when 'jump' then pending << state.fetch('target')
              when 'start' then pending << state.fetch('target') if position.zero?
              when 'end' then pending << state.fetch('target') if position == codepoints.length
              when 'char' then consuming << index
              else failure(source, path, 'unknown portable pattern opcode')
              end
            end
            break if position == codepoints.length
            current = []
            consuming.each do |index|
              state = states.fetch(index)
              state.fetch('ranges').each do |low, high|
                spend(source, path)
                if codepoints[position].between?(low, high)
                  current << state.fetch('target')
                  break
                end
              end
            end
            break if current.empty?
          end
        end
        false
      end

      def scalar(check, value, path)
        source, op, kind = check.fetch('source'), check.fetch('op'), Internal.json_kind(value)
        valid = true
        case op
        when 'always' then valid = check.fetch('value').equal?(true)
        when 'type'
          types = check.fetch('types')
          valid = types.include?(kind) || kind == 'number' && types.include?('integer') && number(value, source, path).integral?
        when 'required'
          if kind == 'object'
            check.fetch('names').each do |name|
              spend(source, path)
              valid = mismatch(source, Internal.child_path(path, name), 'required property is absent') unless value.key?(name)
            end
          end
        when 'bound'
          if kind == 'number'
            order = number(value, source, path).compare(number(check.fetch('value'), source, path))
            valid = check.fetch('maximum') ? order.negative? : order.positive?
            valid ||= order.zero? && !check.fetch('exclusive')
          end
        when 'multipleOf'
          if kind == 'number'
            valid = number(value, source, path).multiple_of?(number(check.fetch('value'), source, path)) do |amount|
              @numeric -= amount
              failure(source, path, 'exact divisibility work limit exceeded') if @numeric.negative?
            end
          end
        when 'count'
          if kind == check.fetch('target')
            order = Exact.new(value.length.to_s).compare(number(check.fetch('value'), source, path))
            valid = check.fetch('maximum') ? order <= 0 : order >= 0
          end
        when 'const' then valid = equal?(value, check.fetch('value'), source, path)
        when 'enum'
          valid = false
          check.fetch('values').each do |literal|
            spend(source, path)
            if equal?(value, literal, source, path)
              valid = true
              break
            end
          end
        when 'uniqueItems'
          if kind == 'array'
            value.each_with_index do |child, i|
              spend(source, path)
              i.times do |j|
                spend(source, path)
                if equal?(child, value[j], source, path)
                  valid = false
                  break
                end
              end
              break unless valid
            end
          end
        when 'pattern' then valid = pattern?(check.fetch('program'), value, source, path) if kind == 'string'
        else failure(source, path, 'unknown portable validation opcode')
        end
        valid || mismatch(source, path, "#{op} assertion rejected the value")
      end
    end
  end
end
