# frozen_string_literal: true
require 'uri'
module __NAMESPACE__
  # @api private
  module Internal
    class ProgramGuard
      def contains_source?(parent, value)
        parent['document'] == value['document'] && (parent['pointer'] == value['pointer'] || value['pointer'].start_with?(parent['pointer'] + '/'))
      end
      def anchor_name?(name)
        name.instance_of?(String) && /\A[A-Za-z_][A-Za-z0-9_.-]*\z/n.match?(name.b)
      end
      def uri_parts(value, at)
        unless value.instance_of?(String) && value.ascii_only? && !/[\x00-\x20\x7f<>"{}|\\^`]/n.match?(value.b) && !/%(?![0-9a-fA-F]{2})/.match?(value)
          fail_program('invalid absolute resource URI', at)
        end
        # Generic RFC syntax only. Scheme-specific URI classes can reject
        # otherwise valid logical identifiers (which are never fetched here).
        parsed = ::URI::RFC3986_PARSER.split(value)
        fail_program('resource URI must be absolute', at) unless parsed.first
        match = /\A(?<scheme>[A-Za-z][A-Za-z0-9+.-]*):(?:(?:\/\/)(?<authority>[^\/?#]*))?(?<path>[^?#]*)(?:\?(?<query>[^#]*))?(?:#(?<fragment>.*))?\z/.match(value)
        fail_program('invalid resource URI components', at) unless match
        match.named_captures
      rescue ::URI::InvalidURIError
        fail_program('invalid absolute resource URI', at)
      end
      def fragment_encode(value)
        value.bytes.map { |b| b.between?(65, 90) || b.between?(97, 122) || b.between?(48, 57) || "-._~!$&'()*+,;=:@/?".bytes.include?(b) ? b.chr : format('%%%02X', b) }.join
      end
      def fragment_decode(value, at)
        fail_program('invalid resource fragment escape', at) if /%(?![0-9A-Fa-f]{2})/.match?(value)
        bytes = value.b.gsub(/%([0-9a-fA-F]{2})/n) { [$1.to_i(16)].pack('C') }.force_encoding(Encoding::UTF_8)
        fail_program('resource fragment is not UTF-8', at) unless bytes.valid_encoding?
        bytes
      end
      def resource_document(parts)
        out = parts['scheme'].downcase + ':'
        if (authority = parts['authority'])
          user, separator, host = authority.rpartition('@')
          authority = separator.empty? ? authority.downcase : user + '@' + host.downcase
          out << '//' << authority
        end
        out << parts['path']
        out << '?' << parts['query'] if parts['query']
        out
      end
      def uri_key(value, at)
        parts = uri_parts(value, at)
        document = resource_document(parts)
        fragment = fragment_decode(parts['fragment'] || '', at)
        fragment.empty? ? document : document + '#' + fragment_encode(fragment)
      end
      def canonical_document(value, at)
        parts = uri_parts(value, at)
        input, output = parts['path'].dup, +''
        until input.empty?
          if input.start_with?('../') then input = input[3..]
          elsif input.start_with?('./') then input = input[2..]
          elsif input.start_with?('/./') then input = input[2..]
          elsif input == '/.' then input = '/'
          elsif input.start_with?('/../') || input == '/..'
            input = input == '/..' ? '/' : input[3..]
            output = output[0...(output.rindex('/') || 0)]
          elsif input == '.' || input == '..' then input = ''
          else
            cut = input.index('/', input.start_with?('/') ? 1 : 0) || input.length
            output << input[0...cut]; input = input[cut..]
          end
        end
        fail_program('unrepresentable authorityless resource path', at) if !parts['authority'] && output.start_with?('//')
        parts['path'] = output
        resource_document(parts)
      end
      def check_resources
        context = object(@program['resourceContext'])
        fields(context, %w[resources nodeScopes])
        resources, scopes = array(context['resources']), array(context['nodeScopes'])
        fail_program('resource node scopes must align with nodes') unless scopes.length == @nodes.length
        fail_program('resource metadata exceeds node ceiling') if resources.length > 10_000
        physical, aliases = {}, {}
        resources.each_with_index do |resource, i|
          object(resource); at = source(resource['source'])
          fields(resource, %w[source kind canonicalUri baseUri aliases declarationSource dynamicAnchors], at)
          key = [at['document'], at['pointer']]
          fail_program('duplicate physical resource', at) if physical[key]
          physical[key] = true
          kind = resource['kind']
          fail_program('unknown resource kind', at) unless %w[document openApiDocument schema].include?(kind)
          canonical = uri_key(resource['canonicalUri'], at)
          base = resource['baseUri']; base_parts = uri_parts(base, at)
          fail_program('resource base URI must be fragment-free', at) unless base_parts['fragment'].nil?
          base_key = uri_key(base, at)
          if canonical_document(resource['canonicalUri'], at) != base || kind != 'openApiDocument' && canonical != base_key
            fail_program('inconsistent resource canonical/base URI', at)
          end
          fail_program('missing resource declaration operand', at) unless resource.key?('declarationSource')
          if resource['declarationSource']
            declaration = source(resource['declarationSource'])
            keyword = kind == 'schema' ? '$id' : kind == 'openApiDocument' ? '$self' : nil
            fail_program('identifier declaration is not at resource boundary', declaration) unless keyword && declaration == child(at, keyword)
          elsif !at['pointer'].empty? || canonical != uri_key(at['document'], at)
            fail_program('undeclared resource must keep retrieval identity', at)
          end
          names = strings(resource['aliases'], at).map do |name|
            normalized = uri_key(name, at)
            fail_program('resource alias identifies multiple physical resources', at) if aliases.key?(normalized) && aliases[normalized] != i
            aliases[normalized] = i
            normalized
          end
          fail_program('resource aliases omit canonical/base URI', at) unless names.include?(canonical) && names.include?(base_key)
        end
        used = {}
        scopes.each_with_index do |scope, i|
          at = @nodes[i]['source']
          fail_program('node scope requires three operands', at) unless scope.instance_of?(Array) && scope.length == 3
          scope[0] = integer(scope[0], at)
          fail_program('node scope resource is outside registry', at) if scope[0] >= resources.length
          schema = source(scope[1]); resource = resources[scope[0]]
          unless contains_source?(resource['source'], schema) && contains_source?(schema, at) && (resource['kind'] != 'schema' || resource['source'] == schema)
            fail_program('resource/schema roots do not contain physical node', at)
          end
          suffix = at['pointer'][resource['source']['pointer'].length..]
          expected = resource['baseUri'] + (suffix.empty? ? '' : '#' + fragment_encode(suffix))
          fail_program('node canonical address is inconsistent', at) unless scope[2] == expected
          used[scope[0]] = true
        end
        fail_program('resource registry has unreferenced metadata') unless used.length == resources.length
        resources.each_with_index do |resource, i|
          at = resource['source']; anchors = array(resource['dynamicAnchors'], at)
          fail_program('dynamic anchor entry requires three operands', at) unless anchors.all? { |a| a.instance_of?(Array) && a.length == 3 }
          names = strings(anchors.map(&:first), at)
          fail_program('dynamic bindings must be sorted by decoded name', at) unless names == names.sort
          anchors.each do |entry|
            name, location, target_index = entry
            source(location); target_index = target(target_index, location)
            unless anchor_name?(name) && location == child(@nodes[target_index]['source'], '$dynamicAnchor') && scopes[target_index][0] == i
              fail_program('invalid dynamic binding name/source/target resource', location)
            end
            entry[2] = target_index
          end
        end
      end
    end
  end
end
