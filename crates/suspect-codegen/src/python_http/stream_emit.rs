//! Emitted-only typed event decode for the Python HTTP adapter.
//!
//! The compiled `http_protocol::StreamSemanticsPlan` becomes per-operation
//! `iter_<op>_events` (plus async variants) generator methods on the generated
//! `Client`/`AsyncClient` classes plus a small module-level helper section and
//! frozen per-operation descriptor constants — all inside the generated
//! `_client.py`, with the typed event and completion dataclasses in the public
//! `operations.py`. The existing untyped stream iteration stays the transport:
//! the events generators run the same framing, limits and cancellation through
//! a copied operation wire whose stream item codec passes the framer's parsed
//! envelope values through leniently, and then apply the compiled per-kind
//! decode, sentinel and completion semantics themselves. Recognized event kinds
//! decode through the operation's existing stream item codec into their
//! declared model type; undeclared event kinds surface through the typed
//! unknown-event alternative without failing the stream; invalid payloads of
//! recognized kinds remain decoding errors. Static runtime files are never
//! modified, and operations without a discriminated stream schema emit no new
//! bytes at all.

use std::collections::BTreeSet;

use super::{HttpPlan, PlannedOperation, allocate, python_name};
use crate::http_protocol::{
    EventPayload, MediaPlan, Representation, ResponseStatus, StreamEventPlan, StreamFraming,
    StreamOperationPlan,
};
use crate::python_models::{PyDecl, PyType};
use suspect_ir::contract::SchemaId;

/// Extra `_client.py` imports; pushed only when a typed stream operation exists.
pub(super) const IMPORTS: &str = "import copy\nimport types\nfrom collections.abc import AsyncIterator, Iterator\nfrom typing import Any\nfrom ._runtime import SdkError\nfrom ._wire import location as _events_location\nfrom . import _registry as _events_registry, json_runtime as J\nfrom .codec_runtime import CodecError\n";

/// Module-level helper section emitted into `_client.py` before the clients.
pub(super) const HELPERS: &str = "\n\nclass _RawStreamEvents:\n    \"\"\"Identity stream codec for typed event decoding: the framer's parsed envelope values pass through unvalidated so undeclared event kinds stay representable, and the compiled per-kind codecs decode recognized kinds.\"\"\"\n\n    def decode_value(self, value: object) -> object:\n        return value\n\n\n_EVENTS_RAW_CODEC = _RawStreamEvents()\n# A synthetic source no plan document can declare; the raw codec registers here.\n_EVENTS_RAW_SOURCE = {'document': 'suspect://typed-stream-events', 'pointer': '/raw-envelope'}\n\n\ndef _events_register_raw_codec() -> None:\n    \"\"\"Register the raw envelope reader once, under a source the plan never owns.\"\"\"\n    codecs = _events_registry.codecs()\n    key = _events_location(_EVENTS_RAW_SOURCE)\n    if key not in codecs:\n        codecs[key] = _EVENTS_RAW_CODEC\n\n\ndef _events_value(value: object) -> object:\n    \"\"\"Optional model members decode as Unset when absent; typed event metadata carries None.\"\"\"\n    return None if isinstance(value, Unset) else value\n\n\nclass _EventsTerminal:\n    \"\"\"Internal terminal marker carrying one typed stream's completion metadata.\"\"\"\n\n    __slots__ = ('completion',)\n\n    def __init__(self, completion: Any) -> None:\n        self.completion = completion\n\n\nclass _AsyncStreamEvents:\n    \"\"\"Async typed events iterator: yields only events and exposes the documented { reason, usage } completion metadata as its `completion` attribute once the stream ends, because async generators cannot return values. Closing early leaves the completion unset.\"\"\"\n\n    __slots__ = ('_inner', 'completion')\n\n    def __init__(self, inner: AsyncIterator[Any]) -> None:\n        self._inner = inner\n        self.completion: Any = None\n\n    def __aiter__(self) -> '_AsyncStreamEvents':\n        return self\n\n    async def __anext__(self) -> Any:\n        item = await self._inner.__anext__()\n        if isinstance(item, _EventsTerminal):\n            self.completion = item.completion\n            raise StopAsyncIteration\n        return item\n\n    async def aclose(self) -> None:\n        await self._inner.aclose()\n";

/// One typed stream operation's allocated emission inputs.
pub(super) struct StreamEventsEntry {
    /// Operation index into the generated registry.
    pub index: usize,
    /// Allocated method name, shared by both client flavors.
    pub events_name: String,
    /// Allocated private inner async generator name for the async flavor.
    pub inner_name: String,
    /// `pascal` stem shared with the operation's own allocated names.
    pub stem: String,
    /// Allocated public `operations.py` type names.
    pub events_type: String,
    pub completion_type: String,
    /// Per-kind event class names, in declaration order.
    pub kind_types: Vec<String>,
    pub unknown_type: String,
    /// Declared event kinds in declaration order.
    pub kinds: Vec<String>,
    /// The declared terminal sentinel token, matched on frame data before any
    /// payload decoding.
    pub sentinel: Option<String>,
    pub keep_final_usage: bool,
    /// The decoded model attribute carrying the last-event id, when declared.
    pub id_field: Option<String>,
    /// The decoded model attribute carrying the reconnection hint, when declared.
    pub retry_field: Option<String>,
    /// The declared model type of each metadata attribute, rendered for `operations.py`.
    pub id_type: Option<String>,
    pub retry_type: Option<String>,
    /// The stream item model name: the declared model every kind decodes into.
    pub item_model: String,
    /// The stream item schema identity, for the strict per-kind codec lookup.
    pub item_schema: (String, String),
    /// Location of the stream media in the wire, for the lenient wire copy.
    pub response_index: usize,
    pub media_index: usize,
    /// The operation source, for decoding-failure branding and documentation.
    pub source: (String, String),
    /// Whether the operation takes a request body or content-type keyword.
    pub has_body: bool,
    pub content_type_parameter: bool,
}

/// The shared planner's operation identity for one planned operation: the
/// operation id, or `METHOD /path` when unnamed (the pagination convention).
fn identity(op: &PlannedOperation) -> String {
    op.wire()
        .operation_id()
        .map(|located| located.value().clone())
        .unwrap_or_else(|| format!("{} {}", op.wire().method().as_str(), op.wire().path()))
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

/// The operation's single success stream media matching one compiled entry,
/// reported with its response/media indices. Mixed or multiple success media
/// keep the conservative untyped path, because the typed decode would
/// otherwise own responses it cannot name.
fn stream_media<'a>(
    op: &'a PlannedOperation,
    compiled: &StreamOperationPlan,
) -> Option<(usize, usize, &'a MediaPlan)> {
    let mut found: Option<(usize, usize, &MediaPlan)> = None;
    for (response_index, response) in op.wire().responses().iter().enumerate() {
        if !matches!(
            response.status(),
            ResponseStatus::Exact(200..=299) | ResponseStatus::Range(2)
        ) {
            continue;
        }
        for (media_index, media) in response.media().iter().enumerate() {
            if !matches!(media.representation(), Representation::Stream { .. }) {
                continue;
            }
            if media.media_type().declared() != compiled.media_type {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some((response_index, media_index, media));
        }
    }
    found
}

/// Unwrap absent-value wrappers around a value type.
fn strip(ty: &PyType) -> &PyType {
    match ty {
        PyType::Optional(inner) | PyType::Nullable(inner) => strip(inner),
        other => other,
    }
}

/// Render a value type as an `operations.py` annotation using that module's imports.
fn render_type(ty: &PyType, symbols: &std::collections::BTreeMap<SchemaId, String>) -> String {
    match ty {
        PyType::Primitive(name) => match *name {
            "types.NoneType" => "None".into(),
            "_json.JsonNumber" => "JsonNumber".into(),
            other => (*other).to_owned(),
        },
        PyType::JsonValue => "JsonValue".into(),
        PyType::Named(id) => format!("models.{}", symbols[id]),
        PyType::Nullable(inner) => format!("{} | None", render_type(inner, symbols)),
        PyType::Optional(inner) => format!("{} | Unset", render_type(inner, symbols)),
        PyType::List(inner) => format!("list[{}]", render_type(inner, symbols)),
        PyType::Map(inner) => format!("dict[str, {}]", render_type(inner, symbols)),
        PyType::Literal(values) => format!(
            "Literal[{}]",
            values
                .iter()
                .map(|value| match value {
                    serde_json::Value::Null => "None".into(),
                    serde_json::Value::Bool(true) => "True".into(),
                    serde_json::Value::Bool(false) => "False".into(),
                    serde_json::Value::Number(number) => number.to_string(),
                    other => q(&other.to_string()),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        PyType::Union(types) => {
            let rendered = types
                .iter()
                .map(|(_, ty)| render_type(ty, symbols))
                .collect::<Vec<_>>();
            if rendered.iter().any(|rendered| rendered == "None") {
                format!("Union[{}]", rendered.join(", "))
            } else {
                rendered.join(" | ")
            }
        }
    }
}

/// The declared model type of one envelope metadata attribute, rendered for
/// `operations.py`; unresolvable shapes annotate `object`.
fn metadata_type(plan: &HttpPlan, item_schema: &SchemaId, wire: &str) -> String {
    let Some(PyDecl::Dataclass { fields, .. }) =
        plan.codecs().models().declarations().get(item_schema)
    else {
        return "object".into();
    };
    match fields.iter().find(|field| field.wire == wire) {
        Some(field) => render_type(strip(&field.ty), plan.symbols()),
        None => "object".into(),
    }
}

/// Every typed stream operation, in plan order, with allocated method names
/// shared by both client flavors and allocated public `operations.py` types.
pub(super) fn prepared(plan: &HttpPlan) -> Vec<StreamEventsEntry> {
    let streams = plan.stream_semantics();
    if streams.streams.is_empty() {
        return Vec::new();
    }
    let mut used: BTreeSet<String> = [
        "close",
        "aclose",
        "_call",
        "_prepare",
        "_exchange",
        "_configure",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    used.extend(plan.operations().iter().map(|op| op.snake_name.clone()));
    // The public operations.py type names participate in one allocation space
    // with the models and the per-operation result/error classes.
    used.extend(plan.symbols().values().cloned());
    for op in plan.operations() {
        used.extend([
            op.success_type.clone(),
            op.async_success_type.clone(),
            op.error_type.clone(),
            op.async_error_type.clone(),
        ]);
        for response in op.responses() {
            used.extend([
                response.class_name.clone(),
                response.async_class_name.clone(),
                response.error_class_name.clone(),
                response.async_error_class_name.clone(),
            ]);
        }
    }
    let mut result = Vec::new();
    for (index, op) in plan.operations().iter().enumerate() {
        let Some(compiled) = streams
            .streams
            .iter()
            .find(|stream| stream.operation == identity(op))
        else {
            continue;
        };
        let Some((response_index, media_index, _)) = stream_media(op, compiled) else {
            continue;
        };
        if !admits_typed_events(compiled) {
            continue;
        }
        let Some(item_codec) = &compiled.item_codec else {
            continue;
        };
        let item_schema = item_codec.schema().id();
        let Some(item_model) = plan.symbols().get(item_schema) else {
            continue;
        };
        let id_wire = compiled
            .item_metadata
            .as_ref()
            .and_then(|metadata| metadata.id_field.clone());
        let retry_wire = compiled
            .item_metadata
            .as_ref()
            .and_then(|metadata| metadata.retry_field.clone());
        let stem = crate::rust_models::pascal(&op.operation_id);
        result.push(StreamEventsEntry {
            index,
            events_name: allocate(&format!("iter_{}_events", op.snake_name), &mut used),
            inner_name: allocate(&format!("iter_{}_events_stream", op.snake_name), &mut used),
            stem,
            events_type: allocate(
                &format!("{}Event", crate::rust_models::pascal(&op.operation_id)),
                &mut used,
            ),
            completion_type: allocate(
                &format!("{}Completion", crate::rust_models::pascal(&op.operation_id)),
                &mut used,
            ),
            kind_types: compiled
                .events
                .iter()
                .map(|event| {
                    allocate(
                        &format!(
                            "{}{}Event",
                            crate::rust_models::pascal(&op.operation_id),
                            crate::rust_models::pascal(&event.event_name)
                        ),
                        &mut used,
                    )
                })
                .collect(),
            unknown_type: allocate(
                &format!(
                    "{}UnknownEvent",
                    crate::rust_models::pascal(&op.operation_id)
                ),
                &mut used,
            ),
            kinds: compiled
                .events
                .iter()
                .map(|event| event.event_name.clone())
                .collect(),
            sentinel: compiled
                .sentinel
                .enabled
                .then(|| compiled.sentinel.token.clone()),
            keep_final_usage: compiled.terminal.keep_final_usage,
            id_field: id_wire.as_deref().map(python_name),
            retry_field: retry_wire.as_deref().map(python_name),
            id_type: id_wire
                .as_ref()
                .map(|wire| metadata_type(plan, item_schema, wire)),
            retry_type: retry_wire
                .as_ref()
                .map(|wire| metadata_type(plan, item_schema, wire)),
            item_model: item_model.clone(),
            item_schema: (
                item_schema.document().as_str().to_owned(),
                item_schema.pointer().to_owned(),
            ),
            response_index,
            media_index,
            source: {
                let source = op.wire().source().terminal().source();
                (
                    source.document().as_str().to_owned(),
                    source.pointer().to_owned(),
                )
            },
            has_body: op.wire().body().is_some(),
            content_type_parameter: op
                .body
                .as_ref()
                .is_some_and(|body| body.content_type_parameter),
        });
    }
    result
}

fn q(text: &str) -> String {
    super::native_examples::quote(text)
}

/// A single-quoted Python literal for generated identifier-safe strings.
fn sq(text: &str) -> String {
    let mut out = String::from("'");
    for character in text.chars() {
        match character {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(character),
        }
    }
    out.push('\'');
    out
}

/// The constant name of one operation's frozen descriptor.
fn descriptor_name(entry: &StreamEventsEntry) -> String {
    format!("_{}", entry.events_name.to_ascii_uppercase())
}

/// The frozen per-operation descriptor constants and lazy operation builders,
/// emitted at `_client.py` module level before the clients.
pub(super) fn descriptors(entries: &[StreamEventsEntry]) -> String {
    let mut code = String::new();
    for entry in entries {
        let name = descriptor_name(entry);
        let sentinel = entry.sentinel.as_deref().unwrap_or("");
        let constructors = entry
            .kinds
            .iter()
            .zip(&entry.kind_types)
            .map(|(kind, class)| format!("{}: operations.{},", sq(kind), class))
            .collect::<Vec<_>>()
            .join(" ");
        #[allow(clippy::format_in_format_args)]
        code.push_str(&format!(
            "\n\n# Frozen typed-stream descriptor compiled from the source stream semantics plan.\n{name} = types.MappingProxyType({{\n    'operation': {},\n    'response': {},\n    'media': {},\n    'kinds': {},\n    'sentinel': {},\n    'keep_final_usage': {},\n    'events': types.MappingProxyType({{{}}}),\n    'item_codec': {{'document': {}, 'pointer': {}}},\n    'id_field': {},\n    'retry_field': {},\n    'source': {{'document': {}, 'pointer': {}}},\n}})\n",
            entry.index,
            entry.response_index,
            entry.media_index,
            format!(
                "({})",
                entry
                    .kinds
                    .iter()
                    .map(|kind| sq(kind))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            sq(sentinel),
            if entry.keep_final_usage { "True" } else { "False" },
            constructors,
            q(&entry.item_schema.0),
            q(&entry.item_schema.1),
            sq(entry.id_field.as_deref().unwrap_or("None")),
            sq(entry.retry_field.as_deref().unwrap_or("None")),
            q(&entry.source.0),
            q(&entry.source.1),
        ));
        code.push_str(&format!(
            "\n\ndef _events_operation_{}() -> Any:\n    \"\"\"The {} operation over the lenient envelope wire, rebuilt per call.\"\"\"\n    operation = copy.copy(_events_registry.operation({name}['operation']))\n    wire = dict(operation.wire)\n    responses = list(wire['responses'])\n    chosen = dict(responses[{name}['response']])\n    media = list(chosen['media'])\n    item = dict(media[{name}['media']])\n    representation = dict(item['representation'])\n    stream = dict(representation['stream'])\n    stream['item_codec'] = {{'schema': {{'id': _EVENTS_RAW_SOURCE}}}}\n    representation['stream'] = stream\n    item['representation'] = representation\n    media[{name}['media']] = item\n    chosen['media'] = media\n    responses[{name}['response']] = chosen\n    wire['responses'] = responses\n    operation.wire = wire\n    return operation\n",
            entry.index,
            entry.stem,
            name = name,
        ));
    }
    code
}

/// The public `operations.py` typed event, unknown and completion dataclasses
/// plus the discriminated union alias, emitted for exactly the typed stream
/// operations and referenced by the generated `_client.py` iterators.
pub(super) fn operations_module(plan: &HttpPlan) -> String {
    let entries = prepared(plan);
    let mut code = String::new();
    for entry in &entries {
        let model = format!("models.{}", entry.item_model);
        let metadata_fields = {
            let mut fields = String::new();
            if let Some(ty) = &entry.id_type {
                fields.push_str(&format!("    id: {ty} | None = None\n"));
            }
            if let Some(ty) = &entry.retry_type {
                fields.push_str(&format!("    retry: {ty} | None = None\n"));
            }
            fields
        };
        for (kind, class) in entry.kinds.iter().zip(&entry.kind_types) {
            code.push_str(&format!(
                "@dataclasses.dataclass(frozen=True, kw_only=True)\nclass {class}:\n    {}\n    kind: Literal[{}]\n    data: {model}\n{metadata_fields}\n",
                q(&format!(
                    "A declared {} event of the {} stream: the framed envelope decoded to its declared model type. An invalid payload for this kind remains a decoding error.",
                    kind, entry.stem
                )),
                sq(kind),
                metadata_fields = metadata_fields,
            ));
        }
        code.push_str(&format!(
            "@dataclasses.dataclass(frozen=True, kw_only=True)\nclass {}:\n    {}\n    kind: Literal['unknown']\n    event: str\n    data: str\n\n",
            entry.unknown_type,
            q("An event kind the source never declared: the raw frame data stays representable without failing the stream."),
        ));
        let union = entry
            .kind_types
            .iter()
            .chain(std::iter::once(&entry.unknown_type))
            .cloned()
            .collect::<Vec<_>>()
            .join(" | ");
        code.push_str(&format!("{}: TypeAlias = {}\n", entry.events_type, union));
        code.push_str(&format!(
            "@dataclasses.dataclass(frozen=True, kw_only=True)\nclass {}:\n    {}\n    reason: Literal['sentinel', 'eof']\n    usage: {} | None\n\n",
            entry.completion_type,
            q(&format!(
                "Terminal metadata of one completed {} stream: why it completed and the preserved final usage frame. The typed iterator's return value (StopIteration.value); for-loops discard it.",
                entry.stem
            )),
            entry.events_type,
        ));
    }
    code
}
fn docstring(entry: &StreamEventsEntry, asynchronous: bool) -> String {
    let completion = if asynchronous {
        "the returned iterator's `completion` attribute once the stream ends (async generators cannot return values; closing early leaves it unset)"
    } else {
        "StopIteration.value on the final `next()`"
    };
    let sentinel = match &entry.sentinel {
        Some(token) => format!(
            " The declared {} sentinel completes the stream before any payload decoding, preserving the final usage frame as the completion's terminal metadata and issuing no further reads.",
            token
        ),
        None => String::new(),
    };
    format!(
        "Lazily yields the typed events of {} over the existing stream item iteration. A declared event kind decodes the framed envelope through the operation's stream item codec into its declared model type; an event kind the source never declared surfaces through the typed {} alternative without failing the stream; an invalid payload for a recognized kind remains a decoding error. Per-item metadata (the event name as `kind`, plus id/retry when the item schema declares them) is explicit, and the {{ reason, usage }} completion metadata is read from {}. Iteration is pull-driven: early break or close cancels the response body and issues no further reads. The direct {}(...) call and its untyped stream iterator are unchanged.\nSource: {}#{}",
        entry.stem, entry.unknown_type, completion, entry.stem, entry.source.0, entry.source.1,
    ) + &sentinel
}

/// The shared per-item decode body: sentinel first, then the per-kind decode
/// with the compiled unknown alternative, then the compiled keep-final-usage
/// holding policy. `asynchronous` selects the awaiting flavor and the
/// terminal-marker completion channel async generators require.
fn decode_body(entry: &StreamEventsEntry, asynchronous: bool) -> String {
    let next_value = if asynchronous {
        "await events.__anext__()"
    } else {
        "next(events)"
    };
    let stop = if asynchronous {
        "StopAsyncIteration"
    } else {
        "StopIteration"
    };
    let mut metadata_values = String::new();
    if let Some(field) = &entry.id_field {
        metadata_values.push_str(&format!(", id=_events_value(item.{field})"));
    }
    if let Some(field) = &entry.retry_field {
        metadata_values.push_str(&format!(", retry=_events_value(item.{field})"));
    }
    // The sentinel completes the stream before any payload decoding; the
    // compiled keep-final-usage policy holds one frame behind so the last data
    // frame before the sentinel or end of body becomes the completion's usage.
    let eof = if asynchronous {
        format!(
            "                    yield _EventsTerminal(operations.{}(reason='eof', usage=held if descriptor['keep_final_usage'] else None))\n                    return\n",
            entry.completion_type
        )
    } else {
        format!(
            "                    return operations.{}(reason='eof', usage=held if descriptor['keep_final_usage'] else None)\n",
            entry.completion_type
        )
    };
    let sentinel_return = if asynchronous {
        format!(
            "                    yield _EventsTerminal(operations.{}(reason='sentinel', usage=held))\n                    return\n",
            entry.completion_type
        )
    } else {
        format!(
            "                    return operations.{}(reason='sentinel', usage=held)\n",
            entry.completion_type
        )
    };
    format!(
        "                try:\n                    envelope = {next_value}\n                except {stop}:\n{eof}                data = envelope.get('data') if isinstance(envelope, dict) else None\n                if descriptor['sentinel'] and data == descriptor['sentinel']:\n{sentinel_return}                kind = envelope.get('event') if isinstance(envelope, dict) else None\n                if not isinstance(kind, str) or kind == '':\n                    kind = 'message'\n                if kind in descriptor['kinds']:\n                    try:\n                        item = codec.decode_value(envelope)\n                    except (CodecError, J.JsonError, ValueError, TypeError) as error:\n                        raise SdkError('resource-limit' if isinstance(error, (CodecError, J.JsonError)) and error.kind in ('resource', J.RESOURCE_LIMIT) else 'response-decoding', source, cause=error) from None\n                    typed = descriptor['events'][kind](kind=kind, data=item{metadata_values})\n                else:\n                    typed = operations.{unknown_type}(kind='unknown', event=kind, data=data if isinstance(data, str) else '')\n                if descriptor['keep_final_usage']:\n                    if held is not None:\n                        yield held\n                    held = typed\n                else:\n                    yield typed\n",
        next_value = next_value,
        stop = stop,
        eof = eof,
        sentinel_return = sentinel_return,
        metadata_values = metadata_values,
        unknown_type = entry.unknown_type,
    )
}

/// The prologue shared by both flavors: lenient wire call and lookup setup.
fn prologue(entry: &StreamEventsEntry, asynchronous: bool) -> String {
    let awaited = if asynchronous { "await " } else { "" };
    let mut code = String::new();
    if entry.has_body {
        code.push_str("        body = kwargs.pop('body', UNSET)\n");
        if entry.content_type_parameter {
            code.push_str("        content_type = kwargs.pop('content_type', UNSET)\n");
        }
    }
    let call = if entry.has_body && entry.content_type_parameter {
        format!("{awaited}self._call(operation, kwargs, body, content_type)")
    } else if entry.has_body {
        format!("{awaited}self._call(operation, kwargs, body)")
    } else {
        format!("{awaited}self._call(operation, kwargs)")
    };
    format!(
        "{code}        _events_register_raw_codec()\n        descriptor = {name}\n        operation = _events_operation_{index}()\n        codec = _events_registry.codec({{'schema': {{'id': descriptor['item_codec']}}}})\n        source = _events_location(descriptor['source'])\n        response = cast(Any, {call})\n        events = response.data\n        held: operations.{events_type} | None = None\n        try:\n            while True:\n",
        code = code,
        name = descriptor_name(entry),
        call = call,
        events_type = entry.events_type,
        index = entry.index,
    )
}

/// The per-client typed events generator methods for one operation. The sync
/// flavor is a generator whose documented return value carries the completion;
/// the async flavor returns an `_AsyncStreamEvents` iterator whose
/// `completion` attribute carries it, because async generators cannot return
/// values.
pub(super) fn methods(entry: &StreamEventsEntry, asynchronous: bool) -> String {
    let mut code = format!(
        "\n    def {}(self, **kwargs: Any) -> {}:\n        {}\n",
        entry.events_name,
        if asynchronous {
            "_AsyncStreamEvents".to_owned()
        } else {
            format!("Iterator[operations.{}]", entry.events_type)
        },
        q(&docstring(entry, asynchronous)),
    );
    if asynchronous {
        code.push_str(&format!(
            "        return _AsyncStreamEvents(self.{}(**kwargs))\n",
            entry.inner_name
        ));
        code.push_str(&format!(
            "\n    async def {}(self, **kwargs: Any) -> AsyncIterator[Any]:\n        {}\n",
            entry.inner_name,
            q(&format!(
                "Inner generator of {}; see that method for the documented typed decode, sentinel and completion semantics.",
                entry.events_name
            )),
        ));
        code.push_str(&prologue(entry, true));
        code.push_str(&decode_body(entry, true));
        code.push_str("        finally:\n            await events.aclose()\n");
    } else {
        code.push_str(&prologue(entry, false));
        code.push_str(&decode_body(entry, false));
        code.push_str("        finally:\n            events.close()\n");
    }
    code
}
