//! Generated typed SSE event decoding for the Java HTTP SDK.
//!
//! The shared, generation-time `http_protocol::plan_stream_semantics` outcome
//! compiles into one generated `StreamEvents.java` holding a closeable event
//! iterator per discriminated SSE stream operation, with a sealed hierarchy of
//! per-kind events, the typed unknown-event alternative and a terminal
//! completion. Static runtime files and the shared planner stay untouched, and
//! plans without an emittable typed stream operation emit no new file at all.
//!
//! The direct call's untyped `EventStream` stays the transport: the events
//! iterator starts the same exchange through the runtime (identical server,
//! security, parameter, body and deadline handling) with the stream item
//! framing, limits and cancellation unchanged, and only the item codec is
//! replaced by a lenient envelope reader bound to a validation root that
//! admits every framed envelope. The events iterator then applies the
//! compiled semantics itself: recognized event kinds decode through the
//! operation's existing stream item codec into their declared model type,
//! undeclared kinds surface through the typed unknown alternative without
//! failing the stream, invalid payloads of recognized kinds remain decoding
//! errors, and a declared sentinel completes the stream before any payload
//! decoding while preserving the final usage frame as the completion's
//! terminal metadata.
//!
//! Traversal is lazy and pull-based, mirroring `Pagination.java`: the request
//! starts only inside `hasNext()`, `next()` returns one already-decoded
//! event, `close()` stops consumption and cancels the response, and the
//! documented completion is read from `completion()` once the stream ends.

use std::collections::BTreeSet;

use super::{
    SdkPlan,
    http::JavaOperation,
    models::{JavaDeclaration, allocate, javadoc, q},
    protocol::JavaValue,
};
use crate::http_protocol::{
    EventPayload, StreamEventPlan, StreamFraming, StreamOperationPlan,
};
use suspect_schema::{ProgramInstruction, ProgramType};

/// One declared envelope metadata member of the decoded item model.
struct Metadata {
    /// The native accessor name.
    member: String,
    /// The rendered native type of the metadata value.
    ty: String,
    required: bool,
}

/// One operation's emission-ready typed event decode.
struct StreamOperation<'a> {
    op: &'a JavaOperation,
    /// The single success stream media descriptor, for the lenient exchange.
    descriptor: String,
    /// The response and media indexes guarding the events branch.
    response_index: usize,
    media_index: usize,
    /// Allocated iterator, event-set, per-kind, unknown and completion names.
    iterator: String,
    /// The allocated client exchange seam method name.
    seam: String,
    event: String,
    kind_types: Vec<String>,
    unknown: String,
    completion: String,
    /// Declared event kinds in declaration order.
    kinds: Vec<String>,
    /// The declared terminal sentinel token, matched on frame data before any
    /// payload decoding.
    sentinel: Option<String>,
    keep_final_usage: bool,
    /// The decoded model members carrying the declared metadata, when any.
    id: Option<Metadata>,
    retry: Option<Metadata>,
    /// The stream item codec holder (the `CODEC` constant's enclosing class).
    item_codec: String,
    /// The item model's native type: every declared kind's payload type.
    item_type: String,
}

/// Whether one compiled stream plan admits the typed event decode: SSE framing
/// whose discriminator is the parsed envelope's `event` field, and a
/// discriminated event set (at least two declared kinds, or one declared kind
/// compiled from discrimination evidence with a JSON payload).
fn admits_typed_events(stream: &StreamOperationPlan) -> bool {
    stream.framing == StreamFraming::ServerSentEvents
        && stream
            .item_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.event_field.as_deref() == Some("event"))
        && match stream.events.as_slice() {
            [] => false,
            [only] => {
                only.event_name != StreamEventPlan::DEFAULT_EVENT_NAME
                    && matches!(only.payload, EventPayload::Json { .. })
            }
            _ => true,
        }
}

/// Whether one compiled program node admits every framed SSE envelope (always
/// a JSON object): no checks at all, or only trivially true or object-typed
/// checks. The lenient envelope reader binds to such a root so undeclared
/// event kinds stay representable without a static runtime change.
fn admits_envelopes(program: &suspect_schema::OwnedProgram, target: usize) -> bool {
    let Some(node) = program.nodes.get(target) else {
        return false;
    };
    node.checks.iter().all(|check| match &check.instruction {
        ProgramInstruction::Always { value: true } => true,
        ProgramInstruction::Type { types } => types.contains(&ProgramType::Object),
        _ => false,
    })
}

/// The validation root the lenient envelope reader binds to. `None` keeps
/// every operation on its untyped path.
fn envelope_root(plan: &SdkPlan) -> Option<usize> {
    let program = plan.program();
    program
        .roots
        .iter()
        .map(|root| root.target)
        .find(|target| admits_envelopes(program, *target))
}

/// The operation's single success stream media, with its response and media
/// indexes. Mixed or multiple success stream media keep the conservative
/// untyped path, because the typed decode would otherwise own responses it
/// cannot name.
fn stream_media(op: &JavaOperation) -> Option<(usize, usize)> {
    let mut found: Option<(usize, usize)> = None;
    for (index, response) in op.responses.iter().enumerate() {
        if !response.can_succeed() {
            continue;
        }
        for (media, item) in response.media.iter().enumerate() {
            if !matches!(item.value, JavaValue::Stream { request: false, .. }) {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some((index, media));
        }
    }
    found
}

/// One declared envelope metadata member of the decoded item model, when the
/// item schema declares the field and the native model can name it.
fn metadata_of(
    plan: &SdkPlan,
    stream: &StreamOperationPlan,
    wire: Option<&str>,
) -> Option<Metadata> {
    let codec = stream.item_codec.as_ref()?;
    let symbol = plan.models().symbol(codec.schema().id())?;
    let JavaDeclaration::Object { fields, .. } = symbol.declaration() else {
        return None;
    };
    let field = fields
        .iter()
        .find(|field| Some(field.wire.as_str()) == wire)?;
    Some(Metadata {
        member: field.name.clone(),
        ty: plan.models().render_type(&field.ty),
        required: field.required,
    })
}

/// Compile every emittable typed stream operation of the plan, in plan order.
/// The compiled plans are only lowered onto native descriptors here; an
/// operation whose native representation cannot express the walk is dropped.
fn operations<'a>(plan: &'a SdkPlan) -> Vec<StreamOperation<'a>> {
    let streams = plan.stream_semantics();
    if streams.streams.is_empty() {
        return Vec::new();
    }
    let mut types: BTreeSet<String> = BTreeSet::new();
    let mut methods = client_members(plan);
    let mut compiled = Vec::new();
    for op in plan.operations() {
        let identity = op
            .wire
            .operation_id()
            .map(|located| located.value().clone())
            .unwrap_or_else(|| format!("{} {}", op.wire.method().as_str(), op.wire.path()));
        let Some(stream) = streams
            .streams
            .iter()
            .find(|stream| stream.operation == identity)
        else {
            continue;
        };
        if !admits_typed_events(stream) {
            continue;
        }
        let Some((response_index, media_index)) = stream_media(op) else {
            continue;
        };
        let Some(item_codec) = &stream.item_codec else {
            continue;
        };
        let item_schema = item_codec.schema().id();
        if plan.models().symbol(item_schema).is_none() {
            continue;
        }
        let response = &op.responses[response_index];
        let media = &response.media[media_index];
        let stem = crate::rust_models::pascal(&op.method_name);
        let metadata = |wire: Option<&str>| metadata_of(plan, stream, wire);
        let iterator = allocate(&format!("{stem}Events"), &mut types);
        let event = allocate(&format!("{stem}Event"), &mut types);
        let kind_types = stream
            .events
            .iter()
            .map(|event| {
                allocate(
                    &format!("{stem}{}Event", crate::rust_models::pascal(&event.event_name)),
                    &mut types,
                )
            })
            .collect();
        let unknown = allocate(&format!("{stem}UnknownEvent"), &mut types);
        let completion = allocate(&format!("{stem}Completion"), &mut types);
        let mut seam = format!("{iterator}Stream");
        seam[..1].make_ascii_lowercase();
        let seam = allocate(&seam, &mut methods);
        compiled.push(StreamOperation {
            op,
            descriptor: media.descriptor.clone(),
            response_index,
            media_index,
            iterator,
            seam,
            event,
            kind_types,
            unknown,
            completion,
            kinds: stream
                .events
                .iter()
                .map(|event| event.event_name.clone())
                .collect(),
            sentinel: stream.sentinel.enabled.then(|| stream.sentinel.token.clone()),
            keep_final_usage: stream.terminal.keep_final_usage,
            id: metadata(stream.item_metadata.as_ref().and_then(|m| m.id_field.as_deref())),
            retry: metadata(
                stream
                    .item_metadata
                    .as_ref()
                    .and_then(|m| m.retry_field.as_deref()),
            ),
            item_codec: plan.models().codec(item_schema).holder.clone(),
            item_type: plan.models().native_type(item_schema),
        });
    }
    compiled
}
/// The client member names the exchange seams must not collide with.
fn client_members(plan: &SdkPlan) -> BTreeSet<String> {
    let mut methods: BTreeSet<String> = [
        "close",
        "wait",
        "notify",
        "notifyAll",
        "getClass",
        "hashCode",
        "equals",
        "toString",
        "options",
        "operationMetadata",
        "fromEnv",
        "_credentialEnvOptions",
        "_readCredentialEnv",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for op in plan.operations() {
        methods.insert(op.method_name.clone());
        methods.insert(op.async_method_name.clone());
    }
    methods
}

/// The client exchange seams, appended inside the generated client class. Each
/// seam performs the direct method's exact request preparation through the
/// runtime and hands back the lenient envelope stream, so the events iterator
/// shares every server, security, parameter and deadline decision. Empty when
/// nothing emits.
pub(crate) fn client_seams(plan: &SdkPlan) -> String {
    let Some(_) = envelope_root(plan) else {
        return String::new();
    };
    let compiled = operations(plan);
    let mut out = String::new();
    for stream in &compiled {
        let label = format!("{} {}", stream.op.http_method, stream.op.path);
        out.push_str(&format!(
            "    /** Starts the raw typed-events exchange for {label}; StreamEvents consumes the lenient envelopes. */\n    EventStream<JsonValue> {seam}(Client.{input} input,RequestOptions options) {{\n        return HttpRuntime.await(runtime.call(OP{index},options,c->{{\n{prepare}            return new HttpRuntime.Prepared(parameters,body);\n        }},raw->{{\n            if(raw.responseIndex!={response_index}||raw.mediaIndex!={media_index})throw raw.failure(\"unexpected-response\");\n            return raw.stream(Protocol.object({descriptor}),StreamEvents.RAW_ENVELOPE);\n        }}),OP{index}.source());\n    }}\n",
            seam = stream.seam,
            input = stream.op.input_type,
            index = plan
                .operations()
                .iter()
                .position(|candidate| std::ptr::eq(candidate, stream.op))
                .unwrap_or_default(),
            prepare = super::http_emit::prepare_fragment(plan, stream.op),
            response_index = stream.response_index,
            media_index = stream.media_index,
            descriptor = q(&stream.descriptor),
        ));
    }
    out
}

/// The completion prose shared by the iterator documentation.
fn stop_rule(stream: &StreamOperation<'_>) -> String {
    match &stream.sentinel {
        Some(token) => format!(
            "The declared {token} sentinel completes the stream before any payload decoding, and the end of the response body completes it otherwise."
        ),
        None => "The end of the response body completes the stream.".into(),
    }
}

/// The decoded-model metadata constructor values for one recognized kind.
fn metadata_values(stream: &StreamOperation<'_>) -> String {
    let mut out = String::new();
    for member in [&stream.id, &stream.retry].into_iter().flatten() {
        if member.required {
            out.push_str(&format!(",item.{}()", member.member));
        } else {
            out.push_str(&format!(
                ",item.{}().isPresent()?item.{}().value():null",
                member.member, member.member
            ));
        }
    }
    out
}

/// The per-kind record components for the declared metadata, when any.
fn metadata_components(stream: &StreamOperation<'_>) -> String {
    let mut out = String::new();
    if let Some(member) = &stream.id {
        out.push_str(&format!(",{} id", member.ty));
    }
    if let Some(member) = &stream.retry {
        out.push_str(&format!(",{} retry", member.ty));
    }
    out
}

/// The per-kind event records, in declaration order.
fn kind_records(stream: &StreamOperation<'_>) -> String {
    let mut out = String::new();
    for (index, kind) in stream.kinds.iter().enumerate() {
        out.push_str(&format!(
            "        /** A declared {kind} event: the framed envelope decoded to its declared model type. An invalid payload for this kind remains a decoding error. */\n        public record {class}(String kind,{item_type} data{components}) implements {event} {{}}\n",
            class = stream.kind_types[index],
            item_type = stream.item_type,
            components = metadata_components(stream),
            event = stream.event,
        ));
    }
    out
}

/// The public factory method for one compiled walk.
fn factory(stream: &StreamOperation<'_>) -> String {
    let mut name = stream.iterator.clone();
    name[..1].make_ascii_lowercase();
    format!(
        "    /** Start a lazy typed-event walk over {{@code input}}; the request starts inside {{@link {iterator}#hasNext()}}, {{@code close()}} stops consumption, and {{@link {iterator}#completion()}} carries the terminal metadata after the stream ends. {stop} */\n    public static {iterator} {name}(Client client,Client.{input} input) {{\n        return new {iterator}(client,input);\n    }}\n",
        iterator = stream.iterator,
        name = name,
        input = stream.op.input_type,
        stop = stop_rule(stream),
    )
}

/// The closeable typed-event iterator class for one compiled walk.
#[allow(clippy::too_many_lines)]
fn iterator_class(stream: &StreamOperation<'_>) -> String {
    let sentinel = match &stream.sentinel {
        Some(token) => format!(
            "                // The declared {token} sentinel completes the stream before any payload decoding.\n                if (data.equals({token})) {{\n                    completion = new {completion}(\"sentinel\",held);\n                    envelopes.close();\n                    done = true;\n                    break;\n                }}\n",
            token = q(token),
            completion = stream.completion,
        ),
        None => String::new(),
    };
    let mut kinds = String::new();
    for (index, kind) in stream.kinds.iter().enumerate() {
        let keyword = if index == 0 { "if" } else { "else if" };
        kinds.push_str(&format!(
            "                {keyword} (kind.equals({kind})) {{\n                    try {{\n                        var item = {codec}.CODEC.decodeValue(envelope);\n                        typed = new {class}(kind,item{values});\n                    }}\n                    catch (CodecException error) {{ throw failed(SOURCE,error); }}\n                }}\n",
            kind = q(kind),
            class = stream.kind_types[index],
            codec = stream.item_codec,
            values = metadata_values(stream),
        ));
    }
    let eof_usage = if stream.keep_final_usage { "held" } else { "null" };
    let hold = if stream.keep_final_usage {
        "                if (held != null) {\n                    pending = held;\n                    held = typed;\n                    break;\n                }\n                held = typed;\n"
    } else {
        "                pending = typed;\n                break;\n"
    };
    let mut out = String::new();
    out.push_str(&format!(
        "    /**\n     * {prose}\n     */\n    public static final class {iterator} implements java.util.Iterator<{event}>, AutoCloseable {{\n        private static final String SOURCE = {source};\n        private final Client client;\n        private final Client.{input} input;\n        private EventStream<JsonValue> envelopes;\n        private java.util.Iterator<JsonValue> frames;\n        private {event} pending, held;\n        private {completion} completion;\n        private boolean done, closed;\n        private {iterator}(Client client,Client.{input} input) {{ this.client=client; this.input=input; }}\n        /** Pulls, decodes and buffers the next event; the request starts here on the first call and every read shares the direct call's framing, limits and cancellation. */\n        @Override public boolean hasNext() {{\n            if (closed || done) return false;\n            if (pending != null) return true;\n            if (envelopes == null) {{\n                envelopes = client.{seam}(input,RequestOptions.defaults());\n                frames = envelopes.iterator();\n            }}\n            while (pending == null) {{\n                if (!frames.hasNext()) {{\n                    completion = new {completion}(\"eof\",{eof_usage});\n                    done = true;\n                    break;\n                }}\n                JsonValue envelope = frames.next();\n                String data = envelope instanceof JsonObject object && object.values().get(\"data\") instanceof JsonString text ? text.value() : \"\";\n{sentinel}                String kind = envelope instanceof JsonObject object && object.values().get(\"event\") instanceof JsonString text && !text.value().isEmpty() ? text.value() : \"message\";\n                {event} typed;\n{kinds}                else {{\n                    typed = new {unknown}(kind,data);\n                }}\n{hold}            }}\n            return pending != null;\n        }}\n        /** The next decoded event; exactly what the source frame declared. */\n        @Override public {event} next() {{\n            if (!hasNext()) throw new java.util.NoSuchElementException(\"the typed event stream has ended\");\n            {event} event = pending;\n            pending = null;\n            return event;\n        }}\n        /** Stop the walk; no further frame is read and the response is cancelled. The completion stays unset when the stream had not ended. */\n        @Override public void close() {{\n            closed = true;\n            if (envelopes != null) envelopes.close();\n        }}\n        /** The documented terminal metadata once the stream completed, or null before that. @return completion */\n        public {completion} completion() {{\n            return completion;\n        }}\n    }}\n",
        prose = prose(stream),
        iterator = stream.iterator,
        event = stream.event,
        input = stream.op.input_type,
        completion = stream.completion,
        seam = stream.seam,
        eof_usage = eof_usage,
        sentinel = sentinel,
        kinds = kinds,
        hold = hold,
        unknown = stream.unknown,
        source = q(&format!(
            "{}#{}",
            stream.op.source.document(),
            stream.op.source.pointer()
        )),
    ));
    out
}

/// The sealed event hierarchy for one compiled walk: the event interface, the
/// per-kind records with their declared metadata, the typed unknown
/// alternative and the terminal completion record.
fn event_types(stream: &StreamOperation<'_>) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "    /** One decoded typed event of the {operation} stream. */\n    public sealed interface {event} {{\n        /** The declared event name, or {{@code unknown}} for a kind the source never declared. @return kind */\n        String kind();\n    }}\n",
        operation = stream.op.operation_id,
        event = stream.event,
    ));
    out.push_str(&kind_records(stream));
    out.push_str(&format!(
        "    /** An event kind the source never declared: the raw frame data stays representable without failing the stream. */\n    public record {unknown}(String event,String data) implements {event} {{\n        /** The constant unknown kind. @return kind */\n        @Override public String kind() {{ return \"unknown\"; }}\n    }}\n    /** Terminal metadata of one completed stream: why it completed and the preserved final usage frame. */\n    public record {completion}(String reason,{event} usage) {{}}\n",
        unknown = stream.unknown,
        completion = stream.completion,
        event = stream.event,
    ));
    out
}

/// Javadoc-escaped prose for one compiled walk.
fn prose(stream: &StreamOperation<'_>) -> String {
    javadoc(&format!(
        "Lazily walks the typed events of {}, one request at a time.\n\nThe request starts only inside hasNext() and every frame is decoded through the operation's existing stream item codec: a declared kind decodes to its declared model type, an undeclared kind surfaces through the typed {} alternative without failing the stream, and an invalid payload for a recognized kind fails the walk with a decoding error. {} The completion metadata is documented on {}#completion(). Calling close() or abandoning iteration guarantees no further read.\n\nSource: {}#{}",
        stream.op.method_name,
        stream.unknown,
        stop_rule(stream),
        stream.iterator,
        stream.op.source.document(),
        stream.op.source.pointer()
    ))
}

/// The complete `StreamEvents.java` source body, or `None` when nothing emits.
pub(crate) fn source(plan: &SdkPlan) -> Option<String> {
    let root = envelope_root(plan)?;
    let compiled = operations(plan);
    if compiled.is_empty() {
        return None;
    }
    let mut out = String::from(
        "/**\n * Generated typed SSE event decoding for the source-selected discriminated\n * stream operations of this package.\n *\n * <p>Every iterator keeps one request in flight over the shared runtime: the\n * request starts only inside {@code hasNext()} when a not-yet-decoded event\n * is required, the framed envelopes arrive through the direct call's stream\n * item framing, limits and cancellation with a lenient envelope reader,\n * recognized event kinds decode through the operation's existing stream item\n * codec, undeclared kinds surface through the typed unknown alternative\n * without failing the stream, and invalid payloads of recognized kinds remain\n * decoding errors. Calling {@code close()} or abandoning iteration stops\n * consumption and cancels the response; {@code completion()} exposes the\n * documented terminal metadata once the stream ends and is null before that.\n */\npublic final class StreamEvents {\n    private StreamEvents() {}\n    /** Lenient envelope reader: the framed envelope passes through unvalidated so undeclared event kinds stay representable; recognized kinds decode through the operation's stream item codec below. */\n    static final ModelCodec<JsonValue> RAW_ENVELOPE = new ModelCodec<>(",
    );
    out.push_str(&root.to_string());
    out.push_str(
        ",(value,context)->value,(value,context)->value);\n\n    /** A decoding failure of one recognized kind, branded like the untyped stream path. */\n    private static SdkException failed(String source,CodecException error) {\n        String kind = switch (error.kind()) {\n            case \"resource\", \"evaluation_failure\" -> \"resource-limit\";\n            case \"cancelled\" -> \"cancelled\";\n            default -> \"invalid-stream-item\";\n        };\n        return new SdkException(kind,source,0,new byte[0],false,error.source(),error.instancePath());\n    }\n\n",
    );
    for stream in &compiled {
        out.push_str(&factory(stream));
    }
    for stream in &compiled {
        out.push_str(&event_types(stream));
    }
    for stream in &compiled {
        out.push_str(&iterator_class(stream));
    }
    out.push_str("}\n");
    Some(out)
}
