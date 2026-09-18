//! Emitted-only typed SSE event decoding for the generated PHP client.
//!
//! The shared, generation-time `http_protocol::plan_stream_semantics` outcome
//! compiles into per-operation `<op>Events` generator methods on the generated
//! `Client` plus readonly per-kind event, unknown-event and completion classes
//! — all inside the generated `Client.php`. The existing untyped stream item
//! iteration stays the transport: the events generators run the same framing,
//! limits and cancellation through an `ItemStream` whose item codec passes the
//! framed envelope through leniently, and then apply the compiled per-kind
//! decode, sentinel and completion semantics themselves. Recognized event
//! kinds decode through the operation's existing stream item codec into their
//! declared model type; undeclared event kinds surface through the typed
//! unknown-event alternative without failing the stream; invalid payloads of
//! recognized kinds remain decoding errors. Static runtime files are never
//! modified, and operations without a discriminated stream schema emit no new
//! bytes at all.
//!
//! The generator's documented completion channel is the generator return
//! value: the terminal `<Op>Completion` instance is read with `->getReturn()`
//! after iteration ends, mirroring the other backends' `StopIteration.value`
//! semantics; `foreach` loops discard it.

use std::collections::BTreeSet;
use std::fmt::Write;

use super::emit::{doc_tags, php, Emitter};
use super::models::{self, Shape};
use super::{Operation, Payload, Response, SdkPlan};
use crate::http_protocol::{EventPayload, StreamEventPlan, StreamFraming, StreamOperationPlan};
use suspect_ir::contract::SchemaId;

/// One operation's emission-ready typed event decode.
pub(super) struct StreamEventsOperation<'a> {
    /// Source operation identity for documentation and error identity.
    operation_id: &'a str,
    /// Source declaration location, for documentation and error identity.
    source: String,
    /// Allocated generator method name on the generated client.
    method: String,
    /// Allocated per-kind class names, in declaration order.
    event_types: Vec<String>,
    unknown_type: String,
    completion_type: String,
    /// Declared event kinds in declaration order.
    kinds: Vec<String>,
    /// The declared terminal sentinel token, matched on frame data before any
    /// payload decoding.
    sentinel: Option<String>,
    /// Whether the last data frame before the sentinel or end of body is
    /// preserved as the completion's terminal usage instead of being yielded.
    keep_final_usage: bool,
    /// The decoded model member carrying the last-event id, when declared.
    id_member: Option<String>,
    /// The decoded model member carrying the reconnection hint, when declared.
    retry_member: Option<String>,
    /// Native constructor types of the metadata members, when declared.
    id_type: Option<String>,
    retry_type: Option<String>,
    /// Whether the decoded item model itself is nullable.
    item_nullable: bool,
    /// The operation's client method and input, for the shared request
    /// preparation and documentation.
    operation: &'a Operation,
    /// The planned success response carrying the stream media.
    response: &'a Response,
    /// The operation's existing stream item codec lookup, shared by every kind.
    codec: String,
    /// The item model's native constructor type.
    item_type: String,
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

/// The operation's single success stream response matching one compiled
/// entry. Mixed or multiple success stream media keep the conservative
/// untyped path, because the typed decode would otherwise own responses it
/// cannot name.
fn stream_response<'a>(
    operation: &'a Operation,
    compiled: &StreamOperationPlan,
) -> Option<&'a Response> {
    let mut found: Option<&Response> = None;
    for response in &operation.responses {
        if !response.success {
            continue;
        }
        if !matches!(response.payload, Payload::Stream(_)) {
            continue;
        }
        if response
            .media
            .as_ref()
            .is_some_and(|media| media.wire.media_type().declared() != compiled.media_type)
        {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(response);
    }
    found
}

/// One decoded model member and its native constructor type, for the declared
/// envelope metadata fields. Unresolvable shapes keep the metadata implicit.
fn metadata_member(
    plan: &SdkPlan,
    schema: &SchemaId,
    wire_name: &str,
) -> Option<(String, String)> {
    let mut shape = &plan.models().nodes[schema].shape;
    loop {
        match shape {
            Shape::Ref(target) => shape = &plan.models().nodes[target].shape,
            Shape::Object { fields, .. } => {
                let field = fields.iter().find(|field| field.wire == wire_name)?;
                let ty = plan.models().type_name(&field.source, false);
                let ty = if field.required {
                    ty
                } else {
                    models::union(vec![ty, "Absent".into()])
                };
                return Some((field.name.clone(), ty));
            }
            _ => return None,
        }
    }
}

/// Every typed stream operation, in plan order, with allocated method and
/// class names. The allocated public class names join one allocation space
/// with the models and operation classes so nothing else can take them, and
/// the generator methods join the client's method space.
pub(super) fn operations(plan: &SdkPlan) -> Vec<StreamEventsOperation<'_>> {
    let streams = plan.stream_semantics();
    if streams.streams.is_empty() {
        return Vec::new();
    }
    let mut classes: BTreeSet<String> = models::reserved_symbols();
    for node in plan.models().nodes.values() {
        classes.insert(node.name.to_ascii_lowercase());
    }
    for operation in plan.operations() {
        classes.insert(operation.input.to_ascii_lowercase());
        classes.insert(operation.error.to_ascii_lowercase());
        for response in &operation.responses {
            classes.insert(response.name.to_ascii_lowercase());
            classes.insert(response.headers.name.to_ascii_lowercase());
        }
    }
    let mut methods: BTreeSet<String> = ["__construct".into(), "exchange".into()]
        .into_iter()
        .collect();
    if plan.credential_env().is_some() {
        methods.insert("fromenv".into());
    }
    for operation in plan.operations() {
        methods.insert(operation.method.clone());
        // Pagination walkers allocate in their own space; reserving their
        // method bases keeps both method families collision-free together.
        for suffix in ["Pages", "Items", "NextPage"] {
            methods.insert(format!("{}{suffix}", operation.method).to_ascii_lowercase());
        }
    }
    let mut compiled = Vec::new();
    for operation in plan.operations() {
        let Some(stream) = streams
            .streams
            .iter()
            .find(|stream| stream.operation == operation.id)
        else {
            continue;
        };
        if !admits_typed_events(stream) {
            continue;
        }
        let Some(response) = stream_response(operation, stream) else {
            continue;
        };
        let Payload::Stream(item_schema) = &response.payload else {
            continue;
        };
        let Some(node) = plan.models().nodes.get(item_schema) else {
            continue;
        };
        let codec = node.codecs.from_value.clone();
        let stem = models::pascal(&operation.id);
        let (id_member, id_type) = stream
            .item_metadata
            .as_ref()
            .and_then(|metadata| metadata.id_field.as_deref())
            .and_then(|wire_name| metadata_member(plan, item_schema, wire_name))
            .map_or((None, None), |(member, ty)| (Some(member), Some(ty)));
        let (retry_member, retry_type) = stream
            .item_metadata
            .as_ref()
            .and_then(|metadata| metadata.retry_field.as_deref())
            .and_then(|wire_name| metadata_member(plan, item_schema, wire_name))
            .map_or((None, None), |(member, ty)| (Some(member), Some(ty)));
        compiled.push(StreamEventsOperation {
            operation_id: &operation.id,
            source: format!(
                "{}#{}",
                operation.source.document(),
                operation.source.pointer()
            ),
            method: models::allocate(&format!("{}Events", operation.method), &mut methods),
            event_types: stream
                .events
                .iter()
                .map(|event| {
                    models::allocate(
                        &format!("{stem}Event{}", models::pascal(&event.event_name)),
                        &mut classes,
                    )
                })
                .collect(),
            unknown_type: models::allocate(&format!("{stem}EventUnknown"), &mut classes),
            completion_type: models::allocate(&format!("{stem}Completion"), &mut classes),
            kinds: stream
                .events
                .iter()
                .map(|event| event.event_name.clone())
                .collect(),
            sentinel: stream.sentinel.enabled.then(|| stream.sentinel.token.clone()),
            keep_final_usage: stream.terminal.keep_final_usage,
            id_member,
            id_type,
            retry_member,
            retry_type,
            item_nullable: node.nullable,
            operation,
            response,
            codec,
            item_type: plan.models().type_name(item_schema, false),
        });
    }
    compiled
}

fn q(text: &str) -> String {
    php(text)
}

/// The per-frame typed decode body: sentinel first, then the per-kind decode
/// with the compiled unknown alternative, then the compiled keep-final-usage
/// holding policy.
fn decode_body(entry: &StreamEventsOperation<'_>) -> String {
    let codec = &entry.codec;
    let mut out = String::new();
    out.push_str("                    $members = $envelope->kind === JsonKind::Object ? $envelope->asObject() : [];\n");
    out.push_str("                    $data = $members['data'] ?? null;\n");
    out.push_str("                    $data = $data !== null && $data->kind === JsonKind::String ? $data->asString() : '';\n");
    if let Some(sentinel) = &entry.sentinel {
        // The declared sentinel completes the stream before any payload
        // decoding; the compiled keep-final-usage policy holds one frame
        // behind so the last data frame before the sentinel or end of body
        // becomes the completion's terminal usage.
        writeln!(
            out,
            "                    // The declared {sentinel} sentinel completes the stream before any payload decoding.\n                    if ($data === {}) {{ return new {}('sentinel', $held); }}",
            q(sentinel),
            entry.completion_type
        )
        .unwrap();
    }
    out.push_str("                    $kind = $members['event'] ?? null;\n");
    out.push_str("                    $kind = $kind !== null && $kind->kind === JsonKind::String && $kind->asString() !== '' ? $kind->asString() : 'message';\n");
    for (index, kind) in entry.kinds.iter().enumerate() {
        let keyword = if index == 0 { "if" } else { "elseif" };
        writeln!(out, "                    {keyword} ($kind === {}) {{", q(kind)).unwrap();
        writeln!(
            out,
            "                        try {{ $item = Codecs::{codec}($envelope, $context); }}\n                        catch (JsonError|ValidationError|\\Error $error) {{ throw new SdkError('response_validation', 'stream item violates its source framing/schema', new ResponseCapture($response->status, $response->headers, '', true), $error); }}\n                        $typed = new {}({kind}, $item{});",
            entry.event_types[index],
            metadata_arguments(entry),
            kind = q(kind),
        )
        .unwrap();
        out.push_str("                    }\n");
    }
    writeln!(
        out,
        "                    else {{\n                        $typed = new {}('unknown', $kind, $data);\n                    }}",
        entry.unknown_type
    )
    .unwrap();
    if entry.keep_final_usage {
        out.push_str("                    if ($held !== null) { yield $held; }\n                    $held = $typed;\n");
    } else {
        out.push_str("                    yield $typed;\n");
    }
    out
}

/// Reading one declared metadata member from the decoded model, guarded for a
/// nullable carrier.
fn metadata_value(entry: &StreamEventsOperation<'_>, member: &str) -> String {
    if entry.item_nullable {
        format!("($item === null ? null : $item->{member})")
    } else {
        format!("$item->{member}")
    }
}

/// The decoded-model metadata constructor arguments, when declared.
fn metadata_arguments(entry: &StreamEventsOperation<'_>) -> String {
    let mut out = String::new();
    if let Some(member) = &entry.id_member {
        out.push_str(", ");
        out.push_str(&metadata_value(entry, member));
    }
    if let Some(member) = &entry.retry_member {
        out.push_str(", ");
        out.push_str(&metadata_value(entry, member));
    }
    out
}

/// The response and media indexes of the compiled stream payload, matching the
/// direct method's branch condition exactly.
fn response_media_indexes(entry: &StreamEventsOperation<'_>) -> (usize, usize) {
    let status = entry
        .operation
        .wire
        .responses()
        .iter()
        .position(|candidate| candidate.status_key() == entry.response.status_key)
        .unwrap_or(0);
    let media = entry
        .response
        .media
        .as_ref()
        .and_then(|media| {
            entry
                .operation
                .wire
                .responses()
                .iter()
                .find(|candidate| candidate.status_key() == entry.response.status_key)
                .and_then(|candidate| {
                    candidate
                        .media()
                        .iter()
                        .position(|candidate| candidate.source().use_site().source() == &media.source)
                })
        })
        .unwrap_or(0);
    (status, media)
}

/// The constructor-argument default fragment reused from the direct method.
fn argument_default(operation: &Operation) -> String {
    let mut required = operation.parameters.iter().any(|p| p.wire.required());
    if !operation.body.is_empty() && operation.body_required {
        required = true;
    }
    if required {
        String::new()
    } else {
        format!(" = new {}()", operation.input)
    }
}

fn events_doc(entry: &StreamEventsOperation<'_>) -> String {
    let sentinel = match &entry.sentinel {
        Some(token) => format!(
            " The declared {token} sentinel completes the stream before any payload decoding, preserving the final usage frame as the completion's terminal metadata and issuing no further reads."
        ),
        None => String::new(),
    };
    format!(
        "Lazily yields the typed events of {} over the existing stream item iteration. A declared event kind decodes the framed envelope through the operation's stream item codec into its declared model type; an event kind the source never declared surfaces through the typed {} alternative without failing the stream; an invalid payload for a recognized kind remains a decoding error. Per-item metadata (the event name as `kind`, plus id/retry when the item schema declares them) is explicit, and the terminal {} completion carries the documented {{ reason, usage }} metadata as the generator's return value, read with ->getReturn() after iteration ends. Iteration is pull-driven: early break or abandoning the generator stops the reader and issues no further reads. The direct {}(...) call and its untyped stream iterator are unchanged.\nSource: {}",
        entry.operation_id,
        entry.unknown_type,
        entry.completion_type,
        entry.operation.method,
        entry.source,
    ) + &sentinel
}

/// The typed events generator methods appended inside the generated client
/// class, in plan order. Empty when nothing carries a discriminated SSE set.
pub(super) fn client_methods(plan: &SdkPlan) -> String {
    let compiled = operations(plan);
    if compiled.is_empty() {
        return String::new();
    }
    let emitter = Emitter::new(plan);
    let mut out = String::new();
    for entry in &compiled {
        out.push_str(&events_method(&emitter, entry));
    }
    out
}

#[allow(clippy::too_many_lines)]
fn events_method(emitter: &Emitter<'_>, entry: &StreamEventsOperation<'_>) -> String {
    let response = entry.response;
    let (status_index, media_index) = response_media_indexes(entry);
    let mut out = String::new();
    doc_tags(
        &mut out,
        &events_doc(entry),
        &[
            format!("@param {} $input Source-bound native arguments.", entry.operation.input),
            "@param RequestOptions|null $options Per-call limits and explicit security selection."
                .into(),
            format!(
                "@return \\Generator<{}> yielding the typed events; the terminal \\{} completion is the generator's return value, read with ->getReturn() after iteration ends (foreach loops discard it).",
                events_union(entry),
                entry.completion_type
            ),
            "@throws SdkError Preparation, transport, response validation, resource or decoding failure."
                .into(),
        ],
    );
    let mut body = String::new();
    body.push_str("        try {\n");
    body.push_str(&emitter.request_preparation(entry.operation));
    body.push_str(
        "        try { $response = Protocol::exchange($metadata, $this->credentials, $this->transport, $this->options, $options, $call, $values, $payload, true); }\n        catch (JsonError|ValidationError|\\Error $error) { throw new SdkError('request_validation', 'request violates its protocol representation', previous: $error); }\n",
    );
    body.push_str("        try {\n        $status = Protocol::matchStatus(Protocol::get($metadata, 'responses'), $response->status);\n        $declaration = Protocol::list($metadata, 'responses')[$status];\n        $forbidden = Protocol::text($metadata, 'method') === 'HEAD' || $response->status < 200 || in_array($response->status, [204, 205, 304], true);\n        $media = -1;\n        if (!$forbidden && Protocol::list($declaration, 'media') !== []) {\n            $contentType = $response->headerValues('content-type');\n            if (count($contentType) !== 1) { throw new SdkError('unexpected_media', 'exactly one Content-Type is required'); }\n            $media = Protocol::matchMedia(Protocol::get($declaration, 'media'), $contentType[0]);\n        }\n        $context = new CodecContext($call->control);\n");
    writeln!(
        body,
        "        if ($status === {status_index} && $media === {media_index} && $response->status >= 200 && $response->status < 300 && !$forbidden) {{",
    )
    .unwrap();
    if let Some(media) = &response.media
        && let crate::http_protocol::Representation::Stream { stream } = media.wire.representation()
    {
        writeln!(
            body,
            "            $items = new ItemStream($response, 'server-sent-events', static fn (JsonValue $item): JsonValue => $item, $call, {}, {}, {}, {});\n            $call->check();\n            $held = null;\n            try {{\n                foreach ($items as $envelope) {{\n{}                }}\n            }} finally {{ $items->close(); }}",
            stream.max_item_bytes(),
            q(&format!("{}Events", entry.operation_id)),
            q(&entry.source),
            response.wire.max_body_bytes(),
            decode_body(entry),
        )
        .unwrap();
        if entry.keep_final_usage {
            writeln!(body, "            return new {}('eof', $held);", entry.completion_type).unwrap();
        } else {
            writeln!(body, "            return new {}('eof', null);", entry.completion_type).unwrap();
        }
    }
    body.push_str("        }\n");
    body.push_str("        throw new SdkError('unexpected_status', 'no admitted native response variant');\n");
    body.push_str("        } catch (SdkError $error) { throw $error->withCapture(Protocol::capture($response, $call)); }\n");
    writeln!(
        body,
        "        }} catch (SdkError $error) {{ throw $error->at({}, {}); }}\n    }}",
        q(&format!("{}Events", entry.operation_id)),
        q(&entry.source),
    )
    .unwrap();
    writeln!(
        out,
        "    public function {}({} $input{}, ?RequestOptions $options = null): \\Generator {{\n{}",
        entry.method,
        entry.operation.input,
        argument_default(entry.operation),
        body
    )
    .unwrap();
    out
}

/// The discriminated event union, including the typed unknown alternative.
fn events_union(entry: &StreamEventsOperation<'_>) -> String {
    models::union(
        entry
            .event_types
            .iter()
            .cloned()
            .chain([entry.unknown_type.clone()])
            .collect(),
    )
}

/// The readonly per-kind event, unknown-event and completion classes appended
/// after the generated client class. Empty when nothing emits.
pub(super) fn classes(plan: &SdkPlan) -> String {
    let compiled = operations(plan);
    let mut out = String::new();
    for entry in &compiled {
        for (index, kind) in entry.kinds.iter().enumerate() {
            let class = &entry.event_types[index];
            doc_tags(
                &mut out,
                &format!(
                    "A declared {kind} event of the {} stream: the framed envelope decoded to its declared model type. An invalid payload for this kind remains a decoding error.",
                    entry.operation_id
                ),
                &[],
            );
            writeln!(
                out,
                "final readonly class {class} {{\n    public function __construct(\n        public readonly string $kind,\n        public readonly {} $data,{}\n    ) {{}}\n}}",
                entry.item_type,
                metadata_constructor(entry),
            )
            .unwrap();
        }
        doc_tags(
            &mut out,
            "An event kind the source never declared: the raw frame data stays representable without failing the stream.",
            &[],
        );
        writeln!(
            out,
            "final readonly class {} {{\n    public function __construct(\n        public readonly string $kind,\n        public readonly string $event,\n        public readonly string $data,\n    ) {{}}\n}}",
            entry.unknown_type
        )
        .unwrap();
        doc_tags(
            &mut out,
            &format!(
                "Terminal metadata of one completed {} stream: why it completed and the preserved final usage frame. This is the typed generator's return value, read with ->getReturn(); foreach loops discard it.",
                entry.operation_id
            ),
            &[],
        );
        writeln!(
            out,
            "final readonly class {} {{\n    public function __construct(\n        public readonly string $reason,\n        public readonly {} $usage,\n    ) {{}}\n}}",
            entry.completion_type,
            models::union(
                entry
                    .event_types
                    .iter()
                    .cloned()
                    .chain([entry.unknown_type.clone(), "null".into()])
                    .collect()
            ),
        )
        .unwrap();
    }
    out
}

/// The declared metadata constructor members, emitted only for the envelope
/// fields the item schema declares, typed from the decoded model.
fn metadata_constructor(entry: &StreamEventsOperation<'_>) -> String {
    let mut out = String::new();
    if let (Some(member), Some(ty)) = (&entry.id_member, &entry.id_type) {
        out.push_str(&format!("\n        public readonly {ty} ${member},"));
    }
    if let (Some(member), Some(ty)) = (&entry.retry_member, &entry.retry_type) {
        out.push_str(&format!("\n        public readonly {ty} ${member},"));
    }
    out
}
