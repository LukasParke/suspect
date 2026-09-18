//! Emitted-only typed event decode for the C# HTTP adapter.
//!
//! The compiled `http_protocol::StreamSemanticsPlan` becomes per-operation
//! typed events members on the generated `Client` plus a generated
//! `StreamEvents.g.cs` holding the typed event, unknown and completion shapes
//! and the per-frame decoder — exactly for the operations whose stream plan
//! carries a discriminated SSE event set. The existing untyped stream iteration
//! stays the transport: the events methods run the same framing, limits and
//! cancellation through a generated wire call over the operation's own request
//! preparation whose stream item decode reads the framed envelope leniently,
//! and then apply the compiled per-kind decode, sentinel and completion
//! semantics themselves. Recognized event kinds decode through the operation's
//! existing stream item codec into their declared model type; undeclared event
//! kinds surface through the typed unknown-event alternative without failing
//! the stream; invalid payloads of recognized kinds remain decoding errors.
//! Static runtime files are never modified, and operations without a
//! discriminated stream schema emit nothing at all.

use super::{
    SdkPlan,
    emit::{quote, source, xml},
    models::{CsDecl, CsType},
};
use crate::http_protocol as p;
use std::collections::BTreeSet;
use suspect_ir::contract::SchemaId;

/// One envelope metadata member the item schema declares, with its decoded
/// model property and the item codec's requiredness.
#[derive(Debug, Clone)]
pub(super) struct Metadata {
    /// The item model's allocated property name.
    property: String,
    /// The rendered native type of the metadata value.
    ty: String,
    /// Whether the item codec requires the member on every envelope.
    required: bool,
}

impl Metadata {
    /// The generated expression reading one decoded item's metadata value.
    fn read(&self) -> String {
        if self.required {
            format!("item.{}", self.property)
        } else {
            format!(
                "item.{}.HasValue ? item.{}.Value : null",
                self.property, self.property
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
    pub input_type: String,
    /// The stream item model name: the declared model every kind decodes into.
    pub item_model: String,
    /// Declared event kinds in declaration order with their allocated member
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
    pub completion_type: String,
    pub reason_type: String,
    /// Allocated Client member names.
    pub events_method: String,
    pub completion_method: String,
    pub source_method: String,
    pub core_method: String,
    /// Allocated internal frame-decoder reference: the static class and method.
    pub decoder: String,
    /// The operation's index into the generated registry.
    pub operation_index: usize,
    /// The success response index whose stream media carries the events.
    pub response_index: usize,
}

impl Events {
    /// The nullable suffix of one framed event: only a compiled sentinel makes
    /// a frame carry no event.
    fn frame_nullable(&self) -> &'static str {
        if self.sentinel.is_some() {
            "?"
        } else {
            ""
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

/// The operation's single success response owning exactly one stream media of
/// the compiled media type. Mixed, multiple or default-status success dispatch
/// keep the conservative untyped path, because the typed decode would otherwise
/// own responses it cannot name.
fn stream_response<'a>(
    op: &'a super::PlannedOperation,
    compiled: &p::StreamOperationPlan,
) -> Option<(usize, &'a super::PlannedResponse)> {
    let mut found: Option<(usize, &super::PlannedResponse)> = None;
    for (index, response) in op.responses.iter().enumerate() {
        if !response.may_succeed() {
            continue;
        }
        if matches!(response.wire.status(), p::ResponseStatus::Default) {
            return None;
        }
        if response.media.len() != 1 || !response.media[0].is_stream() {
            return None;
        }
        if response.media[0].wire.media_type().declared() != compiled.media_type {
            return None;
        }
        if found.is_some() {
            return None;
        }
        found = Some((index, response));
    }
    found
}

/// Unwrap nullable wrappers around one metadata type; the event metadata
/// itself is always nullable because a frame may carry none.
fn metadata_type(plan: &SdkPlan, ty: &CsType) -> String {
    let rendered = plan.models.render_type(ty);
    if rendered.ends_with('?') {
        rendered
    } else {
        format!("{rendered}?")
    }
}

/// The declared model member one envelope metadata field decodes into.
fn metadata(
    plan: &SdkPlan,
    item_schema: &SchemaId,
    wire: Option<&str>,
) -> Option<Metadata> {
    let key = super::models::key(item_schema);
    let Some(CsDecl::Record { fields, .. }) = plan.models.declarations.get(&key) else {
        return None;
    };
    let field = fields.iter().find(|field| Some(field.wire.as_str()) == wire)?;
    Some(Metadata {
        property: field.name.clone(),
        ty: metadata_type(plan, &field.ty),
        required: field.required,
    })
}

fn exported(kind: &str) -> String {
    crate::rust_models::pascal(kind)
}

/// Compile the emittable subset of the compiled stream plan, allocating every
/// generated name from the client's method and model/type spaces. A stream
/// plan with no emittable operation emits nothing.
pub(super) fn compiled(plan: &SdkPlan) -> Vec<Events> {
    let mut methods: BTreeSet<String> = plan
        .operations()
        .iter()
        .map(|op| op.method_name.clone())
        .collect();
    methods.insert("Client".into());
    methods.insert("Dispose".into());
    if plan.credential_env().is_some() {
        methods.insert(super::credential_env::FACTORY.into());
    }
    let mut used: BTreeSet<String> = plan
        .models()
        .names
        .values()
        .cloned()
        .chain(super::reserved_types().into_iter().map(str::to_owned))
        .collect();
    let decoder_class = super::allocate("StreamEvents", &mut used);
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
        let Some((response_index, _)) = stream_response(op, stream) else {
            continue;
        };
        let Some(item_codec) = &stream.item_codec else {
            continue;
        };
        let item_schema = item_codec.schema().id();
        let Some(item_model) = plan.models().names.get(&super::models::key(item_schema)) else {
            continue;
        };
        let stem = op
            .method_name
            .strip_suffix("Async")
            .unwrap_or(&op.method_name);
        let stem = if stem.is_empty() {
            op.method_name.as_str()
        } else {
            stem
        };
        let events_type = super::allocate(&format!("{stem}Event"), &mut used);
        let completion_type = super::allocate(&format!("{stem}Completion"), &mut used);
        let reason_type = super::allocate(&format!("{stem}CompletionReason"), &mut used);
        let decode = super::allocate(&format!("Decode{stem}Frame"), &mut used);
        let events_method = super::allocate(&format!("{stem}EventsAsync"), &mut methods);
        let completion_method =
            super::allocate(&format!("{stem}EventsCompletionAsync"), &mut methods);
        let source_method = super::allocate(&format!("{stem}EventsSourceAsync"), &mut methods);
        let core_method = super::allocate(&format!("{stem}EventsCoreAsync"), &mut methods);
        let mut members: BTreeSet<String> = [
            "Kind",
            "Id",
            "Retry",
            "Event",
            "Data",
            "UnknownRecord",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let kinds = stream
            .events
            .iter()
            .map(|event| {
                (
                    event.event_name.clone(),
                    super::allocate(&exported(&event.event_name), &mut members),
                )
            })
            .collect();
        result.push(Events {
            operation: op.operation_id.clone(),
            location: source(&op.source),
            method: op.method_name.clone(),
            input_type: op.input_type.clone(),
            item_model: item_model.clone(),
            kinds,
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
            completion_type,
            reason_type,
            events_method,
            completion_method,
            source_method,
            core_method,
            decoder: format!("{decoder_class}.{decode}"),
            operation_index,
            response_index,
        });
    }
    result
}

/// Whether any operation lowers into a typed events emission.
pub(super) fn emits(plan: &SdkPlan) -> bool {
    !compiled(plan).is_empty()
}

/// The generated `src/StreamEvents.g.cs`: the per-operation typed event and
/// completion shapes plus one internal static class of per-frame decoders.
/// Called only when at least one operation lowers into a typed events emission.
pub(super) fn module(plan: &SdkPlan) -> String {
    let entries = compiled(plan);
    let mut out = super::emit::header(plan);
    for entry in &entries {
        out.push_str(&event_type(entry));
        out.push_str(&completion_type(entry));
    }
    let decoder_class = decoder_class_name(&entries);
    out.push_str(&format!("/// <summary>Generated typed SSE frame decoding for this package's typed stream operations: the declared sentinel completes before any payload decoding, recognized kinds decode through the operation's stream item codec, and undeclared kinds surface through the typed unknown alternative.</summary>\ninternal static class {decoder_class}\n{{\n"));
    for entry in &entries {
        out.push_str(&decoder_method(entry));
    }
    out.push_str("}\n");
    out
}

/// The allocated internal frame-decoder class name, recomputed from the same
/// deterministic allocation sequence.
fn decoder_class_name(entries: &[Events]) -> &str {
    entries
        .first()
        .map(|entry| entry.decoder.split('.').next().unwrap_or("StreamEvents"))
        .unwrap_or("StreamEvents")
}

/// The metadata members the item schema declares, on the typed event base.
fn metadata_members(entry: &Events) -> String {
    let mut out = String::new();
    if let Some(id) = &entry.id_metadata {
        out.push_str(&format!(
            "    /// <summary>The last-event id when the item schema declares it; null when the frame carried none.</summary>\n    public {} Id {{ get; init; }}\n",
            id.ty
        ));
    }
    if let Some(retry) = &entry.retry_metadata {
        out.push_str(&format!(
            "    /// <summary>The reconnection hint in milliseconds when the item schema declares it; null when the frame carried none.</summary>\n    public {} Retry {{ get; init; }}\n",
            retry.ty
        ));
    }
    out
}

/// The metadata arguments of one recognized kind's construction.
fn metadata_arguments(entry: &Events) -> String {
    let mut out = String::new();
    if let Some(id) = &entry.id_metadata {
        out.push_str(&format!(", Id = {}", id.read()));
    }
    if let Some(retry) = &entry.retry_metadata {
        out.push_str(&format!(", Retry = {}", retry.read()));
    }
    out
}

/// The per-operation discriminated event union.
fn event_type(entry: &Events) -> String {
    let mut out = format!(
        "/// <summary>One typed event of the {operation} stream: declared kinds decode the framed envelope through the operation's stream item codec into their declared model type; an event kind the source never declared surfaces through the typed unknown alternative without failing the stream; invalid payloads of recognized kinds remain decoding errors. Source: {location}.</summary>\npublic abstract record {events}\n{{\n    private {events}() {{ }}\n    /// <summary>The declared event name of this frame; `unknown` marks a kind the source never declared.</summary>\n    public abstract string Kind {{ get; }}\n{metadata}",
        operation = xml(&entry.operation),
        location = xml(&entry.location),
        events = entry.events_type,
        metadata = metadata_members(entry),
    );
    for (kind, member) in &entry.kinds {
        out.push_str(&format!(
            "    /// <summary>A declared `{kind}` event of the {operation} stream: the framed envelope decoded to its declared model type. An invalid payload for this kind remains a decoding error.</summary>\n    public sealed record {member} : {events}\n    {{\n        /// <summary>The decoded {model} envelope.</summary>\n        public required {model} Data {{ get; init; }}\n        /// <summary>The declared event name.</summary>\n        public override string Kind => {kind_literal};\n    }}\n",
            kind = xml(kind),
            kind_literal = quote(kind),
            operation = xml(&entry.operation),
            model = entry.item_model,
            member = member,
            events = entry.events_type,
        ));
    }
    out.push_str(&format!(
        "    /// <summary>An event kind the source never declared: the raw frame data stays representable without failing the stream.</summary>\n    public sealed record UnknownRecord : {events}\n    {{\n        /// <summary>The undeclared event name as received.</summary>\n        public required string Event {{ get; init; }}\n        /// <summary>The raw frame data text.</summary>\n        public required string Data {{ get; init; }}\n        /// <summary>The undeclared-kind marker.</summary>\n        public override string Kind => \"unknown\";\n    }}\n}}\n\n",
        events = entry.events_type,
    ));
    out
}

/// The per-operation completion reason enum and terminal metadata record.
fn completion_type(entry: &Events) -> String {
    format!(
        "/// <summary>`Sentinel` when the declared terminal token completed the stream before any payload decoding, `Eof` when the response body ended.</summary>\npublic enum {reason}\n{{\n    /// <summary>The declared terminal token completed the stream before any payload decoding.</summary>\n    Sentinel,\n    /// <summary>The response body ended.</summary>\n    Eof,\n}}\n\n/// <summary>Terminal metadata of one completed {operation} stream: why it completed and the preserved final usage frame. The typed events iterator discards it; read it through {completion_method}. Source: {location}.</summary>\npublic sealed record {completion}({reason} Reason, {events}? Usage);\n\n",
        reason = entry.reason_type,
        operation = xml(&entry.operation),
        location = xml(&entry.location),
        completion = entry.completion_type,
        completion_method = entry.completion_method,
        events = entry.events_type,
    )
}

/// The per-frame decoder of one operation: the sentinel is matched on the
/// frame data before any payload decoding, recognized kinds decode through the
/// operation's stream item codec, and undeclared kinds stay representable.
fn decoder_method(entry: &Events) -> String {
    let frame = format!("{}{}", entry.events_type, entry.frame_nullable());
    let sentinel = match &entry.sentinel {
        Some(token) => format!(
            "        // The declared {token} sentinel completes the stream before any payload decoding.\n        if (data == {token_literal}) {{ return null; }}\n",
            token = xml(token),
            token_literal = quote(token)
        ),
        None => String::new(),
    };
    let cases = entry
        .kinds
        .iter()
        .map(|(kind, member)| {
            format!(
                "            case {kind_literal}:\n            {{\n                var item = Codecs.Decode{model}(frame);\n                return new {events}.{member} {{ Data = item{metadata} }};\n            }}\n",
                kind_literal = quote(kind),
                model = entry.item_model,
                events = entry.events_type,
                member = member,
                metadata = metadata_arguments(entry),
            )
        })
        .collect::<Vec<_>>()
        .join("");
    format!(
        "    internal static {frame} {decode}(byte[] frame)\n    {{\n        var envelope = JsonRuntime.Parse(frame);\n        if (envelope.ValueKind != JsonValueKind.Object)\n        {{\n            return new {events}.UnknownRecord {{ Event = \"message\", Data = \"\" }};\n        }}\n        var data = envelope.TryGetProperty(\"data\", out var payload) && payload.ValueKind == JsonValueKind.String ? payload.GetString()! : \"\";\n{sentinel}        var kind = envelope.TryGetProperty(\"event\", out var declared) && declared.ValueKind == JsonValueKind.String && declared.GetString() is {{ Length: > 0 }} name ? name : \"message\";\n        switch (kind)\n        {{\n{cases}            default:\n                return new {events}.UnknownRecord {{ Event = kind, Data = data }};\n        }}\n    }}\n",
        frame = frame,
        decode = entry
            .decoder
            .split('.')
            .next_back()
            .unwrap_or("DecodeFrame"),
        events = entry.events_type,
        sentinel = sentinel,
        cases = cases,
    )
}

/// The per-operation Client members appended to the generated `Client`.
pub(super) fn client_methods(plan: &SdkPlan) -> String {
    let entries = compiled(plan);
    let mut out = String::new();
    for entry in &entries {
        let op = &plan.operations()[entry.operation_index];
        out.push_str(&client_entry(entry, plan, op));
    }
    out
}

/// The per-operation Client members: the typed events iterator, the
/// documented completion accessor, the shared core iteration and the lenient
/// wire call over the operation's own request preparation.
fn client_entry(entry: &Events, plan: &SdkPlan, op: &super::PlannedOperation) -> String {
    let events = entry.events_type.as_str();
    let frame = format!("{events}{}", entry.frame_nullable());
    let mut summary = format!(
        "Lazily yields the typed events of {operation} over the existing stream item iteration. A declared event kind decodes the framed envelope through the operation's stream item codec into its declared model type; an event kind the source never declared surfaces through the typed unknown alternative without failing the stream; an invalid payload for a recognized kind remains a decoding error. Per-item metadata (the event name as Kind, plus Id/Retry when the item schema declares them) is explicit.",
        operation = xml(&entry.operation),
    );
    if let Some(token) = &entry.sentinel {
        summary.push_str(&format!(" The declared {} sentinel completes the stream before any payload decoding, preserving the final usage frame as the completion's terminal metadata and issuing no further reads.", xml(token)));
    }
    summary.push_str(&format!(
        " Iteration is pull-driven: early break or cancellation closes the response body and issues no further reads. The direct {method}(...) call and its untyped stream are unchanged. Source: {location}.",
        method = entry.method,
        location = xml(&entry.location),
    ));
    let holding = if entry.keep_final_usage {
        "                if (held is not null)\n                {\n                    yield return held;\n                }\n                held = frame;\n"
    } else {
        "                yield return frame;\n"
    };
    let held_decl = if entry.keep_final_usage {
        format!(
            "            {events}? held = null;\n",
            events = entry.events_type,
        )
    } else {
        String::new()
    };
    let eof_usage = if entry.keep_final_usage {
        "held"
    } else {
        "null"
    };
    let sentinel_check = if entry.sentinel.is_some() {
        format!(
            "                if (frame is null)\n                {{\n                    completion?.Invoke(new {completion}({reason}.Sentinel, held));\n                    yield break;\n                }}\n",
            completion = entry.completion_type,
            reason = entry.reason_type,
        )
    } else {
        String::new()
    };
    let mut out = format!(
        "    /// <summary>{summary}</summary>\n    public async global::System.Collections.Generic.IAsyncEnumerable<{events}> {events_method}({input} input, [global::System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)\n    {{\n        await foreach (var item in {core_method}(input, null, cancellationToken))\n        {{\n            yield return item;\n        }}\n    }}\n    /// <summary>Drains the typed events of {operation} and returns the documented completion: why the stream completed and the preserved final usage frame. See {events_method} for the typed decode, metadata and cancellation semantics.</summary>\n    public async Task<{completion}> {completion_method}({input} input, CancellationToken cancellationToken = default)\n    {{\n        {completion}? terminal = null;\n        await foreach (var item in {core_method}(input, done => terminal = done, cancellationToken))\n        {{\n            _ = item;\n        }}\n        return terminal ?? new {completion}({reason}.Eof, null);\n    }}\n    /// <summary>Shared typed events iteration over the lenient frame transport; see {events_method}.</summary>\n    private async global::System.Collections.Generic.IAsyncEnumerable<{events}> {core_method}({input} input, global::System.Action<{completion}>? completion, [global::System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)\n    {{\n        var stream = await {source_method}(input, null, cancellationToken).ConfigureAwait(false);\n        var frames = stream.GetAsyncEnumerator(cancellationToken);\n        try\n        {{\n{held_decl}            while (await frames.MoveNextAsync().ConfigureAwait(false))\n            {{\n                var frame = frames.Current;\n{sentinel_check}{holding}            }}\n            completion?.Invoke(new {completion}({reason}.Eof, {eof_usage}));\n        }}\n        finally\n        {{\n            await frames.DisposeAsync().ConfigureAwait(false);\n            await stream.DisposeAsync().ConfigureAwait(false);\n        }}\n    }}\n",
        summary = summary,
        events = events,
        events_method = entry.events_method,
        input = entry.input_type,
        core_method = entry.core_method,
        operation = xml(&entry.operation),
        completion = entry.completion_type,
        completion_method = entry.completion_method,
        reason = entry.reason_type,
        source_method = entry.source_method,
        held_decl = held_decl,
        sentinel_check = sentinel_check,
        holding = holding,
        eof_usage = eof_usage,
    );
    out.push_str(&source_method(entry, plan, op, &frame));
    out
}

/// The lenient wire call: the operation's own request preparation and typed
/// failure dispatch, with the stream item decode replaced by the generated
/// frame reader.
fn source_method(
    entry: &Events,
    plan: &SdkPlan,
    op: &super::PlannedOperation,
    frame: &str,
) -> String {
    let mut cases = format!(
        "                case {index}:\n                {{\n                    return raw.Stream({decoder});\n                }}\n",
        index = entry.response_index,
        decoder = entry.decoder,
    );
    for (j, r) in op.responses.iter().enumerate() {
        if j == entry.response_index || !r.may_fail() {
            continue;
        }
        cases.push_str(&format!("                case {j}:\n                {{\n"));
        cases.push_str(&super::http::response_headers_local(r));
        cases.push_str(&super::http::response_data_assignment(plan, r));
        cases.push_str(&format!(
            "                    throw new {}(data, raw{});\n",
            r.error_type_name,
            if r.header_type.is_some() {
                ", headers"
            } else {
                ""
            }
        ));
        cases.push_str("                }\n");
    }
    format!(
        "    /// <summary>The {operation} stream over the existing framing, limits and cancellation with the framed envelope read leniently, so the typed decode below owns per-kind validation and undeclared event kinds stay representable.</summary>\n    private Task<HttpStream<{frame}>> {source_method}({input} input, RequestOptions? requestOptions, CancellationToken cancellationToken)\n    {{\n        return _runtime.CallAsync<HttpStream<{frame}>>({operation_index}, requestOptions, cancellationToken, () =>\n        {{\n            if (input is null) throw new SdkException(SdkErrorKind.RequestRepresentation);\n            var operation = ProtocolData.Operation({operation_index});\n            var request = new WireRequest();\n{prepare}            return request;\n        }}, static raw =>\n        {{\n            switch (raw.ResponseIndex)\n            {{\n{cases}                default: throw new UnexpectedResponseException(raw);\n            }}\n        }}, _credentials.Get);\n    }}\n",
        operation = xml(&entry.operation),
        frame = frame,
        source_method = entry.source_method,
        input = entry.input_type,
        operation_index = entry.operation_index,
        prepare = super::http::request_preparation(plan, op),
        cases = cases,
    )
}
