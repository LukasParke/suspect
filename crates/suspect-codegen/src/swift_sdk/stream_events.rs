//! Generated-only typed event decode for the Swift package.
//!
//! The compiled `http_protocol::StreamSemanticsPlan` becomes one per-operation
//! pull-based typed event sequence plus a per-operation events exchange member
//! on the generated `Client` — all emitted into the generated
//! `StreamEvents.swift`. Static runtime files and the shared planner stay
//! untouched. The events sequence reuses the operation's existing request
//! construction and the runtime's own `HTTPFramer` framing, item and transfer
//! ceilings and cancellation, and applies the compiled per-kind decode,
//! terminal sentinel and completion policy itself: recognized event kinds
//! decode through the operation's existing stream item codec into their
//! declared model type, undeclared event kinds surface through the typed
//! unknown alternative without failing the stream, and invalid payloads of
//! recognized kinds remain branded decoding errors. The direct operation
//! calls and their untyped `HTTPEventStream` iteration are unchanged, and
//! operations without a discriminated stream schema emit no new bytes at all.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::{
    PlannedOperation, SdkPlan,
    models::{Declaration, ModelPlan},
};
use crate::http_protocol::{
    EventPayload, Representation, StreamEventPlan, StreamFraming, StreamOperationPlan,
    StreamSemanticsPlan,
};

/// One operation's emission-ready typed event decode.
#[derive(Debug, Clone)]
pub struct StreamEventsOperation {
    /// Index into `SdkPlan::operations`.
    pub operation: usize,
    /// Source operation identity for documentation.
    pub operation_id: String,
    /// Method and path label for documentation.
    pub label: String,
    /// Rendered source location for documentation.
    pub source: String,
    /// Allocated public client method returning the event sequence.
    pub events_method: String,
    /// Allocated internal client member performing the events exchange.
    pub request_method: String,
    /// Allocated public typed event enum.
    pub event_type: String,
    /// Allocated public completion struct.
    pub completion_type: String,
    /// Allocated public event sequence struct.
    pub sequence_type: String,
    /// The operation input struct.
    pub input_type: String,
    /// Whether the input may be omitted at the call site.
    pub default_input: bool,
    /// The stream item model type every declared kind decodes into.
    pub payload: String,
    /// The operation's existing stream item codec symbol.
    pub item_codec: String,
    /// Declared event kinds with their allocated Swift case labels, in
    /// declaration order.
    pub kinds: Vec<(String, String)>,
    /// The declared terminal sentinel token, matched on frame data before any
    /// payload decoding.
    pub sentinel: Option<String>,
    /// Whether the last data frame before the sentinel or end of body is
    /// preserved as the completion's terminal usage instead of being yielded.
    pub keep_final_usage: bool,
    /// The decoded model member carrying the last-event id, when declared.
    pub id_member: Option<MetadataMember>,
    /// The decoded model member carrying the reconnection hint, when declared.
    pub retry_member: Option<MetadataMember>,
    /// The framed SSE item byte ceiling.
    pub item_limit: u64,
}

/// One declared envelope metadata member read from the decoded item model.
#[derive(Debug, Clone)]
pub struct MetadataMember {
    /// The decoded model's Swift member name.
    pub name: String,
    /// The unwrapped Swift type carried by the event case (`String?` for an
    /// optional field, the plain core type for a required one).
    pub ty: String,
    /// Whether the read unwraps a `valueIfPresent` first.
    pub unwrapped: bool,
}

/// The compiled typed-event emission carried by one Swift plan.
#[derive(Debug, Clone)]
pub struct StreamEventsPlan {
    /// The full compiled stream-semantics outcome this emission follows.
    pub outcome: StreamSemanticsPlan,
    /// Allocated name of the shared terminal box emitted beside the sequences.
    pub terminal_type: String,
    /// Emission-ready typed event decode for exactly the operations whose
    /// compiled stream plan carries a discriminated SSE event set.
    pub operations: Vec<StreamEventsOperation>,
}

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
        if !response.may_succeed() {
            continue;
        }
        if response.always_empty || response.may_be_empty || response.media.len() != 1 {
            continue;
        }
        if !matches!(
            response.media[0].wire.representation(),
            Representation::Stream { .. }
        ) {
            continue;
        }
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

/// One declared envelope metadata member resolved over the item model, or
/// `None` when the decoded payload type cannot name it.
fn metadata_member(
    models: &ModelPlan,
    item_schema: &suspect_ir::contract::SchemaId,
    field: &str,
) -> Option<MetadataMember> {
    let Declaration::Object { fields, .. } = models.declarations.get(item_schema)? else {
        return None;
    };
    let found = fields.iter().find(|candidate| candidate.wire == field)?;
    // Required non-null fields read directly; every other wrapper reads its
    // present value as an optional.
    let unwrapped = !found.required || found.model_type.nullable;
    Some(MetadataMember {
        name: found.name.clone(),
        ty: if unwrapped {
            format!("{}?", found.model_type.core.render(models))
        } else {
            found.model_type.core.render(models)
        },
        unwrapped,
    })
}

/// Compile the typed-event subset of the stream semantics plan against the
/// planned operations, model bindings and symbol allocation. Allocated type
/// and member names join the plan's reserved sets so nothing else can take
/// them. `Some` whenever the document declares any stream media, so the
/// compiled outcome stays inspectable even when no operation emits events.
pub(super) fn plan(
    operations: &[PlannedOperation],
    models: &ModelPlan,
    semantics: &StreamSemanticsPlan,
    type_names: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> Option<StreamEventsPlan> {
    if semantics.streams.is_empty() {
        return None;
    }
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
        let item_schema = item_codec.schema().id();
        let Some(codec_name) = models.codecs.get(item_schema) else {
            continue;
        };
        let stem = super::exported(&operation.operation_id);
        let mut cases = BTreeSet::from(["unknown".to_owned()]);
        let mut kinds = Vec::new();
        for event in &compiled.events {
            let label = super::allocate(&super::member(&event.event_name), &mut cases);
            kinds.push((event.event_name.clone(), label));
        }
        let id_member = compiled
            .item_metadata
            .as_ref()
            .and_then(|metadata| metadata.id_field.clone())
            .and_then(|field| metadata_member(models, item_schema, &field));
        let retry_member = compiled
            .item_metadata
            .as_ref()
            .and_then(|metadata| metadata.retry_field.clone())
            .and_then(|field| metadata_member(models, item_schema, &field));
        planned.push(StreamEventsOperation {
            operation: index,
            operation_id: identity(operation),
            label: format!("{} {}", operation.method, operation.path),
            source: format!(
                "{}#{}",
                operation.source.document(),
                operation.source.pointer()
            ),
            events_method: super::allocate(&format!("{}Events", operation.method_name), methods),
            request_method: super::allocate(
                &format!("{}EventsRequest", operation.method_name),
                methods,
            ),
            event_type: super::allocate(&format!("{stem}Event"), type_names),
            completion_type: super::allocate(&format!("{stem}Completion"), type_names),
            sequence_type: super::allocate(&format!("{stem}EventSequence"), type_names),
            input_type: operation.input_type.clone(),
            default_input: operation.default_input(),
            payload: models.ty(item_schema),
            item_codec: codec_name.clone(),
            kinds,
            sentinel: compiled
                .sentinel
                .enabled
                .then(|| compiled.sentinel.token.clone()),
            keep_final_usage: compiled.terminal.keep_final_usage,
            id_member,
            retry_member,
            item_limit: compiled.max_item_bytes,
        });
    }
    Some(StreamEventsPlan {
        outcome: semantics.clone(),
        terminal_type: super::allocate("StreamEventTerminal", type_names),
        operations: planned,
    })
}

fn q(value: &str) -> String {
    super::protocol_metadata::q(value)
}

fn doc(out: &mut String, text: &str, indent: &str) {
    for line in text.lines() {
        let _ = writeln!(out, "{indent}/// {}", super::emit::prose(line));
    }
}

/// Generator-controlled documentation is emitted verbatim; only embedded
/// source-derived fragments pass through `doc`.
fn doc_raw(out: &mut String, text: &str, indent: &str) {
    for line in text.lines() {
        let _ = writeln!(out, "{indent}/// {line}");
    }
}

/// The declared event case with its typed payload and declared metadata
/// members, for one kind's allocated label.
fn event_case(
    operation: &StreamEventsOperation,
    kind: &str,
    label: &str,
    indent: &str,
    out: &mut String,
) {
    let mut members = format!("data: {}", operation.payload);
    if let Some(member) = &operation.id_member {
        let _ = write!(members, ", {}: {}", member.name, member.ty);
    }
    if let Some(member) = &operation.retry_member {
        let _ = write!(members, ", {}: {}", member.name, member.ty);
    }
    doc(
        out,
        &format!(
            "A declared {kind:?} event of the {} stream: the framed SSE envelope decoded through the operation's stream item codec into its declared model type. An invalid payload for this kind remains a decoding error.",
            operation.operation_id
        ),
        indent,
    );
    let _ = writeln!(out, "{indent}case {label}({members})");
}

/// The metadata read expressions over one decoded envelope item.
fn metadata_reads(operation: &StreamEventsOperation) -> String {
    let mut reads = String::new();
    for member in [&operation.id_member, &operation.retry_member]
        .into_iter()
        .flatten()
    {
        let unwrap = if member.unwrapped {
            ".valueIfPresent"
        } else {
            ""
        };
        let _ = write!(reads, ", {}: item.{}{}", member.name, member.name, unwrap);
    }
    reads
}

/// The `data` member read of one parsed envelope, exactly as the compiled
/// sentinel policy sees it: a non-string member reads as empty.
const DATA_READ: &str = "        let data: String\n        if case .object(let envelope) = value, let raw = envelope[\"data\"], case .string(let text) = raw {\n            data = text\n        } else {\n            data = \"\"\n        }\n";

/// The terminal box shared between one typed event sequence and its in-flight
/// iterator. The first terminal observation wins; the sequence reads it after
/// the iteration ends.
fn terminal_type(name: &str) -> String {
    format!(
        "/// Terminal metadata shared between a typed event sequence and its\n/// in-flight iterator. The first terminal observation wins.\nfinal class {name}<Value: Sendable>: @unchecked Sendable {{\n    private let lock = NSLock()\n    private var stored: Value?\n\n    func complete(_ value: Value) {{\n        lock.lock()\n        if stored == nil {{ stored = value }}\n        lock.unlock()\n    }}\n\n    var value: Value? {{\n        lock.lock()\n        defer {{ lock.unlock() }}\n        return stored\n    }}\n}}\n"
    )
}

/// Render the generated `StreamEvents.swift` module: the shared terminal box
/// plus the typed event, completion and sequence declarations and the per-op
/// client members. `None` keeps the package free of any typed stream byte.
pub(super) fn emit(plan: &SdkPlan) -> Option<String> {
    let events = plan.stream_events.as_ref()?;
    if events.operations.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str("import Foundation\n\n");
    out.push_str(
        "/// Generated typed event decode for this package's declared SSE event\n/// streams. Every sequence reuses its operation's request construction and the\n/// generated runtime's own framing, byte ceilings and cancellation, and adds\n/// the compiled per-kind decode, the terminal sentinel and the completion\n/// policy: recognized event kinds decode through the operation's existing\n/// stream item codec, undeclared kinds surface through the typed unknown\n/// alternative without failing the stream, and invalid payloads of recognized\n/// kinds remain branded decoding errors. Iteration is pull-driven: the request\n/// starts on the first pull, and early break or cancellation releases the\n/// transfer and issues no further reads. The direct operation calls and their\n/// untyped `HTTPEventStream` iteration are unchanged.\n\n",
    );
    out.push_str(&terminal_type(&events.terminal_type));
    for operation in &events.operations {
        operation_code(plan, operation, &mut out);
    }
    Some(out)
}

fn operation_code(plan: &SdkPlan, operation: &StreamEventsOperation, out: &mut String) {
    let op = &plan.operations[operation.operation];
    let _ = writeln!(
        out,
        "\n// ---- {} — {} ----\n",
        super::emit::prose(op.method.as_str()),
        super::emit::prose(op.path.as_str())
    );
    let mut cases = String::new();
    for (kind, label) in &operation.kinds {
        event_case(operation, kind, label, "    ", &mut cases);
    }
    let _ = writeln!(
        out,
        "{}public enum {}: Sendable, Equatable {{\n{cases}    /// An event kind the source never declared: the raw frame data stays\n    /// representable without failing the stream.\n    case unknown(event: String, data: String)\n}}\n",
        doc_comment(operation),
        operation.event_type,
    );
    doc_raw(
        out,
        &format!(
            "Terminal metadata of one completed {} stream: why it completed and\nthe preserved final usage frame. Read ``{}/{}`` after the\nsequence ends; a `sentinel` completion preserved the last data frame\nbefore the declared terminal token instead of yielding it.",
            operation.operation_id, operation.sequence_type, "completion"
        ),
        "",
    );
    let _ = writeln!(
        out,
        "public struct {}: Sendable {{\n    /// `sentinel` when the declared terminal token completed the stream\n    /// before any payload decoding, `eof` when the response body ended.\n    public enum Reason: Sendable, Equatable {{\n        case sentinel\n        case eof\n    }}\n\n    public let reason: Reason\n    /// The last data frame before the sentinel or end of body, decoded like an\n    /// ordinary event and preserved as terminal instead of being yielded; nil\n    /// when no frame preceded completion or the compiled policy preserves\n    /// nothing.\n    public let usage: {}?\n\n    init(reason: Reason, usage: {}?) {{\n        self.reason = reason\n        self.usage = usage\n    }}\n}}\n",
        operation.completion_type, operation.event_type, operation.event_type,
    );
    sequence_code(plan, operation, out);
    client_code(plan, op, operation, out);
}

fn doc_comment(operation: &StreamEventsOperation) -> String {
    let mut docs = String::new();
    doc_raw(
        &mut docs,
        &format!(
            "One typed event of the {} stream. Declared kinds decode the framed\nSSE envelope through the operation's stream item codec into their declared\nmodel type; an event kind the source never declared surfaces through the\ntyped unknown alternative without failing the stream. Invalid payloads of\nrecognized kinds remain decoding errors.",
            operation.operation_id
        ),
        "",
    );
    docs.push_str("///\n");
    doc(&mut docs, &format!("Source: {}.", operation.source), "");
    docs
}

/// The pull-based event sequence: the framing loop over the runtime's own
/// framer, the compiled sentinel and completion policy, and the per-kind
/// decode through the operation's existing stream item codec.
fn sequence_code(plan: &SdkPlan, operation: &StreamEventsOperation, out: &mut String) {
    let terminal = plan
        .stream_events
        .as_ref()
        .map(|events| events.terminal_type.as_str())
        .unwrap_or("StreamEventTerminal");
    let sentinel_token = operation.sentinel.clone().unwrap_or_default();
    let sentinel_literal = if operation.sentinel.is_some() {
        format!(
            "        sentinel = true\n        sentinelToken = {}\n",
            q(&sentinel_token)
        )
    } else {
        "        sentinel = false\n        sentinelToken = \"\"\n".to_owned()
    };
    let kind_switch = operation
        .kinds
        .iter()
        .map(|(kind, label)| {
            format!(
                "        case {}:\n            let item = try decodeEnvelope(value)\n            typed = .{}(data: item{})",
                q(kind),
                label,
                metadata_reads(operation)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    // The compiled keep-final-usage policy is a sequence constant: when it
    // holds, every frame but the final one is yielded one pull late.
    let hold = "        if keepFinalUsage {\n            if let previous = held {\n                held = typed\n                return previous\n            }\n            held = typed\n            return nil\n        }\n        return typed\n";
    let sentinel_doc = match &operation.sentinel {
        Some(token) => format!(
            "the declared {} sentinel completes the stream before any payload\n/// decoding while preserving the final usage frame as ``{}/{}``.",
            token, operation.sequence_type, "completion"
        ),
        None => "the response body's end completes the stream.".to_owned(),
    };
    doc_raw(
        out,
        &format!(
            "Pull-based typed events of {}. Iteration reuses the operation's request\nconstruction and the runtime's own framing, item and transfer ceilings and\ncancellation: the request starts on the first pull, early break or\ncancellation releases the transfer and issues no further reads, and {}\nRecognized event kinds decode through the operation's stream item codec;\nundeclared kinds surface through the typed unknown alternative; invalid\npayloads of recognized kinds remain decoding errors. The direct call and\nits untyped ``HTTPEventStream`` iteration are unchanged.",
            operation.label, sentinel_doc
        ),
        "",
    );
    let _ = writeln!(
        out,
        "public struct {sequence}: AsyncSequence, AsyncIteratorProtocol, Sendable {{\n    public typealias Element = {event}\n    let client: Client\n    var pending: {input}\n    var options: RequestOptions\n    let itemLimit: Int\n    let keepFinalUsage: Bool\n    let sentinel: Bool\n    let sentinelToken: String\n    let terminal: {terminal_type}<{completion}>\n    var stream: HTTPStreamResponse?\n    var totalLimit = 0\n    var iterator: HTTPByteStream.AsyncIterator?\n    var lease: HTTPStreamLease?\n    var framer: HTTPFramer\n    var chunk = Data()\n    var offset = 0\n    var total = 0\n    var held: {event}?\n    var finished = false\n\n    init(client: Client, input: {input}, options: RequestOptions) {{\n        self.client = client\n        pending = input\n        self.options = options\n        itemLimit = {item_limit}\n        keepFinalUsage = {keep}\n{sentinel_literal}        terminal = {terminal_type}()\n        framer = HTTPFramer(framing: .serverSentEvents, limit: {item_limit})\n    }}\n\n    public func makeAsyncIterator() -> Self {{ self }}\n\n    /// Terminal metadata of the completed stream; nil until the sequence ends.\n    public var completion: {completion}? {{ terminal.value }}\n\n    /// The next typed event, or nil only after the sequence has ended. The\n    /// request starts when `next` is first awaited; dropping the sequence\n    /// before that never issues one.\n    public mutating func next() async throws -> {event}? {{\n        if finished {{ return nil }}\n        if stream == nil {{\n            try Task.checkCancellation()\n            let (response, responseLimit) = try await client.{request}(pending, options: options)\n            stream = response\n            iterator = response.body.makeAsyncIterator()\n            lease = HTTPStreamLease {{ response.close() }}\n            totalLimit = responseLimit\n        }}\n        do {{\n            while true {{\n                try Task.checkCancellation()\n                while offset < chunk.endIndex {{\n                    let byte = chunk[offset]\n                    offset += 1\n                    if let value = try framer.push(byte) {{\n                        if let event = try consume(value) {{ return event }}\n                        if finished {{ return nil }}\n                    }}\n                }}\n                guard let iterator else {{ return nil }}\n                guard let next = try await iterator.next() else {{\n                    terminal.complete({completion}(reason: .eof, usage: keepFinalUsage ? held : nil))\n                    finished = true\n                    stream?.close()\n                    return nil\n                }}\n                guard next.count <= totalLimit - total else {{ throw TransportError.responseTooLarge(limit: totalLimit) }}\n                total += next.count\n                chunk = next\n                offset = chunk.startIndex\n            }}\n        }} catch {{\n            finished = true\n            stream?.close()\n            if let error = error as? SDKError {{ throw error }}\n            if Task.isCancelled || error is CancellationError {{ throw CancellationError() }}\n            let captured = HTTPResponse(status: stream?.status ?? 0, headers: stream?.headers ?? [], body: framer.errorCapture)\n            if let failure = error as? ValidationError {{\n                throw SDKError(.responseDecoding, source: Codecs.{codec}.source, response: captured, captureLimit: Client.maxCaptureBytes, validation: failure)\n            }}\n            if let failure = error as? JsonError {{\n                throw SDKError(.responseDecoding, source: Codecs.{codec}.source, response: captured, captureLimit: Client.maxCaptureBytes, json: failure)\n            }}\n            if case TransportError.responseTooLarge = error {{\n                throw SDKError(.responseTooLarge, source: Codecs.{codec}.source, response: captured, captureLimit: Client.maxCaptureBytes)\n            }}\n            if case TransportError.timedOut = error {{\n                throw SDKError(.timeout, source: Codecs.{codec}.source, response: captured, captureLimit: Client.maxCaptureBytes)\n            }}\n            throw SDKError(.transport, source: Codecs.{codec}.source, response: captured, captureLimit: Client.maxCaptureBytes)\n        }}\n    }}\n\n    /// One parsed frame's typed event, or nil while the compiled\n    /// keep-final-usage policy holds it or the frame completed the stream.\n    private mutating func consume(_ value: JsonValue) throws -> {event}? {{\n{data_read}        // The declared terminal sentinel, when compiled, completes the stream\n        // before any payload decoding.\n        if sentinel, data == sentinelToken {{\n            terminal.complete({completion}(reason: .sentinel, usage: held))\n            finished = true\n            stream?.close()\n            return nil\n        }}\n        let kind: String\n        if case .object(let envelope) = value, let raw = envelope[\"event\"], case .string(let name) = raw, !name.isEmpty {{\n            kind = name\n        }} else {{\n            kind = \"message\"\n        }}\n        let typed: {event}\n        switch kind {{\n{kind_switch}\n        default:\n            typed = .unknown(event: kind, data: data)\n        }}\n{hold}    }}\n\n    /// Decodes one framed envelope through the operation's existing stream\n    /// item codec; an invalid payload of a recognized kind remains a branded\n    /// decoding error and releases the transfer.\n    private mutating func decodeEnvelope(_ value: JsonValue) throws -> {payload} {{\n        do {{\n            return try Codecs.{codec}.decodeValue(value, limits: JsonLimits(maxBytes: itemLimit))\n        }} catch {{\n            finished = true\n            stream?.close()\n            let captured = HTTPResponse(status: stream?.status ?? 0, headers: stream?.headers ?? [], body: framer.errorCapture)\n            if let failure = error as? ValidationError {{\n                throw SDKError(.responseDecoding, source: Codecs.{codec}.source, response: captured, captureLimit: Client.maxCaptureBytes, validation: failure)\n            }}\n            if let failure = error as? JsonError {{\n                throw SDKError(.responseDecoding, source: Codecs.{codec}.source, response: captured, captureLimit: Client.maxCaptureBytes, json: failure)\n            }}\n            throw error\n        }}\n    }}\n}}\n",
        sequence = operation.sequence_type,
        event = operation.event_type,
        input = operation.input_type,
        completion = operation.completion_type,
        terminal_type = terminal,
        request = operation.request_method,
        item_limit = operation.item_limit,
        keep = operation.keep_final_usage,
        sentinel_literal = sentinel_literal,
        codec = operation.item_codec,
        kind_switch = kind_switch,
        hold = hold,
        payload = operation.payload,
        data_read = DATA_READ,
    );
}

/// The per-operation client members: the internal events exchange (the direct
/// call's request construction and response-rule handling, handing the open
/// declared stream to the typed events sequence) and the public factory.
fn client_code(
    plan: &SdkPlan,
    op: &super::PlannedOperation,
    operation: &StreamEventsOperation,
    out: &mut String,
) {
    let mut code = String::new();
    code.push_str("extension Client {\n");
    doc(
        &mut code,
        &format!(
            "The {} events exchange: the direct call's request construction and\nresponse-rule handling, handing the open declared stream to the typed events\nsequence instead of its strict item codec. Source: {}#{}.",
            op.operation_id,
            op.source.document(),
            op.source.pointer()
        ),
        "    ",
    );
    let _ = writeln!(
        code,
        "    func {}(_ input: {}, options requestOptions: RequestOptions) async throws -> (HTTPStreamResponse, Int) {{",
        operation.request_method, operation.input_type
    );
    super::protocol_emit::request_prelude(&mut code, op, plan);
    let success = op
        .responses
        .iter()
        .position(|response| response.may_succeed())
        .expect("gated single success response");
    // Failure responses mirror the direct call's decoding exactly. The ones
    // that collect the transferred body are the only ones that reassign
    // `response`; every failure branch reads `noBody`.
    let mut mutable = false;
    let mut has_failures = false;
    for (index, response) in op.responses.iter().enumerate() {
        if index == success {
            continue;
        }
        has_failures = true;
        if !response.always_empty {
            mutable = true;
        }
    }
    let op_source = super::protocol_metadata::source(&op.source);
    let _ = write!(
        code,
        "        let stream = try await open(request, source: {op_source})\n        {response_decl} response = HTTPResponse(status: stream.status, headers: stream.headers, body: Data())\n        do {{\n            let responseIndex = try HTTPResponseRule.select(response, from: {metadata}.responses, source: {op_source}, capture: Self.maxCaptureBytes)\n",
        response_decl = if mutable { "var" } else { "let" },
        metadata = op.metadata_name,
    );
    if has_failures {
        let _ = writeln!(
            code,
            "            let noBody = HTTPBuild.forbidden(method: {}, status: response.status)",
            q(&op.method)
        );
    }
    let _ = writeln!(
        code,
        "            switch responseIndex {{\n            case {success}:\n                let contentType = try HTTPMediaType.contentType(response.headers)\n                let mediaIndex = try HTTPMediaType.select(contentType, from: {metadata}.responses[{success}].media)\n                switch mediaIndex {{\n                case 0: return (stream, request.maxResponseBytes)\n                default: throw JsonError(.representation, \"unmatched response representation\")\n                }}",
        metadata = op.metadata_name,
    );
    for (index, response) in op.responses.iter().enumerate() {
        if index != success {
            failure_case(&mut code, op, plan, index, response);
        }
    }
    let _ = writeln!(
        code,
        "            default: throw SDKError(.unexpectedResponse, source: {op_source}, response: response, captureLimit: Self.maxCaptureBytes)\n            }}\n        }} catch let error as {error_type} {{ throw error }}\n        catch is CancellationError {{ stream.close(); throw CancellationError() }}\n        catch let error as ValidationError {{ stream.close(); throw SDKError(.responseDecoding, source: error.source, response: response, captureLimit: Self.maxCaptureBytes, validation: error) }}\n        catch let error as HTTPWireFailure {{ stream.close(); throw SDKError(.responseDecoding, source: error.source, response: response, captureLimit: Self.maxCaptureBytes, json: error.error) }}\n        catch let error as JsonError {{ stream.close(); throw SDKError(.responseDecoding, source: {op_source}, response: response, captureLimit: Self.maxCaptureBytes, json: error) }}\n        catch {{ stream.close(); throw transportFailure(error, source: {op_source}) }}\n    }}\n",
        error_type = op.error_type,
    );
    let default_input = if operation.default_input {
        " = .init()"
    } else {
        ""
    };
    doc(
        &mut code,
        &format!(
            "Lazily yields the typed events of {}. See ``{}`` for the sentinel,\nunknown-kind and completion semantics; the direct {} call and its untyped\nstream iteration are unchanged.",
            operation.operation_id, operation.event_type, op.method_name
        ),
        "    ",
    );
    let _ = writeln!(
        code,
        "    public func {}(_ input: {}{default_input}, options requestOptions: RequestOptions = .init()) -> {} {{\n        {}(client: self, input: input, options: requestOptions)\n    }}\n}}\n",
        operation.events_method,
        operation.input_type,
        operation.sequence_type,
        operation.sequence_type,
        default_input = default_input,
    );
    out.push_str(&code);
}

/// One declared failure response of the events exchange, decoded exactly like
/// the direct call's response handling (the declared event stream is the only
/// success representation, so every other response is a failure here).
#[allow(clippy::too_many_lines)]
fn failure_case(
    out: &mut String,
    op: &super::PlannedOperation,
    plan: &SdkPlan,
    index: usize,
    response: &super::PlannedResponse,
) {
    let _ = writeln!(
        out,
        "            case {index}:\n                let data: {}\n                let declaredContentType: String?",
        response.type_name
    );
    if response.always_empty {
        out.push_str("                stream.close()\n                data = HTTPNoContent(); declaredContentType = nil\n");
    } else {
        if response.may_be_empty {
            out.push_str("                if noBody {\n                    stream.close()\n                    data = .none; declaredContentType = nil\n                } else {\n");
        }
        if response.media.is_empty() {
            out.push_str("                response = try await stream.collect(limit: request.maxResponseBytes)\n");
            let _ = writeln!(
                out,
                "                data = {}; declaredContentType = nil",
                if response.is_enum {
                    ".bytes(response.body)"
                } else {
                    "response.body"
                }
            );
        } else {
            let _ = writeln!(
                out,
                "                let contentType = try HTTPMediaType.contentType(response.headers)\n                let mediaIndex = try HTTPMediaType.select(contentType, from: {}.responses[{index}].media)\n                switch mediaIndex {{",
                op.metadata_name
            );
            for (media_index, media) in response.media.iter().enumerate() {
                let _ = writeln!(out, "                case {media_index}:");
                let is_stream =
                    matches!(media.wire.representation(), Representation::Stream { .. });
                if !is_stream {
                    out.push_str("                    response = try await stream.collect(limit: request.maxResponseBytes)\n");
                }
                let expr = super::protocol_emit::decode_media(media, plan, true);
                let _ = writeln!(
                    out,
                    "                    data = {}\n                    declaredContentType = {}",
                    if response.is_enum {
                        format!(".{}({expr})", media.case_name)
                    } else {
                        expr
                    },
                    super::protocol_metadata::q(media.wire.media_type().declared())
                );
            }
            out.push_str("                default: throw JsonError(.representation, \"unmatched response representation\")\n                }\n");
        }
        if response.may_be_empty {
            out.push_str("                }\n");
        }
    }
    let base = format!(
        "responseValue(response, data: data, declaredContentType: declaredContentType, links: {}, noBody: noBody)",
        super::protocol_metadata::links(response.wire.links())
    );
    let value = if let Some(headers) = &response.header_type {
        format!(
            "try {}(base: {base}, typedHeaders: {headers}.decode(response.headers, limit: Self.maxResponseBytes))",
            response.response_type
        )
    } else {
        base
    };
    let _ = writeln!(
        out,
        "                let value = {value}\n                throw {}.{}(value)",
        op.error_type, response.case_name
    );
}
