# frozen_string_literal: true

module __NAMESPACE__
  # Base for SDK-owned failures. Messages never contain response bodies or credentials.
  class SdkError < StandardError
    attr_reader :kind, :source, :instance_path, :operation_id, :status, :headers, :capture, :truncated

    def initialize(message, kind: :sdk, source: nil, instance_path: '', operation_id: nil,
                   status: nil, headers: {}, capture: ''.b, truncated: false)
      super(message)
      @kind, @source, @instance_path, @operation_id = kind, source, instance_path, operation_id
      @status, @headers, @capture, @truncated = status, headers, capture, truncated
    end
  end

  # Invalid JSON syntax, native JSON-domain value, or JSON resource exhaustion.
  class JsonError < SdkError
    def initialize(message, **context)
      super(message, kind: :json, **context)
    end
  end

  # A value cannot be represented by the source-bound native model.
  class CodecError < SdkError
    def initialize(message, kind: :codec, **context)
      super(message, kind: kind, **context)
    end
  end

  # Completed schema rejection; distinct from incomplete evaluation.
  class ValidationError < CodecError
    def initialize(message, **context)
      super(message, kind: :invalid, **context)
    end
  end

  # Resource exhaustion or nonproductive recursion. Logical trials cannot suppress it.
  class EvaluationFailure < CodecError
    def initialize(message, **context)
      super(message, kind: :evaluation_failure, **context)
    end
  end

  # Explicit absence. Only {UNSET} is an absent value; nil is JSON null.
  class Unset
    def inspect = 'UNSET'
    alias to_s inspect
  end
  UNSET = Unset.new.freeze
  Unset.private_class_method(:new)

  # Exact JSON numeric token, including decimals and symbolic unbounded exponents.
  # Integer inputs are also accepted by numeric codecs. Float is never coerced.
  # @example Exact construction
  #   JsonNumber.new('9007199254740993.000000000000000001').token
  class JsonNumber
    include Comparable
    TOKEN = /\A-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?\z/n.freeze
    attr_reader :token

    def initialize(token)
      unless token.instance_of?(String) && token.bytesize <= Internal::POLICY[:max_number_bytes] && token.ascii_only? && TOKEN.match?(token)
        raise JsonError, 'expected a bounded exact JSON numeric token'
      end
      @token = token.dup.freeze
      freeze
    end

    def <=>(other)
      return nil unless other.instance_of?(JsonNumber) || other.instance_of?(Integer)
      Internal::Exact.new(@token).compare(Internal::Exact.new(Internal.number_token(other)))
    end

    def eql?(other) = other.instance_of?(JsonNumber) && (self <=> other).zero?
    def hash = Internal::Exact.new(@token).key.hash
    def integer? = Internal::Exact.new(@token).integral?
    def inspect = "JsonNumber(#{@token.inspect})"
    alias to_s token

    # Explicit bounded conversion. Huge exponents are never expanded implicitly.
    # @param max_digits [Integer] maximum expanded decimal digits, at most 4096
    # @return [Integer]
    # @raise [RangeError] non-integral value or expansion outside the requested bound
    def to_i(max_digits: 4096)
      unless max_digits.instance_of?(Integer) && max_digits.between?(1, 4096)
        raise ArgumentError, 'max_digits must be in 1..4096'
      end
      number = Internal::Exact.new(@token)
      raise RangeError, 'JSON number is not mathematically integral' unless number.integral?
      return 0 if number.sign.zero?
      raise RangeError, 'integer expansion exceeds max_digits' if number.digits.length + number.exponent > max_digits
      number.sign * number.digits.to_i * (10**number.exponent)
    end

    # Explicit lossy conversion, never called by a codec or transport.
    # @return [Float]
    def to_f
      value = Float(@token)
      raise RangeError, 'number is outside finite Float range' unless value.finite?
      value
    end
  end

  # Strict exact JSON. Duplicate keys, lone surrogates, non-finite numbers,
  # object hooks, symbols, cyclic containers and binary floats are rejected.
  module Json
    module_function

    # @return [nil, Boolean, String, JsonNumber, Array, Hash]
    def parse(text, max_bytes: Internal::POLICY[:max_response_bytes],
              max_depth: Internal::POLICY[:max_json_depth], max_work: Internal::POLICY[:max_json_work])
      Reader.new(text, max_bytes, max_depth, max_work).read
    end

    # @return [String] UTF-8 JSON preserving every numeric token
    def dump(value, max_bytes: Internal::POLICY[:max_request_bytes],
             max_depth: Internal::POLICY[:max_json_depth], max_work: Internal::POLICY[:max_json_work])
      Writer.new(max_bytes, max_depth, max_work).write(value)
    end

    # @api private
    class Limits
      def initialize(max_bytes, max_depth, max_work)
        unless [max_bytes, max_work].all? { |n| n.instance_of?(Integer) && n.between?(1, 128 * 1024 * 1024) } &&
               max_depth.instance_of?(Integer) && max_depth.between?(1, 128)
          raise ArgumentError, 'JSON limits must be positive and within the finite runtime profile'
        end
        @max_bytes, @max_depth, @work = max_bytes, max_depth, max_work
      end

      def spend(amount = 1)
        @work -= amount
        raise JsonError, 'JSON work limit exceeded' if @work.negative?
      end

      def depth!(depth)
        raise JsonError, 'JSON nesting limit exceeded' if depth > @max_depth
      end
    end

    # @api private
    class Reader < Limits
      NUMBER = /\G-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/n.freeze
      HEX = /\A[0-9a-fA-F]{4}\z/n.freeze

      def initialize(text, max_bytes, max_depth, max_work)
        super(max_bytes, max_depth, max_work)
        raise JsonError, 'JSON input must be a String' unless text.instance_of?(String)
        raise JsonError, 'JSON input byte limit exceeded' if text.bytesize > @max_bytes
        spend(text.bytesize)
        @text = text.dup.force_encoding(Encoding::UTF_8)
        raise JsonError, 'JSON input is not valid UTF-8' unless @text.valid_encoding?
        @text.force_encoding(Encoding::BINARY)
        @at = 0
      end

      def read
        value = value(0)
        whitespace
        fail_json('trailing JSON bytes') unless @at == @text.bytesize
        value
      end

      private

      def fail_json(message)
        raise JsonError, "#{message} at byte #{@at}"
      end

      def whitespace
        while [9, 10, 13, 32].include?(@text.getbyte(@at))
          spend
          @at += 1
        end
      end

      def value(depth)
        depth!(depth)
        spend
        whitespace
        case @text.getbyte(@at)
        when 34 then string
        when 123 then object(depth)
        when 91 then array(depth)
        when 116 then literal('true', true)
        when 102 then literal('false', false)
        when 110 then literal('null', nil)
        else
          match = NUMBER.match(@text, @at)
          fail_json('expected a JSON value') unless match
          token = match[0]
          @at = match.end(0)
          JsonNumber.new(token)
        end
      end

      def literal(token, value)
        fail_json('invalid JSON literal') unless @text.byteslice(@at, token.bytesize) == token
        @at += token.bytesize
        value
      end

      def object(depth)
        @at += 1
        result = {}
        whitespace
        if @text.getbyte(@at) == 125
          @at += 1
          return result
        end
        loop do
          spend
          whitespace
          fail_json('expected an object key') unless @text.getbyte(@at) == 34
          key = string
          fail_json('duplicate decoded object key') if result.key?(key)
          whitespace
          fail_json('expected colon') unless @text.getbyte(@at) == 58
          @at += 1
          result[key] = value(depth + 1)
          whitespace
          byte = @text.getbyte(@at)
          @at += 1
          break if byte == 125
          fail_json('expected comma or object end') unless byte == 44
        end
        result
      end

      def array(depth)
        @at += 1
        result = []
        whitespace
        if @text.getbyte(@at) == 93
          @at += 1
          return result
        end
        loop do
          result << value(depth + 1)
          whitespace
          byte = @text.getbyte(@at)
          @at += 1
          break if byte == 93
          fail_json('expected comma or array end') unless byte == 44
        end
        result
      end

      def hex_unit
        text = @text.byteslice(@at, 4)
        fail_json('invalid Unicode escape') unless text && HEX.match?(text)
        @at += 4
        text.to_i(16)
      end

      def string
        @at += 1
        result = +''.b
        loop do
          spend
          byte = @text.getbyte(@at)
          fail_json('unterminated JSON string') unless byte
          @at += 1
          break if byte == 34
          if byte == 92
            escape = @text.getbyte(@at)
            @at += 1
            case escape
            when 34, 47, 92 then result << escape
            when 98 then result << 8
            when 102 then result << 12
            when 110 then result << 10
            when 114 then result << 13
            when 116 then result << 9
            when 117
              unit = hex_unit
              if unit.between?(0xD800, 0xDBFF)
                fail_json('missing low surrogate') unless @text.byteslice(@at, 2) == '\\u'
                @at += 2
                low = hex_unit
                fail_json('invalid low surrogate') unless low.between?(0xDC00, 0xDFFF)
                unit = 0x10000 + ((unit - 0xD800) << 10) + low - 0xDC00
              elsif unit.between?(0xDC00, 0xDFFF)
                fail_json('unpaired low surrogate')
              end
              result << [unit].pack('U').b
            else fail_json('invalid string escape')
            end
          elsif byte < 32
            fail_json('unescaped string control character')
          else
            result << byte
          end
        end
        result.force_encoding(Encoding::UTF_8)
        fail_json('invalid UTF-8 string') unless result.valid_encoding?
        result
      end
    end

    # @api private
    class Writer < Limits
      def initialize(max_bytes, max_depth, max_work)
        super
        @output, @active = +'', {}
      end

      def write(value)
        value(value, 0)
        @output
      end

      private

      def append(text)
        raise JsonError, 'JSON output byte limit exceeded' if @output.bytesize + text.bytesize > @max_bytes
        spend(text.bytesize)
        @output << text
      end

      def string(text)
        Internal.utf8!(text)
        spend(text.bytesize)
        append('"')
        text.each_codepoint do |codepoint|
          case codepoint
          when 34 then append('\\"')
          when 92 then append('\\\\')
          when 0...32 then append(format('\\u%04x', codepoint))
          else append([codepoint].pack('U'))
          end
        end
        append('"')
      end

      def value(value, depth)
        depth!(depth)
        spend
        case
        when value.nil? then append('null')
        when value.equal?(true) then append('true')
        when value.equal?(false) then append('false')
        when value.instance_of?(String) then string(value)
        when value.instance_of?(JsonNumber), value.instance_of?(Integer) then append(Internal.number_token(value))
        when value.instance_of?(Array), value.instance_of?(Hash)
          identity = value.object_id
          raise JsonError, 'cyclic JSON container' if @active[identity]
          @active[identity] = true
          begin
            if value.instance_of?(Array)
              append('[')
              value.each_with_index { |child, i| append(',') unless i.zero?; value(child, depth + 1) }
              append(']')
            else
              value.each_key { |key| Internal.utf8!(key); spend(key.bytesize + 1) }
              append('{')
              value.keys.sort.each_with_index do |key, i|
                append(',') unless i.zero?
                string(key)
                append(':')
                value(value[key], depth + 1)
              end
              append('}')
            end
          ensure
            @active.delete(identity)
          end
        else raise JsonError, 'value is outside the exact JSON domain (Float and UNSET are not JSON values)'
        end
      end
    end
  end

  # @api private
  module Internal
    module_function

    def number_token(value)
      return value.token if value.instance_of?(JsonNumber)
      unless value.instance_of?(Integer) && value.bit_length <= POLICY[:max_number_bytes] * 4
        raise JsonError, 'expected a bounded exact numeric value'
      end
      token = value.to_s
      raise JsonError, 'numeric token byte limit exceeded' if token.bytesize > POLICY[:max_number_bytes]
      token
    end

    def utf8!(value)
      unless value.instance_of?(String) && value.valid_encoding? &&
             (value.encoding == Encoding::UTF_8 || value.ascii_only? && value.encoding.ascii_compatible?)
        raise JsonError, 'expected a UTF-8 string'
      end
      value
    end

    def freeze_tree(value)
      case value
      when Hash then value.each { |key, child| key.freeze; freeze_tree(child) }
      when Array then value.each { |child| freeze_tree(child) }
      end
      value.freeze
    end

    # Normalized exact decimal. Exponent magnitude is never expanded.
    class Exact
      attr_reader :sign, :digits, :exponent

      def initialize(token)
        negative = token.start_with?('-')
        mantissa, exponent = token.delete_prefix('-').split(/[eE]/, 2)
        whole, fraction = mantissa.split('.', 2)
        fraction ||= ''
        coefficient = (whole + fraction).sub(/\A0+/, '')
        if coefficient.empty?
          @sign, @digits, @exponent = 0, '0', 0
        else
          @sign = negative ? -1 : 1
          @digits = coefficient.sub(/0+\z/, '')
          @exponent = (exponent || '0').to_i - fraction.length + coefficient.length - @digits.length
        end
      end

      def key = [@sign, @digits, @exponent]
      def integral? = @sign.zero? || @exponent >= 0

      def compare(other)
        return @sign <=> other.sign if @sign != other.sign
        return 0 if @sign.zero?
        order = (@digits.length + @exponent) <=> (other.digits.length + other.exponent)
        if order.zero?
          length = [@digits.length, other.digits.length].max
          order = @digits.ljust(length, '0') <=> other.digits.ljust(length, '0')
        end
        @sign * order
      end

      def multiple_of?(divisor)
        return true if @sign.zero?
        shift = @exponent - divisor.exponent
        return false if shift.negative?
        yield(@digits.length + divisor.digits.length)
        a, b = @digits.to_i, divisor.digits.to_i
        [2, 5].each do |prime|
          powers = 0
          loop do
            yield([1, b.bit_length / 8].max)
            break if powers >= shift || (b % prime) != 0
            b /= prime
            powers += 1
          end
        end
        yield([1, a.bit_length / 8].max * [1, b.bit_length / 8].max)
        (a % b).zero?
      end
    end
  end
end
