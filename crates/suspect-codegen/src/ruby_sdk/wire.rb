# frozen_string_literal: true
require 'base64'
module __NAMESPACE__
  # Caller-owned Basic credential; UTF-8 bytes are encoded explicitly.
  class BasicCredential
    attr_reader :username, :password
    def initialize(username:, password:)
      Internal.utf8!(username); Internal.utf8!(password)
      raise ArgumentError, 'invalid Basic username' if username.include?(':')
      raise ArgumentError, 'Basic credential exceeds header policy' if username.bytesize + password.bytesize > Internal::POLICY[:max_header_bytes] / 2
      @username, @password = username.dup.freeze, password.dup.freeze
      freeze
    end
    def inspect = "#<#{self.class.name}>"
  end

  # OAuth/OIDC attachment is caller-selected, including its HTTP scheme.
  class AuthorizationCredential
    attr_reader :scheme, :token
    def initialize(scheme:, token:)
      unless scheme.instance_of?(String) && Internal::Wire.token?(scheme) && token.instance_of?(String) && !token.empty? && token.bytesize <= Internal::POLICY[:max_header_bytes] && !/[\x00-\x20\x7f]/n.match?(token.b)
        raise ArgumentError, 'invalid explicit Authorization credential'
      end
      @scheme, @token = scheme.dup.freeze, token.dup.freeze
      freeze
    end
    def inspect = "#<#{self.class.name}>"
  end

  # Immutable source/flow/scope metadata delivered to a credential provider.
  class CredentialContext
    attr_reader :name, :source, :scheme, :permissions, :metadata, :operation_id, :kind, :scopes, :roles, :flows, :metadata_url, :discovery_url, :url_base, :server_url
    def initialize(requirement, operation_id, server_url: nil)
      @name, @source = requirement.fetch('name'), Internal::Wire.source(requirement.fetch('source'))
      @scheme, @permissions = requirement.fetch('scheme'), requirement.fetch('permissions')
      @metadata, @operation_id = requirement.fetch('credential'), operation_id
      @kind = @metadata.fetch('kind')
      names = @permissions.fetch('names').map { |n| n.fetch('value') }.freeze
      @scopes = @permissions['kind'] == 'scopes' ? names : [].freeze
      @roles = @permissions['kind'] == 'roles' ? names : [].freeze
      @server_url = server_url&.dup&.freeze
      @url_base = %w[o-auth2 oauth2 open-id-connect].include?(@kind) ? 'effective-server' : nil
      @flows = (@metadata['flows'] || []).map { |f| OAuthFlow.new(f, server_url: @server_url) }.freeze
      @metadata_url = @metadata['metadata_url']&.fetch('value')
      @discovery_url = @metadata['discovery_url']&.fetch('value')
      freeze
    end
    def inspect = "#<#{self.class.name}>"
  end

  # Immutable metadata only; constructing a flow never performs authentication.
  class OAuthFlow
    attr_reader :kind, :source, :authorization_url, :token_url, :refresh_url, :device_authorization_url, :scopes, :url_base, :server_url
    def initialize(wire, server_url: nil)
      @kind, @source = wire['kind'], Internal::Wire.source(wire['source'])
      @authorization_url, @token_url = wire['authorization_url']&.fetch('value'), wire['token_url']&.fetch('value')
      @refresh_url, @device_authorization_url = wire['refresh_url']&.fetch('value'), wire['device_authorization_url']&.fetch('value')
      @scopes = wire.fetch('scopes').transform_values { |v| v.fetch('value') }.freeze
      @url_base, @server_url = 'effective-server', server_url&.dup&.freeze
      freeze
    end
  end

  # @api private
  module Internal
    module Wire
      module_function
      def source(value)
        return value if value.instance_of?(String)
        value = value['terminal'] || value
        value = value['source'] || value
        value.fetch('document') + '#' + value.fetch('pointer')
      end
      def integer(value) = value.instance_of?(Integer) ? value : value.to_i(max_digits: 20)
      def binding_source(value) = source(value['use_site'] || value)
      def bound(value) = value && integer(value.fetch('value'))
      def codec_index(codec) = SCHEMA_INDICES.fetch(source(codec.fetch('schema').fetch('id')))
      def token?(value) = /\A[!#$%&'*+.^_`|~0-9A-Za-z-]+\z/n.match?(value.b)
      def fail_at(at, message, kind = :request)
        raise RequestError.new(message, kind: kind, source: source(at))
      end
      def scalar(value, kind = nil)
        case kind
        when 'string' then raise CodecError, 'expected String' unless value.instance_of?(String)
        when 'boolean' then raise CodecError, 'expected boolean' unless value.equal?(true) || value.equal?(false)
        when 'integer' then raise CodecError, 'expected mathematical integer' unless (value.instance_of?(Integer) || value.instance_of?(JsonNumber)) && Exact.new(Internal.number_token(value)).integral?
        when 'number' then raise CodecError, 'expected exact number' unless value.instance_of?(Integer) || value.instance_of?(JsonNumber)
        end
        return Internal.utf8!(value) if value.instance_of?(String)
        return value ? 'true' : 'false' if value.equal?(true) || value.equal?(false)
        Internal.number_token(value)
      end
      def text_value(text, kind)
        Internal.utf8!(text)
        case kind
        when 'string' then text
        when 'boolean'
          raise CodecError, 'invalid boolean text' unless ['true', 'false'].include?(text)
          text == 'true'
        when 'integer', 'number'
          number = JsonNumber.new(text)
          raise CodecError, 'nonintegral integer text' if kind == 'integer' && !number.integer?
          number
        else raise CodecError, 'unknown scalar text representation'
        end
      end
      def percent(value, mode = 'uri-component')
        return value.dup if mode == 'none'
        Internal.utf8!(value)
        raise ResourceLimitError, 'wire value byte limit exceeded' if value.bytesize > POLICY[:max_request_bytes]
        out = +''; i = 0
        while i < value.bytesize
          b = value.getbyte(i)
          if mode == 'reserved-expansion' && b == 37 && value.byteslice(i + 1, 2)&.match?(/\A[0-9a-fA-F]{2}\z/)
            out << value.byteslice(i, 3); i += 3; next
          end
          pass = b.between?(65, 90) || b.between?(97, 122) || b.between?(48, 57) || (mode == 'form-url-encoded' ? '*-._' : '-._~').bytes.include?(b) || mode == 'reserved-expansion' && ":/?#[]@!$&'()*+,;=".bytes.include?(b)
          out << (pass ? b.chr : mode == 'form-url-encoded' && b == 32 ? '+' : format('%%%02X', b))
          raise ResourceLimitError, 'encoded wire value exceeds byte limit' if out.bytesize > POLICY[:max_request_bytes]
          i += 1
        end
        out
      end
      def unpercent(value, form = false)
        raise CodecError, 'malformed percent escape' unless Internal.valid_percent?(value)
        value = value.tr('+', ' ') if form
        decoded = value.b.gsub(/%([0-9A-Fa-f]{2})/n) { [$1.to_i(16)].pack('C') }.force_encoding(Encoding::UTF_8)
        Internal.utf8!(decoded)
      end
      def encode_value(value, mode, location, style = nil, composite = false)
        Internal.utf8!(value)
        if mode == 'none' && value.each_codepoint.any? { |c| (c < 32 || c.between?(127, 159)) && !(location == 'header' && c == 9) }
          raise CodecError, 'control character in header/cookie/part'
        end
        if mode == 'none' && location == 'cookie' && value.bytes.any? { |b| [32, 9, 34, 44, 59, 92].include?(b) || b > 126 }
          raise CodecError, 'cookie value requires caller escaping'
        end
        raise CodecError, 'value contains an ambiguous style delimiter' if style == 'spaceDelimited' && value.include?(' ') || style == 'pipeDelimited' && value.include?('|') || style == 'deepObject' && /[\[\]]/.match?(value)
        if mode == 'reserved-expansion'
          hazard = case location
                   when 'path' then /[#\[\]\/\?]/
                   when 'query', 'querystring' then /[#\[\]&=+]/
                   when 'cookie' then /[;,]/
                   else /\b\B/
                   end
          active = composite && case style
                               when 'simple', 'form', 'cookie' then value.include?(',')
                               when 'label' then /[.,]/.match?(value)
                               when 'matrix' then /[;,]/.match?(value)
                               else false
                               end
          raise CodecError, 'reserved expansion requires pre-escaped hazards/delimiters' if hazard.match?(value) || active
        end
        # Raw header/cookie composite delimiters cannot be disambiguated on decode.
        delimiter = case style
                    when 'label' then /[.,=]/
                    when 'matrix' then /[;,=]/
                    when 'form', 'cookie', 'deepObject' then /[&,;=]/
                    else /[,=]/
                    end
        raise CodecError, 'raw composite delimiter requires an API-defined escape' if mode == 'none' && composite && delimiter.match?(value)
        raise CodecError, 'label composite dot requires an API-defined escape' if composite && style == 'label' && value.include?('.')
        percent(value, mode)
      end
      def serialize(name, location, spec, value)
        mode = spec.fetch('percent_encoding')
        if spec['kind'] == 'content'
          media = spec.fetch('media_type')
          text = json_media?(media) ? Json.dump(value) : scalar(value)
          text = encode_value(text, mode, location)
          return ['query', 'cookie'].include?(location) ? percent(name) + '=' + text : text
        end
        shape, style, explode = spec.fetch('shape'), spec.fetch('style'), spec.fetch('explode')
        key = percent(name, location == 'header' || style == 'cookie' ? 'none' : 'uri-component')
        encode = ->(s) { encode_value(s, mode, location, style, shape['kind'] != 'scalar') }
        items, pairs, atom = [], [], nil
        case shape.fetch('kind')
        when 'scalar' then atom = encode.call(scalar(value, shape.fetch('scalar')))
        when 'array'
          raise CodecError, 'empty or non-array composite' unless value.instance_of?(Array) && !value.empty?
          items = value.map { |v| encode.call(scalar(v, shape.fetch('items'))) }
        when 'flat-object'
          raise CodecError, 'empty or non-object composite' unless value.instance_of?(Hash) && !value.empty?
          value.keys.each { |k| Internal.utf8!(k) }
          value.keys.sort.each do |k|
            kind = shape.fetch('properties')[k]
            unless kind
              extra = shape.fetch('additional')
              raise CodecError, 'undeclared flat member' if extra['kind'] == 'forbidden'
              kind = extra['scalar'] if extra['kind'] == 'typed'
            end
            pairs << [encode.call(k), encode.call(scalar(value[k], kind))]
          end
        else raise CodecError, 'unknown wire shape'
        end
        flat = ->(sep) { pairs.flatten.join(sep) }
        paired = ->(sep) { pairs.map { |k, v| k + '=' + v }.join(sep) }
        matrix = ->(k, v) { ';' + k + (v.empty? ? '' : '=' + v) }
        array = shape['kind'] == 'array'
        result = case style
                 when 'simple' then atom || (array ? items.join(',') : explode ? paired.call(',') : flat.call(','))
                 when 'label' then '.' + (atom || (array ? items.join(explode ? '.' : ',') : explode ? paired.call('.') : flat.call(',')))
                 when 'matrix'
                   if atom then matrix.call(key, atom)
                   elsif explode then array ? items.map { |v| matrix.call(key, v) }.join : pairs.map { |k, v| matrix.call(k, v) }.join
                   else ';' + key + '=' + (array ? items.join(',') : flat.call(','))
                   end
                 when 'form', 'cookie'
                   sep = style == 'cookie' ? '; ' : '&'
                   if atom then key + '=' + atom
                   elsif explode then array ? items.map { |v| key + '=' + v }.join(sep) : paired.call(sep)
                   else key + '=' + (array ? items.join(',') : flat.call(','))
                   end
                 when 'spaceDelimited', 'pipeDelimited'
                   sep = style == 'spaceDelimited' ? '%20' : '%7C'
                   key + '=' + (array ? items.join(sep) : flat.call(sep))
                 when 'deepObject' then pairs.map { |k, v| key + '%5B' + k + '%5D=' + v }.join('&')
                 else raise CodecError, 'unknown parameter style'
                 end
        raise ResourceLimitError, 'serialized parameter exceeds URL/header policy' if result.bytesize > POLICY[:max_url_bytes]
        result
      end
      def json_media?(media)
        range = media.fetch('range')
        range['kind'] == 'concrete' && (range['type_name'] == 'application' && range['subtype'] == 'json' || range['subtype'].end_with?('+json'))
      end
      # Inverse of the retained finite RFC6570 strategy for encoded part values.
      # Active delimiters were guarded during encoding; duplicate names fail.
      def deserialize(name, spec, text)
        shape, style, explode, mode = spec.values_at('shape', 'style', 'explode', 'percent_encoding')
        decode = ->(value) { mode == 'none' ? Internal.utf8!(value) : unpercent(value) }
        atom = ->(value, type) { text_value(decode.call(value), type) }
        key = percent(name.to_s)
        object = lambda do |pairs|
          values = {}
          pairs.each do |k, v|
            raise CodecError, 'malformed object serialization' unless k && v
            k = decode.call(k)
            raise CodecError, 'duplicate object serialization key' if values.key?(k)
            type = shape['properties'][k] || shape['additional']['scalar']
            raise CodecError, 'untyped object member has no unambiguous text value' unless type
            values[k] = atom.call(v, type)
          end
          values
        end
        if style == 'deepObject'
          pairs = text.split('&', -1).map do |entry|
            k, v = entry.split('=', 2)
            prefix = key + '%5B'
            raise CodecError, 'malformed deepObject part' unless k&.start_with?(prefix) && k.end_with?('%5D') && v
            [k[prefix.length...-3], v]
          end
          return object.call(pairs)
        end
        if style == 'label'
          raise CodecError, 'missing label prefix' unless text.start_with?('.')
          text = text[1..]
        elsif style == 'matrix'
          raise CodecError, 'missing matrix prefix' unless text.start_with?(';')
          fields = text[1..].split(';', -1).map { |s| k, v = s.split('=', 2); [k, v || ''] }
          if explode && shape['kind'] != 'scalar'
            return object.call(fields) if shape['kind'] == 'flat-object'
            raise CodecError, 'wrong matrix item name' unless fields.all? { |k, _| k == key }
            return fields.map { |_, v| atom.call(v, shape['items']) }
          end
          raise CodecError, 'ambiguous matrix field' unless fields.length == 1 && fields[0][0] == key
          text = fields[0][1]
        elsif %w[form cookie spaceDelimited pipeDelimited].include?(style)
          separator = style == 'cookie' ? '; ' : '&'
          if explode && shape['kind'] != 'scalar'
            fields = text.split(separator, -1).map { |s| s.split('=', 2) }
            return object.call(fields) if shape['kind'] == 'flat-object'
            raise CodecError, 'wrong repeated field name' unless fields.all? { |k, v| k == key && v }
            return fields.map { |_, v| atom.call(v, shape['items']) }
          end
          raise CodecError, 'missing encoded field name' unless text.start_with?(key + '=')
          text = text[(key.length + 1)..]
        end
        return atom.call(text, shape['scalar']) if shape['kind'] == 'scalar'
        sep = style == 'spaceDelimited' ? '%20' : style == 'pipeDelimited' ? '%7C' : style == 'label' && explode ? '.' : ','
        tokens = text.split(sep, -1)
        return tokens.map { |v| atom.call(v, shape['items']) } if shape['kind'] == 'array'
        pairs = explode ? tokens.map { |t| t.split('=', 2) } : tokens.each_slice(2).to_a
        object.call(pairs)
      end
      def split_quoted(value, separator = ';')
        parts, current, quoted, escaped = [], +'', false, false
        value.each_char do |c|
          if escaped then current << c; escaped = false
          elsif quoted && c == '\\' then current << c; escaped = true
          elsif c == '"' then current << c; quoted = !quoted
          elsif !quoted && c == separator then parts << current; current = +''
          else current << c
          end
        end
        raise CodecError, 'unterminated quoted HTTP value' if quoted || escaped
        parts << current
      end
      def parse_media(value)
        raise CodecError, 'invalid Content-Type' unless value.instance_of?(String) && value.bytesize <= POLICY[:max_header_bytes] && !/[\x00-\x08\x0a-\x1f\x7f]/n.match?(value.b)
        parts = split_quoted(value)
        type, subtype, extra = parts.shift.strip.split('/', -1)
        raise CodecError, 'invalid concrete Content-Type' unless type && subtype && !extra && token?(type) && token?(subtype) && !type.include?('*') && !subtype.include?('*')
        params = {}
        parts.each do |part|
          key, raw = part.strip.split('=', 2)
          raise CodecError, 'invalid/duplicate media parameter' unless raw && token?(key) && !params.key?(key.downcase)
          if raw.start_with?('"')
            raise CodecError, 'invalid quoted media parameter' unless raw.end_with?('"') && raw.length >= 2
            text = raw[1...-1]; decoded = +''; escape = false
            text.each_char do |c|
              if escape then decoded << c; escape = false
              elsif c == '\\' then escape = true
              elsif c == '"' then raise CodecError, 'unescaped media quote'
              else decoded << c
              end
            end
            raise CodecError, 'incomplete quoted pair' if escape
          else
            raise CodecError, 'invalid media value' unless token?(raw)
            decoded = raw
          end
          params[key.downcase] = decoded
        end
        {'type_name' => type.downcase, 'subtype' => subtype.downcase, 'parameters' => params, 'declared' => value}
      end
      def match_media(media, content_type)
        actual = parse_media(content_type)
        candidates = media.select do |m|
          declared = m.fetch('media_type'); range = declared.fetch('range')
          (range['kind'] == 'any' || range['type_name'] == actual['type_name'] && (range['kind'] == 'type' || range['subtype'] == actual['subtype'])) && declared.fetch('parameters').all? { |k, v| actual['parameters'].key?(k) && (k == 'charset' ? actual['parameters'][k].casecmp?(v) : actual['parameters'][k] == v) }
        end
        selected = candidates.max_by { |m| r = m['media_type']['range']['kind']; [r == 'concrete' ? 2 : r == 'type' ? 1 : 0, m['media_type']['parameters'].length] }
        raise CodecError, 'undeclared Content-Type' unless selected
        charset = actual['parameters']['charset']
        raise CodecError, 'unsupported text charset' if %w[text stream form].include?(selected['representation']['kind']) && charset && !charset.casecmp?('utf-8')
        [selected, actual]
      end
      def status_match(responses, status)
        responses.select do |r|
          s = r.fetch('status')
          s['kind'] == 'default' || s['kind'] == 'exact' && integer(s['value']) == status || s['kind'] == 'range' && integer(s['value']) == status / 100
        end.max_by { |r| {'exact' => 3, 'range' => 2, 'default' => 1}.fetch(r['status']['kind']) }
      end
      def absolute_url(value, allow_document = false)
        Internal.utf8!(value)
        raise ArgumentError, 'invalid HTTP URL' if value.bytesize > POLICY[:max_url_bytes] || !value.ascii_only? || /[\x00-\x20\x7f\\{}]/.match?(value) || !Internal.valid_percent?(value)
        uri = ::URI.parse(value)
        unless ['http', 'https'].include?(uri.scheme) && uri.hostname && !uri.hostname.empty? && !uri.userinfo && !uri.fragment && (allow_document || !uri.query) && uri.port.between?(1, 65_535)
          raise ArgumentError, 'invalid HTTP URL authority or components'
        end
        uri
      rescue ::URI::InvalidURIError
        raise ArgumentError, 'invalid HTTP URL', cause: nil
      end
      # RFC3986 path processing operates on literal path segments only. In
      # particular, %2e/%2f and their original hex case are never decoded here.
      def remove_dot_segments(path)
        input, output = path.dup, +''
        until input.empty?
          if input.start_with?('../') then input = input[3..]
          elsif input.start_with?('./') then input = input[2..]
          elsif input.start_with?('/./') then input = input[2..]
          elsif input == '/.' then input = '/'
          elsif input.start_with?('/../')
            input = input[3..]; output.sub!(%r{/?[^/]*\z}, '')
          elsif input == '/..'
            input = '/'; output.sub!(%r{/?[^/]*\z}, '')
          elsif input == '.' || input == '..' then input = ''
          else
            slash = input.index('/', input.start_with?('/') ? 1 : 0)
            length = slash || input.length
            output << input[0, length]; input = input[length..]
          end
        end
        output
      end
      def uri_parts(value)
        match = /\A(?:(?<scheme>[A-Za-z][A-Za-z0-9+.-]*):)?(?:(?:\/\/)(?<authority>[^\/?#]*))?(?<path>[^?#]*)(?:\?(?<query>[^#]*))?(?:#(?<fragment>.*))?\z/.match(value)
        raise ArgumentError, 'invalid URI reference' unless match
        match.named_captures
      end
      def resolve_server_url(base, reference)
        ref = uri_parts(reference)
        if ref['scheme']
          target = ref.dup
          target['path'] = remove_dot_segments(ref['path'])
        else
          absolute_url(base, true)
          origin = uri_parts(base)
          target = ref.dup; target['scheme'] = origin['scheme']
          if ref['authority']
            target['path'] = remove_dot_segments(ref['path'])
          else
            target['authority'] = origin['authority']
            if ref['path'].empty?
              target['path'] = origin['path']; target['query'] ||= origin['query']
            else
              merged = if ref['path'].start_with?('/')
                         ref['path']
                       elsif origin['authority'] && origin['path'].empty?
                         '/' + ref['path']
                       else
                         origin['path'].sub(%r{[^/]*\z}, '') + ref['path']
                       end
              target['path'] = remove_dot_segments(merged)
            end
          end
        end
        raise ArgumentError, 'resolved server requires an HTTP authority' unless target['authority'] && %w[http https].include?(target['scheme'].downcase)
        url = target['scheme'] + '://' + target['authority'] + target['path']
        url += '?' + target['query'] if target['query']
        url += '#' + target['fragment'] if target['fragment']
        absolute_url(url)
        url
      end
      def server(servers, selector, variables, document_url, override)
        return Internal.server_url(override) if override
        candidates = servers.fetch('candidates')
        selected = if selector.nil? || selector.equal?(UNSET)
                     raise ArgumentError, 'select a server when several are declared' if candidates.length != 1
                     candidates.first
                   elsif selector.instance_of?(Integer) then candidates.fetch(selector)
                   elsif selector.instance_of?(String) then candidates.find { |s| s['name'] && s['name']['value'] == selector }
                   end
        raise ArgumentError, 'unknown server selection' unless selected
        raise ArgumentError, 'server_variables must be a string Hash' unless variables.instance_of?(Hash) && variables.all? { |k, v| k.instance_of?(String) && v.instance_of?(String) }
        declarations = selected.fetch('variables').to_h { |v| [v.fetch('name'), v] }
        raise ArgumentError, 'undeclared server variable override' unless (variables.keys - declarations.keys).empty?
        expanded = selected.fetch('template').gsub(/\{([^{}]+)\}/) do
          v = declarations.fetch($1); value = variables.fetch($1, v.fetch('default').fetch('value'))
          raise ArgumentError, 'server variable is outside its enum' if v['values'] && !v['values'].any? { |x| x['value'] == value }
          value
        end
        raise ArgumentError, 'invalid expanded server URL' if /[\x00-\x20\x7f\\?#{}]/.match?(expanded) || !Internal.valid_percent?(expanded)
        # document_base is a physical retrieval-document root. Logical $self/$id
        # addresses and the source of an inherited operation are not URL bases.
        base = document_url || selected.fetch('document_base').fetch('source').fetch('document')
        resolve_server_url(base, expanded)
      rescue IndexError, KeyError, ::URI::InvalidURIError
        raise ArgumentError, 'invalid server selection or template', cause: nil
      end
    end
  end
end
