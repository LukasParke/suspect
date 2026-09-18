//! Emitted-only typed SSE event decode for the native C++ client.
//!
//! The shared, generation-time `http_protocol::plan_stream_semantics` outcome
//! lowers into one generated `include/<package>/stream_events.hpp` holding,
//! per discriminated SSE stream operation: the per-kind event payload structs,
//! the typed unknown alternative, the completion metadata and a RAII pull
//! typed-event pager with `next`/`value`/`completion`/`error`, plus matching
//! methods on the generated `Client` (declared in the emitted `client.hpp` only
//! when pagers exist; the pager classes are forward declared there and defined
//! here).
//!
//! The existing untyped stream iteration stays the transport: the pager's first
//! pull opens the exchange through a generated client method that reuses the
//! direct call's exact request preparation, status matching and typed failure
//! surface, and then applies the compiled per-kind decode, sentinel and
//! completion semantics itself over the runtime's parsed envelopes. Recognized
//! event kinds decode through the operation's existing stream item codec into
//! their declared model type; undeclared event kinds surface through the typed
//! unknown-event alternative without failing the stream; invalid payloads of
//! recognized kinds remain branded decoding errors. Static runtime files are
//! never modified, and operations without a discriminated SSE stream schema
//! emit nothing at all.

use std::collections::BTreeSet;

use super::emit::string;
use super::models::{allocate, pascal, ModelPlan};
use super::protocol::{PlannedOperation, PlannedResponseCase, ValueKind};
use super::SdkPlan;
use crate::http_protocol as wire;
use crate::http_protocol::{EventPayload, Representation, StreamEventPlan, StreamFraming};
use suspect_ir::contract::SourceId;

/// One discriminated SSE stream operation's compiled emission.
#[derive(Debug, Clone)]
pub struct StreamEventsEntry {
    /// Source identity of the streamed operation.
    pub operation: String,
    pub source: SourceId,
    /// The generated client method this pager extends.
    pub method: String,
    pub input_type: String,
    pub error_type: String,
    /// The declared model every recognized kind's envelope decodes into.
    pub item_type: String,
    /// The stream item model's codec index, for the typed decode and the
    /// validation root, mirroring the untyped item stream's decode.
    pub item_index: usize,
    /// Declared event kinds in declaration order.
    pub kinds: Vec<String>,
    /// The declared terminal sentinel token, matched on the frame data before
    /// any payload decoding.
    pub sentinel: Option<String>,
    /// Whether the last data frame before the sentinel or end of body is
    /// preserved as the completion's terminal usage instead of being delivered.
    pub keep_final_usage: bool,
    /// Whether the declared envelope metadata carries the last-event id.
    pub id_metadata: bool,
    /// Whether the declared envelope metadata carries the reconnection hint.
    pub retry_metadata: bool,
    /// Allocated, collision-free public type names.
    pub event: String,
    pub completion: String,
    pub pager: String,
    /// Allocated client method names: the pager factory and the exchange
    /// opener the pager consumes.
    pub events_method: String,
    pub open_method: String,
    /// The per-kind decoded payload struct names, in declaration order.
    pub kind_types: Vec<String>,
    pub unknown_type: String,
}

/// The compiled typed-stream emission carried by one plan.
#[derive(Debug, Clone)]
pub struct StreamEventsPlan {
    /// The shared, infallible stream-semantics selection this emission follows.
    pub semantics: wire::StreamSemanticsPlan,
    pub operations: Vec<StreamEventsEntry>,
}

impl StreamEventsPlan {
    /// Whether this plan emits typed event pagers at all.
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
    stream.framing == StreamFraming::ServerSentEvents
        && stream
            .item_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.event_field.as_deref() == Some("event"))
        && discriminated(stream)
}

/// The operation's single success response whose ordinary body is exactly one
/// declared stream media matching one compiled entry, with no error response
/// declaring a stream body. Mixed or multiple success media keep the
/// conservative untyped path, because the typed decode would otherwise own
/// responses it cannot name.
fn stream_target<'a>(
    operation: &'a PlannedOperation,
    compiled: &wire::StreamOperationPlan,
) -> Option<(&'a PlannedResponseCase, &'a wire::MediaPlan)> {
    let successes: Vec<&super::protocol::PlannedResponse> = operation
        .responses
        .iter()
        .filter(|response| response.can_succeed())
        .collect();
    if successes.len() != 1 {
        return None;
    }
    if operation.responses.iter().filter(|r| r.can_fail()).any(|r| {
        r.cases.iter().any(|case| {
            matches!(
                case.value.kind,
                ValueKind::Stream {
                    framing: StreamFraming::ServerSentEvents,
                    ..
                }
            )
        })
    }) {
        return None;
    }
    let response = successes[0];
    let ordinary: Vec<&PlannedResponseCase> =
        response.cases.iter().filter(|case| !case.forbidden).collect();
    if ordinary.len() != 1 {
        return None;
    }
    let media = ordinary[0].media.as_ref()?;
    if !matches!(media.representation(), Representation::Stream { .. }) {
        return None;
    }
    if media.media_type().declared() != compiled.media_type {
        return None;
    }
    Some((ordinary[0], media))
}

/// Lower the compiled stream-semantics selection for one plan. Operations
/// whose compiled selection cannot be expressed through the direct call's
/// single success stream response are left without pagers instead of emitting
/// guesses.
pub(super) fn lower(
    models: &ModelPlan,
    semantics: &wire::StreamSemanticsPlan,
    operations: &[PlannedOperation],
    names: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> StreamEventsPlan {
    let mut compiled = Vec::new();
    for stream in &semantics.streams {
        let Some(operation) = operations
            .iter()
            .find(|operation| operation.operation_id == stream.operation)
        else {
            continue;
        };
        if !admits_typed_events(stream) {
            continue;
        }
        let Some((_, _media)) = stream_target(operation, stream) else {
            continue;
        };
        let Some(item_codec) = &stream.item_codec else {
            continue;
        };
        let Some(symbol) = models.symbol(item_codec.schema().id()) else {
            continue;
        };
        let stem = pascal(&operation.method_name);
        let kind_types = stream
            .events
            .iter()
            .map(|event| allocate(&format!("{stem}{}EventData", pascal(&event.event_name)), names))
            .collect();
        compiled.push(StreamEventsEntry {
            operation: operation.operation_id.clone(),
            source: operation.source.clone(),
            method: operation.method_name.clone(),
            input_type: operation.input_type.clone(),
            error_type: operation.error_type.clone(),
            item_type: symbol.cpp_type.clone(),
            item_index: symbol.index,
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
            event: allocate(&format!("{stem}Event"), names),
            completion: allocate(&format!("{stem}Completion"), names),
            pager: allocate(&format!("{stem}EventsPager"), names),
            events_method: allocate(&format!("{}_events", operation.method_name), methods),
            open_method: allocate(&format!("{}_events_stream", operation.method_name), methods),
            kind_types,
            unknown_type: allocate(&format!("{stem}UnknownEventData"), names),
        });
    }
    StreamEventsPlan {
        semantics: semantics.clone(),
        operations: compiled,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// The `readonly id`/`readonly retry` metadata members, emitted only for the
/// envelope fields the item schema declares.
fn metadata_members(entry: &StreamEventsEntry) -> String {
    let mut members = String::new();
    if entry.id_metadata {
        members.push_str("    /// Last-event id envelope metadata, when the frame carries one.\n    Presence<std::string> id;\n");
    }
    if entry.retry_metadata {
        members.push_str("    /// Reconnection hint envelope metadata in milliseconds, when the frame carries one.\n    Presence<JsonNumber> retry;\n");
    }
    members
}

/// One typed event struct: the per-kind payload variant, the frame's kind name
/// and the declared envelope metadata. The declared model payload alternatives
/// are not default-constructible, so the default constructor value-initializes
/// the typed unknown alternative, whose raw frame fields always are.
fn event_struct(entry: &StreamEventsEntry) -> String {
    let alternatives = entry
        .kind_types
        .iter()
        .chain(std::iter::once(&entry.unknown_type))
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "/// One typed event of the {} stream. Declared kinds decode the framed\n/// envelope through the operation's stream item codec into their declared\n/// model type; an event kind the source never declared surfaces through the\n/// typed unknown alternative without failing the stream; invalid payloads of\n/// recognized kinds remain decoding errors. The `kind` member is the frame's\n/// event name and the metadata members repeat the envelope fields the item\n/// schema declares.\nstruct {} {{\n    /// Per-kind decoded payloads in declaration order, with the typed\n    /// unknown alternative last.\n    using Payload = std::variant<{}>;\n    Payload payload;\n    /// The frame's event name; the SSE default kind `message` applies when\n    /// the envelope field is absent or empty.\n    std::string kind;\n{}    /// Default-constructs into the typed unknown alternative, whose raw\n    /// frame fields are always representable.\n    {}() : payload({}{{}}) {{}}\n}};\n\n",
        entry.operation, entry.event, alternatives, metadata_members(entry), entry.event, entry.unknown_type,
    )
}

fn completion_struct(entry: &StreamEventsEntry) -> String {
    let sentinel = match &entry.sentinel {
        Some(token) => format!(
            " `sentinel` when the declared {} token completed the stream before any\n    /// payload decoding,",
            token
        ),
        None => " `sentinel` when a declared terminal token completed the stream before\n    /// any payload decoding,".to_owned(),
    };
    format!(
        "/// Terminal metadata of one completed {} typed stream: why it completed\n/// and the preserved final usage frame.{} `eof` when the\n/// response body ended.\nstruct {} {{\n    enum class Reason {{ Sentinel, Eof }};\n    /// Why the stream completed.\n    Reason reason = Reason::Eof;\n    /// The last data frame before the sentinel or end of body, decoded like\n    /// an ordinary event and preserved as terminal instead of being\n    /// delivered; disengaged when no frame preceded completion or the\n    /// compiled policy preserves nothing.\n    Presence<{}> usage;\n}};\n\n",
        entry.operation, sentinel, entry.completion, entry.event,
    )
}

fn payload_structs(entry: &StreamEventsEntry) -> String {
    let mut out = String::new();
    for (kind, name) in entry.kinds.iter().zip(&entry.kind_types) {
        out.push_str(&format!(
            "/// A declared `{kind}` event of the {} stream: the framed envelope decoded\n/// through the operation's stream item codec into its declared model type.\n/// An invalid payload for this kind remains a decoding error.\nstruct {name} {{\n    /// Decoded envelope payload.\n    {} data;\n}};\n\n",
            entry.operation, entry.item_type,
        ));
    }
    out.push_str(&format!(
        "/// An event kind the source never declared: the raw frame data stays\n/// representable without failing the stream.\nstruct {} {{\n    /// The undeclared wire kind.\n    std::string kind;\n    /// The raw frame data, unchanged.\n    std::string data;\n}};\n\n",
        entry.unknown_type,
    ));
    out
}

/// The pager's per-frame decode body: the sentinel check before any payload
/// decoding, the per-kind typed decode through the item codec, the compiled
/// unknown alternative, and the keep-final-usage holding policy.
fn decode_chain(entry: &StreamEventsEntry) -> String {
    let mut chain = String::new();
    let last = entry.kind_types.len();
    for (index, (kind, _)) in entry.kinds.iter().zip(&entry.kind_types).enumerate() {
        chain.push_str(&format!(
            "        {} (kind == {}) {{\n            try {{\n                state_->context->validation.require({}, envelope, \"\");\n                auto model = detail::decode_{}(envelope, *state_->context, \"\");\n                state_->context->control.check();\n                typed.payload.emplace<{}>({}{{std::move(model)}});\n            }} catch (detail::Failure& failure) {{\n                auto error = state_->fail(std::move(failure.error));\n                state_.reset();\n                error_ = std::move(error);\n                done_ = true;\n                return false;\n            }}\n",
            if index == 0 { "if" } else { "} else if" },
            string(kind),
            entry.item_index,
            entry.item_index,
            index,
            entry.kind_types[index],
        ));
    }
    chain.push_str(&format!(
        "        }} else {{\n            typed.payload.emplace<{last}>({unknown}{{kind, data}});\n        }}\n",
        last = last,
        unknown = entry.unknown_type,
    ));
    chain
}

fn envelope_metadata(entry: &StreamEventsEntry) -> String {
    let mut out = String::new();
    if entry.id_metadata {
        out.push_str("        if (object) {\n            const auto found = object->find(\"id\");\n            if (found != object->end() && found->second.is<std::string>()) typed.id = found->second.as<std::string>();\n        }\n");
    }
    if entry.retry_metadata {
        out.push_str("        if (object) {\n            const auto found = object->find(\"retry\");\n            if (found != object->end() && found->second.is<JsonNumber>()) typed.retry = found->second.as<JsonNumber>();\n        }\n");
    }
    out
}

fn pager_struct(entry: &StreamEventsEntry) -> String {
    let sentinel = entry.sentinel.as_ref().map(|token| {
        format!(
            "        // The declared {} sentinel completes the stream before any payload decoding.\n        if (data == {}) {{\n            state_.reset();\n            done_ = true;\n            completion_.reason = {}::Reason::Sentinel;\n            completion_.usage = std::move(held_);\n            return false;\n        }}\n",
            token,
            string(token),
            entry.completion
        )
    });
    let delivery = if entry.keep_final_usage {
        "        if (held_) value_ = std::move(held_);\n        held_ = std::move(typed);\n        return value_.has_value();\n"
    } else {
        "        value_ = std::move(typed);\n        return true;\n"
    };
    let eof_usage = if entry.keep_final_usage {
        "            completion_.usage = std::move(held_);\n"
    } else {
        ""
    };
    format!(
        "/// Pull typed-event pager over {method}. The first next() opens the exchange\n/// exactly like the direct call, through the client and its transport; the\n/// pager owns its request state and the stream lease, so destruction between\n/// calls stops the walk without firing another request and closes the\n/// response body. Each next() applies the compiled stream semantics to one\n/// framed envelope: the declared sentinel completes the stream before any\n/// payload decoding and preserves the final usage frame as the completion's\n/// terminal metadata, recognized kinds decode through the operation's stream\n/// item codec, undeclared kinds surface through the typed unknown\n/// alternative, and invalid payloads of recognized kinds remain decoding\n/// errors. Iteration is pull-driven: early break or destruction cancels the\n/// response body and issues no further reads.\nclass {pager} {{\npublic:\n    {pager}(const Client& client, {input} input, CallOptions options)\n        : client_(&client), options_(std::move(options)), input_(std::move(input)) {{}}\n    {pager}(const {pager}&) = delete;\n    {pager}& operator=(const {pager}&) = delete;\n    /** Pulls the next typed event into out; false only after the stream\n     * completed (sentinel or end of body) or failed. After a false result,\n     * completion() carries the terminal metadata on an ordinary completion\n     * and error() carries the cause on a failure. */\n    bool next({event}& out) {{\n        if (done_ || error_) return false;\n        if (!open()) return false;\n        while (!done_ && !error_) {{\n            if (pull()) {{\n                out = std::move(*value_);\n                value_.reset();\n                return true;\n            }}\n        }}\n        return false;\n    }}\n    /** The event pulled by the last true next(); valid only after true. */\n    const {event}& value() const {{ return *value_; }}\n    /** Terminal metadata; meaningful after a false next() with error()\n     * disengaged. On an early failure it stays default. */\n    const {completion}& completion() const {{ return completion_; }}\n    /** The terminal failure; engaged exactly when the last false next()\n     * failed rather than completed. */\n    const Presence<SdkError>& error() const {{ return error_; }}\n\nprivate:\n    /** Opens the exchange through the generated opener on the first pull. */\n    bool open() {{\n        if (state_) return true;\n        auto opened = client_->{open}(input_, options_);\n        if (!opened) {{\n            // The opener's failure surface is always the branded SdkError:\n            // every outcome but the declared stream representation is\n            // thrown as an unexpected response with retained metadata.\n            error_ = std::move(std::get<SdkError>(std::move(opened).error()));\n            done_ = true;\n            return false;\n        }}\n        state_ = std::move(opened).value();\n        return true;\n    }}\n    /** Applies the compiled semantics to one framed envelope; true when an\n     * event was delivered into value_, false when the walk ended. */\n    bool pull() {{\n        auto frame = state_->next();\n        if (!frame) {{\n            error_ = std::move(frame).error();\n            state_.reset();\n            done_ = true;\n            return false;\n        }}\n        if (!frame.value()) {{\n            state_.reset();\n            done_ = true;\n            completion_.reason = {completion}::Reason::Eof;\n{eof_usage}            return false;\n        }}\n        const auto& envelope = *frame.value();\n        const JsonValue::Object* object = envelope.is<JsonValue::Object>() ? &envelope.as<JsonValue::Object>() : nullptr;\n        std::string data;\n        if (object) {{\n            const auto found = object->find(\"data\");\n            if (found != object->end() && found->second.is<std::string>()) data = found->second.as<std::string>();\n        }}\n{sentinel}        std::string kind = \"message\";\n        if (object) {{\n            const auto found = object->find(\"event\");\n            if (found != object->end() && found->second.is<std::string>()) {{\n                const auto& value = found->second.as<std::string>();\n                if (!value.empty()) kind = value;\n            }}\n        }}\n        {event} typed;\n        typed.kind = kind;\n{metadata}{chain}{delivery}    }}\n\n    const Client* client_;\n    CallOptions options_;\n    {input} input_;\n    std::unique_ptr<detail::ItemState> state_;\n    Presence<{event}> value_;\n    Presence<{event}> held_;\n    {completion} completion_;\n    Presence<SdkError> error_;\n    bool done_ = false;\n}};\n",
        method = entry.method,
        pager = entry.pager,
        input = entry.input_type,
        event = entry.event,
        completion = entry.completion,
        open = entry.open_method,
        eof_usage = eof_usage,
        sentinel = sentinel.as_deref().unwrap_or(""),
        metadata = envelope_metadata(entry),
        chain = decode_chain(entry),
        delivery = delivery,
    )
}

fn client_definition(entry: &StreamEventsEntry) -> String {
    format!(
        "inline {} Client::{}(const {}& input, CallOptions options) const {{\n    return {}(*this, input, std::move(options));\n}}\n",
        entry.pager, entry.events_method, entry.input_type, entry.pager,
    )
}

/// The generated `include/<package>/stream_events.hpp`.
pub(super) fn header(plan: &SdkPlan, events: &StreamEventsPlan) -> String {
    let name = &plan.config.name;
    let mut out = String::new();
    out.push_str("#pragma once\n");
    out.push_str(
        "/** @file stream_events.hpp Generated typed SSE event pagers for this\n * package's source-declared discriminated stream operations.\n *\n * Every pager is a concrete RAII pull type: the first next() opens the\n * exchange exactly like the direct call through the client's transport and\n * owns its request state and the stream lease, so destruction between calls\n * stops the walk without firing another request, and cancellation/deadline\n * policy is the CallOptions passed at construction. The declared sentinel\n * completes the stream before any payload decoding and preserves the final\n * usage frame as the completion's terminal metadata; recognized event kinds\n * decode through the operation's existing stream item codec; undeclared\n * kinds surface through the typed unknown alternative without failing the\n * stream; invalid payloads of recognized kinds remain decoding errors.\n */\n",
    );
    out.push_str(&format!("#include \"{name}/client.hpp\"\n"));
    out.push_str("#include <memory>\n#include <optional>\n#include <string>\n#include <utility>\n#include <variant>\n\n");
    out.push_str(&format!("namespace {} {{\n\n", plan.config.namespace));
    for entry in &events.operations {
        out.push_str(&payload_structs(entry));
        out.push_str(&event_struct(entry));
        out.push_str(&completion_struct(entry));
        out.push_str(&pager_struct(entry));
        out.push('\n');
        out.push_str(&client_definition(entry));
        out.push('\n');
        // The exchange opener is defined on the generated client, inline in
        // this header: the direct call's exact request preparation, status
        // matching and typed failure surface, returning the raw stream state
        // the pager consumes instead of decoding stream items.
        if let Some(operation) = plan
            .operations
            .iter()
            .find(|operation| operation.method_name == entry.method)
        {
            out.push_str(&super::emit::http::events_opener(plan, operation, entry));
            out.push('\n');
        }
    }
    out.push_str(&format!("}} // namespace {}\n", plan.config.namespace));
    out
}

/// Forward declarations injected into the emitted `client.hpp` before the
/// `Client` class, so the pager-returning methods can be declared there.
pub(super) fn client_forward_declarations(plan: &SdkPlan, events: &StreamEventsPlan) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "/// Generated typed stream event pagers; complete definitions live in\n/// <{}/stream_events.hpp>.\n",
        plan.config.name
    ));
    for entry in &events.operations {
        out.push_str(&format!("class {};\n", entry.pager));
    }
    out
}

/// Per-operation pager-returning and exchange-opening method declarations
/// inside the `Client` class.
pub(super) fn client_declarations(events: &StreamEventsPlan) -> String {
    let mut out = String::new();
    for entry in &events.operations {
        out.push_str(&format!(
            "    /// Pull typed-event pager over {method}; the first next() opens the\n    /// exchange exactly like the direct call and every next() applies the\n    /// compiled typed stream semantics to one framed envelope.\n",
            method = entry.method,
        ));
        out.push_str(&format!(
            "    [[nodiscard]] {} {}(const {}& input, CallOptions options = {{}}) const;\n",
            entry.pager, entry.events_method, entry.input_type,
        ));
        out.push_str(
            "    /// Opens the streamed exchange without decoding frames; the\n    /// typed-event pager consumes the returned stream state.\n",
        );
        out.push_str(&format!(
            "    [[nodiscard]] Result<std::unique_ptr<detail::ItemState>, {}> {}(const {}& input, CallOptions options = {{}}) const;\n",
            entry.error_type, entry.open_method, entry.input_type,
        ));
    }
    out
}
