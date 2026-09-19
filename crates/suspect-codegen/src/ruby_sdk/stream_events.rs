//! Generated-only typed event decode for the Ruby gem.
//!
//! The compiled `http_protocol::StreamSemanticsPlan` becomes per-operation
//! `<op>_events` methods on the generated `Client` plus per-operation frozen
//! `Data` event classes, the completion carrier and the events wrapper — all
//! emitted into the generated `client.rb` and its RBS signatures. Static
//! runtime files and the shared planner stay untouched. The events exchange
//! reuses the operation's existing request construction and the runtime's own
//! framing, ceilings, declared-error handling and cancellation, with the
//! parsed envelope values passed through unvalidated so undeclared event
//! kinds stay representable; the compiled per-kind decode, terminal sentinel
//! and completion policy are applied in the emitted loop. Recognized event
//! kinds decode through the operation's existing stream item codec into their
//! declared model type; undeclared event kinds surface through the typed
//! unknown alternative without failing the stream; invalid payloads of
//! recognized kinds remain branded decoding errors. The direct operation
//! calls and their untyped `ItemStream` iteration are unchanged, and
//! operations without a discriminated stream schema emit no new bytes at all.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::{models, ModelPlan, NativeType, PlannedOperation, SdkPlan};
use crate::http_protocol::{
    EventPayload, StreamEventPlan, StreamFraming, StreamOperationPlan, StreamSemanticsPlan,
};
use suspect_ir::contract::SchemaId;

/// One operation's emission-ready typed event decode.
#[derive(Debug, Clone)]
pub struct StreamEventsOperation {
    /// Index into `SdkPlan::operations`.
    pub operation: usize,
    /// The shared planner's operation identity, for documentation.
    pub identity: String,
    /// The operation's source, for documentation and typed failures.
    pub source: String,
    /// Allocated client method returning the events wrapper.
    pub events_name: String,
    /// Allocated private per-op decoder name for the strict per-kind decode.
    pub decoder_name: String,
    /// Allocated per-op events wrapper class.
    pub events_class: String,
    /// Allocated per-op completion carrier.
    pub completion_class: String,
    /// Declared event kinds with their allocated frozen event classes, in
    /// declaration order.
    pub kind_classes: Vec<(String, String)>,
    /// Allocated typed unknown event class.
    pub unknown_class: String,
    /// The operation's existing stream item codec symbol (`Codecs::X`).
    pub item_codec: String,
    /// The item schema's RBS type, for the decoded payload member.
    pub item_type: Option<String>,
    /// The declared terminal sentinel token, matched on frame data before any
    /// payload decoding.
    pub sentinel: Option<String>,
    pub keep_final_usage: bool,
    /// Whether the declared envelope metadata carries the last-event id.
    pub id_metadata: bool,
    /// Whether the declared envelope metadata carries the reconnection hint.
    pub retry_metadata: bool,
    /// The direct call's parameter keywords, in wire order.
    pub keywords: Vec<String>,
    /// Whether the operation declares a request body keyword.
    pub has_body: bool,
}

/// The compiled typed-event emission carried by one Ruby plan.
#[derive(Debug, Clone)]
pub struct StreamEventsPlan {
    /// The full compiled stream-semantics outcome this emission follows.
    pub outcome: StreamSemanticsPlan,
    /// Allocated name of the shared raw-envelope framer emitted beside the
    /// event classes.
    pub parser_class: String,
    /// Emission-ready typed event decode for exactly the operations whose
    /// compiled stream plan carries a discriminated SSE event set.
    pub operations: Vec<StreamEventsOperation>,
}

/// The runtime constants the emitted event classes share the gem namespace
/// with. Emitted class names are allocated against these so a generated event
/// class can never shadow the generated runtime.
const RUNTIME_CONSTANTS: &[&str] = &[
    "Client", "ApiResponse", "ApiError", "SdkError", "RequestError", "ResponseError",
    "TransportError", "TimeoutError", "CancelledError", "ResourceLimitError",
    "CancellationToken", "ExchangeContext", "PreparedRequest", "WireResponse",
    "NetHTTPTransport", "ItemStream", "Codec", "CodecError", "JsonError", "Json",
    "JsonNumber", "Exact", "Model", "WireModel", "Link", "Bytes", "NoContent",
    "CredentialContext", "BasicCredential", "AuthorizationCredential", "ValidationError",
    "EvaluationFailure", "Internal", "Models", "Types", "Codecs", "NO_CONTENT",
];

/// The per-call option keywords the direct methods declare; the events
/// methods accept exactly the same set.
const OPTION_KEYWORDS: &[&str] = &[
    "timeout",
    "cancellation",
    "max_response_bytes",
    "max_capture_bytes",
    "content_type",
    "accept",
    "security",
    "server",
    "server_variables",
    "document_url",
];

/// The shared planner's operation identity for one planned operation: the
/// operation id, or `METHOD /path` when unnamed (the pagination convention).
fn identity(operation: &PlannedOperation) -> String {
    operation
        .wire
        .operation_id()
        .map(|located| located.value().clone())
        .unwrap_or_else(|| {
            format!(
                "{} {}",
                operation.wire.method().as_str(),
                operation.wire.path()
            )
        })
}

/// Whether the compiled event set is discriminated: at least two declared
/// kinds, or one declared kind compiled from discrimination evidence whose
/// payload is a declared JSON structure. The single un-discriminated default
/// event keeps the existing untyped path byte-for-byte.
fn discriminated(stream: &StreamOperationPlan) -> bool {
    match stream.events.as_slice() {
        [] => false,
        [only] => {
            only.event_name != StreamEventPlan::DEFAULT_EVENT_NAME
                && matches!(only.payload, EventPayload::Json { .. })
        }
        _ => true,
    }
}

/// Whether one compiled stream plan admits the typed event decode: SSE framing
/// whose discriminator is the parsed envelope's `event` field, and a
/// discriminated event set. Everything else keeps its existing untyped path.
fn admits_typed_events(stream: &StreamOperationPlan) -> bool {
    stream.framing == StreamFraming::ServerSentEvents
        && stream
            .item_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.event_field.as_deref() == Some("event"))
        && discriminated(stream)
}

/// The operation's single success response owning exactly one media — the
/// declared stream media this compiled entry names. Anything else keeps the
/// conservative untyped path, because the typed exchange would otherwise own
/// responses it cannot name.
fn stream_media<'a>(
    operation: &'a PlannedOperation,
    compiled: &StreamOperationPlan,
) -> Option<&'a super::PlannedResponse> {
    let mut found: Option<&super::PlannedResponse> = None;
    for response in &operation.responses {
        if !response.can_succeed() || response.body_forbidden {
            continue;
        }
        if response.media.len() != 1 {
            continue;
        }
        let NativeType::Stream(_) = response.media[0].value_type else {
            continue;
        };
        if response.media[0].wire.media_type().declared() != compiled.media_type {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(response);
    }
    found
}

fn q(text: &str) -> String {
    serde_json::to_string(text).unwrap().replace('#', "\\#")
}

/// Compile the typed-event subset of the stream semantics plan against the
/// planned operations, model bindings and name allocation. `Some` whenever the
/// document declares any stream media, so the compiled outcome stays
/// inspectable even when no operation emits events.
pub(super) fn plan(
    models: &ModelPlan,
    records: &[super::NativeRecord],
    operations: &[PlannedOperation],
    semantics: &StreamSemanticsPlan,
    used: &mut BTreeSet<String>,
) -> Option<StreamEventsPlan> {
    if semantics.streams.is_empty() {
        return None;
    }
    used.extend(RUNTIME_CONSTANTS.iter().map(|name| (*name).to_owned()));
    for symbol in models.symbols() {
        used.insert(symbol.name.clone());
    }
    for operation in operations {
        used.insert(operation.error_class.clone());
        for response in &operation.responses {
            used.insert(response.class_name.clone());
            used.insert(response.error_class.clone());
            if let Some(headers) = &response.headers_class {
                used.insert(headers.clone());
            }
        }
    }
    for record in records {
        used.insert(record.name.clone());
    }
    let parser_class = models::allocate("TypedEventParser", used);
    let mut planned = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        let Some(compiled) = semantics
            .streams
            .iter()
            .find(|stream| stream.operation == identity(operation))
        else {
            continue;
        };
        if !admits_typed_events(compiled) || stream_media(operation, compiled).is_none() {
            continue;
        }
        let Some(item_codec) = &compiled.item_codec else {
            continue;
        };
        let item_schema: &SchemaId = item_codec.schema().id();
        let Some(symbol) = models.source_symbol(item_schema) else {
            continue;
        };
        let stem = crate::rust_models::pascal(&operation.operation_id);
        let kind_classes = compiled
            .events
            .iter()
            .map(|event| {
                let class = models::allocate(
                    &format!(
                        "{stem}{}Event",
                        crate::rust_models::pascal(&event.event_name)
                    ),
                    used,
                );
                (event.event_name.clone(), class)
            })
            .collect();
        planned.push(StreamEventsOperation {
            operation: index,
            identity: identity(operation),
            source: {
                let source = operation.source.clone();
                format!("{}#{}", source.document(), source.pointer())
            },
            events_name: models::allocate(&format!("{}_events", operation.method_name), used),
            decoder_name: models::allocate(&format!("{}_typed_event", operation.method_name), used),
            events_class: models::allocate(&format!("{stem}Events"), used),
            completion_class: models::allocate(&format!("{stem}Completion"), used),
            kind_classes,
            unknown_class: models::allocate(&format!("{stem}UnknownEvent"), used),
            item_codec: symbol.name.clone(),
            item_type: Some(format!("Types::{}", symbol.type_name)),
            sentinel: compiled
                .sentinel
                .enabled
                .then(|| compiled.sentinel.token.clone()),
            keep_final_usage: compiled.terminal.keep_final_usage,
            id_metadata: compiled
                .item_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.id_field.is_some()),
            retry_metadata: compiled
                .item_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.retry_field.is_some()),
            keywords: operation
                .parameters
                .iter()
                .map(|p| p.keyword.clone())
                .collect(),
            has_body: operation.body.is_some(),
        });
    }
    Some(StreamEventsPlan {
        outcome: semantics.clone(),
        parser_class,
        operations: planned,
    })
}

/// The event members every declared kind's frozen event class carries: the
/// kind name, the decoded payload, and the declared envelope metadata.
fn event_members(operation: &StreamEventsOperation) -> Vec<&'static str> {
    let mut members = vec![":kind", ":data"];
    if operation.id_metadata {
        members.push(":id");
    }
    if operation.retry_metadata {
        members.push(":retry");
    }
    members
}

/// The frozen per-operation event, unknown, completion and wrapper classes
/// plus the shared raw-envelope framer, emitted into the generated
/// `client.rb` beside the operation response classes.
pub(super) fn classes(events: &StreamEventsPlan) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "  # @api private\n  # The typed events framer: the generated runtime's own SSE/JSON-lines\n  # framing, byte ceilings and cancellation, with the parsed envelope values\n  # passed through unvalidated so undeclared event kinds stay representable.\n  # The compiled per-kind decode happens in the emitted events loop.\n  class {} < Internal::ItemParser\n    def emit(value)\n      @context.check!\n      @items += 1\n      raise ResourceLimitError.new('stream item count ceiling exceeded', kind: :resource_limit, source: @source) if @items > Internal::POLICY[:max_stream_items]\n      yield value\n    end\n  end\n",
        events.parser_class
    );
    for operation in &events.operations {
        for (kind, class) in &operation.kind_classes {
            let _ = write!(
                out,
                "  # A declared {} event of the {} stream: the framed SSE envelope\n  # decoded to its declared model type. An invalid payload for this kind\n  # remains a decoding error.\n  {} = Data.define({})\n",
                kind,
                operation.identity,
                class,
                event_members(operation).join(", "),
            );
        }
        let _ = write!(
            out,
            "  # An event kind the source never declared: the raw frame data stays\n  # representable without failing the stream.\n  {} = Data.define(:kind, :event, :data)\n",
            operation.unknown_class
        );
        let _ = write!(
            out,
            "  # Terminal metadata of one completed {} stream: why it completed and\n  # the preserved final usage frame. Read completion after the events end;\n  # event loops discard it.\n  {} = Data.define(:reason, :usage)\n",
            operation.identity, operation.completion_class
        );
        let _ = write!(
            out,
            "  # Typed events of the {} stream: the lazy events Enumerator plus the\n  # documented completion metadata, readable once the events end. Closing\n  # early leaves the completion unset.\n  class {}\n    attr_reader :events\n\n    def initialize(events)\n      @events = events\n      @completion = nil\n    end\n\n    # Terminal metadata of the completed stream; nil until the events end.\n    def completion\n      @completion\n    end\n\n    # @api private\n    def complete!(value)\n      @completion = value unless @completion\n      nil\n    end\n  end\n",
            operation.identity, operation.events_class
        );
    }
    out
}

/// The per-operation typed events methods on the generated `Client`, plus the
/// shared events exchange. Called only when the plan carries emittable typed
/// stream operations.
pub(super) fn client_methods(plan: &SdkPlan, events: &StreamEventsPlan) -> String {
    let mut out = String::new();
    for operation in &events.operations {
        out.push_str(&events_method(plan, operation));
    }
    out.push_str(CONSUME_METHOD);
    let mut names: Vec<String> = events
        .operations
        .iter()
        .map(|operation| format!(":{}", operation.decoder_name))
        .collect();
    names.push(":events_consume".to_owned());
    let _ = writeln!(out, "    private {}", names.join(", "));
    out
}

/// The RBS class declarations for one plan's emitted typed events: the event,
/// unknown, completion and wrapper classes.
pub(super) fn signatures(events: &StreamEventsPlan) -> String {
    let mut out = String::new();
    for operation in &events.operations {
        for (kind, class) in &operation.kind_classes {
            let _ = write!(
                out,
                "  # A declared {} event of the {} stream.\n  class {} < Data\n    attr_accessor kind: String\n    attr_accessor data: {}\n",
                kind,
                operation.identity,
                class,
                operation.item_type.as_deref().unwrap_or("json_value"),
            );
            if operation.id_metadata {
                out.push_str("    attr_accessor id: String?\n");
            }
            if operation.retry_metadata {
                out.push_str("    attr_accessor retry: JsonNumber?\n");
            }
            out.push_str("  end\n");
        }
        let _ = write!(
            out,
            "  # An event kind the source never declared.\n  class {} < Data\n    attr_accessor kind: String\n    attr_accessor event: String\n    attr_accessor data: String\n  end\n",
            operation.unknown_class
        );
        let union = operation
            .kind_classes
            .iter()
            .map(|(_, class)| class.as_str())
            .chain(std::iter::once(operation.unknown_class.as_str()))
            .collect::<Vec<_>>()
            .join(" | ");
        let _ = write!(
            out,
            "  # Terminal metadata of one completed {} stream.\n  class {} < Data\n    attr_accessor reason: :sentinel | :eof\n    attr_accessor usage: ({union})?\n  end\n\n  # Typed events of the {} stream.\n  class {}\n    attr_reader events: Enumerator[({union})]\n\n    def completion: () -> {}?\n  end\n",
            operation.identity,
            operation.completion_class,
            operation.identity,
            operation.events_class,
            operation.completion_class,
        );
    }
    out
}

/// The per-operation `Client` method signatures.
pub(super) fn client_signatures(events: &StreamEventsPlan) -> String {
    let mut out = String::new();
    for operation in &events.operations {
        let _ = writeln!(
            out,
            "    def {}: (**untyped) -> {}",
            operation.events_name, operation.events_class
        );
    }
    out
}

/// The parsed envelope's `data` member, exactly as the compiled sentinel
/// policy sees it: a non-string member reads as empty.
const ENVELOPE_DATA: &str =
    "            data = envelope.instance_of?(Hash) ? envelope['data'] : nil\n            data = data.instance_of?(String) ? data : ''\n";

/// The parsed envelope's `event` member, with the shared missing/empty
/// fallback to the `message` kind.
const ENVELOPE_KIND: &str =
    "            kind = envelope.instance_of?(Hash) ? envelope['event'] : nil\n            kind = 'message' unless kind.instance_of?(String) && !kind.empty?\n";

fn events_method(plan: &SdkPlan, operation: &StreamEventsOperation) -> String {
    let op = &plan.operations()[operation.operation];
    let mut out = String::new();
    let _ = write!(
        out,
        "    # Lazily yields the typed events of {} over the existing stream item\n    # iteration. A declared event kind decodes the framed envelope through the\n    # operation's stream item codec into its declared model type; an event kind\n    # the source never declared surfaces through the typed unknown alternative\n    # without failing the stream; an invalid payload for a recognized kind\n    # remains a decoding error. Per-item metadata (the event name as kind, plus\n    # id/retry when the item schema declares them) is explicit, and the\n    # {{ reason, usage }} completion metadata is read from {}.completion once\n    # the events end. Iteration is lazy: no request happens before the first\n    # next, breaking out releases the transfer and issues no further reads",
        operation.identity, operation.events_class
    );
    if let Some(token) = &operation.sentinel {
        let _ = write!(
            out,
            ", and the declared {} sentinel completes the stream before any\n    # payload decoding while preserving the final usage frame",
            token
        );
    }
    let _ = write!(
        out,
        ".\n    # The direct {}(...) call and its untyped ItemStream are unchanged.\n    # Source: {}.\n    def {}(**kwargs)\n",
        op.method_name, operation.source, operation.events_name
    );
    let mut keywords: Vec<String> = operation
        .keywords
        .iter()
        .map(|keyword| format!(":{keyword}"))
        .collect();
    if operation.has_body {
        keywords.push(":body".to_owned());
    }
    keywords.extend(OPTION_KEYWORDS.iter().map(|keyword| format!(":{keyword}")));
    let _ = writeln!(
        out,
        "      kwargs.each_key do |name|\n        raise ArgumentError, \"unknown keyword: #{{name}}\" unless [{}].include?(name)\n      end",
        keywords.join(", ")
    );
    let parameters = operation
        .keywords
        .iter()
        .map(|keyword| format!("kwargs.fetch(:{keyword}, UNSET)"))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(out, "      parameters = [{parameters}]");
    // The exchange always passes a body value; body-less operations pass the
    // unset marker exactly like the direct call's default argument.
    if operation.has_body {
        out.push_str("      body = kwargs.fetch(:body, UNSET)\n");
    } else {
        out.push_str("      body = UNSET\n");
    }
    for keyword in OPTION_KEYWORDS {
        let default = if *keyword == "cancellation" { "nil" } else { "UNSET" };
        let _ = writeln!(
            out,
            "      {keyword} = kwargs.fetch(:{keyword}, {default})"
        );
    }
    let _ = write!(
        out,
        "      limit = Internal.bounded_limit(max_response_bytes.equal?(UNSET) ? @max_response_bytes : max_response_bytes, @max_response_bytes, 'max_response_bytes')\n      capture = Internal.bounded_limit(max_capture_bytes.equal?(UNSET) ? [@max_capture_bytes, limit].min : max_capture_bytes, [@max_capture_bytes, limit].min, 'max_capture_bytes')\n      wrapper = nil\n      events = Enumerator.new do |yielder|\n        op = Internal::OPERATIONS.fetch({index})\n        raise RequestError.new('client is closed', source: op[:source], operation_id: op[:id]) if @closed\n        context = ExchangeContext.new(timeout: timeout.equal?(UNSET) ? @timeout : timeout, cancellation: cancellation, operation_id: op[:id], source: op[:source])\n        lease = nil\n        begin\n          chunks = nil; descriptor = nil\n          context.run do\n            request = prepare_request(op, parameters, body, content_type, accept, security, server, server_variables, document_url)\n            lease = Internal::ExchangeLease.new(@transport, request, context)\n            chunks, descriptor = events_consume(op, lease, context, limit, capture)\n          end\n          parser = {parser}.new(descriptor, context)\n          held = nil\n          stopped = false\n          parser.each(chunks) do |envelope|\n{envelope_data}{sentinel_block}{envelope_kind}            typed =\n              case kind\n",
        index = operation.operation,
        parser = plan
            .stream_events()
            .map(|events| events.parser_class.as_str())
            .unwrap_or("TypedEventParser"),
        envelope_data = ENVELOPE_DATA,
        envelope_kind = ENVELOPE_KIND,
        sentinel_block = sentinel_block(operation),
    );
    for (kind, class) in &operation.kind_classes {
        let _ = write!(
            out,
            "              when {}\n                item = {}(envelope)\n                {class}.new(kind: {}, data: item",
            q(kind),
            operation.decoder_name,
            q(kind),
        );
        if operation.id_metadata {
            out.push_str(", id: envelope['id']");
        }
        if operation.retry_metadata {
            out.push_str(", retry: envelope['retry']");
        }
        out.push_str(")\n");
    }
    let _ = write!(
        out,
        "              else\n                {}.new(kind: 'unknown', event: kind, data: data)\n              end\n",
        operation.unknown_class
    );
    if operation.keep_final_usage {
        out.push_str(
            "            if held.nil?\n              held = typed\n            else\n              previous = held\n              held = typed\n              yielder << previous\n            end\n",
        );
    } else {
        out.push_str("            yielder << typed\n");
    }
    let eof_usage = if operation.keep_final_usage {
        "usage: held"
    } else {
        "usage: nil"
    };
    let _ = write!(
        out,
        "          end\n          unless stopped\n            wrapper.complete!({completion}.new(reason: :eof, {eof_usage}))\n          end\n        ensure\n          lease&.close\n        end\n      end\n      wrapper = {events_class}.new(events)\n      wrapper\n    end\n\n    # @api private\n    # The strict per-kind decode through the operation's existing stream item\n    # codec; an invalid payload of a recognized kind remains a branded\n    # decoding error.\n    def {decoder}(envelope)\n      Codecs::{codec}.decode(envelope)\n    rescue CodecError, JsonError, ValidationError, EvaluationFailure => error\n      raise ResponseError.new('typed event payload does not satisfy its source codec', kind: :response_decoding, source: error.respond_to?(:source) && error.source || {source}, operation_id: {identity}), cause: error\n    end\n",
        completion = operation.completion_class,
        eof_usage = eof_usage,
        events_class = operation.events_class,
        decoder = operation.decoder_name,
        codec = operation.item_codec,
        source = q(&operation.source),
        identity = q(&operation.identity),
    );
    out
}

/// The declared sentinel completes the stream before any payload decoding.
fn sentinel_block(operation: &StreamEventsOperation) -> String {
    match &operation.sentinel {
        Some(token) => format!(
            "            if data == {}\n              wrapper.complete!({}.new(reason: :sentinel, usage: held))\n              stopped = true\n              break\n            end\n",
            q(token),
            operation.completion_class,
        ),
        None => String::new(),
    }
}

/// The shared typed events exchange: the direct call's response handling with
/// the declared event stream handed back raw instead of being decoded into an
/// `ItemStream`. Every other outcome mirrors `consume_response` exactly.
#[allow(clippy::too_many_lines)]
const CONSUME_METHOD: &str = r#"
    # @api private
    # The streaming exchange of a typed events call: the direct call's response
    # handling with the declared event stream handed back raw instead of being
    # decoded into an ItemStream. Everything else mirrors consume_response.
    def events_consume(op, response, context, limit, capture_limit)
      status = response.status
      raise TransportError.new('invalid transport status', source: op[:source], operation_id: op[:id]) unless status.instance_of?(Integer) && status.between?(100, 599)
      headers = Internal.response_headers(response.headers)
      selected = Internal::Wire.status_match(op[:wire]['responses'], status)
      native = selected && op[:responses].fetch(selected['status_key'])
      metadata = {source: selected ? Internal::Wire.source(selected['source']) : op[:source], operation_id: op[:id], status: status, headers: headers}
      forbidden = op[:wire]['method'] == 'HEAD' || status.between?(100, 199) || [204, 205, 304].include?(status)
      payload = Internal::PayloadSession.new
      typed_headers = selected && payload.decode_headers(selected['headers'], headers, 'headers:' + Internal::Wire.binding_source(selected['source']))
      links = selected ? selected['links'].to_h { |l| [l['name'], Link.new(l)] }.freeze : {}.freeze
      media, actual, media_error = nil, nil, nil
      if selected && !forbidden && !selected['media'].empty?
        begin
          raise CodecError, 'missing/repeated Content-Type' unless headers['content-type']&.length == 1
          media, actual = Internal::Wire.match_media(selected['media'], headers['content-type'].first)
          encoding = headers['content-encoding']
          raise CodecError, 'encoded structured response is unsupported' if media['representation']['kind'] != 'binary' && encoding && encoding != ['identity']
        rescue CodecError => error
          media_error = error
        end
      end
      limit = [limit, Internal::Wire.integer(selected['max_body_bytes'])].min if selected
      length, transfer = headers['content-length'], headers['transfer-encoding']
      if !forbidden && (length && (length.length != 1 || !/\A[0-9]+\z/n.match?(length[0].b) || transfer) || transfer && (transfer.length != 1 || transfer[0].downcase != 'chunked'))
        raise TransportError.new('ambiguous response framing', **metadata)
      end
      if !forbidden && length && (length[0].length > 16 || length[0].to_i > limit)
        raise ResourceLimitError.new('declared response byte ceiling exceeded', truncated: true, **metadata)
      end
      chunks = Internal::LimitedChunks.new(response, context, limit, capture_limit, metadata)
      if media && media['representation']['kind'] == 'stream' && !forbidden
        descriptor = media['representation']['stream']
        if status.between?(200, 299)
          return [chunks, descriptor]
        end
        # Error streams are bounded and validated before becoming API errors.
        values = []
        Internal::ItemParser.new(descriptor, context).each(chunks) { |value| values << value }
        raise native[:error].new(data: ItemStream.completed(values, metadata[:source]), content_type: headers['content-type'].first, typed_headers: typed_headers, links: links, **chunks.details)
      end
      bytes = +''.b
      unless forbidden
        chunks.each_chunk do |chunk|
          break if (!selected || media_error) && chunks.truncated
          bytes << chunk.b
        end
      end
      raise ResponseError.new('undeclared HTTP status', kind: :unexpected_status, **chunks.details) unless selected
      raise ResponseError.new('unexpected response media', kind: :unexpected_media, **chunks.details), cause: media_error if media_error
      if !forbidden && length && bytes.bytesize != length[0].to_i
        raise TransportError.new('response content length was not satisfied', **chunks.details)
      end
      begin
        data = forbidden ? NO_CONTENT : media ? payload.decode(media, bytes, actual) : Bytes.new(bytes)
      rescue JsonError, CodecError => error
        raise ResponseError.new('response does not satisfy its source codec', **chunks.details), cause: error
      end
      if status.between?(200, 299)
        # A success response that is not the declared event stream carries no
        # typed events.
        raise ResponseError.new('the selected response media is not the declared event stream', kind: :unexpected_media, **chunks.details)
      end
      raise native[:error].new(data: data, content_type: headers['content-type']&.first, typed_headers: typed_headers, links: links, **chunks.details)
    rescue CodecError => error
      raise ResponseError.new('response header does not satisfy its source codec', **(metadata || {source: op[:source], operation_id: op[:id]})), cause: error
    end
"#;
