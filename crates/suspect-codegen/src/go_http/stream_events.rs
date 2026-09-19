//! Emitted typed event iteration for the Go HTTP adapter.
//!
//! The compiled `http_protocol::StreamSemanticsPlan` becomes one per-operation
//! typed events iterator in the generated package for exactly the operations
//! whose stream plan carries a discriminated SSE event set. The existing
//! untyped stream item iteration stays the transport: the events iterator runs
//! the same framing, byte limits and cancellation, while the assembled SSE
//! envelope passes through leniently so the compiled per-kind decode, sentinel
//! and completion semantics apply here. Recognized event kinds decode through
//! the operation's existing stream item codec into its declared model type;
//! undeclared event kinds surface through the typed unknown alternative
//! without failing the stream; invalid payloads of recognized kinds remain
//! decoding errors. Static runtime files are never modified, and operations
//! without a discriminated stream schema emit nothing at all.

use std::collections::{BTreeMap, BTreeSet};

use super::emit::q;
use super::*;
use crate::go_models::GoDecl;

/// One declared envelope metadata member copied from the decoded model.
#[derive(Debug, Clone)]
pub struct MetadataField {
    /// Allocated Go field name on the item model, e.g. `ID`.
    pub field: String,
    /// Exact declared Go type of that model field.
    pub go_type: String,
}

/// One operation's emission-ready typed event decode.
#[derive(Debug, Clone)]
pub struct StreamEventsOperation {
    /// Operation index into the generated descriptor table.
    pub index: usize,
    /// Source operation identity for documentation.
    pub operation_id: String,
    /// Exported client method this iterator extends, e.g. `StreamChat`.
    pub method_name: String,
    pub input_type: String,
    /// Concrete success response wrapper carrying the stream, e.g.
    /// `StreamChatStatus200`.
    pub response_type: String,
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
    pub id_metadata: Option<MetadataField>,
    pub retry_metadata: Option<MetadataField>,
    /// Allocated, collision-free public type and method names.
    pub events_type: String,
    pub completion_type: String,
    pub iterator_type: String,
    pub events_method: String,
}

/// Compiled typed-event helpers carried by one plan. Present only when at
/// least one operation emits a typed events iterator, so no-policy output
/// stays byte-identical.
#[derive(Debug, Clone)]
pub struct StreamEventsPlan {
    pub operations: Vec<StreamEventsOperation>,
}

impl StreamEventsPlan {
    /// Whether this plan emits the typed event helpers at all.
    #[must_use]
    pub fn emits(&self) -> bool {
        !self.operations.is_empty()
    }
}

/// Whether the compiled event set is discriminated: at least two declared
/// kinds, or one declared kind compiled from discrimination evidence whose
/// payload is a declared JSON structure. The single un-discriminated default
/// event keeps the existing untyped path byte-for-byte.
fn discriminated(stream: &protocol::StreamOperationPlan) -> bool {
    match stream.events.as_slice() {
        [] => false,
        [only] => {
            only.event_name != protocol::StreamEventPlan::DEFAULT_EVENT_NAME
                && matches!(only.payload, protocol::EventPayload::Json { .. })
        }
        _ => true,
    }
}

/// Whether one compiled stream plan admits the typed event decode: SSE framing
/// whose discriminator is the parsed envelope's `event` field, and a
/// discriminated event set. Everything else keeps its existing untyped path.
fn admits_typed_events(stream: &protocol::StreamOperationPlan) -> bool {
    stream.framing == protocol::StreamFraming::ServerSentEvents
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
    compiled: &protocol::StreamOperationPlan,
) -> Option<&'a PlannedResponse> {
    let mut found: Option<&PlannedResponse> = None;
    for response in &operation.responses {
        if !matches!(
            response.status(),
            protocol::ResponseStatus::Exact(200..=299) | protocol::ResponseStatus::Range(2)
        ) {
            continue;
        }
        let Some(media) = response.media.as_ref() else {
            continue;
        };
        if !matches!(
            media.wire.representation(),
            protocol::Representation::Stream { .. }
        ) {
            continue;
        }
        if media.wire.media_type().declared() != compiled.media_type {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(response);
    }
    found
}

/// One declared envelope metadata member resolved against the item model's
/// planned fields; unresolvable shapes keep the member off the event type.
fn metadata_field(
    models: &crate::go_models::ModelPlan,
    item_model: &str,
    wire: &str,
) -> Option<MetadataField> {
    let descriptors = models.descriptors();
    for (key, declaration) in &descriptors.declarations {
        let GoDecl::Struct { fields, .. } = declaration else {
            continue;
        };
        if descriptors.names.get(key).map(String::as_str) != Some(item_model) {
            continue;
        }
        if let Some(field) = fields.iter().find(|field| field.wire == wire) {
            return Some(MetadataField {
                field: field.name.clone(),
                go_type: field.ty.render(&descriptors.names),
            });
        }
    }
    None
}

/// Lower the compiled stream semantics into this backend's emission plan,
/// reserving every emitted type and method name. `None` keeps the plan cheap:
/// no reserved names, no emitted file, byte-identical no-policy output.
pub(super) fn plan(
    semantics: &protocol::StreamSemanticsPlan,
    operations: &[PlannedOperation],
    symbols: &BTreeMap<SchemaId, String>,
    models: &crate::go_models::ModelPlan,
    names: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> Option<StreamEventsPlan> {
    let mut result = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        let Some(compiled) = semantics
            .streams
            .iter()
            .find(|stream| stream.operation == operation.operation_id)
        else {
            continue;
        };
        if !admits_typed_events(compiled) {
            continue;
        }
        let Some(response) = stream_media(operation, compiled) else {
            continue;
        };
        let Some(item_codec) = &compiled.item_codec else {
            continue;
        };
        let Some(item_model) = symbols.get(item_codec.schema().id()) else {
            continue;
        };
        result.push(StreamEventsOperation {
            index,
            operation_id: operation.operation_id.clone(),
            method_name: operation.method_name.clone(),
            input_type: operation.input_type.clone(),
            response_type: response.type_name.clone(),
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
            id_metadata: compiled.item_metadata.as_ref().and_then(|metadata| {
                metadata
                    .id_field
                    .as_deref()
                    .and_then(|wire| metadata_field(models, item_model, wire))
            }),
            retry_metadata: compiled.item_metadata.as_ref().and_then(|metadata| {
                metadata
                    .retry_field
                    .as_deref()
                    .and_then(|wire| metadata_field(models, item_model, wire))
            }),
            events_type: allocate(&format!("{}Event", operation.method_name), names),
            completion_type: allocate(&format!("{}Completion", operation.method_name), names),
            iterator_type: allocate(&format!("{}EventIterator", operation.method_name), names),
            events_method: allocate(&format!("{}Events", operation.method_name), methods),
        });
    }
    (!result.is_empty()).then_some(StreamEventsPlan { operations: result })
}

fn prose(text: &str) -> String {
    text.replace(['\n', '\r'], " ").replace("*/", "* /")
}

/// The shared lenient framing helper: the untyped Stream's SSE line assembly,
/// byte limits and cancellation, with the assembled envelope passing through
/// unvalidated. Emitted once, only when a typed event stream exists.
const FRAMER: &str = r#"// httpEventFramer reads the same SSE framing as the untyped Stream: identical
// line assembly, comments, multiline data, envelope fields, byte limits and
// context cancellation, with the assembled envelope passing through
// unvalidated so the typed decode below owns per-kind validation and
// undeclared event kinds stay representable.
type httpEventFramer struct {
	raw                 *rawHTTPResponse
	media               httpMediaPlan
	reader              *bufio.Reader
	limit               int
	skipLF, first, done bool
	used                int
	event               map[string]Value
	data                strings.Builder
	hasData             bool
}

// httpEventFramerOf frames the body already taken by an untyped Stream: the
// transport, byte limits and reader are exactly the untyped iterator's.
func httpEventFramerOf[T any](stream *Stream[T]) *httpEventFramer {
	return &httpEventFramer{raw: stream.raw, media: stream.media, reader: stream.reader, limit: stream.itemLimit, event: map[string]Value{}}
}

func (f *httpEventFramer) line() (string, bool, error) {
	var data []byte
	for {
		b, err := f.reader.ReadByte()
		if err == io.EOF {
			return string(data), true, nil
		}
		if err != nil {
			return "", false, err
		}
		if f.skipLF {
			f.skipLF = false
			if b == '\n' {
				continue
			}
		}
		if b == '\r' && f.media.Framing != "json-lines" {
			f.skipLF = true
			return string(data), false, nil
		}
		if b == '\n' {
			return string(data), false, nil
		}
		if len(data) >= f.limit {
			return "", false, httpProblem("resource-limit")
		}
		data = append(data, b)
	}
}

// next returns the next lenient envelope, or (nil, nil) at a clean end of body.
func (f *httpEventFramer) next() (map[string]Value, error) {
	if f.done {
		return nil, nil
	}
	if err := f.raw.owned.ctx.Err(); err != nil {
		return nil, f.fail(err)
	}
	for {
		line, eof, err := f.line()
		if err != nil {
			return nil, f.fail(err)
		}
		// HTML framing discards a final block without a terminating blank line.
		if eof {
			return nil, f.complete()
		}
		line = httpSSEUTF8([]byte(line))
		if f.first {
			f.first = false
			line = strings.TrimPrefix(line, "\uFEFF")
		}
		f.used += len(line) + 1
		if f.used > f.limit {
			return nil, f.fail(httpProblem("resource-limit"))
		}
		if line == "" {
			if !f.hasData {
				f.event = map[string]Value{}
				f.used = 0
				continue
			}
			envelope := f.event
			envelope["data"] = strings.TrimSuffix(f.data.String(), "\n")
			f.event = map[string]Value{}
			f.used = 0
			f.data.Reset()
			f.hasData = false
			return envelope, nil
		}
		if line[0] == ':' {
			continue
		}
		name, value, _ := strings.Cut(line, ":")
		value = strings.TrimPrefix(value, " ")
		switch name {
		case "data":
			f.data.WriteString(value)
			f.data.WriteByte('\n')
			f.hasData = true
		case "event":
			f.event["event"] = value
		case "id":
			if !strings.ContainsRune(value, 0) {
				f.event["id"] = value
			}
		case "retry":
			if httpDigits(value) {
				integer, err := ParseInteger(value)
				if err != nil {
					// Leading zeros are valid SSE decimal digits but not JSON number tokens.
					normalized := strings.TrimLeft(value, "0")
					if normalized == "" {
						normalized = "0"
					}
					integer, err = ParseInteger(normalized)
				}
				if err != nil {
					return nil, f.fail(err)
				}
				f.event["retry"] = integer
			}
		}
	}
}

// fail brands a framing failure exactly like the untyped Stream and releases
// the response body.
func (f *httpEventFramer) fail(err error) error {
	f.done = true
	failure := f.raw.fail("response-decoding", f.media.Source, err)
	failure.Truncated = true
	_ = f.raw.owned.finish()
	return failure
}

// complete releases the response body at a clean end of stream.
func (f *httpEventFramer) complete() error {
	f.done = true
	_ = f.raw.owned.finish()
	return nil
}
"#;

/// Render `go/stream_events.go`: the shared lenient framing helper plus one
/// typed event iterator per planned operation. Called only when at least one
/// operation emits typed events.
pub(super) fn emit(plan: &HttpPlan, events: &StreamEventsPlan) -> String {
    let _ = plan;
    let mut code = String::from(
        "// Code generated by suspect. DO NOT EDIT.\n//\n// Typed event iteration for the source-selected SSE stream operations. Each\n// iterator runs the untyped stream's framing, byte limits and cancellation,\n// while the compiled stream semantics plan owns the typed decode: the declared\n// sentinel completes the stream before any payload decoding and preserves the\n// final usage frame, recognized event kinds decode through the operation's\n// stream item codec into its declared model type, undeclared kinds surface\n// through the typed unknown alternative without failing the stream, and\n// invalid payloads of recognized kinds remain decoding errors. The direct\n// operation methods and their untyped *Stream iterators are unchanged.\npackage sdk\n\nimport (\n\t\"bufio\"\n\t\"context\"\n\t\"errors\"\n\t\"io\"\n\t\"strings\"\n)\n\n",
    );
    code.push_str(FRAMER);
    for operation in &events.operations {
        code.push_str(&operation_code(operation));
    }
    code
}

fn metadata_members(operation: &StreamEventsOperation) -> String {
    let mut members = String::new();
    if let Some(id) = &operation.id_metadata {
        members.push_str(&format!(
            "\t// {} repeats the decoded envelope's declared id field; the zero value for the unknown alternative.\n\t{} {}\n",
            id.field, id.field, id.go_type
        ));
    }
    if let Some(retry) = &operation.retry_metadata {
        members.push_str(&format!(
            "\t// {} repeats the decoded envelope's declared reconnection hint; the zero value for the unknown alternative.\n\t{} {}\n",
            retry.field, retry.field, retry.go_type
        ));
    }
    members
}

fn metadata_values(operation: &StreamEventsOperation) -> String {
    let mut values = String::new();
    if let Some(id) = &operation.id_metadata {
        values.push_str(&format!("{}: item.{}, ", id.field, id.field));
    }
    if let Some(retry) = &operation.retry_metadata {
        values.push_str(&format!("{}: item.{}, ", retry.field, retry.field));
    }
    values
}

fn operation_code(operation: &StreamEventsOperation) -> String {
    let kinds = operation
        .events
        .iter()
        .map(|kind| q(kind))
        .collect::<Vec<_>>()
        .join(", ");
    let mut code = format!(
        "// {events_type} is one typed event of the {operation_id} stream, compiled from the\n// source stream semantics plan. Declared kinds decode the framed envelope\n// through the operation's stream item codec into its declared model type; an\n// event kind the source never declared surfaces through the typed unknown\n// alternative without failing the stream, and invalid payloads of recognized\n// kinds remain decoding errors.\ntype {events_type} struct {{\n\t// Kind is the framed envelope's event name: one of the declared kinds\n\t// ({kinds}), or \"unknown\" when the source sent an event kind this stream\n\t// never declared.\n\tKind string\n\t// Data is the decoded envelope model for a declared kind; nil for the\n\t// unknown alternative.\n\tData *{item_model}\n{metadata_members}\t// UnknownEvent is the undeclared event name when Kind is \"unknown\".\n\tUnknownEvent string\n\t// UnknownData is the raw frame data when Kind is \"unknown\".\n\tUnknownData string\n}}\n\n",
        events_type = operation.events_type,
        operation_id = prose(&operation.operation_id),
        kinds = kinds,
        item_model = operation.item_model,
        metadata_members = metadata_members(operation),
    );
    code.push_str(&format!(
        "// {completion_type} is the terminal metadata of one completed {operation_id}\n// typed event stream: why it completed and the preserved final usage frame.\n// Range loops discard it; Completion reads it after Next reports false.\ntype {completion_type} struct {{\n\t// Reason is \"sentinel\" when the declared terminal token completed the\n\t// stream before any payload decoding, \"eof\" when the response body ended.\n\tReason string\n\t// Usage is the last data frame before the sentinel or end of body, decoded\n\t// like an ordinary event and preserved as terminal instead of being\n\t// yielded; nil when no frame preceded completion or the compiled policy\n\t// preserves nothing.\n\tUsage *{events_type}\n}}\n\n",
        completion_type = operation.completion_type,
        operation_id = prose(&operation.operation_id),
        events_type = operation.events_type,
    ));

    // The sentinel completes the stream before any payload decoding; the
    // compiled keep-final-usage policy holds one frame behind so the last data
    // frame before the sentinel or end of body becomes the completion's usage.
    let sentinel = match &operation.sentinel {
        Some(token) => format!(
            "\t\t// The declared {token} sentinel completes the stream before any payload decoding.\n\t\tif data == {token_lit} {{\n\t\t\tit.finish(\"sentinel\", it.held)\n\t\t\treturn false\n\t\t}}\n",
            token = prose(token),
            token_lit = q(token),
        ),
        None => String::new(),
    };
    let case_labels = if operation.events.is_empty() {
        String::new()
    } else {
        format!(
            "case {}:",
            operation
                .events
                .iter()
                .map(|kind| q(kind))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let metadata_values = metadata_values(operation);
    let hold = if operation.keep_final_usage {
        "\t\tif it.held != nil {\n\t\t\tit.value, it.held = *it.held, typed\n\t\t\treturn true\n\t\t}\n\t\tit.held = typed"
    } else {
        "\t\tit.value = *typed\n\t\treturn true"
    };
    let eof_usage = if operation.keep_final_usage {
        "it.held"
    } else {
        "nil"
    };
    code.push_str(&format!(
        "// {iterator_type} lazily yields the typed events of {method} over the existing\n// stream item iteration: the framing, limits and cancellation are the untyped\n// *Stream's. The declared sentinel, when compiled, completes the stream\n// before any payload decoding and preserves the final usage frame as the\n// completion's terminal metadata. Iteration is pull-driven: stopping, or\n// calling Close, cancels the response body and issues no further reads. The\n// direct {method} method and its untyped *Stream iterator are unchanged.\ntype {iterator_type} struct {{\n\tclient     *Client\n\tctx        context.Context\n\tinput      {input}\n\tframer     *httpEventFramer\n\theld       *{events_type}\n\tvalue      {events_type}\n\tcompletion {completion_type}\n\terr        error\n\tdone       bool\n}}\n\n// {events_method} returns a lazy typed event iterator for {method}. The first\n// Next issues the request exactly once.\nfunc (c *Client) {events_method}(ctx context.Context, input {input}) *{iterator_type} {{\n\treturn &{iterator_type}{{client: c, ctx: ctx, input: input}}\n}}\n\n// Next decodes the next typed event and reports whether one was produced.\n// Event returns it. After false, Err carries a failure, while a completed\n// stream leaves Err nil and Completion carries the terminal metadata.\nfunc (it *{iterator_type}) Next() bool {{\n\tif it.err != nil || it.done {{\n\t\treturn false\n\t}}\n\tfor {{\n\t\tif it.framer == nil {{\n\t\t\treply, err := it.client.{method}(it.ctx, it.input)\n\t\t\tif err != nil {{\n\t\t\t\tit.err = err\n\t\t\t\treturn false\n\t\t\t}}\n\t\t\tstream, ok := reply.({response})\n\t\t\tif !ok {{\n\t\t\t\tit.err = httpError(\"unexpected-response\", operation{index}, operation{index}.Source, errors.New(\"the typed event stream met a success representation the stream plan cannot read\"))\n\t\t\t\treturn false\n\t\t\t}}\n\t\t\tit.framer = httpEventFramerOf(stream.Data)\n\t\t}}\n\t\tenvelope, err := it.framer.next()\n\t\tif err != nil {{\n\t\t\tit.err = err\n\t\t\treturn false\n\t\t}}\n\t\tif envelope == nil {{\n\t\t\tit.finish(\"eof\", {eof_usage})\n\t\t\treturn false\n\t\t}}\n\t\tdata, _ := envelope[\"data\"].(string)\n{sentinel}\t\tkind, ok := envelope[\"event\"].(string)\n\t\tif !ok || kind == \"\" {{\n\t\t\tkind = \"message\"\n\t\t}}\n\t\tvar typed *{events_type}\n\t\tswitch kind {{\n\t\t{cases}\n\t\t\titem, err := Codecs.{item_model}.DecodeValue(envelope)\n\t\t\tif err != nil {{\n\t\t\t\tit.err = it.framer.fail(err)\n\t\t\t\treturn false\n\t\t\t}}\n\t\t\ttyped = &{events_type}{{Kind: kind, Data: &item, {metadata_values}}}\n\t\tdefault:\n\t\t\ttyped = &{events_type}{{Kind: \"unknown\", UnknownEvent: kind, UnknownData: data}}\n\t\t}}\n{hold}\n\t}}\n}}\n\n// Event returns the event produced by the last successful Next.\nfunc (it *{iterator_type}) Event() {events_type} {{\n\treturn it.value\n}}\n\n// Completion returns the terminal metadata of the completed stream: why it\n// completed and the preserved final usage frame. Valid after Next reports\n// false without an error; range loops discard it.\nfunc (it *{iterator_type}) Completion() {completion_type} {{\n\treturn it.completion\n}}\n\n// Err returns the terminal cause after Next reports false, or nil while the\n// iterator is healthy. Cancellation surfaces as the context's error.\nfunc (it *{iterator_type}) Err() error {{\n\treturn it.err\n}}\n\n// Close releases the response body. Idempotent, and always safe to defer.\nfunc (it *{iterator_type}) Close() {{\n\tif it.framer != nil {{\n\t\t_ = it.framer.raw.owned.finish()\n\t}}\n\tit.done = true\n}}\n\n// finish records the documented completion metadata and releases the body.\nfunc (it *{iterator_type}) finish(reason string, usage *{events_type}) {{\n\tit.done = true\n\tit.completion = {completion_type}{{Reason: reason, Usage: usage}}\n\tif it.framer != nil {{\n\t\t_ = it.framer.raw.owned.finish()\n\t}}\n}}\n\n",
        iterator_type = operation.iterator_type,
        method = operation.method_name,
        input = operation.input_type,
        events_type = operation.events_type,
        completion_type = operation.completion_type,
        events_method = operation.events_method,
        response = operation.response_type,
        index = operation.index,
        eof_usage = eof_usage,
        sentinel = sentinel,
        cases = case_labels,
        item_model = operation.item_model,
        metadata_values = metadata_values,
        hold = hold,
    ));
    code
}
