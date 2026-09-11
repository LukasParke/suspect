# frozen_string_literal: true
module __NAMESPACE__
  # @api private
  module Internal
    class ValidationSession
      def enter_resource(index, source, path)
        resource = @resource_context.fetch('nodeScopes').fetch(index).first
        return nil if @entered_resources.key?(resource)
        spend(source, path)
        previous = @resource_identity
        # Exact prefix/resource interning, not a digest-only cycle key. Ruby's
        # Hash checks the complete integer pair even if key hashes collide.
        key = [previous, resource]
        @contexts[key] ||= @contexts.length + 1
        @resource_identity = @contexts.fetch(key)
        @entered_resources[resource] = true
        @resource_stack << resource
        previous
      end
      def leave_resource(previous)
        return if previous.nil?
        resource = @resource_stack.pop
        @entered_resources.delete(resource)
        @resource_identity = previous
      end
      def dynamic_target(check, path)
        target = check.fetch('target')
        name = check.fetch('anchor')
        return target if name.nil?
        source = check.fetch('source')
        # The initial target is never entered merely to perform this search.
        @resource_stack.each do |resource|
          spend(source, path)
          @resource_context.fetch('resources').fetch(resource).fetch('dynamicAnchors').each do |candidate, _, index|
            spend(source, path)
            return index if candidate == name
          end
        end
        target
      end
    end
  end
end
