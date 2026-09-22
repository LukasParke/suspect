# frozen_string_literal: true
module __NAMESPACE__
  # Backpressured, source-validated native Enumerator. Use #each (which ensures
  # cleanup) or close explicitly after #next. The total deadline remains active
  # even while the consumer is paused. No sentinel or reconnect loop exists.
  class ItemStream < Enumerator
    attr_reader :source
    def initialize(lease, descriptor, context, values: nil)
      @lease, @descriptor, @context = lease, descriptor, context
      @source = Internal::Wire.source(descriptor.fetch('source'))
      @closed = false
      super() do |out|
        begin
          if values
            values.each { |value| out << value }
          else
            parser = Internal::ItemParser.new(descriptor, context)
            parser.each(lease) { |value| out << value }
          end
        ensure
          close
        end
      end
    end
    def close
      return nil if @closed
      @closed = true
      @lease&.close
      nil
    end
    def closed? = @closed
    def next
      @context&.check!
      raise StopIteration, 'stream is closed' if @closed
      super
    end
    def inspect = "#<#{self.class.name} closed=#{@closed}>"
    # @api private
    def self.completed(values, source)
      new(nil, {'source' => source}, nil, values: values)
    end
  end

  # @api private
  module Internal
    class LeaseClosed < Exception; end
    # Block-based transport lifetime is retained on one worker. Only one bounded
    # chunk is handed to a waiting consumer; reads are demand-driven.
    class ExchangeLease
      attr_reader :status, :headers
      def initialize(transport, request, context)
        @context, @commands, @chunks, @ready = context, Queue.new, SizedQueue.new(1), Queue.new
        @closed, @error, @done = false, nil, false
        @worker = Thread.new do
          Thread.current.report_on_exception = false
          yielded, ready = false, false
          begin
            context.run do
              transport.exchange(request: request, context: context) do |response|
                begin
                  raise TransportError, 'transport yielded multiple responses' if yielded
                  yielded = true
                  @ready << [response.status, response.headers]
                  ready = true
                  @commands.pop
                  response.each_chunk do |chunk|
                    context.check!
                    raise TransportError, 'body chunks must be Strings' unless chunk.instance_of?(String)
                    if chunk.empty?
                      @chunks << ''.b
                      @commands.pop
                    else
                      offset = 0
                      while offset < chunk.bytesize
                        @chunks << chunk.byteslice(offset, 16 * 1024).b.freeze
                        offset += 16 * 1024
                        @commands.pop
                      end
                    end
                  end
                ensure
                  primary = $!
                  begin response.close
                  rescue StandardError
                    raise unless primary
                  end
                end
              end
              raise TransportError, 'transport did not yield a response' unless yielded
            end
          rescue LeaseClosed
            nil
          rescue Exception => error
            @error = error
          ensure
            @done = true
            @ready << nil unless ready
            @chunks.close
          end
        end
        first = @ready.pop
        raise_error if @error
        raise TransportError, 'transport did not produce metadata' unless first
        @status, @headers = first
      rescue Exception
        close
        raise
      end
      def each_chunk
        loop do
          @context.check!
          raise_error if @error
          break if @closed
          @commands << true
          chunk = @chunks.pop
          raise_error if @error
          break unless chunk
          yield chunk
        end
        nil
      end
      def close
        return nil if @closed
        @closed = true
        @worker.raise(LeaseClosed) if @worker&.alive? && !@done
        @worker&.join
        nil
      end
      def closed? = @closed
      def raise_error
        error = @error
        raise error if error.is_a?(SdkError) || !error.is_a?(StandardError)
        if error.is_a?(::Net::OpenTimeout) || error.is_a?(::Net::ReadTimeout) || error.is_a?(::Net::WriteTimeout)
          raise TimeoutError.new('stream transport timed out', source: @context.source, operation_id: @context.operation_id), cause: error
        end
        raise TransportError.new('stream transport failed', source: @context.source, operation_id: @context.operation_id), cause: error
      end
    end

    class LimitedChunks
      attr_reader :capture, :truncated, :bytes
      def initialize(response, context, limit, capture_limit, metadata)
        @response, @context, @limit, @capture_limit, @metadata = response, context, limit, capture_limit, metadata
        @capture, @bytes, @count, @truncated = +''.b, 0, 0, false
      end
      def each_chunk
        @response.each_chunk do |chunk|
          @context.check!
          raise TransportError.new('body chunks must be Strings', kind: :transport, **details) unless chunk.instance_of?(String)
          @count += 1; @bytes += chunk.bytesize
          left = @capture_limit - @capture.bytesize
          @capture << chunk.byteslice(0, left).b if left.positive?
          @truncated ||= chunk.bytesize > left
          if @count > POLICY[:max_response_chunks] || @bytes > @limit
            @truncated = true
            raise ResourceLimitError.new('response byte/chunk ceiling exceeded', kind: :resource_limit, **details)
          end
          yield chunk
        end
        nil
      end
      def details = @metadata.merge(capture: @capture.dup.freeze, truncated: @truncated)
      def close = @response.close
      def closed? = @response.closed?
    end

    class ItemParser
      def initialize(descriptor, context)
        @descriptor, @context = descriptor, context
        @max = Wire.integer(descriptor['max_item_bytes'])
        @codec, @items = CodecSession.new, 0
        @source = Wire.source(descriptor['source'])
      end
      def emit(value)
        @context.check!
        @items += 1
        raise ResourceLimitError.new('stream item count ceiling exceeded', kind: :resource_limit, source: @source) if @items > POLICY[:max_stream_items]
        index = Wire.codec_index(@descriptor['item_codec'])
        yield @codec.decode(index, value)
      rescue CodecError, JsonError => error
        raise ResponseError.new('stream item does not satisfy its source codec', kind: :response_decoding, source: error.source || @source, operation_id: @context.operation_id), cause: error
      end
      def each(chunks)
        buffer = +''.b; first = true; skip_lf = false
        sse = @descriptor['framing'] == 'server-sent-events'
        data, event, id, retry_value, event_bytes = [], nil, nil, nil, 0
        process = lambda do |raw|
          text = raw.dup.force_encoding(Encoding::UTF_8)
          if sse
            # WHATWG UTF-8 decoding replaces invalid sequences for event streams.
            text = text.scrub
            text = text.delete_prefix("\uFEFF") if first
            first = false
            event_bytes += raw.bytesize + 1
            raise ResourceLimitError, 'SSE event byte ceiling exceeded' if event_bytes > @max
            if text.empty?
              unless data.empty?
                value = {'data' => data.join("\n")}
                value['event'] = event unless event.nil? || event.empty?
                value['id'] = id unless id.nil?
                value['retry'] = retry_value unless retry_value.nil?
                emit(value) { |v| yield v }
              end
              data, event, event_bytes = [], nil, 0
            elsif !text.start_with?(':')
              key, value = text.split(':', 2); value ||= ''; value = value.delete_prefix(' ')
              case key
              when 'data' then data << value
              when 'event' then event = value
              when 'id' then id = value unless value.include?("\0")
              when 'retry'
                if /\A[0-9]+\z/n.match?(value.b)
                  # HTML retry is decimal digits, not Ruby's padded/octal syntax.
                  digits = value.sub(/\A0+/, ''); digits = '0' if digits.empty?
                  retry_value = JsonNumber.new(digits)
                end
              end
            end
          else
            text = text.delete_suffix("\r")
            raise ResponseError.new('blank JSON line', kind: :response_decoding, source: @source, operation_id: @context.operation_id) if text.empty?
            emit(Json.parse(text, max_bytes: @max)) { |v| yield v }
          end
        end
        chunks.each_chunk do |chunk|
          chunk.each_byte do |byte|
            if sse && skip_lf
              skip_lf = false
              next if byte == 10
            end
            if byte == 10 || sse && byte == 13
              process.call(buffer)
              buffer = +''.b
              skip_lf = sse && byte == 13
            else
              buffer << byte
              raise ResourceLimitError.new('stream line/item byte ceiling exceeded', kind: :resource_limit, source: @source) if buffer.bytesize > @max
            end
          end
        end
        # SSE EOF discards an unfinished event; JSON lines permit a final record
        # without a trailing LF. No [DONE] or record-separator shortcut exists.
        process.call(buffer) unless sse || buffer.empty?
      rescue JsonError, CodecError => error
        raise ResponseError.new('invalid stream framing/item', kind: :response_decoding, source: @source, operation_id: @context.operation_id), cause: error
      end
    end
  end
end
