//! Emitted-only typed SSE event decoding for the Kotlin coroutine client.
//!
//! The shared, generation-time `http_protocol::plan_stream_semantics` outcome
//! lowers into a generated `StreamEvents.kt` holding, per discriminated SSE
//! stream operation: the sealed typed-event union (one data class per declared
//! kind plus the typed unknown alternative and the terminal completion
//! element) and per-operation members on the generated `Client`: a cold
//! `<op>Events` `Flow` whose collection initiates the exchange.
//!
//! The existing untyped stream iteration stays the transport: the events flow
//! reuses the direct call's exact request preparation and the runtime
//! collector's framing, limits and cancellation, then applies the compiled
//! per-kind decode, sentinel and completion semantics itself. Recognized event
//! kinds decode through the operation's existing stream item codec into their
//! declared model type; undeclared event kinds surface through the typed
//! unknown-event alternative without failing the stream; invalid payloads of
//! recognized kinds remain branded decoding errors. Because Kotlin `Flow`s
//! cannot return values, the documented completion is the flow's final
//! element: a `<Op>CompletionEvent` carrying the reason (`sentinel`|`eof`)
//! and the preserved final usage frame. Early collection end cancels the
//! exchange and issues no further reads. The `held` and `terminal` slots and
//! the terminal translation exist on every typed flow, so the documented
//! final completion element has one uniform shape; without the compiled
//! policy the slot stays null and the eof completion carries no usage.
//! Static runtime files are never modified, and operations without a
//! discriminated SSE stream schema emit no new bytes at all.

use std::collections::BTreeSet;

use super::emit::{header, kdoc, quote, source};
use super::models::{self, ModelPlan};
use super::{PlannedOperation, PlannedResponse};
use crate::http_protocol as wire;
use crate::http_protocol::{EventPayload, StreamEventPlan};
use suspect_ir::contract::{SchemaId, SourceId};

/// One discriminated SSE stream operation's compiled emission.
#[derive(Debug, Clone)]
pub struct StreamEventsEntry {
    /// Index into the generated protocol operation registry.
    pub index: usize,
    /// Source identity of the streamed operation.
    pub operation: String,
    pub source: SourceId,
    /// The generated client method this flow extends.
    pub method: String,
    /// The allocated cold-flow member name.
    pub events_method: String,
    pub input_type: String,
    /// The stream item model: the declared type every recognized kind's
    /// envelope decodes into (bound after model planning).
    pub item_type: String,
    /// The stream item codec, shared with the direct call's untyped iteration.
    pub item_codec: String,
    /// Declared event kinds in declaration order.
    pub kinds: Vec<String>,
    /// The declared terminal sentinel token, matched on the frame data before
    /// any payload decoding.
    pub sentinel: Option<String>,
    /// Whether the last data frame before the sentinel or end of body is
    /// preserved as the completion's terminal usage instead of being yielded.
    pub keep_final_usage: bool,
    /// Whether the declared envelope metadata carries the last-event id.
    pub id_metadata: bool,
    /// Whether the declared envelope metadata carries the reconnection hint.
    pub retry_metadata: bool,
    /// Whether the operation input has a constructor default.
    pub input_default: bool,
    /// Allocated public type names.
    pub event_type: String,
    pub unknown_type: String,
    pub completion_type: String,
    pub completion_reason_type: String,
    pub completion_event_type: String,
    pub terminal_type: String,
    /// Per-kind event data class names, in declaration order.
    pub kind_types: Vec<String>,
    /// The stream item schema identity, bound to its model after planning.
    pub(crate) item_schema: SchemaId,
}

/// The compiled typed-stream emission carried by one plan.
#[derive(Debug, Clone, Default)]
pub struct StreamEventsPlan {
    /// The shared, infallible stream-semantics selection this emission follows.
    pub semantics: wire::StreamSemanticsPlan,
    pub operations: Vec<StreamEventsEntry>,
}

impl StreamEventsPlan {
    /// Whether this plan emits typed event flows at all.
    #[must_use]
    pub fn emits(&self) -> bool {
        !self.operations.is_empty()
    }
}

/// Whether the compiled event set is discriminated: at least two declared
/// kinds, or one declared kind compiled from discrimination evidence whose
/// payload is a declared JSON structure. The single un-discriminated default
/// event keeps the existing untyped path byte-for-byte.
fn discriminated(stream: &wire::StreamOperationPlan) -> bool {
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
fn admits_typed_events(stream: &wire::StreamOperationPlan) -> bool {
    stream.framing == wire::StreamFraming::ServerSentEvents
        && stream
            .item_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.event_field.as_deref() == Some("event"))
        && discriminated(stream)
}

/// The operation's single success response whose declared body is exactly the
/// compiled stream media; other success representations keep the conservative
/// untyped path, because the typed decode would otherwise own responses it
/// cannot name.
fn stream_target<'a>(
    operation: &'a PlannedOperation,
    compiled: &wire::StreamOperationPlan,
) -> Option<&'a PlannedResponse> {
    let stream: Vec<&PlannedResponse> = operation
        .responses
        .iter()
        .filter(|r| r.success && r.stream)
        .collect();
    if stream.len() != 1 {
        return None;
    }
    let response = stream[0];
    // Other success representations stay unread by the typed flow.
    if operation.responses.iter().any(|r| {
        r.success && !r.stream && r.disposition != wire::ResponseBodyDisposition::ForbiddenByHttp
    }) {
        return None;
    }
    let media = response.media.as_ref()?;
    if media.wire.media_type().declared() != compiled.media_type {
        return None;
    }
    Some(response)
}

/// Phase one, before model planning: compile the per-operation guards and
/// reserve every type name the typed streams will declare, so model symbols
/// can never take them.
pub(super) fn reserve(
    operations: &[PlannedOperation],
    semantics: &wire::StreamSemanticsPlan,
    type_names: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> Vec<StreamEventsEntry> {
    let mut entries = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        let Some(stream) = semantics
            .streams
            .iter()
            .find(|stream| stream.operation == operation.operation_id)
        else {
            continue;
        };
        if !admits_typed_events(stream) || stream_target(operation, stream).is_none() {
            continue;
        }
        let Some(item_codec) = &stream.item_codec else {
            continue;
        };
        let stem = models::type_name(&operation.operation_id);
        entries.push(StreamEventsEntry {
            index,
            operation: operation.operation_id.clone(),
            source: operation.source.clone(),
            method: operation.method_name.clone(),
            events_method: models::allocate(&format!("{}Events", operation.method_name), methods),
            input_type: operation.input_type.clone(),
            item_type: String::new(),
            item_codec: String::new(),
            kinds: stream
                .events
                .iter()
                .map(|event| event.event_name.clone())
                .collect(),
            sentinel: stream
                .sentinel
                .enabled
                .then(|| stream.sentinel.token.clone()),
            keep_final_usage: stream.terminal.keep_final_usage,
            id_metadata: stream
                .item_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.id_field.is_some()),
            retry_metadata: stream
                .item_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.retry_field.is_some()),
            input_default: operation.input_has_default(),
            event_type: models::allocate(&format!("{stem}Event"), type_names),
            unknown_type: models::allocate(&format!("{stem}UnknownEvent"), type_names),
            completion_type: models::allocate(&format!("{stem}Completion"), type_names),
            completion_reason_type: models::allocate(
                &format!("{stem}CompletionReason"),
                type_names,
            ),
            completion_event_type: models::allocate(&format!("{stem}CompletionEvent"), type_names),
            terminal_type: models::allocate(&format!("{stem}Terminal"), type_names),
            kind_types: stream
                .events
                .iter()
                .map(|event| {
                    models::allocate(
                        &format!("{stem}{}Event", models::type_name(&event.event_name)),
                        type_names,
                    )
                })
                .collect(),
            item_schema: item_codec.schema().id().clone(),
        });
    }
    entries
}

/// Phase two, after model planning: bind each entry to its stream item model
/// and codec. Entries whose item model never reached the native plan are
/// dropped instead of emitting guesses.
pub(super) fn bind(entries: &mut Vec<StreamEventsEntry>, models: &ModelPlan) {
    entries.retain_mut(|entry| {
        let Some(symbol) = models.symbol(&entry.item_schema) else {
            return false;
        };
        entry.item_type = symbol.kotlin_type.clone();
        entry.item_codec = symbol.codec_name.clone();
        true
    });
}

// ---------------------------------------------------------------------------
// Type rendering
// ---------------------------------------------------------------------------

/// The optional envelope metadata constructor parameters, emitted only for
/// the fields the item schema declares.
fn metadata_members(entry: &StreamEventsEntry) -> String {
    let mut out = String::new();
    if entry.id_metadata {
        out.push_str("    /** Last-event id envelope metadata, when the frame carries one. */\n    public val id: String? = null,\n");
    }
    if entry.retry_metadata {
        out.push_str("    /** Reconnection hint envelope metadata in milliseconds, when the frame carries one. */\n    public val retry: JsonNumber? = null,\n");
    }
    out
}

fn metadata_args(entry: &StreamEventsEntry) -> String {
    let mut out = String::new();
    if entry.id_metadata {
        out.push_str(", id = (envelope.values[\"id\"] as? JsonString)?.value");
    }
    if entry.retry_metadata {
        out.push_str(", retry = envelope.values[\"retry\"] as? JsonNumber");
    }
    out
}

/// The per-frame typed decode: sentinel first, then the per-kind decode with
/// the compiled unknown alternative, then the keep-final-usage holding policy.
fn decode_body(entry: &StreamEventsEntry) -> String {
    let sentinel = match &entry.sentinel {
        Some(token) => format!(
            "                    // The declared {token} sentinel completes the stream before any payload decoding.\n                    if (data == {}) throw {}({}({}.Sentinel, {}))\n",
            quote(token),
            entry.terminal_type,
            entry.completion_type,
            entry.completion_reason_type,
            if entry.keep_final_usage { "held" } else { "null" },
        ),
        None => String::new(),
    };
    let branches = entry
        .kinds
        .iter()
        .zip(&entry.kind_types)
        .map(|(kind, class)| {
            format!(
                "            {} -> {}(Codecs.{}.decodeUsing(envelope, frame, \"\"){})",
                quote(kind),
                class,
                entry.item_codec,
                metadata_args(entry),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let delivery = if entry.keep_final_usage {
        // `held` is a captured local, so `send(held)` would infer a nullable
        // flow element; `?.let` sends the exact non-null event type.
        "                    held?.let { send(it) }\n                    held = typed\n"
    } else {
        "                    send(typed)\n"
    };
    format!(
        "                    val data = (envelope.values[\"data\"] as? JsonString)?.value ?: \"\"\n{sentinel}                    val kind = (envelope.values[\"event\"] as? JsonString)?.value?.takeIf {{ it.isNotEmpty() }} ?: \"message\"\n                    val typed: {event} = when (kind) {{\n{branches}\n                        else -> {unknown}(event = kind, data = data)\n                    }}\n{delivery}",
        event = entry.event_type,
        unknown = entry.unknown_type,
        sentinel = sentinel,
        branches = branches,
        delivery = delivery,
    )
}

/// The generated `StreamEvents.kt` type declarations for exactly the typed
/// stream operations.
pub(super) fn types_file(plan: &super::Plan, events: &StreamEventsPlan) -> String {
    let mut out = header(plan);
    for entry in &events.operations {
        out.push_str(&format!(
            "/** One typed event of the {} stream. Declared kinds decode the framed envelope through the operation's stream item codec into their declared model type; an event kind the source never declared surfaces through the typed {} alternative without failing the stream, and invalid payloads of recognized kinds remain decoding errors. The `kind` member is the frame's event name; the metadata members repeat the envelope fields the item schema declares. */\npublic sealed interface {} {{\n    /** The frame's event name; `unknown` on the typed unknown alternative and `completion` on the terminal element. */\n    public val kind: String\n}}\n\n",
            kdoc(&entry.operation),
            entry.unknown_type,
            entry.event_type,
        ));
        for (kind, class) in entry.kinds.iter().zip(&entry.kind_types) {
            out.push_str(&format!(
                "/** A declared {} event of the {} stream: the framed envelope decoded through the operation's stream item codec into its declared model type. An invalid payload for this kind remains a decoding error. */\npublic data class {}(\n    /** Decoded envelope payload. */\n    public val data: {},\n{}) : {} {{\n    /** The declared event kind. */\n    public override val kind: String get() = {}\n}}\n\n",
                kdoc(kind),
                kdoc(&entry.operation),
                class,
                entry.item_type,
                metadata_members(entry),
                entry.event_type,
                quote(kind),
            ));
        }
        out.push_str(&format!(
            "/** An event kind the source never declared: the raw frame data stays representable without failing the stream. */\npublic data class {}(\n    /** The undeclared wire kind. */\n    public val event: String,\n    /** The raw frame data, unchanged. */\n    public val data: String,\n) : {} {{\n    /** The typed unknown alternative. */\n    public override val kind: String get() = \"unknown\"\n}}\n\n",
            entry.unknown_type, entry.event_type,
        ));
        out.push_str(&format!(
            "/** Why one {} typed stream completed. */\npublic enum class {} {{\n    /** The declared {} terminal token completed the stream before any payload decoding. */\n    Sentinel,\n    /** The response body ended. */\n    Eof,\n}}\n\n",
            kdoc(&entry.operation),
            entry.completion_reason_type,
            entry.sentinel.as_deref().unwrap_or(""),
        ));
        out.push_str(&format!(
            "/** Terminal metadata of one completed {} typed stream: why it completed and the preserved final usage frame. This is the flow's final {} element's payload. */\npublic data class {}(\n    /** Why the stream completed. */\n    public val reason: {},\n    /** The last data frame before the sentinel or end of body, decoded like an ordinary event and preserved as terminal instead of being yielded; null when no frame preceded completion or the compiled policy preserves nothing. */\n    public val usage: {}?,\n)\n\n",
            kdoc(&entry.operation),
            entry.event_type,
            entry.completion_type,
            entry.completion_reason_type,
            entry.event_type,
        ));
        out.push_str(&format!(
            "/** The final flow element of one completed {} typed stream, carrying its terminal metadata. */\npublic data class {}(\n    /** Terminal metadata. */\n    public val completion: {},\n) : {} {{\n    /** The terminal completion element. */\n    public override val kind: String get() = \"completion\"\n}}\n\n",
            kdoc(&entry.operation),
            entry.completion_event_type,
            entry.completion_type,
            entry.event_type,
        ));
        out.push_str(&format!(
            "/** Internal terminal marker that stops the exchange at the declared sentinel; the typed-events flow translates it into the final completion element. */\ninternal class {}(\n    /** Terminal metadata. */\n    public val completion: {},\n) : RuntimeException(\"typed stream completed at its declared terminal sentinel\")\n\n",
            entry.terminal_type, entry.completion_type,
        ));
    }
    out
}

/// The KDoc for one operation's typed-events flow.
fn member_kdoc(entry: &StreamEventsEntry) -> String {
    let sentinel = match &entry.sentinel {
        Some(token) => format!(
            "The declared {} sentinel, when compiled, completes the stream before any payload decoding and issues no further reads.",
            kdoc(token)
        ),
        None => "A declared terminal sentinel, when the source declares one, completes the stream before any payload decoding and issues no further reads.".to_owned(),
    };
    format!(
        "Lazily yields the typed events of {operation} over the existing stream item iteration. Collection initiates the exchange exactly like the direct {method}(...) call and every collection re-fetches it; a declared event kind decodes the framed envelope through the operation's stream item codec into its declared model type, an event kind the source never declared surfaces through the typed {unknown} alternative without failing the stream, and an invalid payload for a recognized kind remains a decoding error. {sentinel} Per-item metadata (the event name as `kind`, plus id/retry when the item schema declares them) is explicit. Because flows cannot return values, the documented completion is this flow's final element: a {completion_event} whose completion carries the reason (`sentinel` when the declared terminal token completed the stream, `eof` when the response body ended) and the preserved final usage frame when the compiled policy keeps one. Collection is lazy and pull-driven: early cancellation ends the collection, cancels the response body and issues no further reads. The direct {method}(...) call and its untyped item flow are unchanged.\nSource: {source}",
        operation = kdoc(&entry.operation),
        method = entry.method,
        unknown = entry.unknown_type,
        completion_event = entry.completion_event_type,
        sentinel = sentinel,
        source = source(&entry.source),
    )
}

/// The typed-events flow member's signature: the KDoc, the cold-flow
/// declaration and the channelFlow opener. The rich emitter continues with
/// the direct call's shared request preparation and [`flow_tail`].
pub(super) fn flow_signature(entry: &StreamEventsEntry) -> String {
    let default = if entry.input_default {
        format!(" = {}()", entry.input_type)
    } else {
        String::new()
    };
    format!(
        "    /** {} */\n    public fun {}(input: {}{}, requestOptions: RequestOptions = RequestOptions()): Flow<{}> = channelFlow {{\n",
        kdoc(&member_kdoc(entry)),
        entry.events_method,
        entry.input_type,
        default,
        entry.event_type,
    )
}

/// The typed-events flow member's tail: the typed decode over the runtime
/// collector, the sentinel terminal translation and the final completion
/// element, closing the flow. The `held` and `terminal` slots and the
/// terminal translation exist on every typed flow so the final completion
/// element has one uniform shape; without the compiled policy the slot stays
/// null and the eof completion carries no usage.
pub(super) fn flow_tail(entry: &StreamEventsEntry) -> String {
    let mut out = format!(
        "            var held: {}? = null\n            var terminal: {}? = null\n            try {{\n",
        entry.event_type, entry.completion_type,
    );
    out.push_str("                collectProtocol(descriptor, transport, request, options, requestOptions, control) { response ->\n                    val frame = ModelBudget(options.codecLimits, control::check)\n                    if (response.forbidden || response.mediaIndex == null) {\n");
    out.push_str(&format!(
        "                        decode{}(response, frame)\n                        throw SdkException(FailureKind.UNEXPECTED_STATUS, \"the typed-events flow reads only the declared stream representation\", response.info)\n                    }}\n                    val envelope = (response.value as ProtocolValue.Json).value as? JsonObject ?: throw SdkException(FailureKind.RESPONSE_VALIDATION, \"stream item is not an SSE envelope\", response.info)\n",
        entry.index,
    ));
    out.push_str(&decode_body(entry));
    out.push_str("                }\n");
    out.push_str(&format!(
        "            }} catch (done: {}) {{\n                terminal = done.completion\n            }}\n            send({}(terminal ?: {}({}.Eof, held)))\n",
        entry.terminal_type,
        entry.completion_event_type,
        entry.completion_type,
        entry.completion_reason_type,
    ));
    out.push_str("        }\n    }.buffer(0)\n");
    out
}
