//! Generation-time typed-stream semantics planning (golden-defaults M4).
//!
//! [`plan`] compiles one semantic stream plan for every operation whose success
//! response carries a declared [`Representation::Stream`]. The wire framing and
//! the parsed item codec remain the protocol plan's decision (see
//! [`StreamPlan`]); this module adds the consumer semantics above that framing:
//! declared event kinds, typed payload decoding, the terminal sentinel policy,
//! completion behavior, the typed unknown-event alternative, fatal-event
//! marking, envelope metadata, and the informational frame rules each native
//! framing already implements.
//!
//! Every decision is evidence-based and recorded in the plan:
//!
//! - Event kinds come from the declared item schema only. A discriminating
//!   property (`event`, `type` or `kind`, compared without `_`/`-` separators,
//!   string-valued with `const`/`enum` values) or a `oneOf` whose variants each
//!   declare exactly one const/enum discriminator compiles one event per
//!   declared kind. Anything else compiles a single default event. Ambiguous
//!   evidence never guesses: it records an explanation instead.
//! - A `Json` payload means the parsed item is a declared JSON structure: the
//!   item schema itself, or the discriminated `oneOf` variant. A missing item
//!   schema keeps the raw frame text representable (`Text`), and an impossible
//!   item schema (`false`) compiles `Empty`. Nested JSON inside an envelope
//!   field (including `contentSchema` annotations) is not decoded; the protocol
//!   plan's envelope codec stays the single declared JSON input.
//! - The `[DONE]` sentinel is an API convention rather than a universal SSE
//!   rule, so it is enabled only by evidence: an `x-` annotation whose key
//!   names a sentinel, or a description sentence declaring a `[TOKEN]` next to
//!   stream-termination wording. The evidence source is recorded on the policy.
//! - Fatal events are undeclared unless an `x-` annotation whose key names
//!   fatal events, or a description sentence naming the kind next to
//!   error/fatal wording, marks the kind.
//! - The protocol plan is atomic: when it is not admitted it carries no
//!   operations, so declared streams compile conservatively from the contract
//!   (no validated item codec, `Text` payloads) with an explanation, and
//!   consumers never lose the declaration entirely.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;
use suspect_ir::contract::{Contract, Operation, SchemaId, SourceId};

use super::SourceLocation;
use super::model::{
    CodecInput, CodecRef, Located, MediaPlan, OperationPlan, ProtocolPlan, Representation,
    ResponsePlan, ResponseStatus, SchemaUse, StreamFraming, StreamPlan,
};

/// Complete generation-time typed-stream planning for one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct StreamSemanticsPlan {
    /// One entry per operation stream media, in protocol-plan operation order.
    pub streams: Vec<StreamOperationPlan>,
}

/// One operation's compiled stream semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamOperationPlan {
    /// Operation identifier, or `METHOD /path` when unnamed (the pagination
    /// identity convention).
    pub operation: String,
    /// Declared sequential media type of the stream response body.
    pub media_type: String,
    /// The native framing the protocol plan selected for this media type.
    pub framing: StreamFraming,
    /// The declared OAS 3.2 `itemSchema` codec. Always present for an admitted
    /// stream declaration; absent for the conservative unadmitted plan.
    pub item_codec: Option<CodecRef>,
    /// Declared event kinds in declaration order; a single default event when
    /// no discrimination evidence exists.
    pub events: Vec<StreamEventPlan>,
    pub sentinel: SentinelPolicy,
    pub terminal: TerminalPolicy,
    /// Future event types surface through a typed unknown-event alternative;
    /// invalid payloads of recognized kinds remain decoding failures.
    pub unknown_events: UnknownEventPolicy,
    /// Informational framing facts derived only from the media framing; they
    /// document what the native framings already implement.
    pub frame_rules: FrameRules,
    /// Declared envelope metadata fields (SSE `event`/`id`/`retry` property
    /// names) when the item schema declares any; JSON lines carry none.
    pub item_metadata: Option<StreamItemMetadata>,
    /// Bound for a single decoded item, from the adapter's stream-item limit.
    pub max_item_bytes: u64,
    /// Evidence notes for every non-mechanical decision.
    pub explanations: Vec<String>,
    /// Location of the stream media declaration.
    pub source: SourceLocation,
}

/// One declared event kind and how its frames decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamEventPlan {
    /// The declared event kind value, or [`StreamEventPlan::DEFAULT_EVENT_NAME`]
    /// for the single default kind compiled without discrimination evidence.
    pub event_name: String,
    pub payload: EventPayload,
    /// Protocol-declared fatal events versus ordinary application events. The
    /// v1 rule: undeclared unless an `x-` fatal annotation or a description
    /// sentence names the kind next to error/fatal wording.
    pub fatal: bool,
}
impl StreamEventPlan {
    /// The kind name compiled when no discrimination evidence exists, or when
    /// discrimination is ambiguous.
    pub const DEFAULT_EVENT_NAME: &'static str = "default";

    fn default_event(payload: EventPayload) -> Self {
        Self {
            event_name: Self::DEFAULT_EVENT_NAME.to_owned(),
            payload,
            fatal: false,
        }
    }
}

/// How one event kind's frame data decodes. The payload is the parsed item:
/// the SSE envelope object, or the JSON-lines record.
// The public typed plan retains the shared, unboxed codec reference style.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum EventPayload {
    /// The item schema declares the structure; frames decode with the
    /// referenced codec (the item schema, or a discriminated variant).
    Json { codec: CodecRef },
    /// No item schema is declared; the frame data stays raw UTF-8 text.
    Text,
    /// The declared item schema admits no item, so frames carry no data.
    Empty,
}

/// The terminal sentinel policy. `[DONE]` is an API convention rather than a
/// universal SSE rule, so enabling always requires recorded source evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SentinelPolicy {
    /// Enabled only when the operation/response description or an explicit
    /// `x-` annotation declares a terminal sentinel.
    pub enabled: bool,
    /// The exact wire token compared against the frame data before JSON
    /// decoding; empty while disabled.
    pub token: String,
    pub stage: SentinelStage,
    /// Where the enabling evidence was found.
    pub evidence: Option<SentinelEvidence>,
}

/// Sentinels are matched on the raw frame data, before JSON decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SentinelStage {
    BeforeJsonDecode,
}

/// The recorded evidence that enables a sentinel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SentinelEvidence {
    /// An `x-` annotation whose key names a sentinel carried the token.
    Annotation { source: SourceLocation },
    /// A description sentence declares the token next to stream-termination
    /// wording.
    Description { source: SourceLocation },
}

/// How the stream completes, and which final frames are preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TerminalPolicy {
    /// A declared sentinel completes the stream.
    pub on_sentinel: TerminalAction,
    /// End of the response body completes the stream.
    pub on_eof: TerminalAction,
    /// When sentinel evidence exists, the last data frame before the
    /// sentinel/EOF is the final usage/completion frame and is preserved as
    /// terminal rather than surfaced as an ordinary event.
    pub keep_final_usage: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalAction {
    Complete,
}

/// Future event types surface through a typed unknown-event alternative while
/// invalid payloads of recognized kinds remain decoding failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct UnknownEventPolicy {
    pub representation: UnknownEventRepresentation,
    pub invalid_payload: InvalidPayloadPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnknownEventRepresentation {
    TypedUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvalidPayloadPolicy {
    DecodingFailure,
}

/// Informational framing facts, derived only from the media framing. They
/// describe what the native framings already implement; no source text or
/// schema keyword changes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FrameRules {
    /// SSE: lines beginning with `:` are comments and change no state.
    pub comments_ignored: bool,
    /// SSE: repeated `data` lines join with `\n` into one data value.
    pub multiline_data_joined: bool,
    /// SSE: `event`/`id`/`data`/`retry` field order is independent.
    pub field_order_independent: bool,
    /// SSE: a blank line dispatches the event.
    pub blank_line_delimits: bool,
    /// JSON lines: each line carries exactly one JSON item.
    pub line_delimited: bool,
    /// End of the response body completes the stream.
    pub eof_completes: bool,
}
impl FrameRules {
    fn for_framing(framing: StreamFraming) -> Self {
        match framing {
            StreamFraming::ServerSentEvents => Self {
                comments_ignored: true,
                multiline_data_joined: true,
                field_order_independent: true,
                blank_line_delimits: true,
                line_delimited: false,
                eof_completes: true,
            },
            StreamFraming::JsonLines => Self {
                comments_ignored: false,
                multiline_data_joined: false,
                field_order_independent: false,
                blank_line_delimits: false,
                line_delimited: true,
                eof_completes: true,
            },
        }
    }
}

/// Declared envelope metadata fields, named by the item schema property that
/// carries them. Native runtimes expose these per event (the event kind name,
/// the last-event id, the reconnection hint in milliseconds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamItemMetadata {
    pub event_field: Option<String>,
    pub id_field: Option<String>,
    pub retry_field: Option<String>,
}

/// Compile one semantic stream plan per declared operation stream media.
///
/// Infallible: the protocol plan's own diagnostics already cover admission, and
/// every non-mechanical decision here carries an explanation on the plan.
pub fn plan(contract: &Contract, protocol: &ProtocolPlan) -> StreamSemanticsPlan {
    let mut streams = Vec::new();
    if protocol.is_admitted() {
        for operation in protocol.operations() {
            for response in operation.responses() {
                if !is_success(response.status()) {
                    continue;
                }
                for media in response.media() {
                    let Representation::Stream { stream } = media.representation() else {
                        continue;
                    };
                    streams.push(compile_admitted(
                        contract, operation, response, media, stream,
                    ));
                }
            }
        }
    } else {
        // The plan carries no operations, so declared client streams compile
        // conservatively straight from the contract, with an explanation.
        for operation in contract.operations() {
            streams.extend(compile_unadmitted(
                contract,
                operation,
                protocol.capabilities().limits().stream_item(),
            ));
        }
    }
    StreamSemanticsPlan { streams }
}

fn compile_admitted(
    contract: &Contract,
    operation: &OperationPlan,
    response: &ResponsePlan,
    media: &MediaPlan,
    stream: &StreamPlan,
) -> StreamOperationPlan {
    let framing = stream.framing();
    let item_codec = stream.item_codec().cloned();
    let (mut events, metadata, mut explanations) =
        compile_items(contract, item_codec.as_ref(), framing);
    if item_codec.is_none() {
        // Schemaless admission compiles exactly the conservative unadmitted
        // text plan: no validated codec exists, so frame text stays untyped.
        explanations.insert(
            0,
            "schemaless-stream-events-v1 admits this schemaless stream with untyped frames; no item schema was declared, so no item codec was compiled".to_owned(),
        );
    }
    let annotations = operation_extensions(operation.annotations());
    let operation_description = operation.description();
    let response_description = response.declared_description();
    let sentinel = sentinel_policy(
        operation_description.map(|located| (located.source(), located.value().as_str())),
        response_description.map(|located| (located.source(), located.value().as_str())),
        &annotations,
    );
    note_sentinel(&mut explanations, &sentinel);
    mark_fatal(
        &mut events,
        operation_description.map(|located| located.value().as_str()),
        response_description.map(|located| located.value().as_str()),
        &annotations,
    );
    let terminal = TerminalPolicy {
        on_sentinel: TerminalAction::Complete,
        on_eof: TerminalAction::Complete,
        keep_final_usage: sentinel.enabled,
    };
    StreamOperationPlan {
        operation: operation_identity(operation),
        media_type: media.media_type().declared().to_owned(),
        framing,
        item_codec,
        events,
        sentinel,
        terminal,
        unknown_events: UnknownEventPolicy {
            representation: UnknownEventRepresentation::TypedUnknown,
            invalid_payload: InvalidPayloadPolicy::DecodingFailure,
        },
        frame_rules: FrameRules::for_framing(framing),
        item_metadata: metadata,
        max_item_bytes: stream.max_item_bytes(),
        explanations,
        source: stream.source().clone(),
    }
}

/// The conservative plan compiled from raw declarations when the protocol plan
/// is not admitted: no validated codec exists, so frame text stays untyped.
fn compile_unadmitted(
    contract: &Contract,
    operation: Operation<'_>,
    max_item_bytes: u64,
) -> Vec<StreamOperationPlan> {
    let raw = operation.raw();
    let operation_source = operation.source().clone();
    let identity = operation
        .operation_id()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "{} {}",
                operation.method().as_str(),
                operation.path_template().unwrap_or_default()
            )
        });
    let operation_description = operation.description().map(str::to_owned);
    let operation_description_location =
        location_of(contract, &operation_source.child("description"));
    let annotations: Vec<Extension> = raw
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| key.starts_with("x-"))
        .map(|(key, value)| Extension {
            key: key.clone(),
            value: value.clone(),
            source: location_of(contract, &operation_source.child(key)),
        })
        .collect();
    let mut streams = Vec::new();
    for (status_key, _) in raw
        .get("responses")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if !success_status_key(status_key) {
            continue;
        }
        let response_source = resolve_declaration(
            contract,
            &operation_source.child("responses").child(status_key),
        );
        let Some(response_raw) = contract.source(&response_source) else {
            continue;
        };
        let response_description = response_raw
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let response_description_location =
            location_of(contract, &response_source.child("description"));
        let Some(content) = response_raw.get("content").and_then(Value::as_object) else {
            continue;
        };
        for media_key in content.keys() {
            let Some(framing) = stream_framing(media_key) else {
                continue;
            };
            let (mut events, _, mut explanations) = compile_items(contract, None, framing);
            explanations.insert(
                0,
                "the protocol plan is not admitted, so no stream codec was compiled; this conservative plan keeps frame text untyped"
                    .to_owned(),
            );
            let sentinel = sentinel_policy(
                operation_description
                    .as_deref()
                    .map(|text| (&operation_description_location, text)),
                response_description
                    .as_deref()
                    .map(|text| (&response_description_location, text)),
                &annotations,
            );
            note_sentinel(&mut explanations, &sentinel);
            mark_fatal(
                &mut events,
                operation_description.as_deref(),
                response_description.as_deref(),
                &annotations,
            );
            let terminal = TerminalPolicy {
                on_sentinel: TerminalAction::Complete,
                on_eof: TerminalAction::Complete,
                keep_final_usage: sentinel.enabled,
            };
            streams.push(StreamOperationPlan {
                operation: identity.clone(),
                media_type: media_key.clone(),
                framing,
                item_codec: None,
                events,
                sentinel,
                terminal,
                unknown_events: UnknownEventPolicy {
                    representation: UnknownEventRepresentation::TypedUnknown,
                    invalid_payload: InvalidPayloadPolicy::DecodingFailure,
                },
                frame_rules: FrameRules::for_framing(framing),
                item_metadata: None,
                max_item_bytes,
                explanations,
                source: location_of(contract, &response_source.child("content").child(media_key)),
            });
        }
    }
    streams
}

/// Compile the declared event kinds, envelope metadata, and evidence notes for
/// one item schema (or their conservative absence).
fn compile_items(
    contract: &Contract,
    item_codec: Option<&CodecRef>,
    framing: StreamFraming,
) -> (
    Vec<StreamEventPlan>,
    Option<StreamItemMetadata>,
    Vec<String>,
) {
    let Some(codec) = item_codec else {
        return (
            vec![StreamEventPlan::default_event(EventPayload::Text)],
            None,
            vec!["no item schema is declared; frame text stays an untyped text payload".to_owned()],
        );
    };
    let Some((terminal, raw)) = resolve(contract, codec.schema().id()) else {
        return (
            vec![StreamEventPlan::default_event(EventPayload::Json {
                codec: codec.clone(),
            })],
            None,
            vec!["the item schema does not resolve to an indexed schema; compiled a single default event with the declared item codec".to_owned()],
        );
    };
    if raw == &Value::Bool(false) {
        return (
            vec![StreamEventPlan::default_event(EventPayload::Empty)],
            None,
            vec!["the item schema declares no possible item; frames carry no data".to_owned()],
        );
    }
    let mut explanations = Vec::new();
    if let Some(variants) = raw.get("oneOf").and_then(Value::as_array) {
        return match variant_events(contract, &terminal, variants, codec) {
            VariantEvents::Compiled { events } => {
                explanations.push("each oneOf variant declares one const/enum event discriminator; compiled one event per variant kind".to_owned());
                (
                    events,
                    metadata(contract, &terminal, raw, framing),
                    explanations,
                )
            }
            VariantEvents::Ambiguous { reason } => {
                explanations.push(reason);
                (
                    default_json_event(codec),
                    metadata(contract, &terminal, raw, framing),
                    explanations,
                )
            }
        };
    }
    match discriminator(contract, &terminal, raw) {
        Discriminator::Kinds { property, kinds } => {
            explanations.push(format!(
                "the discriminator property {property:?} declares the event kinds; compiled one event per declared kind"
            ));
            let events = kinds
                .into_iter()
                .map(|kind| StreamEventPlan {
                    event_name: kind,
                    payload: EventPayload::Json {
                        codec: codec.clone(),
                    },
                    fatal: false,
                })
                .collect();
            (
                events,
                metadata(contract, &terminal, raw, framing),
                explanations,
            )
        }
        Discriminator::Ambiguous(properties) => {
            explanations.push(format!(
                "event discrimination through {properties:?} is ambiguous, so the declared kinds were not assigned; compiled a single default event instead of guessing"
            ));
            (
                default_json_event(codec),
                metadata(contract, &terminal, raw, framing),
                explanations,
            )
        }
        Discriminator::None => {
            explanations.push(
                "the item schema declares no event discrimination; compiled a single default event"
                    .to_owned(),
            );
            (
                default_json_event(codec),
                metadata(contract, &terminal, raw, framing),
                explanations,
            )
        }
    }
}

fn default_json_event(codec: &CodecRef) -> Vec<StreamEventPlan> {
    vec![StreamEventPlan::default_event(EventPayload::Json {
        codec: codec.clone(),
    })]
}

enum VariantEvents {
    Compiled { events: Vec<StreamEventPlan> },
    Ambiguous { reason: String },
}

/// One event per `oneOf` variant, each discriminated by exactly one declared
/// const/enum kind and decoded with that variant's declared schema.
fn variant_events(
    contract: &Contract,
    terminal: &SchemaId,
    variants: &[Value],
    item_codec: &CodecRef,
) -> VariantEvents {
    let mut events = Vec::new();
    let mut seen = BTreeSet::new();
    for index in 0..variants.len() {
        let id = terminal.child("oneOf").child(&index.to_string());
        if contract.schema(&id).is_none() {
            return VariantEvents::Ambiguous {
                reason: "the oneOf variants are not individually indexed; compiled a single default event instead of guessing"
                    .to_owned(),
            };
        }
        let Some((variant_terminal, variant_raw)) = resolve(contract, &id) else {
            return VariantEvents::Ambiguous {
                reason: "a oneOf variant does not resolve to an indexed schema; compiled a single default event instead of guessing"
                    .to_owned(),
            };
        };
        match discriminator(contract, &variant_terminal, variant_raw) {
            Discriminator::Kinds { kinds, .. } if kinds.len() == 1 => {
                let kind = kinds.into_iter().next().unwrap_or_default();
                if !seen.insert(kind.clone()) {
                    return VariantEvents::Ambiguous {
                        reason: "several oneOf variants declare the same event kind; discrimination is ambiguous"
                            .to_owned(),
                    };
                }
                events.push(StreamEventPlan {
                    event_name: kind,
                    payload: EventPayload::Json {
                        codec: CodecRef {
                            schema: SchemaUse {
                                id: id.clone(),
                                source: item_codec.schema().source().clone(),
                            },
                            input: CodecInput::Json,
                        },
                    },
                    fatal: false,
                });
            }
            _ => {
                return VariantEvents::Ambiguous {
                    reason: "the oneOf variants do not each declare exactly one const/enum event discriminator; compiled a single default event instead of guessing"
                        .to_owned(),
                };
            }
        }
    }
    if events.is_empty() {
        return VariantEvents::Ambiguous {
            reason: "the oneOf declares no variants; compiled a single default event".to_owned(),
        };
    }
    VariantEvents::Compiled { events }
}

enum Discriminator {
    /// No property carries discriminator evidence.
    None,
    /// Several properties could discriminate; never guess between them.
    Ambiguous(Vec<String>),
    /// One property with a non-empty set of unique declared string kinds.
    Kinds {
        property: String,
        kinds: Vec<String>,
    },
}

/// The event/type/kind-like property names recognized as discriminators,
/// compared without `_`/`-` separators.
const DISCRIMINATOR_NAMES: &[&str] = &["event", "type", "kind"];

fn normalize(name: &str) -> String {
    name.chars()
        .map(|character| character.to_ascii_lowercase())
        .filter(|character| *character != '_' && *character != '-')
        .collect()
}

fn discriminator(contract: &Contract, schema_id: &SchemaId, raw: &Value) -> Discriminator {
    let Some(properties) = raw.get("properties").and_then(Value::as_object) else {
        return Discriminator::None;
    };
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();
    for name in properties.keys() {
        if !DISCRIMINATOR_NAMES.contains(&normalize(name).as_str()) {
            continue;
        }
        let Some((_, value)) = resolve(contract, &schema_id.child("properties").child(name)) else {
            continue;
        };
        let Some(kinds) = declared_kinds(value) else {
            continue;
        };
        if kinds.iter().collect::<BTreeSet<_>>().len() != kinds.len() {
            // Repeated values cannot be assigned to one kind each.
            candidates.push((name.clone(), Vec::new()));
            continue;
        }
        candidates.push((name.clone(), kinds));
    }
    match candidates.as_slice() {
        [] => Discriminator::None,
        [(property, kinds)] if !kinds.is_empty() => Discriminator::Kinds {
            property: property.clone(),
            kinds: kinds.clone(),
        },
        [(property, _)] => Discriminator::Ambiguous(vec![property.clone()]),
        many => {
            Discriminator::Ambiguous(many.iter().map(|(property, _)| property.clone()).collect())
        }
    }
}

/// `const`/`enum` string values of a discriminator property schema; `None`
/// when the property declares no usable string kinds.
fn declared_kinds(value: &Value) -> Option<Vec<String>> {
    if let Some(declared) = value.get("const").and_then(Value::as_str) {
        return (!declared.is_empty()).then(|| vec![declared.to_owned()]);
    }
    let values = value.get("enum").and_then(Value::as_array)?;
    let kinds: Option<Vec<String>> = values
        .iter()
        .map(|value| {
            let kind = value.as_str()?;
            (!kind.is_empty()).then(|| kind.to_owned())
        })
        .collect();
    kinds.filter(|kinds| !kinds.is_empty())
}

/// The declared envelope metadata properties, united over the item schema and
/// its `oneOf` variants. Only the SSE field names count; JSON lines carry no
/// envelope metadata.
fn metadata(
    contract: &Contract,
    terminal: &SchemaId,
    raw: &Value,
    framing: StreamFraming,
) -> Option<StreamItemMetadata> {
    if framing != StreamFraming::ServerSentEvents {
        return None;
    }
    let mut names = BTreeSet::new();
    if let Some(properties) = raw.get("properties").and_then(Value::as_object) {
        names.extend(properties.keys().cloned());
    }
    if let Some(variants) = raw.get("oneOf").and_then(Value::as_array) {
        for index in 0..variants.len() {
            if let Some((_, variant_raw)) =
                resolve(contract, &terminal.child("oneOf").child(&index.to_string()))
                && let Some(properties) = variant_raw.get("properties").and_then(Value::as_object)
            {
                names.extend(properties.keys().cloned());
            }
        }
    }
    let declared = |field: &str| names.contains(field).then(|| field.to_owned());
    let item_metadata = StreamItemMetadata {
        event_field: declared("event"),
        id_field: declared("id"),
        retry_field: declared("retry"),
    };
    (item_metadata.event_field.is_some()
        || item_metadata.id_field.is_some()
        || item_metadata.retry_field.is_some())
    .then_some(item_metadata)
}

/// An operation `x-` extension with its own source location.
struct Extension {
    key: String,
    value: Value,
    source: SourceLocation,
}

fn operation_extensions(annotations: &[Located<Value>]) -> Vec<Extension> {
    annotations
        .iter()
        .filter_map(|located| {
            let segment = located.source().source().pointer().rsplit('/').next()?;
            if !segment.starts_with("x-") {
                return None;
            }
            Some(Extension {
                key: segment.replace("~1", "/").replace("~0", "~"),
                value: located.value().clone(),
                source: located.source().clone(),
            })
        })
        .collect()
}

/// Wording that, in the same sentence as a `[TOKEN]`, declares a terminal
/// sentinel. Bare mentions of a token are never sufficient.
const TERMINAL_PHRASES: &[&str] = &[
    "terminat",
    "sentinel",
    "ends the stream",
    "end of the stream",
    "end of stream",
    "stream ends",
    "completes the stream",
    "stream completes",
    "closes the stream",
    "stream closes",
    "final frame",
    "last frame",
];

fn sentinel_policy(
    operation_description: Option<(&SourceLocation, &str)>,
    response_description: Option<(&SourceLocation, &str)>,
    annotations: &[Extension],
) -> SentinelPolicy {
    for extension in annotations {
        if !extension.key.to_ascii_lowercase().contains("sentinel") {
            continue;
        }
        if let Some(token) = extension
            .value
            .as_str()
            .map(str::trim)
            .filter(|token| !token.is_empty() && token.len() <= 64)
        {
            return SentinelPolicy {
                enabled: true,
                token: token.to_owned(),
                stage: SentinelStage::BeforeJsonDecode,
                evidence: Some(SentinelEvidence::Annotation {
                    source: extension.source.clone(),
                }),
            };
        }
    }
    for (source, description) in operation_description
        .into_iter()
        .chain(response_description)
    {
        for sentence in sentences(description) {
            let lowered = sentence.to_ascii_lowercase();
            if TERMINAL_PHRASES
                .iter()
                .any(|phrase| lowered.contains(phrase))
                && let Some(token) = bracketed_token(sentence)
            {
                return SentinelPolicy {
                    enabled: true,
                    token,
                    stage: SentinelStage::BeforeJsonDecode,
                    evidence: Some(SentinelEvidence::Description {
                        source: source.clone(),
                    }),
                };
            }
        }
    }
    SentinelPolicy {
        enabled: false,
        token: String::new(),
        stage: SentinelStage::BeforeJsonDecode,
        evidence: None,
    }
}

fn note_sentinel(explanations: &mut Vec<String>, sentinel: &SentinelPolicy) {
    let Some(evidence) = &sentinel.evidence else {
        return;
    };
    let place = match evidence {
        SentinelEvidence::Annotation { .. } => "by an extension annotation",
        SentinelEvidence::Description { .. } => "in a description",
    };
    explanations.push(format!(
        "the terminal sentinel {:?} is declared {}, so it is matched on frame data before JSON decoding",
        sentinel.token, place
    ));
}

/// Wording that, in the same sentence as an event kind name, marks that kind
/// fatal rather than an ordinary application event.
const FATAL_PHRASES: &[&str] = &[
    "fatal",
    "is an error",
    "error event",
    "indicates an error",
    "signals an error",
    "failure",
];

fn mark_fatal(
    events: &mut [StreamEventPlan],
    operation_description: Option<&str>,
    response_description: Option<&str>,
    annotations: &[Extension],
) {
    let mut fatal: BTreeSet<String> = annotations
        .iter()
        .filter(|extension| extension.key.to_ascii_lowercase().contains("fatal"))
        .flat_map(|extension| match &extension.value {
            Value::String(name) => vec![name.clone()],
            Value::Array(names) => names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    for description in [operation_description, response_description]
        .into_iter()
        .flatten()
    {
        for sentence in sentences(description) {
            if !FATAL_PHRASES
                .iter()
                .any(|phrase| sentence.to_ascii_lowercase().contains(phrase))
            {
                continue;
            }
            let lowered = sentence.to_ascii_lowercase();
            for event in events.iter() {
                if fatal.contains(&event.event_name) {
                    continue;
                }
                if is_whole_word(&lowered, &event.event_name.to_ascii_lowercase()) {
                    fatal.insert(event.event_name.clone());
                }
            }
        }
    }
    for event in events.iter_mut() {
        event.fatal = fatal.contains(&event.event_name);
    }
}

fn sentences(text: &str) -> impl Iterator<Item = &str> {
    text.split(['.', ';', '!', '?', '\n'])
        .map(str::trim)
        .filter(|sentence| !sentence.is_empty())
}

/// Whole-word (case-normalized) containment, so kind `done` does not match
/// inside `randomized`.
fn is_whole_word(text: &str, word: &str) -> bool {
    let word_boundary = |character: char| !character.is_ascii_alphanumeric() && character != '_';
    if word.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(found) = text[start..].find(word) {
        let at = start + found;
        let after = at + word.len();
        if text[..at].chars().next_back().is_none_or(word_boundary)
            && text[after..].chars().next().is_none_or(word_boundary)
        {
            return true;
        }
        start = after;
    }
    false
}

/// The first `[TOKEN]` of the sentence, brackets included, whose content is a
/// short printable run without whitespace or nested brackets.
fn bracketed_token(sentence: &str) -> Option<String> {
    for (start, byte) in sentence.bytes().enumerate() {
        if byte != b'[' {
            continue;
        }
        let Some(close) = sentence[start + 1..]
            .find(']')
            .map(|relative| start + 1 + relative)
        else {
            continue;
        };
        let inner = &sentence[start + 1..close];
        let acceptable = !inner.is_empty()
            && inner.len() <= 32
            && !inner
                .chars()
                .any(|character| character.is_whitespace() || character == '[' || character == ']');
        if acceptable {
            return Some(sentence[start..=close].to_owned());
        }
    }
    None
}

fn is_success(status: ResponseStatus) -> bool {
    matches!(
        status,
        ResponseStatus::Exact(200..=299) | ResponseStatus::Range(2)
    )
}

/// The success status keys accepted for unadmitted raw declarations.
fn success_status_key(key: &str) -> bool {
    if key.eq_ignore_ascii_case("2XX") {
        return true;
    }
    key.parse::<u16>()
        .is_ok_and(|status| (200..=299).contains(&status))
}

/// The stream media essences the protocol planner recognizes; unadmitted raw
/// declarations use the same set so both paths agree on what a stream is.
fn stream_framing(declared: &str) -> Option<StreamFraming> {
    let essence = declared
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "text/event-stream" => Some(StreamFraming::ServerSentEvents),
        "application/jsonl" | "application/x-ndjson" => Some(StreamFraming::JsonLines),
        _ => None,
    }
}

/// Operation display identity: operation id, or `METHOD /path`.
fn operation_identity(operation: &OperationPlan) -> String {
    operation
        .operation_id()
        .map(|located| located.value().clone())
        .unwrap_or_else(|| format!("{} {}", operation.method().as_str(), operation.path()))
}

/// Follow `$ref` chains on a raw HTTP declaration (responses may be shared
/// component objects). An unresolved chain keeps the declaration site.
fn resolve_declaration(contract: &Contract, source: &SourceId) -> SourceId {
    let mut current = source.clone();
    for _ in 0..64 {
        let Some(raw) = contract.source(&current) else {
            return current;
        };
        if raw.get("$ref").is_none() {
            return current;
        }
        match contract.reference_target(&current) {
            Some(target) => current = target.clone(),
            None => return current,
        }
    }
    current
}

/// Follow indexed `$ref` edges to one terminal, non-reference schema value.
fn resolve<'a>(contract: &'a Contract, id: &SchemaId) -> Option<(SchemaId, &'a Value)> {
    let mut current = id.clone();
    for _ in 0..64 {
        let schema = contract.schema(&current)?;
        let raw = schema.raw();
        if raw.get("$ref").is_some() {
            current = schema
                .references()
                .iter()
                .find(|reference| reference.keyword == "$ref")
                .and_then(|reference| reference.target.clone())?;
        } else {
            return Some((current, raw));
        }
    }
    None
}

fn location_of(contract: &Contract, source: &SourceId) -> SourceLocation {
    SourceLocation {
        source: source.clone(),
        span: contract.source_span(source).unwrap_or(0..0),
    }
}
