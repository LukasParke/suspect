//! Generated-only typed event decode for the TypeScript HTTP adapter.
//!
//! The compiled `http_protocol::StreamSemanticsPlan` becomes one per-operation
//! typed events iterator on the generated `operations.ts` for exactly the
//! operations whose stream plan carries a discriminated SSE event set. The
//! existing untyped stream item iteration stays the transport: the events
//! iterator runs the same framing, limits and cancellation through a descriptor
//! whose stream item codec reads the framed envelope leniently, and then
//! applies the compiled per-kind decode, sentinel and completion semantics
//! itself. Recognized event kinds decode through the operation's existing
//! response codecs into their declared model type; undeclared event kinds
//! surface through the typed unknown-event alternative without failing the
//! stream; invalid payloads of recognized kinds remain decoding errors. Static
//! runtime files are never modified, and operations without a discriminated
//! stream schema emit nothing at all.

use std::collections::{BTreeMap, BTreeSet};

use super::{PlannedOperation, src};
use crate::http_protocol::{
    EventPayload, MediaPlan, Representation, ResponseStatus, StreamEventPlan, StreamFraming,
    StreamOperationPlan, StreamSemanticsPlan,
};
use suspect_ir::contract::SchemaId;

/// One operation's emission-ready typed event decode.
pub(super) struct StreamEventsOperation {
    /// Generated operation function name; the events iterator appends `Events`.
    pub function_name: String,
    /// Source operation identity for documentation.
    pub operation_id: String,
    /// Formatted source location for documentation comments.
    pub source: String,
    pub input_type: String,
    pub input_optional: bool,
    /// The response-view model bound to the stream item codec: every declared
    /// kind's payload decodes into this declared model type.
    pub item_model: String,
    /// The response-codec table key replaced with the lenient envelope reader.
    pub item_codec_key: String,
    /// Declared event kinds in declaration order.
    pub events: Vec<String>,
    /// The declared terminal sentinel token, matched on frame data before any
    /// payload decoding.
    pub sentinel: Option<String>,
    /// Whether the last data frame before the sentinel or end of body is
    /// preserved as the completion's terminal usage instead of being yielded.
    pub keep_final_usage: bool,
    /// Whether the declared envelope metadata carries the last-event id.
    pub id_metadata: bool,
    /// Whether the declared envelope metadata carries the reconnection hint.
    pub retry_metadata: bool,
    /// Allocated, collision-free public type names.
    pub events_type: String,
    pub completion_type: String,
}

/// The shared planner's operation identity for one planned operation: the
/// operation id, or `METHOD /path` when unnamed (the pagination convention).
fn identity(operation: &PlannedOperation) -> String {
    operation
        .protocol()
        .operation_id()
        .map(|located| located.value().clone())
        .unwrap_or_else(|| {
            format!(
                "{} {}",
                operation.protocol().method().as_str(),
                operation.protocol().path()
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

/// The operation's single success stream media matching one compiled entry.
/// Mixed or multiple success media keep the conservative untyped path, because
/// the typed decode would otherwise own responses it cannot name.
fn stream_media<'a>(
    operation: &'a PlannedOperation,
    compiled: &StreamOperationPlan,
) -> Option<&'a MediaPlan> {
    let mut found: Option<&MediaPlan> = None;
    for response in operation.protocol().responses() {
        if !matches!(
            response.status(),
            ResponseStatus::Exact(200..=299) | ResponseStatus::Range(2)
        ) {
            continue;
        }
        for media in response.media() {
            if !matches!(media.representation(), Representation::Stream { .. }) {
                continue;
            }
            if media.media_type().declared() != compiled.media_type {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some(media);
        }
    }
    found
}

/// Compiles the typed-event subset of the stream semantics plan against the
/// native operations, the response-view model bindings and the public symbol
/// allocation. The allocated type names are registered as public symbols so a
/// later operation or model can never collide with them.
pub(super) fn prepare(
    operations: &[PlannedOperation],
    symbols: &BTreeMap<SchemaId, String>,
    public: &mut BTreeSet<String>,
    plan: &StreamSemanticsPlan,
) -> Vec<StreamEventsOperation> {
    let mut result = Vec::new();
    for operation in operations {
        let Some(compiled) = plan
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
        let Some(item_model) = symbols.get(item_codec.schema().id()) else {
            continue;
        };
        let stem = super::upper(&operation.function_name);
        result.push(StreamEventsOperation {
            function_name: operation.function_name.clone(),
            operation_id: operation.operation_id.clone(),
            source: src(&operation.source),
            input_type: operation.input_type.clone(),
            input_optional: operation.input_optional(),
            item_model: item_model.clone(),
            item_codec_key: super::protocol_emit::schema_key(item_codec.schema().id()),
            events: compiled
                .events
                .iter()
                .map(|event| event.event_name.clone())
                .collect(),
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
            events_type: allocate(&format!("{stem}Event"), public),
            completion_type: allocate(&format!("{stem}Completion"), public),
        });
        allocate(&format!("{}Events", operation.function_name), public);
    }
    result
}

/// Collision-free public TypeScript symbol names, registered while allocating.
fn allocate(base: &str, public: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !public.insert(name.clone()) {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    name
}

fn q(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

/// The per-operation typed events emission appended to `operations.ts`: the
/// discriminated event union, the completion metadata, and the typed events
/// generator itself. The lenient descriptor clone is emitted by the operations
/// module beside the direct descriptor (`protocol_emit`), as one plain literal
/// so bundlers can shed it with the operation when no typed events iterator is
/// referenced.
pub(super) fn emit(streams: &[StreamEventsOperation]) -> String {
    let mut code = String::new();
    for stream in streams {
        code.push_str(&event_type(stream));
        code.push_str(&completion_type(stream));
        code.push_str(&generator(stream));
    }
    code
}

/// The `readonly id`/`readonly retry` metadata members, emitted only for the
/// envelope fields the item schema declares, typed from the decoded model.
fn metadata_members(stream: &StreamEventsOperation, model: &str) -> String {
    let mut members = String::new();
    if stream.id_metadata {
        members.push_str(&format!("readonly id: {model}['id'], "));
    }
    if stream.retry_metadata {
        members.push_str(&format!("readonly retry: {model}['retry'], "));
    }
    members
}

fn event_type(stream: &StreamEventsOperation) -> String {
    let model = format!("Models.{}", stream.item_model);
    let metadata = metadata_members(stream, &model);
    let variants = stream
        .events
        .iter()
        .map(|kind| {
            format!(
                "  | {{ readonly kind: {}; readonly data: {model}; {metadata} }}",
                q(kind)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}export type {} =\n{variants}\n  | {{ readonly kind: 'unknown'; readonly event: string; readonly data: string }};\n",
        crate::typescript::declaration_comment(
            &format!(
                "One typed event of the {} stream. Declared kinds decode the framed envelope through the operation's stream item codec into their declared model type; an event kind the source never declared surfaces through the typed unknown alternative without failing the stream. The `kind` member is the declared event name; the metadata members repeat the envelope fields the item schema declares.",
                stream.operation_id
            ),
            &stream.source,
            "Compiled from the source stream semantics plan; unrecognized kinds never raise and invalid payloads of recognized kinds remain decoding errors."
        ),
        stream.events_type,
        variants = variants,
    )
}

fn completion_type(stream: &StreamEventsOperation) -> String {
    format!(
        "{}export interface {} {{\n    /** `sentinel` when the declared terminal token completed the stream before any payload decoding, `eof` when the response body ended. */\n    readonly reason: 'sentinel' | 'eof';\n    /** The last data frame before the sentinel or end of body, decoded like an ordinary event and preserved as terminal instead of being yielded; null when no frame preceded completion or the compiled policy preserves nothing. */\n    readonly usage: {} | null;\n}}\n",
        crate::typescript::declaration_comment(
            &format!(
                "Terminal metadata of one completed {} stream: why it completed and the preserved final usage frame. This is the typed iterator's completion value; `for await` loops discard it, and a manual `next()` after the final event reads it.",
                stream.operation_id
            ),
            &stream.source,
            "The completion value is documented behavior, not an exception."
        ),
        stream.completion_type,
        stream.events_type,
    )
}

fn generator(stream: &StreamEventsOperation) -> String {
    let model = format!("Models.{}", stream.item_model);
    let input_default = if stream.input_optional { " = {}" } else { "" }; // The sentinel completes the stream before any payload decoding; the
    // compiled keep-final-usage policy holds one frame behind so the last data
    // frame before the sentinel or end of body becomes the completion's usage.
    let sentinel = match &stream.sentinel {
        Some(token) => format!(
            "            const data = typeof envelope.data === 'string' ? envelope.data : '';\n            // The declared {} sentinel completes the stream before any payload decoding.\n            if (data === {}) return {{ reason: 'sentinel', usage: held }};\n",
            crate::typescript::escape_prose(token),
            q(token)
        ),
        None => String::new(),
    };
    let case_labels = stream
        .events
        .iter()
        .map(|kind| format!("case {}:", q(kind)))
        .collect::<Vec<_>>()
        .join(" ");
    let mut metadata_values = String::new();
    if stream.id_metadata {
        metadata_values.push_str(", id: item.id");
    }
    if stream.retry_metadata {
        metadata_values.push_str(", retry: item.retry");
    }
    let yield_loop = if stream.keep_final_usage {
        "            if (held !== null) yield held;\n            held = typed;\n        }\n        return { reason: 'eof', usage: held };\n"
    } else {
        "            yield typed;\n        }\n        return { reason: 'eof', usage: null };\n"
    };
    format!(
        "{}export async function* {}Events(client: ClientOptions, input: {}{}, call?: CallOptions): AsyncGenerator<{}, {}, undefined> {{\n    const items = ((await executeOperation({}EventsDescriptor, input, client, call)) as {{\n        readonly data: AsyncGenerator<string, void, unknown>;\n    }}).data;\n    let held: {} | null = null;\n    try {{\n        for await (const text of items) {{\n            const envelope = parseJson(text, {{ maxLength: text.length }}) as {{ readonly data?: unknown; readonly event?: unknown }};\n{}            const event = typeof envelope.event === 'string' && envelope.event !== '' ? envelope.event : 'message';\n            let typed: {};\n            switch (event) {{\n                {} {{\n                    let item: {};\n                    try {{ item = Codecs.{}Codec.decode(text); }}\n                    catch (error) {{ throw responseFailure(error, {}Descriptor.source, {}Descriptor.source); }}\n                    typed = {{ kind: event, data: item{} }};\n                    break;\n                }}\n                default:\n                    typed = {{ kind: 'unknown', event, data: typeof envelope.data === 'string' ? envelope.data : '' }};\n            }}\n{}    }}\n    finally {{ await items.return(undefined); }}\n}}\n",
        crate::typescript::declaration_comment(
            &format!(
                "Lazily yields the typed events of {} over the existing stream item iteration.",
                stream.operation_id
            ),
            &stream.source,
            "The declared sentinel token, when compiled, completes the stream before any payload decoding and preserves the final usage frame as the completion's terminal metadata. Recognized event kinds decode through the operation's stream item codec; undeclared kinds surface through the typed unknown alternative; invalid payloads of recognized kinds remain decoding errors. Iteration is pull-driven: early break or return cancels the response body and issues no further reads, and the documented completion value carries the terminal metadata."
        ),
        stream.function_name,
        stream.input_type,
        input_default,
        stream.events_type,
        stream.completion_type,
        stream.function_name,
        stream.events_type,
        sentinel,
        stream.events_type,
        case_labels,
        model,
        stream.item_model,
        stream.function_name,
        stream.function_name,
        metadata_values,
        yield_loop,
    )
}
