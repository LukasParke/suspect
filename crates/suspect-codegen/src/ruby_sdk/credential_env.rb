# frozen_string_literal: true
module __NAMESPACE__
  # @api private
  module Internal
    # @api private
    module CredentialEnvConstructor
      # Only variable names and already-bound attachment kinds are packaged.
      # Both constants are private; initialize is already reserved by native naming.
      BINDINGS = Internal.freeze_tree(__CREDENTIAL_ENV_BINDINGS__)
      OMITTED_AUTH = ::Object.new.freeze
      private_constant :BINDINGS, :OMITTED_AUTH
      private

      def initialize(auth: OMITTED_AUTH, **options)
        if auth.equal?(OMITTED_AUTH)
          credentials = {}
          BINDINGS.each do |name, variable, attachment|
            begin
              value = defined?(::ENV) ? ::ENV[variable] : nil
              next unless value.instance_of?(::String) && !value.empty?
              ceiling = attachment == :query ? POLICY[:max_url_bytes] : POLICY[:max_header_bytes]
              next if value.bytesize > ceiling
              Internal.utf8!(value)
              case attachment
              when :bearer
                next unless /\A[A-Za-z0-9._~+\/-]+=*\z/n.match?(value.b)
              when :header
                next if /[\x00-\x08\x0a-\x1f\x7f]/n.match?(value.b)
              end
              credentials[name] = value.dup.freeze
            rescue ::StandardError, ::SecurityError
              # Unavailable/invalid values stay missing. An unused alternative
              # cannot block anonymous or satisfied calls. Reader exception
              # messages/causes may contain secrets: retain neither.
              next
            end
          end
          auth = credentials.freeze
        end
        super(auth: auth, **options)
      end
    end
  end
  class Client
    prepend Internal::CredentialEnvConstructor
  end
  module Internal
    private_constant :CredentialEnvConstructor
  end
end
