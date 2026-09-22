//! Emitted-only typed event decode for the native Dart HTTP adapter.
//!
//! The compiled `http_protocol::StreamSemanticsPlan` becomes per-operation
//! `<op>Events` streams plus `<op>EventsCompletion` accessors on the generated
//! `Client`, and a generated `lib/src/stream_events.dart` part holding the
//! typed event, unknown and completion classes and the per-frame decoder —
//! exactly for the operations whose stream plan carries a discriminated SSE
//! event set. The existing untyped stream iteration stays the transport: the
//! events streams reuse the operation's own request preparation and the
//! runtime's framing, ceilings, declared-error handling, pause and cancellation
//! with the framed envelope passed through unvalidated, and then apply the
//! compiled per-kind decode, sentinel and completion semantics themselves.
//! Recognized event kinds decode through the operation's existing stream item
//! codec into their declared model type; undeclared event kinds surface through
//! the typed unknown-event alternative without failing the stream; invalid
//! payloads of recognized kinds remain decoding errors. Static runtime files
//! are never modified, and operations without a discriminated stream schema
//! emit no new bytes at all.

use super::{
    Plan, PlannedOperation, PlannedPayload as P,
    models::{Shape, allocate, exported},
};
use crate::http_protocol as p;
use std::collections::BTreeSet;

/// One envelope metadata member the item schema declares, with its decoded
/// model member and the metadata property's rendered type.
#[derive(Debug, Clone)]
pub(super) struct Metadata {
    /// The item model's allocated member name.
    member: String,
    /// The model member's exact value type.
    value_ty: String,
    /// The metadata property type, nullable when the item schema leaves the
    /// member optional.
    ty: String,
    /// Whether the item codec requires the member on every envelope.
    required: bool,
}

impl Metadata {
    /// The generated expression reading one decoded item's metadata value.
    fn read(&self, name: &str) -> String {
        if self.required {
            format!("decoded.{}", self.member)
        } else {
            format!(
                "{name} is Present<{}> ? {name}.value : null",
                self.value_ty
            )
        }
    }
}

/// One operation's emission-ready typed event decode.
#[derive(Debug, Clone)]
pub(super) struct Events {
    /// Source operation identity for documentation.
    pub operation: String,
    /// Source location for documentation.
    pub location: String,
    /// The direct client method this extends.
    pub method: String,
    /// The operation's index into the generated registry.
    pub operation_index: usize,
    /// The stream item model type: the declared model every kind decodes into.
    pub item_type: String,
    /// The stream item codec symbol for the strict per-kind decode.
    pub item_codec: String,
    /// Declared event kinds in declaration order with their allocated class
    /// names.
    pub kinds: Vec<(String, String)>,
    /// The declared terminal sentinel token, matched on frame data before any
    /// payload decoding.
    pub sentinel: Option<String>,
    pub keep_final_usage: bool,
    pub id_metadata: Option<Metadata>,
    pub retry_metadata: Option<Metadata>,
    /// Allocated, collision-free public type names.
    pub events_type: String,
    pub unknown_type: String,
    pub completion_type: String,
    /// Allocated Client member names and the part's frame decoder name.
    pub events_method: String,
    pub completion_method: String,
    pub core_method: String,
    pub frame_name: String,
}

impl Events {
    /// The framed element type: only a compiled sentinel makes a frame carry
    /// no event.
    fn frame_type(&self) -> String {
        if self.sentinel.is_some() {
            format!("{}?", self.events_type)
        } else {
            self.events_type.clone()
        }
    }
}

/// Whether the compiled event set is discriminated: at least two declared
/// kinds, or one declared kind compiled from discrimination evidence whose
/// payload is a declared JSON structure. The single un-discriminated default
/// event keeps the existing untyped path byte-for-byte.
fn discriminated(stream: &p::StreamOperationPlan) -> bool {
    match stream.events.as_slice() {
        [] => false,
        [only] => {
            only.event_name != p::StreamEventPlan::DEFAULT_EVENT_NAME
                && matches!(only.payload, p::EventPayload::Json { .. })
        }
        _ => true,
    }
}

/// Whether one compiled stream plan admits the typed event decode: SSE framing
/// whose discriminator is the parsed envelope's `event` field, and a
/// discriminated event set. Everything else keeps its existing untyped path.
fn admits_typed_events(stream: &p::StreamOperationPlan) -> bool {
    stream.framing == p::StreamFraming::ServerSentEvents
        && stream
            .item_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.event_field.as_deref() == Some("event"))
        && discriminated(stream)
}

/// The operation's single success status owning exactly one stream media of
/// the compiled media type. Mixed, multiple or default-status success dispatch
/// keep the conservative untyped path, because the typed decode would otherwise
/// own responses it cannot name.
fn stream_status<'a>(
    op: &'a PlannedOperation,
    compiled: &p::StreamOperationPlan,
) -> Option<&'a super::PlannedStatus> {
    let mut successes = op
        .statuses
        .iter()
        .filter(|status| status.success_name.is_some());
    let status = successes.next()?;
    if successes.next().is_some() {
        return None;
    }
    if matches!(status.wire.status(), p::ResponseStatus::Default) {
        return None;
    }
    if status.media.len() != 1 {
        return None;
    }
    let P::Stream { .. } = &status.media[0].payload else {
        return None;
    };
    if status.media[0].wire.media_type().declared() != compiled.media_type {
        return None;
    }
    compiled.item_codec.as_ref()?;
    Some(status)
}

/// Unwrap nullable wrappers around one metadata type.
fn nullable_type(ty: String) -> String {
    if ty.ends_with('?') {
        ty
    } else {
        format!("{ty}?")
    }
}

/// The declared model member one envelope metadata field decodes into.
fn metadata(
    plan: &Plan,
    schema: &suspect_ir::contract::SchemaId,
    wire: Option<&str>,
) -> Option<Metadata> {
    let model = plan.models().model(schema)?;
    let Shape::Object { fields, .. } = &model.shape else {
        return None;
    };
    let field = fields
        .iter()
        .find(|field| Some(field.wire_name.as_str()) == wire)?;
    let value_ty = plan.models.optional_ty(field.target);
    Some(Metadata {
        member: field.name.clone(),
        value_ty: value_ty.clone(),
        ty: if field.required {
            value_ty
        } else {
            nullable_type(value_ty)
        },
        required: field.required,
    })
}

/// Escape one span of Dart doc comment text.
fn prose(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
}

/// Compile the emittable subset of the compiled stream plan, allocating every
/// generated name from the client's method and model spaces. A stream plan
/// with no emittable operation emits nothing.
fn compiled(plan: &Plan) -> Vec<(usize, Events)> {
    let mut methods: BTreeSet<String> = plan
        .operations()
        .iter()
        .map(|op| op.method_name.clone())
        .collect();
    methods.extend(
        [
            "close",
            "runtimeType",
            "hashCode",
            "toString",
            "noSuchMethod",
        ]
        .map(str::to_owned),
    );
    let mut names = plan.models().used_names();
    let mut result = Vec::new();
    for (operation_index, op) in plan.operations().iter().enumerate() {
        let Some(stream) = plan
            .stream_semantics()
            .streams
            .iter()
            .find(|stream| stream.operation == op.operation_id)
        else {
            continue;
        };
        if !admits_typed_events(stream) {
            continue;
        }
        if stream_status(op, stream).is_none() {
            continue;
        }
        let Some(item_codec) = &stream.item_codec else {
            continue;
        };
        let item_schema = item_codec.schema().id();
        let Some(item_model) = plan.models().model(item_schema) else {
            continue;
        };
        let stem = exported(&op.method_name);
        let events_type = allocate(&format!("{stem}Event"), &mut names);
        let unknown_type = allocate(&format!("{stem}UnknownEvent"), &mut names);
        let completion_type = allocate(&format!("{stem}Completion"), &mut names);
        let events_method = allocate(&format!("{}Events", op.method_name), &mut methods);
        let completion_method = allocate(
            &format!("{}EventsCompletion", op.method_name),
            &mut methods,
        );
        let core_method = allocate(&format!("_{}EventsCore", op.method_name), &mut methods);
        let frame_name = allocate(&format!("_{}Frame", op.method_name), &mut names);
        result.push((
            operation_index,
            Events {
                operation: op.operation_id.clone(),
                location: format!("{}#{}", op.source.document(), op.source.pointer()),
                method: op.method_name.clone(),
                operation_index,
                item_type: plan.models().native_type(item_schema).expect("item root"),
                item_codec: item_model.codec_name.clone(),
                kinds: stream
                    .events
                    .iter()
                    .map(|event| {
                        let class = allocate(
                            &format!("{stem}{}Event", exported(&event.event_name)),
                            &mut names,
                        );
                        (event.event_name.clone(), class)
                    })
                    .collect(),
                sentinel: stream
                    .sentinel
                    .enabled
                    .then(|| stream.sentinel.token.clone()),
                keep_final_usage: stream.terminal.keep_final_usage,
                id_metadata: metadata(
                    plan,
                    item_schema,
                    stream
                        .item_metadata
                        .as_ref()
                        .and_then(|metadata| metadata.id_field.as_deref()),
                ),
                retry_metadata: metadata(
                    plan,
                    item_schema,
                    stream
                        .item_metadata
                        .as_ref()
                        .and_then(|metadata| metadata.retry_field.as_deref()),
                ),
                events_type,
                unknown_type,
                completion_type,
                events_method,
                completion_method,
                core_method,
                frame_name,
            },
        ));
    }
    result
}

/// The compiled typed-stream emission: the generated `lib/src/stream_events.dart`
/// part and the per-operation Client members, or nothing when no operation
/// lowers into a typed events emission.
pub(super) struct Emission {
    /// The generated part content, without the `part of` header.
    pub(super) part: String,
    /// The per-operation methods appended inside the generated `Client`.
    pub(super) client: String,
}

pub(super) fn emission(plan: &Plan) -> Option<Emission> {
    let entries = compiled(plan);
    if entries.is_empty() {
        return None;
    }
    let mut part = String::new();
    let mut client = String::new();
    for (operation_index, entry) in &entries {
        let op = &plan.operations()[*operation_index];
        part.push_str(&event_types(entry));
        part.push_str(&completion_type(entry));
        part.push_str(&frame_decoder(entry));
        client.push_str(&client_members(op, entry));
    }
    Some(Emission { part, client })
}

/// The metadata fields of one per-kind event class, as named optional
/// constructor parameters.
fn metadata_constructor(entry: &Events) -> String {
    let mut out = String::new();
    if entry.id_metadata.is_some() {
        out.push_str(", {this.id");
        if entry.retry_metadata.is_some() {
            out.push_str(", this.retry");
        }
        out.push('}');
    } else if entry.retry_metadata.is_some() {
        out.push_str(", {this.retry}");
    }
    out
}

/// The metadata members of one per-kind event class.
fn metadata_fields(entry: &Events) -> String {
    let mut out = String::new();
    if let Some(id) = &entry.id_metadata {
        out.push_str(&format!(
            "\n\n  /// The last-event id when the item schema declares it; null when the frame carried none.\n  final {} id;",
            id.ty
        ));
    }
    if let Some(retry) = &entry.retry_metadata {
        out.push_str(&format!(
            "\n\n  /// The reconnection hint in milliseconds when the item schema declares it; null when the frame carried none.\n  final {} retry;",
            retry.ty
        ));
    }
    out
}

/// The metadata arguments of one recognized kind's construction.
fn metadata_arguments(entry: &Events) -> String {
    let mut out = String::new();
    if let Some(id) = &entry.id_metadata {
        out.push_str(&format!(", id: {}", id.read("id")));
    }
    if let Some(retry) = &entry.retry_metadata {
        out.push_str(&format!(", retry: {}", retry.read("retry")));
    }
    out
}

/// The metadata locals of one recognized kind's decode.
fn metadata_locals(entry: &Events) -> String {
    let mut out = String::new();
    if let Some(id) = &entry.id_metadata
        && !id.required
    {
        out.push_str(&format!("      final id = decoded.{};\n", id.member));
    }
    if let Some(retry) = &entry.retry_metadata
        && !retry.required
    {
        out.push_str(&format!("      final retry = decoded.{};\n", retry.member));
    }
    out
}

/// The per-operation typed event and unknown classes.
fn event_types(entry: &Events) -> String {
    let mut out = format!(
        "/// One typed event of the {operation} stream: declared kinds decode the framed envelope through the operation's stream item codec into their declared model type, and an event kind the source never declared surfaces through the typed {unknown} alternative without failing the stream. Source: {location}.\nsealed class {events} {{\n  const {events}._();\n\n  /// The declared event name of this frame; `unknown` marks a kind the source never declared.\n  String get kind;\n}}\n\n",
        operation = prose(&entry.operation),
        location = prose(&entry.location),
        unknown = entry.unknown_type,
        events = entry.events_type,
    );
    for (kind, class) in &entry.kinds {
        out.push_str(&format!(
            "/// A declared `{kind}` event of the {operation} stream: the framed envelope decoded to its declared model type. An invalid payload for this kind remains a decoding error.\nfinal class {class} extends {events} {{\n  const {class}(this.data{constructor}) : super._();\n\n  @override\n  String get kind => {kind_literal};\n\n  /// The decoded {model} envelope.\n  final {model} data;{fields}\n}}\n\n",
            kind = prose(kind),
            kind_literal = super::emit::quote(kind),
            operation = prose(&entry.operation),
            class = class,
            events = entry.events_type,
            model = entry.item_type,
            constructor = metadata_constructor(entry),
            fields = metadata_fields(entry),
        ));
    }
    out.push_str(&format!(
        "/// An event kind the source never declared: the raw frame data stays representable without failing the stream.\nfinal class {unknown} extends {events} {{\n  const {unknown}(this.event, this.data) : super._();\n\n  @override\n  String get kind => 'unknown';\n\n  /// The undeclared event name as received.\n  final String event;\n\n  /// The raw frame data text.\n  final String data;\n}}\n\n",
        unknown = entry.unknown_type,
        events = entry.events_type,
    ));
    out
}

/// The per-operation completion carrier.
fn completion_type(entry: &Events) -> String {
    format!(
        "/// Terminal metadata of one completed {operation} stream: why it completed and the preserved final usage frame. `reason` is `'sentinel'` when the declared terminal token completed the stream before any payload decoding and `'eof'` when the response body ended. The typed events stream discards it; read it through {method}. Source: {location}.\nfinal class {completion} {{\n  const {completion}(this.reason, this.usage);\n\n  final String reason;\n\n  /// The last data frame before the sentinel or end of body, decoded like an ordinary event and preserved as terminal instead of being yielded; null when no frame preceded completion or the compiled policy preserves nothing.\n  final {events}? usage;\n}}\n\n",
        operation = prose(&entry.operation),
        location = prose(&entry.location),
        completion = entry.completion_type,
        method = entry.completion_method,
        events = entry.events_type,
    )
}

/// The per-frame decoder of one operation: the sentinel is matched on the
/// frame data before any payload decoding, recognized kinds decode through the
/// operation's stream item codec, and undeclared kinds stay representable.
fn frame_decoder(entry: &Events) -> String {
    let frame_type = entry.frame_type();
    let sentinel = match &entry.sentinel {
        Some(token) => format!(
            "  // The declared {token} sentinel completes the stream before any payload decoding.\n  if (data == {literal}) {{ return null; }}\n",
            token = prose(token),
            literal = super::emit::quote(token)
        ),
        None => String::new(),
    };
    let cases = entry
        .kinds
        .iter()
        .map(|(kind, class)| {
            format!(
                "    case {literal}:\n      final decoded = _decoded(frame.received, () => {codec}.fromJson(item));\n{locals}      return {class}(decoded{metadata});\n",
                literal = super::emit::quote(kind),
                codec = entry.item_codec,
                locals = metadata_locals(entry),
                class = class,
                metadata = metadata_arguments(entry),
            )
        })
        .collect::<Vec<_>>()
        .join("");
    format!(
        "/// The typed SSE frame decoder of the {operation} stream: the declared sentinel completes before any payload decoding, recognized kinds decode through the operation's stream item codec, and undeclared kinds surface through the typed {unknown} alternative. Static runtime framing, limits and declared-error handling are unchanged.\n{frame_type} {name}(_StreamRecord frame) {{\n  final item = frame.item;\n  if (item == null) {{\n    _decodeOperation{index}(frame.received);\n    throw UnexpectedResponseException(frame.received.raw);\n  }}\n  final envelope = item is JsonObject ? item.values : const <String, JsonValue>{{}};\n  final payload = envelope['data'];\n  final data = payload is JsonString ? payload.value : '';\n{sentinel}  final declared = envelope['event'];\n  final kind = declared is JsonString && declared.value.isNotEmpty ? declared.value : 'message';\n  switch (kind) {{\n{cases}    default:\n      return {unknown}(kind, data);\n  }}\n}}\n\n",
        operation = prose(&entry.operation),
        unknown = entry.unknown_type,
        frame_type = frame_type,
        name = entry.frame_name,
        index = entry.operation_index,
        sentinel = sentinel,
        cases = cases,
    )
}

/// The direct method's named-parameter list and its argument list.
fn call_shape(op: &PlannedOperation) -> (String, String) {
    let mut parameters = Vec::new();
    let mut arguments = Vec::new();
    for parameter in &op.parameters {
        parameters.push(if parameter.required {
            format!("required {} {}", parameter.native_type, parameter.name)
        } else {
            format!(
                "Presence<{}> {} = const Absent()",
                parameter.native_type, parameter.name
            )
        });
        arguments.push(format!("{}: {}", parameter.name, parameter.name));
    }
    if let Some(body) = &op.body {
        parameters.push(if body.required {
            format!("required {} body", body.native_type)
        } else {
            format!("Presence<{}> body = const Absent()", body.native_type)
        });
        arguments.push("body: body".into());
    }
    parameters.push(
        "CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative"
            .to_owned(),
    );
    for argument in [
        "cancellation: cancellation",
        "timeout: timeout",
        "server: server",
        "securityAlternative: securityAlternative",
    ] {
        arguments.push(argument.to_owned());
    }
    (parameters.join(", "), arguments.join(", "))
}

/// The per-operation Client members: the typed events stream, the documented
/// completion accessor and the shared core iteration.
fn client_members(op: &PlannedOperation, entry: &Events) -> String {
    let (parameters, arguments) = call_shape(op);
    let mut summary = format!(
        "Lazily yields the typed events of {operation} over the existing stream item iteration. A declared event kind decodes the framed envelope through the operation's stream item codec into its declared model type; an event kind the source never declared surfaces through the typed {unknown} alternative without failing the stream; an invalid payload for a recognized kind remains a decoding error. Per-item metadata (the event name as `kind`, plus id/retry when the item schema declares them) is explicit.",
        operation = prose(&entry.operation),
        unknown = entry.unknown_type,
    );
    if let Some(token) = &entry.sentinel {
        summary.push_str(&format!(
            " The declared {token} sentinel completes the stream before any payload decoding, preserving the final usage frame as the completion's terminal metadata and issuing no further reads.",
            token = prose(token),
        ));
    }
    summary.push_str(&format!(
        " Iteration is pull-driven: early break or cancel stops the underlying body and issues no further reads. The direct {method}(...) call and its untyped stream are unchanged.\n  /// Source: {location}.",
        method = entry.method,
        location = prose(&entry.location),
    ));
    let holding = if entry.keep_final_usage {
        "      if (held != null) {\n        yield held;\n      }\n      held = frame;\n"
    } else {
        "      yield frame;\n"
    };
    let eof_usage = if entry.keep_final_usage {
        "held"
    } else {
        "null"
    };
    let held_declaration = if entry.keep_final_usage {
        format!("    {events}? held;\n", events = entry.events_type)
    } else {
        String::new()
    };
    let sentinel_usage = if entry.keep_final_usage { "held" } else { "null" };
    let sentinel_check = if entry.sentinel.is_some() {
        format!(
            "      if (frame == null) {{\n        // The declared {token} sentinel completes the stream before any payload decoding.\n        completion?.call({completion}('sentinel', {sentinel_usage}));\n        return;\n      }}\n",
            token = prose(entry.sentinel.as_deref().expect("declared sentinel")),
            completion = entry.completion_type,
        )
    } else {
        String::new()
    };
    format!(
        "  /// {summary}\n  Stream<{events}> {events_method}({{ {parameters} }}) {{\n    return {core_method}(null, {arguments});\n  }}\n  /// Drains the typed events of {operation} and returns the documented completion: why the stream completed and the preserved final usage frame. See {events_method} for the typed decode, metadata and cancellation semantics.\n  /// Source: {location}.\n  Future<{completion}> {completion_method}({{ {parameters} }}) async {{\n    {completion}? terminal;\n    await for (final _ in {core_method}((value) => terminal = value, {arguments})) {{}}\n    return terminal ?? const {completion}('eof', null);\n  }}\n  /// Shared typed events iteration over the lenient frame transport; see {events_method}.\n  Stream<{events}> {core_method}(void Function({completion})? completion, {{ {parameters} }}) async* {{\n    _RequestInput prepare() {{\n      final inputs = <_InputValue>[];\n{preparation}    }}\n    {held_declaration}    await for (final frame in _stream<{frame_type}>(_operation{index}, prepare, {frame_name}, cancellation, timeout, server, securityAlternative)) {{\n{sentinel_check}{holding}    }}\n    completion?.call({completion}('eof', {eof_usage}));\n  }}\n",
        summary = summary,
        held_declaration = held_declaration,
        events = entry.events_type,
        events_method = entry.events_method,
        parameters = parameters,
        core_method = entry.core_method,
        arguments = arguments,
        operation = prose(&entry.operation),
        completion = entry.completion_type,
        completion_method = entry.completion_method,
        location = prose(&entry.location),
        frame_type = entry.frame_type(),
        index = entry.operation_index,
        frame_name = entry.frame_name,
        preparation = super::http_emit::request_preparation(op, entry.operation_index),
        sentinel_check = sentinel_check,
        holding = holding,
        eof_usage = eof_usage,
    )
}
