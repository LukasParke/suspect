//! Compiled typed-event emission inputs for the Rust HTTP adapter.
//!
//! The shared `http_protocol::StreamSemanticsPlan` is lowered here into one
//! per-operation entry for exactly the operations whose stream plan carries a
//! discriminated SSE event set. Rendering happens in
//! `rust_http/emit/stream_events.rs`; this module owns the source admission
//! decisions so the canonical v3 plan can reserve emitted client method names
//! while the operations are being planned.

use std::collections::BTreeMap;

use super::*;
use crate::http_protocol as wire;
use crate::rust_models::{Decl, RepresentationRole, Type};
use suspect_ir::contract::{SchemaId, SourceId};

/// One declared envelope metadata member copied from the decoded model.
#[derive(Debug, Clone)]
pub struct EventMetadata {
    /// Allocated model field name on the item model.
    pub field: String,
    /// The declared field type without its optionality wrapper.
    pub type_: String,
    /// An expression over `item.<field>` producing `Option<type_>`.
    pub conversion: String,
}

/// One operation's emission-ready typed event decode.
#[derive(Debug, Clone)]
pub struct StreamEventsEntry {
    /// The operation module this entry extends.
    pub module: String,
    /// The emitted operation function name; the events constructor appends
    /// `_events`.
    pub function: String,
    /// Source operation identity for documentation.
    pub operation_id: String,
    /// Allocated public type-name stem shared by this entry's types.
    pub stem: String,
    pub input_type: String,
    pub error_type: String,
    /// The stream item codec's declared model: every declared kind's payload
    /// decodes into this declared model type.
    pub item_model: String,
    /// Declared event kinds in declaration order.
    pub events: Vec<String>,
    /// The declared terminal sentinel token, matched on frame data before any
    /// payload decoding.
    pub sentinel: Option<String>,
    /// Whether the last data frame before the sentinel or end of body is
    /// preserved as the completion's terminal usage instead of being yielded.
    pub keep_final_usage: bool,
    pub id_metadata: Option<EventMetadata>,
    pub retry_metadata: Option<EventMetadata>,
    /// Response and media indices of the declared success stream.
    pub response_index: usize,
    pub media_index: usize,
    /// Declared media type of the stream response body.
    pub media_type: String,
    /// Bound for a single decoded item, from the compiled stream plan.
    pub max_item_bytes: usize,
    /// The stream media declaration, for decoding-failure branding.
    pub stream_source: SourceId,
    /// The item codec's schema source, for decoding-failure branding.
    pub item_source: SourceId,
    /// The declared response headers of the success stream, when any.
    pub headers_type: Option<String>,
    /// The response declaration source, for header branding.
    pub headers_source: SourceId,
}

/// Whether the compiled event set is discriminated: at least two declared
/// kinds, or one declared kind compiled from discrimination evidence whose
/// payload is a declared JSON structure. The single un-discriminated default
/// event keeps the existing untyped path byte-for-byte.
fn discriminated(stream: &wire::StreamOperationPlan) -> bool {
    match stream.events.as_slice() {
        [] => false,
        [only] => {
            only.event_name != wire::StreamEventPlan::DEFAULT_EVENT_NAME
                && matches!(only.payload, wire::EventPayload::Json { .. })
        }
        _ => true,
    }
}

/// Whether one compiled stream plan admits the typed event decode: SSE framing
/// whose discriminator is the parsed envelope's `event` field, and a
/// discriminated event set. Everything else keeps its existing untyped path.
fn admits(stream: &wire::StreamOperationPlan) -> bool {
    stream.framing == wire::StreamFraming::ServerSentEvents
        && stream
            .item_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.event_field.as_deref() == Some("event"))
        && discriminated(stream)
}

/// The shared planner's operation identity for one planned operation: the
/// operation id, or `METHOD /path` when unnamed (the pagination convention).
fn identity(op: &PlannedOperation) -> String {
    op.wire()
        .operation_id()
        .map(|located| located.value().clone())
        .unwrap_or_else(|| format!("{} {}", op.wire().method().as_str(), op.wire().path()))
}

/// The operation's single success stream media matching one compiled entry,
/// reported with its response/media indices and the planned response carrying
/// it. Mixed or multiple success media keep the conservative untyped path,
/// because the typed decode would otherwise own responses it cannot name.
fn stream_media<'a>(
    op: &'a PlannedOperation,
    compiled: &wire::StreamOperationPlan,
) -> Option<(
    usize,
    usize,
    &'a PlannedResponse,
    &'a PlannedResponseVariant,
)> {
    let mut found: Option<(usize, usize, &PlannedResponse, &PlannedResponseVariant)> = None;
    for (response_index, response) in op.responses.iter().enumerate() {
        if !matches!(
            response.wire.status(),
            wire::ResponseStatus::Exact(200..=299) | wire::ResponseStatus::Range(2)
        ) {
            continue;
        }
        for variant in &response.variants {
            let Some(media_index) = variant.media_index else {
                continue;
            };
            let media = &response.wire.media()[media_index];
            if !matches!(media.representation(), wire::Representation::Stream { .. }) {
                continue;
            }
            if media.media_type().declared() != compiled.media_type {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some((response_index, media_index, response, variant));
        }
    }
    found
}

/// Strip transparency and optionality wrappers from a planned field type.
fn unwrap_field(ty: &Type) -> Option<(&'static str, &Type)> {
    let mut wrapper = "";
    let mut inner = ty;
    loop {
        match inner {
            Type::Optional(next) if wrapper.is_empty() => {
                wrapper = "optional";
                inner = next;
            }
            Type::Presence(next) if wrapper.is_empty() => {
                wrapper = "presence";
                inner = next;
            }
            Type::Nullable(next) if wrapper.is_empty() => {
                wrapper = "nullable";
                inner = next;
            }
            Type::Boxed(next) => inner = next,
            _ => break,
        }
    }
    Some((wrapper, inner))
}

/// One declared envelope metadata member resolved against the item model's
/// planned fields; unresolvable shapes keep the member off the event type.
fn metadata_field(
    codecs: &CodecPlan,
    item_schema: &SchemaId,
    wire_name: &str,
) -> Option<EventMetadata> {
    let key = (item_schema.clone(), RepresentationRole::Model);
    let Decl::Struct { fields, .. } = codecs.models().declarations.get(&key)? else {
        return None;
    };
    let field = fields.iter().find(|field| field.wire == wire_name)?;
    let (wrapper, inner) = unwrap_field(&field.ty)?;
    let type_ = codecs.models().render_external_type(inner);
    let access = format!("item.{}", field.name);
    let conversion = match wrapper {
        "" => format!("std::option::Option::Some({access}.clone())"),
        "optional" => format!("{access}.clone()"),
        "presence" => format!(
            "match &{access} {{ crate::Presence::Value(value) => std::option::Option::Some(value.clone()), _ => std::option::Option::None }}"
        ),
        "nullable" => format!(
            "match &{access} {{ crate::Nullable::Value(value) => std::option::Option::Some(value.clone()), _ => std::option::Option::None }}"
        ),
        other => unreachable!("unknown wrapper {other}"),
    };
    Some(EventMetadata {
        field: field.name.clone(),
        type_,
        conversion,
    })
}

/// Whether one protocol operation admits the typed event decode for one
/// compiled stream entry: the entry exists for this identity, the shared
/// admission gate passes, exactly one success stream media matches, and the
/// item codec's model was planned. Used while operations are planned so
/// emitted client method names are reserved exactly when emission happens.
pub(super) fn admits_operation(
    protocol_op: &wire::OperationPlan,
    semantics: &wire::StreamSemanticsPlan,
    symbols: &BTreeMap<SchemaId, String>,
) -> bool {
    let identity = protocol_op
        .operation_id()
        .map(|located| located.value().clone())
        .unwrap_or_else(|| format!("{} {}", protocol_op.method().as_str(), protocol_op.path()));
    let Some(compiled) = semantics
        .streams
        .iter()
        .find(|stream| stream.operation == identity)
    else {
        return false;
    };
    if !admits(compiled) {
        return false;
    }
    let mut found = 0usize;
    for response in protocol_op.responses() {
        if !matches!(
            response.status(),
            wire::ResponseStatus::Exact(200..=299) | wire::ResponseStatus::Range(2)
        ) {
            continue;
        }
        for media in response.media() {
            if !matches!(media.representation(), wire::Representation::Stream { .. }) {
                continue;
            }
            if media.media_type().declared() != compiled.media_type {
                continue;
            }
            found += 1;
        }
    }
    if found != 1 {
        return false;
    }
    compiled
        .item_codec
        .as_ref()
        .is_some_and(|codec| symbols.contains_key(codec.schema().id()))
}

/// Lower the compiled stream semantics into one emission entry per admitted
/// operation, in plan order. Operations without a discriminated stream schema
/// produce nothing.
pub(super) fn prepare(
    semantics: &wire::StreamSemanticsPlan,
    operations: &[PlannedOperation],
    symbols: &BTreeMap<SchemaId, String>,
    codecs: &CodecPlan,
) -> Vec<StreamEventsEntry> {
    let mut result = Vec::new();
    for op in operations {
        let Some(compiled) = semantics
            .streams
            .iter()
            .find(|stream| stream.operation == identity(op))
        else {
            continue;
        };
        if !admits(compiled) {
            continue;
        }
        let Some((response_index, media_index, response, _variant)) = stream_media(op, compiled)
        else {
            continue;
        };
        let Some(item_codec) = &compiled.item_codec else {
            continue;
        };
        let item_schema = item_codec.schema().id();
        let Some(item_model) = symbols.get(item_schema) else {
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
        result.push(StreamEventsEntry {
            module: op.module_name.clone(),
            function: op.function_name.clone(),
            operation_id: op.operation_id.clone(),
            stem: crate::rust_models::pascal(&op.module_name),
            input_type: op.input_type.clone(),
            error_type: op.error_type.clone(),
            item_model: item_model.clone(),
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
            id_metadata: id_wire
                .as_deref()
                .and_then(|wire_name| metadata_field(codecs, item_schema, wire_name)),
            retry_metadata: retry_wire
                .as_deref()
                .and_then(|wire_name| metadata_field(codecs, item_schema, wire_name)),
            response_index,
            media_index,
            media_type: compiled.media_type.clone(),
            max_item_bytes: usize::try_from(compiled.max_item_bytes).unwrap_or(usize::MAX),
            stream_source: compiled.source.source().clone(),
            item_source: item_schema.clone(),
            headers_type: response.headers_type.clone(),
            headers_source: response.wire.source().use_site().source().clone(),
        });
    }
    result
}
