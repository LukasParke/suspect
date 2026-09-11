# frozen_string_literal: true
module __NAMESPACE__
  # @api private
  module Internal
    class ValidationSession
      def empty_scope = [{}, {}]

      # Every candidate insertion is charged, including duplicates. Property
      # names are Unicode-scalar lexical; array indices are ascending.
      def merge_scope(into, other, source, path)
        other[0].keys.sort.each { |name| spend(source, path); into[0][name] = true }
        other[1].keys.sort.each { |index| spend(source, path); into[1][index] = true }
      end

      def source_child(source, token)
        {'document' => source.fetch('document'), 'pointer' => Internal.child_path(source.fetch('pointer'), token)}
      end

      def eval_scoped(index, value, path)
        node = @nodes.fetch(index); source = node.fetch('source')
        spend(source, path)
        failure(source, path, 'schema evaluation depth limit exceeded') if @depth >= @limits.fetch('maxDepth')
        entered = @v3 ? enter_resource(index, source, path) : nil
        identity = @v3 ? [index, value.object_id, @resource_identity] : [index, value.object_id]
        if @active[identity]
          leave_resource(entered) if @v3
          failure(source, path, 'nonproductive recursive schema evaluation')
        end
        @active[identity] = true; @depth += 1
        begin
          local, valid = empty_scope, true
          node.fetch('checks').each do |check|
            spend(check.fetch('source'), path)
            ok, produced = apply_scoped(node, check, value, path, local)
            valid = ok && valid
            merge_scope(local, produced, check.fetch('source'), path) if ok
          end
          [valid, valid ? local : empty_scope]
        ensure
          @depth -= 1
          @active.delete(identity)
          leave_resource(entered) if @v3
        end
      end

      def trial_scoped(index, value, path)
        findings, @findings = @findings, []
        begin
          eval_scoped(index, value, path)
        ensure
          @findings = findings
        end
      end

      def apply_scoped(node, check, value, path, local)
        source, op, produced = check.fetch('source'), check.fetch('op'), empty_scope
        case op
        when 'ref'
          return eval_scoped(check.fetch('target'), value, path)
        when 'dynamicRef'
          return eval_scoped(dynamic_target(check, path), value, path)
        when 'allOf', 'anyOf', 'oneOf'
          passing, count = [], 0
          check.fetch('targets').each do |target|
            spend(source, path)
            ok, annotations = op == 'allOf' ? eval_scoped(target, value, path) : trial_scoped(target, value, path)
            if ok
              count += 1; passing << annotations
            end
          end
          valid = op == 'allOf' ? count == check['targets'].length : op == 'anyOf' ? count.positive? : count == 1
          if valid
            passing.each { |scope| merge_scope(produced, scope, source, path) }
          else
            mismatch(source, path, "#{op} branch count rejected the value")
          end
        when 'not'
          valid = !trial_scoped(check.fetch('target'), value, path).first
          mismatch(source, path, 'negated schema accepted the value') unless valid
        when 'if'
          condition, annotations = trial_scoped(check.fetch('condition'), value, path)
          merge_scope(local, annotations, source, path) if condition
          target = check.fetch(condition ? 'thenTarget' : 'elseTarget')
          return target.nil? ? [true, produced] : eval_scoped(target, value, path)
        when 'dependentSchemas'
          valid, passing = true, []
          if value.instance_of?(Hash)
            check.fetch('dependencies').each do |dependency|
              spend(source, path)
              next unless value.key?(dependency.fetch('name'))
              ok, annotations = eval_scoped(dependency.fetch('target'), value, path)
              valid = ok && valid
              passing << annotations if ok
            end
          end
          passing.each { |scope| merge_scope(produced, scope, source, path) } if valid
        when 'dependentRequired'
          valid = true
          if value.instance_of?(Hash)
            check.fetch('dependencies').each do |trigger, names|
              spend(source, path)
              next unless value.key?(trigger)
              names.each do |name|
                spend(source, path)
                valid = mismatch(source_child(source, trigger), path, 'dependent required property is absent') unless value.key?(name)
              end
            end
          end
        when 'required'
          valid = true
          if value.instance_of?(Hash)
            check.fetch('names').each do |name|
              spend(source, path)
              valid = mismatch(source, path, 'required property is absent') unless value.key?(name)
            end
          end
        when 'properties'
          valid = true
          if value.instance_of?(Hash)
            check.fetch('properties').each do |property|
              spend(source, path)
              name = property.fetch('name')
              next unless value.key?(name)
              ok = eval_scoped(property.fetch('target'), value[name], Internal.child_path(path, name)).first
              valid = ok && valid
              produced[0][name] = true
            end
          end
        when 'patternProperties'
          valid = true
          if value.instance_of?(Hash)
            value.keys.sort.each do |name|
              spend(source, path)
              check.fetch('patterns').each do |_, nfa, target|
                spend(source, path)
                next unless scoped_pattern?(nfa, name, source, path)
                ok = eval_scoped(target, value[name], Internal.child_path(path, name)).first
                valid = ok && valid
                produced[0][name] = true
              end
            end
          end
        when 'additionalProperties', 'additionalPropertiesWithPatterns'
          valid = true
          patterns = op == 'additionalPropertiesWithPatterns' ? node.fetch('checks').find { |c| c['op'] == 'patternProperties' }.fetch('patterns') : []
          if value.instance_of?(Hash)
            value.keys.sort.each do |name|
              spend(source, path)
              next if check.fetch('declared').include?(name)
              excluded = patterns.any? do |_, nfa, _|
                spend(source, path)
                scoped_pattern?(nfa, name, source, path)
              end
              next if excluded
              ok = eval_scoped(check.fetch('target'), value[name], Internal.child_path(path, name)).first
              valid = ok && valid
              produced[0][name] = true
            end
          end
        when 'propertyNames'
          valid = true
          if value.instance_of?(Hash)
            value.keys.sort.each do |name|
              spend(source, path)
              # Distinct, stable temporary instance identity for this key. Child
              # evaluation never borrows a value's or another key's scope.
              key = name.dup
              ok = eval_scoped(check.fetch('target'), key, Internal.child_path(path, name)).first
              valid = ok && valid
            end
          end
        when 'items', 'prefixItems'
          valid = true
          if value.instance_of?(Array)
            start = op == 'items' ? check.fetch('start') : 0
            finish = op == 'items' ? value.length : [value.length, check.fetch('targets').length].min
            (start...finish).each do |i|
              spend(source, path)
              target = op == 'items' ? check.fetch('target') : check.fetch('targets').fetch(i)
              ok = eval_scoped(target, value[i], Internal.child_path(path, i.to_s)).first
              valid = ok && valid
              produced[1][i] = true
            end
          end
        when 'contains'
          valid = true
          if value.instance_of?(Array)
            matches = empty_scope
            value.each_with_index do |child, index|
              spend(source, path)
              matches[1][index] = true if trial_scoped(check.fetch('target'), child, Internal.child_path(path, index.to_s)).first
            end
            count = matches[1].length
            minimum, maximum = check.values_at('minimum', 'maximum')
            min_at, max_at = source_child(node['source'], 'minContains'), source_child(node['source'], 'maxContains')
            lower = minimum.nil? ? nil : number(minimum, min_at, path)
            upper = maximum.nil? ? nil : number(maximum, max_at, path)
            contains_ok = count.positive? || lower && lower.sign.zero?
            merge_scope(local, matches, source, path) if contains_ok
            actual = Exact.new(count.to_s)
            valid_min = lower ? actual.compare(lower) >= 0 : count.positive?
            valid_max = upper.nil? || actual.compare(upper) <= 0
            mismatch(minimum.nil? ? source : min_at, path, 'too few contains matches') unless valid_min
            mismatch(max_at, path, 'too many contains matches') unless valid_max
            valid = valid_min && valid_max
          end
        when 'unevaluatedProperties'
          valid = true
          if value.instance_of?(Hash)
            value.keys.sort.each do |name|
              spend(source, path)
              next if local[0].key?(name)
              ok = eval_scoped(check.fetch('target'), value[name], Internal.child_path(path, name)).first
              valid = ok && valid
              produced[0][name] = true
            end
          end
        when 'unevaluatedItems'
          valid = true
          if value.instance_of?(Array)
            value.each_with_index do |child, i|
              spend(source, path)
              next if local[1].key?(i)
              ok = eval_scoped(check.fetch('target'), child, Internal.child_path(path, i.to_s)).first
              valid = ok && valid
              produced[1][i] = true
            end
          end
        when 'pattern'
          valid = !value.instance_of?(String) || scoped_pattern?(check.fetch('program'), value, source, path)
          mismatch(source, path, 'pattern assertion rejected the value') unless valid
        else
          valid = scalar(check, value, path)
        end
        [valid, produced]
      end

      # Same portable Thompson NFA and logical charges as the shared executable
      # profile. v1 keeps its existing implementation and visit behavior.
      def scoped_pattern?(program, text, source, path)
        states = program.fetch('states')
        seen, seeds, epoch, position = Array.new(states.length, 0), [], 0, 0
        scalars = text.each_codepoint
        loop do
          spend(source, path); epoch += 1
          stack, active = [], []
          enqueue = lambda do |index|
            spend(source, path)
            unless seen[index] == epoch
              seen[index] = epoch; stack << index
            end
          end
          enqueue.call(program.fetch('start'))
          seeds.each { |i| enqueue.call(i) }
          until stack.empty?
            spend(source, path)
            index = stack.pop; state = states.fetch(index)
            case state.fetch('op')
            when 'match' then return true
            when 'split' then enqueue.call(state['second']); enqueue.call(state['first'])
            when 'jump' then enqueue.call(state['target'])
            when 'start' then enqueue.call(state['target']) if position.zero?
            when 'end' then enqueue.call(state['target']) if position == text.bytesize
            when 'char' then active << index
            end
          end
          begin
            scalar = scalars.next
          rescue StopIteration
            return false
          end
          position += scalar < 0x80 ? 1 : scalar < 0x800 ? 2 : scalar < 0x10000 ? 3 : 4
          seeds = []
          active.each do |index|
            state = states[index]
            state.fetch('ranges').each do |low, high|
              spend(source, path)
              break if scalar < low
              if scalar <= high
                seeds << state['target']; break
              end
            end
          end
        end
      end
    end
  end
end
