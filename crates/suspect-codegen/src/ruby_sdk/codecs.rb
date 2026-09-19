# frozen_string_literal: true

module __NAMESPACE__
  # Base for generated keyword models. Fields remain mutable; every encode
  # checks the complete source program again, including mutations in children.
  class Model
    # @return [self]
    def validate!
      Internal::CodecSession.new.encode(self.class::SCHEMA_INDEX, self)
      self
    end

    # @return [Hash{String => Object}] exact wire keys and values, without UNSET
    def to_h = Internal::CodecSession.new.encode(self.class::SCHEMA_INDEX, self)

    # @return [String] source-validated exact JSON
    def to_json = Json.dump(to_h)

    # Payloads (including source-declared secrets) are deliberately not inspected.
    def inspect = "#<#{self.class.name}>"
  end

  # One immutable source-bound codec. Raw JSON parsing happens before validation;
  # native object/union materialization happens only after source validation.
  class Codec
    attr_reader :schema_index, :source

    def initialize(schema_index)
      @schema_index = schema_index
      @source = Internal::SHAPES.fetch(schema_index).fetch(:source)
      freeze
    end

    # @param value [Object] exact JSON domain (string keys, no Float or UNSET)
    # @return [Object] the generated native model/union/scalar
    # @raise [ValidationError, EvaluationFailure, CodecError, JsonError]
    def decode(value) = Internal::CodecSession.new.decode(@schema_index, value)

    # @param text [String] exact UTF-8 JSON bytes
    # @return [Object] the generated native model/union/scalar
    def decode_json(text, max_bytes: Internal::POLICY[:max_response_bytes])
      value = Json.parse(text, max_bytes: max_bytes)
      Internal::CodecSession.new.decode(@schema_index, value, parsed: true)
    rescue JsonError => error
      raise JsonError.new(error.message, source: @source, instance_path: error.instance_path), cause: error
    end

    # @return [Object] exact JSON-domain value, validated after native conversion
    def encode(value) = Internal::CodecSession.new.encode(@schema_index, value)

    # @return [String] exact, deterministic source-validated JSON bytes
    def encode_json(value, max_bytes: Internal::POLICY[:max_request_bytes])
      Json.dump(encode(value), max_bytes: max_bytes)
    end
  end

  # @api private
  module Internal
    # All native conversion visits and copied bytes share this per-call budget.
    class CodecSession
      def initialize
        @work, @active = POLICY.fetch(:max_conversion_steps), {}
        @validation = ValidationSession.new
      end

      def failure(index, path, message)
        raise CodecError.new(message, source: SHAPES.fetch(index).fetch(:source), instance_path: path)
      end

      def spend(index, path, depth, amount = 1)
        @work -= amount
        if @work.negative? || depth > POLICY.fetch(:max_json_depth)
          raise EvaluationFailure.new('native conversion work/depth limit exceeded', source: SHAPES.fetch(index).fetch(:source), instance_path: path)
        end
      end

      def encode(index, value)
        wire = encode_node(index, value, '', 0)
        @validation.check(index, wire)
        wire
      end

      def decode(index, value, parsed: false)
        wire = parsed ? value : copy_json(index, value, '', 0)
        @validation.check(index, wire)
        decode_node(index, wire, '', 0)
      end

      def guarded(index, value, path)
        identity = value.object_id
        failure(index, path, 'cyclic native container') if @active[identity]
        @active[identity] = true
        begin
          yield
        ensure
          @active.delete(identity)
        end
      end

      def copy_json(index, value, path, depth)
        spend(index, path, depth)
        case
        when value.nil?, value.equal?(true), value.equal?(false) then value
        when value.instance_of?(JsonNumber), value.instance_of?(Integer)
          spend(index, path, depth, Internal.number_token(value).bytesize)
          value
        when value.instance_of?(String)
          Internal.utf8!(value)
          spend(index, path, depth, value.bytesize)
          value.dup
        when value.instance_of?(Array)
          guarded(index, value, path) do
            value.each_with_index.map { |child, i| copy_json(index, child, Internal.child_path(path, i.to_s), depth + 1) }
          end
        when value.instance_of?(Hash)
          guarded(index, value, path) do
            value.to_h do |key, child|
              Internal.utf8!(key)
              spend(index, path, depth, key.bytesize)
              [key.dup, copy_json(index, child, Internal.child_path(path, key), depth + 1)]
            end
          end
        else failure(index, path, 'value is outside the exact JSON domain; native models, Float, symbols and UNSET are not arbitrary JSON')
        end
      rescue JsonError => error
        raise CodecError.new('invalid exact JSON value', source: SHAPES.fetch(index).fetch(:source), instance_path: path), cause: error
      end

      def native_kind?(index, value, depth)
        spend(index, '', depth)
        shape = SHAPES.fetch(index)
        kind = Internal.json_kind(value)
        case shape.fetch(:kind)
        when :alias then native_kind?(shape.fetch(:target), value, depth + 1)
        when :json, :literal, :dynamic then kind != 'invalid'
        when :never then false
        when :scalar then shape.fetch(:types).include?(kind) || kind == 'number' && shape.fetch(:types).include?('integer')
        when :object then value.instance_of?(MODEL_CLASSES.fetch(index)) || shape.fetch(:nullable) && value.nil?
        when :array then value.instance_of?(Array) || shape.fetch(:nullable) && value.nil?
        when :union then shape.fetch(:branches).any? { |target| native_kind?(target, value, depth + 1) }
        else failure(index, '', 'unknown native representation')
        end
      end

      def encode_node(index, value, path, depth)
        spend(index, path, depth)
        shape = SHAPES.fetch(index)
        case shape.fetch(:kind)
        when :alias then encode_node(shape.fetch(:target), value, path, depth + 1)
        when :never then failure(index, path, 'the source schema has no valid values')
        # Dynamic sites retain exact JSON. Only the complete rooted validator
        # selects a target; conversion never retries a fallback out of context.
        when :json, :literal, :dynamic then copy_json(index, value, path, depth)
        when :scalar
          failure(index, path, 'wrong native scalar kind') unless native_kind?(index, value, depth)
          copy_json(index, value, path, depth)
        when :union
          shape.fetch(:branches).each do |target|
            next unless native_kind?(target, value, depth + 1)
            begin
              wire = encode_node(target, value, path, depth + 1)
            rescue EvaluationFailure
              raise
            rescue CodecError
              next
            end
            # The selected native arm must still be valid. A mutated model
            # cannot silently turn into a different arm that the parent accepts.
            return wire if @validation.matches?(target, wire, path)
          end
          raise ValidationError.new('no source-valid native union arm', source: shape.fetch(:source), instance_path: path)
        when :array
          return nil if value.nil? && shape.fetch(:nullable)
          failure(index, path, 'expected an Array') unless value.instance_of?(Array)
          guarded(index, value, path) do
            value.each_with_index.map do |child, i|
              target = shape.fetch(:prefix)[i] || shape[:items]
              child_path = Internal.child_path(path, i.to_s)
              target ? encode_node(target, child, child_path, depth + 1) : copy_json(index, child, child_path, depth + 1)
            end
          end
        when :object
          return nil if value.nil? && shape.fetch(:nullable)
          failure(index, path, 'expected the generated keyword model class') unless value.instance_of?(MODEL_CLASSES.fetch(index))
          guarded(index, value, path) do
            result = {}
            shape.fetch(:fields).each do |field|
              spend(index, path, depth)
              child = value.instance_variable_get(field.fetch(:ivar))
              child_path = Internal.child_path(path, field.fetch(:wire))
              if child.equal?(UNSET)
                failure(index, child_path, 'required member cannot be UNSET') if field.fetch(:required)
              else
                result[field.fetch(:wire)] = encode_node(field.fetch(:target), child, child_path, depth + 1)
              end
            end
            unless shape.fetch(:extras) == :closed
              extras = value.extra_fields
              failure(index, path, 'extra_fields must be a string-keyed Hash') unless extras.instance_of?(Hash)
              declared = shape.fetch(:fields).map { |field| field.fetch(:wire) }
              extras.each do |key, child|
                Internal.utf8!(key)
                spend(index, path, depth, key.bytesize + 1)
                failure(index, path, 'extra field collides with a declared wire property') if declared.include?(key)
                child_path = Internal.child_path(path, key)
                result[key.dup] = shape[:extras] == :json ? copy_json(index, child, child_path, depth + 1) : encode_node(shape.fetch(:extras), child, child_path, depth + 1)
              end
            end
            result
          end
        else failure(index, path, 'unknown native representation')
        end
      end

      def decode_node(index, value, path, depth)
        spend(index, path, depth)
        shape = SHAPES.fetch(index)
        case shape.fetch(:kind)
        when :alias then decode_node(shape.fetch(:target), value, path, depth + 1)
        when :json, :literal, :scalar, :dynamic then copy_json(index, value, path, depth)
        when :never then failure(index, path, 'a false schema cannot decode a value')
        when :union
          matches = shape.fetch(:branches).select { |target| @validation.matches?(target, value, path) }
          failure(index, path, 'validated union has no native branch') if matches.empty? || shape.fetch(:exclusive) && matches.length != 1
          # Inclusive anyOf uses its first source-order valid branch as a carrier;
          # open-object extras retain all remaining wire members. Every branch
          # was tried with the same evaluation budget before selecting it.
          decode_node(matches.first, value, path, depth + 1)
        when :array
          return nil if value.nil?
          value.each_with_index.map do |child, i|
            target = shape.fetch(:prefix)[i] || shape[:items]
            child_path = Internal.child_path(path, i.to_s)
            target ? decode_node(target, child, child_path, depth + 1) : copy_json(index, child, child_path, depth + 1)
          end
        when :object
          return nil if value.nil?
          model = MODEL_CLASSES.fetch(index).allocate
          declared = shape.fetch(:fields).map { |field| field.fetch(:wire) }
          shape.fetch(:fields).each do |field|
            spend(index, path, depth)
            wire = field.fetch(:wire)
            child = value.key?(wire) ? decode_node(field.fetch(:target), value[wire], Internal.child_path(path, wire), depth + 1) : UNSET
            model.instance_variable_set(field.fetch(:ivar), child)
          end
          unless shape.fetch(:extras) == :closed
            extras = {}
            value.each do |key, child|
              spend(index, path, depth, key.bytesize + 1)
              next if declared.include?(key)
              child_path = Internal.child_path(path, key)
              extras[key.dup] = shape[:extras] == :json ? copy_json(index, child, child_path, depth + 1) : decode_node(shape.fetch(:extras), child, child_path, depth + 1)
            end
            model.extra_fields = extras
          end
          model
        else failure(index, path, 'unknown native representation')
        end
      end
    end
  end
end
