# frozen_string_literal: true
require 'securerandom'
module __NAMESPACE__
  # Finite immutable octets. No filesystem access or JSON-null substitution.
  class Bytes
    attr_reader :data
    def initialize(data)
      raise ArgumentError, 'bytes must be a String' unless data.instance_of?(String)
      raise ResourceLimitError, 'byte input exceeds runtime ceiling' if data.bytesize > [Internal::POLICY[:max_request_bytes], Internal::POLICY[:max_response_bytes]].max
      @data = data.b.freeze
      freeze
    end
    def bytesize = @data.bytesize
    def inspect = "#<#{self.class.name} bytes=#{bytesize}>"
  end
  # HTTP explicitly forbids body decoding; distinct from JSON null and bytes.
  class NoContent; end
  NO_CONTENT = NoContent.new.freeze
  NoContent.private_class_method(:new)

  # Optional MIME metadata for one typed part. Without metadata, plain native
  # values are accepted by generated part fields as a convenience.
  class Part
    attr_reader :data, :filename, :content_type, :headers
    def initialize(data:, filename: nil, content_type: nil, headers: {})
      @data, @filename, @content_type, @headers = data, filename, content_type, headers
    end
    def inspect = "#<#{self.class.name}>"
  end
  class BytePart < Part
    def initialize(bytes:, filename: nil, content_type: nil, headers: {})
      super(data: Bytes.new(bytes), filename: filename, content_type: content_type, headers: headers)
    end
  end

  # A source-bound form/part/header record with native keyword constructors.
  # Its aggregate never enters JSON Schema evaluation when it contains bytes.
  class WireModel
    def validate!
      Internal::PayloadSession.new.validate_record(self)
      self
    end
    def inspect = "#<#{self.class.name}>"
  end
  class Link
    attr_reader :name, :source, :target, :parameters, :request_body, :server, :description
    def initialize(wire)
      @name, @source, @target = wire['name'], Internal::Wire.source(wire['source']), wire['target']
      @parameters = wire['parameters'].transform_values { |v| v['value'] }.freeze
      @request_body = wire['request_body'] ? wire['request_body'].fetch('value') : UNSET
      @server, @description = wire['server'], wire['description']&.fetch('value')
      freeze
    end
  end

  # @api private
  module Internal
    class PayloadSession
      def initialize
        @codec, @work = CodecSession.new, POLICY[:max_conversion_steps]
      end
      def spend(amount = 1)
        @work -= amount
        raise EvaluationFailure, 'payload conversion budget exhausted' if @work.negative?
      end
      def encode_codec(codec, value) = @codec.encode(Wire.codec_index(codec), value)
      def decode_codec(codec, value) = @codec.decode(Wire.codec_index(codec), value)
      def record_key(kind, source) = kind + ':' + Wire.binding_source(source)
      def fields(value, record)
        raise CodecError, 'expected the generated native part/header model' unless value.instance_of?(record.fetch(:class))
        result = {}
        record.fetch(:fields).each do |f|
          spend
          child = value.instance_variable_get(f.fetch(:ivar))
          if child.equal?(UNSET)
            raise CodecError.new('required native member is absent', source: f[:source]) if f[:required]
          else result[f[:wire]] = child
          end
        end
        if record[:additional]
          extras = value.extra_fields
          raise CodecError, 'extra_fields must be a Hash' unless extras.instance_of?(Hash)
          extras.each do |k, v|
            Internal.utf8!(k); spend(k.bytesize + 1)
            raise CodecError, 'extra field collides with a named member' if result.key?(k) || record[:fields].any? { |f| f[:wire] == k }
            result[k] = v
          end
        end
        result
      end
      def build_record(key, fields)
        record = RECORDS.fetch(key)
        model = record.fetch(:class).allocate
        record[:fields].each { |f| model.instance_variable_set(f[:ivar], fields.fetch(f[:wire], UNSET)) }
        if record[:additional]
          known = record[:fields].map { |f| f[:wire] }
          model.extra_fields = fields.reject { |k, _| known.include?(k) }
        end
        model
      end
      def validate_record(value)
        spec = RECORDS.fetch(value.class::RECORD_KEY)
        if spec[:kind] == :headers
          encode_headers(spec[:wire], value)
        else
          prepare_parts(spec[:wire], value)
        end
        nil
      end
      def encode(media, value, actual_type)
        rep = media.fetch('representation')
        result = case rep.fetch('kind')
                 when 'json' then Json.dump(rep['codec'] ? encode_codec(rep['codec'], value) : copy_json(value))
                 when 'text' then Wire.scalar(rep['codec'] ? encode_codec(rep['codec'], value) : value, rep.fetch('scalar'))
                 when 'binary' then byte_value(value, rep.fetch('bytes'))
                 when 'form' then encode_form(media, value)
                 when 'multipart' then return encode_multipart(media, value, actual_type)
                 when 'stream' then encode_items(rep.fetch('stream'), value)
                 else raise CodecError, 'unknown payload representation'
                 end
        raise ResourceLimitError, 'request body ceiling exceeded' if result.bytesize > POLICY[:max_request_bytes]
        [result.b, actual_type]
      end
      def decode(media, bytes, actual)
        rep = media.fetch('representation')
        case rep.fetch('kind')
        when 'json'
          value = Json.parse(bytes)
          rep['codec'] ? decode_codec(rep['codec'], value) : value
        when 'text'
          text = bytes.dup.force_encoding(Encoding::UTF_8)
          value = Wire.text_value(text, rep.fetch('scalar'))
          rep['codec'] ? decode_codec(rep['codec'], value) : value
        when 'binary'
          raise ResourceLimitError, 'source binary byte ceiling exceeded' if bytes.bytesize > Wire.integer(rep['bytes']['max_bytes'])
          Bytes.new(bytes)
        when 'form' then decode_form(media, bytes)
        when 'multipart' then decode_multipart(media, bytes, actual)
        else raise CodecError, 'stream decoding requires its lifetime-owning Enumerator'
        end
      end
      def copy_json(value)
        # Source-free JSON still obeys the same exact JSON domain and byte limits.
        Json.parse(Json.dump(value), max_bytes: POLICY[:max_request_bytes])
      end
      def byte_value(value, policy)
        raise CodecError, 'expected Bytes, never JSON null or a filename' unless value.instance_of?(Bytes)
        raise ResourceLimitError, 'source byte/part ceiling exceeded' if value.bytesize > Wire.integer(policy.fetch('max_bytes'))
        spend(value.bytesize)
        value.data
      end
      def encode_headers(headers, value, managed: false)
        if value.is_a?(WireModel)
          value = fields(value, RECORDS.fetch(value.class::RECORD_KEY))
        end
        raise CodecError, 'part headers require a typed header model or string-keyed Hash' unless value.instance_of?(Hash)
        raise CodecError, 'duplicate case-insensitive part header' unless value.keys.map { |k| k.instance_of?(String) ? k.downcase : k }.uniq.length == value.length
        known = headers.map { |h| h['name'].downcase }
        raise CodecError, 'undeclared part header' unless value.keys.all? { |k| k.instance_of?(String) && known.include?(k.downcase) }
        out = {}
        headers.each do |h|
          key = value.keys.find { |k| k.casecmp?(h['name']) }
          unless key
            raise CodecError.new('required header is absent', source: Wire.source(h['source'])) if h['required']
            next
          end
          wire = encode_codec(h['codec'], value[key])
          raise CodecError, 'part framing headers are managed by the encoder' if managed && %w[content-type content-disposition content-length content-transfer-encoding transfer-encoding].include?(h['name'].downcase)
          out[h['name']] = Wire.serialize(h['name'], 'header', h['serialization'], wire)
        end
        out
      end
      def header_value(h, values)
        spec = h.fetch('serialization')
        if h['name'].casecmp?('set-cookie') && spec['kind'] == 'style' && spec['shape']['kind'] == 'array' && spec['shape']['items'] == 'string'
          return decode_codec(h['codec'], values.dup)
        end
        if spec['kind'] == 'content'
          raise CodecError, 'repeated content header is ambiguous' unless values.length == 1
          value = Wire.json_media?(spec['media_type']) ? Json.parse(values.first) : Wire.text_value(values.first, h['content_media']&.dig('representation', 'scalar') || 'string')
        else
          shape = spec.fetch('shape')
          if shape['kind'] == 'scalar'
            raise CodecError, 'repeated scalar header is ambiguous' unless values.length == 1
            value = Wire.text_value(values.first, shape.fetch('scalar'))
          else
            tokens = values.join(',').split(',', -1).map(&:strip)
            if shape['kind'] == 'array'
              value = tokens.map { |v| Wire.text_value(v, shape.fetch('items')) }
            else
              pairs = spec['explode'] ? tokens.map { |v| v.split('=', 2) } : tokens.each_slice(2).to_a
              value = {}
              pairs.each do |key, raw|
                raise CodecError, 'invalid/duplicate object header member' unless key && raw && !value.key?(key)
                scalar = shape.fetch('properties')[key] || shape.fetch('additional')['scalar']
                raise CodecError, 'header member has no unambiguous scalar type' unless scalar
                value[key] = Wire.text_value(raw, scalar)
              end
            end
          end
        end
        decode_codec(h['codec'], value)
      end
      def decode_headers(headers, raw, key)
        return nil if headers.empty?
        values = {}
        headers.each do |h|
          if raw.key?(h['name'].downcase)
            values[h['name']] = header_value(h, raw.fetch(h['name'].downcase))
          elsif h['required']
            raise CodecError.new('required response/part header is absent', source: Wire.source(h['source']))
          end
        end
        build_record(key, values)
      end
      def definitions(media)
        rep = media['representation']
        if rep['kind'] == 'form'
          f = rep.fetch('form'); [f['rules'], f['fields'], f['additional'], nil]
        else
          m = rep.fetch('multipart')
          m['kind'] == 'named' ? [m['rules'], m['parts'], m['additional'], nil] : [nil, m['prefix'], m['items'], m]
        end
      end
      def structure(rules, values)
        rules.fetch('required').each do |name|
          raise CodecError.new('required form/part is absent', source: Wire.source(name['source'])) unless values.key?(name['value'])
        end
        min, max = Wire.bound(rules['min_properties']), Wire.bound(rules['max_properties'])
        raise CodecError, 'form/part property cardinality rejected' if min && values.length < min || max && values.length > max
      end
      def part_descriptor(parts, additional, name)
        parts.find { |p| p['name'] == name } || (additional['kind'] == 'allowed' ? additional.fetch('part') : (raise CodecError, 'undeclared form/part member'))
      end
      def prepare_parts(media, value)
        rules, parts, additional, positional = definitions(media)
        if positional
          raise CodecError, 'positional multipart requires an Array' unless value.instance_of?(Array)
          min, max = Wire.bound(positional['min_items']), Wire.bound(positional['max_items'])
          raise CodecError, 'positional cardinality rejected' if min && value.length < min || max && value.length > max
          entries = value.each_with_index.map { |v, i| [i.to_s, parts[i] || (additional['kind'] == 'allowed' ? additional['part'] : (raise CodecError, 'extra positional part')), v] }
        else
          values = fields(value, RECORDS.fetch(record_key('media', media['source'])))
          structure(rules, values)
          entries = values.keys.sort.map { |name| [name, part_descriptor(parts, additional, name), values[name]] }
        end
        out = []
        entries.each do |name, p, input|
          values = p['multiplicity'] == 'repeated-array-items' ? input : [input]
          raise CodecError, 'repeated part requires an Array' unless values.instance_of?(Array)
          min, max = Wire.bound(p['min_items']), Wire.bound(p['max_items'])
          raise CodecError, 'empty/invalid repeated part cardinality' if p['multiplicity'] == 'repeated-array-items' && (values.empty? || min && values.length < min || max && values.length > max)
          values.each do |value|
            raise ResourceLimitError, 'part count ceiling exceeded' if out.length >= POLICY[:max_parts]
            part = value.is_a?(Part) ? value : Part.new(data: value)
            rep = p.fetch('representation')
            wire = case rep['kind']
                   when 'binary' then byte_value(part.data, rep.fetch('bytes'))
                   when 'json' then Json.dump(encode_codec(rep['codec'], part.data), max_bytes: POLICY[:max_part_bytes])
                   when 'text' then Wire.scalar(encode_codec(rep['codec'], part.data), rep.fetch('scalar'))
                   when 'style' then Wire.serialize(name, 'query', rep.fetch('serialization'), encode_codec(rep['codec'], part.data))
                   else raise CodecError, 'unknown part representation'
                   end
            raise ResourceLimitError, 'part byte ceiling exceeded' if wire.bytesize > POLICY[:max_part_bytes]
            spend(wire.bytesize + name.bytesize + 1)
            headers = encode_headers(p.fetch('headers'), part.headers, managed: true)
            content = part.content_type || (p['content_types'].length == 1 ? p['content_types'][0]['declared'] : nil)
            if !p['content_types'].empty?
              raise CodecError, 'choose a declared part content_type' unless content
              choices = p['content_types'].map { |m| {'media_type' => m, 'representation' => rep} }
              Wire.match_media(choices, content)
            elsif part.content_type
              raise CodecError, 'style encoding has no content-type choice'
            end
            out << [name, p, part, wire.b, content, headers]
          end
        end
        out
      end
      def encode_form(media, value)
        output = +''; owners = {}
        prepare_parts(media, value).each do |name, p, _, wire, _, _|
          rep = p['representation']
          raise CodecError, 'binary form field is not textual data' if rep['kind'] == 'binary'
          encoded = rep['kind'] == 'style' ? wire : Wire.percent(name, 'form-url-encoded') + '=' + Wire.percent(wire.force_encoding(Encoding::UTF_8), rep.fetch('outer_encoding'))
          encoded.split('&').each do |field|
            key = Wire.unpercent(field.split('=', 2).first, true)
            raise CodecError, 'form encodings target the same field' if owners.key?(key) && owners[key] != name
            owners[key] = name
          end
          raise ResourceLimitError, 'form byte ceiling exceeded' if output.bytesize + encoded.bytesize + 1 > POLICY[:max_request_bytes]
          output << '&' unless output.empty?
          output << encoded
        end
        output
      end
      def encode_multipart(media, value, content_type)
        actual = Wire.parse_media(content_type)
        raise CodecError, 'multipart boundary is encoder-owned' if actual['parameters'].key?('boundary')
        parts = prepare_parts(media, value)
        boundary = 'suspect-' + ::SecureRandom.hex(18)
        raise CodecError, 'generated boundary occurs in a part' if parts.any? { |p| p[3].include?(boundary) }
        out = +''.b
        append = ->(s) { raise ResourceLimitError, 'multipart body ceiling exceeded' if out.bytesize + s.bytesize > POLICY[:max_request_bytes]; out << s.b }
        named = media['representation']['multipart']['kind'] == 'named'
        parts.each do |name, _, part, bytes, content, headers|
          append.call('--' + boundary + "\r\n")
          if named
            disposition = 'form-data; name="' + Wire.percent(name) + '"'
            if part.filename
              Internal.utf8!(part.filename)
              disposition += '; filename="' + Wire.percent(part.filename) + '"'
            end
            append.call('Content-Disposition: ' + disposition + "\r\n")
          elsif part.filename
            raise CodecError, 'positional multipart has no inferred form-data filename disposition'
          end
          append.call('Content-Type: ' + content + "\r\n") if content
          headers.each { |k, v| append.call(k + ': ' + v + "\r\n") }
          append.call("\r\n"); append.call(bytes); append.call("\r\n")
        end
        append.call('--' + boundary + "--\r\n")
        [out, content_type + '; boundary=' + boundary]
      end
      def decode_part(p, bytes, actual_type, headers = {}, filename = nil, mime = false)
        rep = p.fetch('representation')
        raise ResourceLimitError, 'response part byte ceiling exceeded' if bytes.bytesize > POLICY[:max_part_bytes]
        spend(bytes.bytesize)
        actual_type ||= 'text/plain' if mime
        if !p['content_types'].empty? && actual_type
          Wire.match_media(p['content_types'].map { |m| {'media_type' => m, 'representation' => rep} }, actual_type)
        elsif mime && !p['content_types'].empty?
          raise CodecError, 'MIME part is missing Content-Type'
        end
        value = case rep['kind']
                when 'binary'
                  raise ResourceLimitError, 'source byte part ceiling exceeded' if bytes.bytesize > Wire.integer(rep['bytes']['max_bytes'])
                  Bytes.new(bytes)
                when 'json' then decode_codec(rep['codec'], Json.parse(bytes, max_bytes: POLICY[:max_part_bytes]))
                when 'text' then decode_codec(rep['codec'], Wire.text_value(bytes.dup.force_encoding(Encoding::UTF_8), rep['scalar']))
                when 'style' then decode_style_part(p, bytes.dup.force_encoding(Encoding::UTF_8))
                else raise CodecError, 'unknown part representation'
                end
        return value unless mime
        typed_headers = decode_headers(p['headers'], headers, record_key('headers', p['source']))
        Part.new(data: value, filename: filename, content_type: actual_type, headers: typed_headers || {})
      end
      def decode_style_part(p, text)
        value = Wire.deserialize(p['name'].to_s, p['representation']['serialization'], text)
        decode_codec(p['representation']['codec'], value)
      end
      def decode_form(media, bytes)
        text = bytes.dup.force_encoding(Encoding::UTF_8); Internal.utf8!(text)
        rules, parts, additional, = definitions(media)
        groups = {}
        unless text.empty?
          text.split('&', -1).each do |field|
            key, raw = field.split('=', 2)
            raise CodecError, 'form field requires name=value' unless raw
            (groups[Wire.unpercent(key, true)] ||= []) << raw
          end
        end
        values = {}
        # Exploded flat form objects contribute their own property names. Route
        # only source-declared unambiguous names, rather than inventing nesting.
        parts.each do |p|
          rep = p['representation']; spec = rep['serialization']
          if rep['kind'] == 'style' && spec['style'] == 'deepObject'
            shape = spec['shape']; object = {}
            groups.keys.select { |k| k.start_with?(p['name'] + '[') && k.end_with?(']') }.each do |key|
              property = key[(p['name'].length + 1)...-1]
              raise CodecError, 'ambiguous deepObject property' if property.include?('[') || property.include?(']')
              type = shape['properties'][property] || shape['additional']['scalar']
              raise CodecError, 'untyped deepObject field cannot be decoded' unless type
              raw = groups.delete(key)
              raise CodecError, 'duplicate deepObject property' unless raw.length == 1
              object[property] = Wire.text_value(Wire.unpercent(raw.first), type)
            end
            values[p['name']] = decode_codec(rep['codec'], object) unless object.empty?
            next
          end
          next unless rep['kind'] == 'style' && spec['style'] == 'form' && spec['explode'] && spec['shape']['kind'] == 'flat-object'
          shape = spec['shape']; object = {}
          shape['properties'].each do |key, type|
            next unless groups.key?(key)
            raise CodecError, 'exploded object collides with a named form field' if parts.any? { |other| other != p && other['name'] == key }
            raws = groups.delete(key)
            raise CodecError, 'duplicate exploded object value' unless raws.length == 1
            object[key] = Wire.text_value(Wire.unpercent(raws.first), type)
          end
          if shape['additional']['kind'] != 'forbidden'
            # Additional names have a unique owner only for a sole exploded map.
            raise CodecError, 'ambiguous additional exploded form members' unless parts.length == 1 && additional['kind'] == 'forbidden' && shape['additional']['scalar']
            groups.each do |key, raws|
              raise CodecError, 'duplicate exploded additional value' unless raws.length == 1
              object[key] = Wire.text_value(Wire.unpercent(raws.first), shape['additional']['scalar'])
            end
            groups.clear
          end
          values[p['name']] = decode_codec(rep['codec'], object) unless object.empty?
        end
        groups.each do |name, raws|
          p = part_descriptor(parts, additional, name); rep = p['representation']
          decoded = raws.map { |raw| decode_part(p, rep['kind'] == 'style' ? (Wire.percent(name) + '=' + raw).b : Wire.unpercent(raw, rep['outer_encoding'] == 'form-url-encoded').b, nil) }
          if p['multiplicity'] == 'repeated-array-items'
            min, max = Wire.bound(p['min_items']), Wire.bound(p['max_items'])
            raise CodecError, 'repeated form cardinality rejected' if min && decoded.length < min || max && decoded.length > max
            values[name] = decoded
          else
            raise CodecError, 'duplicate singleton form field' unless decoded.length == 1
            values[name] = decoded.first
          end
        end
        structure(rules, values)
        build_record(record_key('media', media['source']), values)
      end
      def mime_headers(raw)
        raise ResourceLimitError, 'part header byte ceiling exceeded' if raw.bytesize > POLICY[:max_header_bytes]
        values = {}
        raw.split("\r\n").each do |line|
          name, value = line.split(':', 2)
          raise CodecError, 'malformed MIME header' unless value && Wire.token?(name)
          raise CodecError, 'MIME transfer encoding is unsupported' if ['content-transfer-encoding', 'transfer-encoding'].include?(name.downcase)
          (values[name] ||= []) << value.strip
        end
        Internal.response_headers(values)
      end
      def disposition(text)
        tokens = Wire.split_quoted(text); raise CodecError, 'named part requires form-data disposition' unless tokens.shift.strip.casecmp?('form-data')
        fields = {}
        tokens.each do |s|
          k, v = s.strip.split('=', 2)
          raise CodecError, 'invalid or duplicate disposition parameter' unless v && !fields.key?(k.downcase)
          v = v[1...-1].gsub(/\\(.)/, '\\1') if v.start_with?('"') && v.end_with?('"')
          fields[k.downcase] = Internal.valid_percent?(v) ? Wire.unpercent(v) : Internal.utf8!(v.dup.force_encoding(Encoding::UTF_8))
        end
        raise CodecError, 'named part is missing name' unless fields['name']
        fields
      end
      def decode_multipart(media, bytes, actual)
        boundary = actual['parameters']['boundary']
        raise CodecError, 'invalid MIME boundary' unless boundary && boundary.bytesize.between?(1, 70) && /\A[0-9A-Za-z'()+_,.\/:=? -]+\z/n.match?(boundary.b) && !boundary.end_with?(' ')
        rules, parts, additional, positional = definitions(media)
        marker = '--' + boundary; at = bytes.start_with?(marker) ? 0 : bytes.index("\r\n" + marker)&.+(2)
        raise CodecError, 'missing opening multipart delimiter' unless at
        fields, array, count = {}, [], 0
        loop do
          after = at + marker.bytesize
          if bytes.byteslice(after, 2) == '--'
            tail = bytes.byteslice(after + 2, bytes.bytesize) || ''.b
            line = tail.split("\r\n", 2).first || ''.b
            raise CodecError, 'invalid closing multipart delimiter' unless /\A[ \t]*\z/n.match?(line)
            break
          end
          raise CodecError, 'invalid multipart delimiter' unless bytes.byteslice(after, 2) == "\r\n"
          head_end = bytes.index("\r\n\r\n", after + 2)
          raise CodecError, 'missing part headers/body boundary' unless head_end
          headers = mime_headers(bytes.byteslice(after + 2, head_end - after - 2))
          body_at = head_end + 4; next_at = body_at
          loop do
            next_at = bytes.index("\r\n" + marker, next_at)
            raise CodecError, 'unterminated multipart body' unless next_at
            tail = bytes.byteslice(next_at + 2 + marker.bytesize, 2)
            break if ["\r\n", '--'].include?(tail)
            next_at += 2
          end
          raw = bytes.byteslice(body_at, next_at - body_at)
          name, filename = nil, nil
          if positional
            p = parts[count] || (additional['kind'] == 'allowed' ? additional['part'] : (raise CodecError, 'undeclared positional part'))
          else
            raise CodecError, 'ambiguous MIME disposition' unless headers['content-disposition']&.length == 1
            d = disposition(headers['content-disposition'].first); name, filename = d['name'], d['filename']
            p = part_descriptor(parts, additional, name)
          end
          raise CodecError, 'ambiguous part media' if headers['content-type'] && headers['content-type'].length != 1
          child = decode_part(p, raw, headers['content-type']&.first, headers, filename, true)
          if positional then array << child
          elsif p['multiplicity'] == 'repeated-array-items' then (fields[name] ||= []) << child
          else
            raise CodecError, 'duplicate singleton MIME part' if fields.key?(name)
            fields[name] = child
          end
          count += 1; raise ResourceLimitError, 'MIME part count limit' if count > POLICY[:max_parts]
          at = next_at + 2
        end
        if positional
          min, max = Wire.bound(positional['min_items']), Wire.bound(positional['max_items'])
          raise CodecError, 'positional multipart cardinality rejected' if min && array.length < min || max && array.length > max
          array
        else
          structure(rules, fields)
          fields.each do |name, values|
            p = part_descriptor(parts, additional, name)
            next unless p['multiplicity'] == 'repeated-array-items'
            min, max = Wire.bound(p['min_items']), Wire.bound(p['max_items'])
            raise CodecError, 'repeated MIME cardinality rejected' if min && values.length < min || max && values.length > max
          end
          build_record(record_key('media', media['source']), fields)
        end
      end
      def encode_items(stream, values)
        raise CodecError, 'stream request requires Enumerable native items' unless values.respond_to?(:each)
        output = +''.b; count = 0
        values.each do |value|
          count += 1; raise ResourceLimitError, 'request stream item count ceiling' if count > POLICY[:max_stream_items]
          wire = encode_codec(stream['item_codec'], value)
          text = if stream['framing'] == 'json-lines'
                   Json.dump(wire, max_bytes: Wire.integer(stream['max_item_bytes'])) + "\n"
                 else
                   raise CodecError, 'SSE item must be an event envelope' unless wire.instance_of?(Hash) && wire['data'].instance_of?(String)
                   raise CodecError, 'unrepresentable SSE event fields' unless (wire.keys - %w[data id event retry]).empty?
                   raise CodecError, 'SSE data CR cannot be preserved' if wire['data'].include?("\r")
                   text = +''
                   %w[id event].each do |key|
                     next unless wire.key?(key)
                     v = wire[key]; raise CodecError, 'SSE field contains a line break/NUL' unless v.instance_of?(String) && !/[\r\n\x00]/.match?(v)
                     text << key + ': ' + v + "\n"
                   end
                   if wire.key?('retry')
                     v = wire['retry']; v = JsonNumber.new(Internal.number_token(v))
                     n = v.to_i; raise CodecError, 'negative SSE retry' if n.negative?
                     text << 'retry: ' + n.to_s + "\n"
                   end
                   wire['data'].split("\n", -1).each { |line| text << 'data: ' + line + "\n" }
                   text + "\n"
                 end
          raise ResourceLimitError, 'request stream item/body byte ceiling' if text.bytesize > Wire.integer(stream['max_item_bytes']) || output.bytesize + text.bytesize > POLICY[:max_request_bytes]
          output << text.b
        end
        output
      end
    end
  end
end
