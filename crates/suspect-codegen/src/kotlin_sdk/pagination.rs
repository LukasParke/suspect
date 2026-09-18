//! Emitted-only pagination iteration for the Kotlin coroutine client.
//!
//! The shared, generation-time `http_protocol::plan_pagination` outcome lowers
//! into per-operation members of the generated `Client`: a cold page
//! `Flow`, an item-flattening `Flow`, a suspending next-input builder and
//! private typed accessors over the decoded page models. Cold flows start
//! their first request on collection and re-fetch on every collection; early
//! collection end never fires the next request. Response pointers resolve at
//! generation time into typed chains over the planned models, so there is no
//! runtime schema search and no JSON re-parsing. Operations whose compiled
//! selection cannot be expressed with typed field accesses are left without
//! members, exactly like the Rust backend. Without configured defaults, or
//! without any emittable paginated operation, nothing is emitted and every
//! other artifact stays byte-identical.

use std::collections::BTreeSet;

use super::models::{ModelPlan, Shape, Symbol};
use super::{NativeType, PlannedOperation};
use crate::http_protocol as wire;
use crate::sdk_defaults::{
    PaginationAdvance, PaginationPattern, PaginationRequestRole, PaginationResponseRole,
};
use suspect_ir::contract::{SchemaId, SourceId};

/// How one walk computes and applies its continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// The offset control advances by the previous page's item count. The
    /// documented stop rules are a zero-item page or a false has-more mark.
    OffsetItemCount,
    /// The one-based page control advances by one. The documented stop rules
    /// are a zero-item page or a false has-more mark.
    PageNext,
    /// The next-cursor pointer feeds the cursor control. The walk stops when
    /// the pointer is absent, null or empty; a repeated identical cursor is a
    /// typed pagination error.
    CursorPointer,
    /// The next-offset pointer feeds the offset control. The walk stops when
    /// the pointer is absent or null; a repeated identical offset is a typed
    /// pagination error.
    OffsetPointer,
}

/// The native carrier of one request-side pagination value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    /// Exact `JsonNumber` control (Kotlin integers decode exactly).
    Integer,
    /// Plain `String` continuation control.
    Text,
}

/// One request control the walk rewrites between pages.
#[derive(Debug, Clone)]
pub struct Control {
    /// Input member carrying the control.
    pub name: String,
    pub kind: ControlKind,
    /// Optional input members are `Presence<T>`; required ones are direct.
    pub optional: bool,
}

/// One paginated operation's compiled emission.
#[derive(Debug, Clone)]
pub struct PaginationOperation {
    /// Index into the planned operations.
    pub index: usize,
    /// Source identity of the paginated operation.
    pub operation: String,
    pub source: SourceId,
    /// The generated client method this walk extends.
    pub method: String,
    pub input_type: String,
    pub result_type: String,
    /// Shared native success payload type; absent when the success responses
    /// have no single payload type and the walk cannot read pages.
    pub data_type: Option<String>,
    pub pages: String,
    pub items: Option<String>,
    pub next_page: String,
    /// Private helper names shared by the flows and the next-input builder.
    pub advance_helper: String,
    pub items_helper: Option<String>,
    pub continuation_helper: Option<String>,
    pub has_more_helper: Option<String>,
    pub advance: Advance,
    pub control: Control,
    /// The limit control and the documented SDK fallback page size it carries
    /// on the first request, when the operation has one.
    pub limit: Option<(Control, u32)>,
    pub initial_offset: Option<u32>,
    /// The element type of the items collection.
    pub item_type: Option<String>,
    /// Whether the operation input has a constructor default.
    pub input_default: bool,
    /// Pre-rendered private accessor members over the compiled pointers.
    pub accessors: String,
    /// Pre-rendered member text: the advance helper and the public flow and
    /// next-input members.
    pub member: String,
}

/// The compiled pagination emission carried by one plan.
#[derive(Debug, Clone)]
pub struct PaginationPlan {
    /// The shared selection this emission follows.
    pub outcome: wire::PaginationOutcome,
    pub operations: Vec<PaginationOperation>,
}

impl PaginationPlan {
    /// Whether this plan emits pagination members at all.
    #[must_use]
    pub fn emits(&self) -> bool {
        !self.operations.is_empty()
    }
}

/// The leaf shape one resolved pointer produces.
#[derive(Debug, Clone)]
enum Leaf {
    Items { item: String },
    Number,
    Text,
    Boolean,
}

/// One resolved pointer: generated statements and the final expression.
struct Walk {
    body: Vec<String>,
    place: String,
    leaf: Leaf,
}

fn segments(pointer: &str) -> Vec<String> {
    if pointer.is_empty() {
        Vec::new()
    } else {
        pointer[1..]
            .split('/')
            .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
            .collect()
    }
}

/// Follow aliases, accumulating nullability, to the resolved symbol.
fn resolved_symbol<'a>(models: &'a ModelPlan, id: &SchemaId) -> Option<(&'a Symbol, bool)> {
    let mut current = id.clone();
    let mut nullable = false;
    for _ in 0..64 {
        let symbol = models.get(&current);
        if symbol.nullable {
            nullable = true;
        }
        match &symbol.shape {
            Shape::Alias(target) => current = target.clone(),
            _ => return Some((symbol, nullable)),
        }
    }
    None
}

/// Resolve one RFC 6901 pointer into a typed chain over the planned page
/// model, starting at the decoded body. `None` means the pointer reads through
/// something the native model cannot express; the caller then leaves the
/// operation without members instead of emitting guesses.
fn walk(
    models: &ModelPlan,
    id: &SchemaId,
    place: &str,
    path: &[String],
    counter: &mut usize,
    body: &mut Vec<String>,
) -> Option<Leaf> {
    let Some(segment) = path.first() else {
        let (symbol, _) = resolved_symbol(models, id)?;
        return match &symbol.shape {
            Shape::Array(item) => Some(Leaf::Items {
                item: match item {
                    Some(item) => models.get(item).kotlin_type.clone(),
                    None => "JsonValue".into(),
                },
            }),
            Shape::Number => Some(Leaf::Number),
            Shape::String => Some(Leaf::Text),
            Shape::Boolean => Some(Leaf::Boolean),
            _ => None,
        };
    };
    let (symbol, _) = resolved_symbol(models, id)?;
    let Shape::Object { fields, .. } = &symbol.shape else {
        return None;
    };
    let field = fields.iter().find(|field| field.wire_name == *segment)?;
    let binding = format!("h{counter}");
    *counter += 1;
    if field.required {
        body.push(format!("val {binding} = {place}.{}", field.name));
    } else {
        body.push(format!(
            "val {binding} = when (val member = {place}.{}) {{",
            field.name
        ));
        body.push("    is Presence.Present -> member.value".to_owned());
        body.push("    Presence.Absent -> null".to_owned());
        body.push("}".to_owned());
    }
    // A nullable model decodes to a nullable value; later hops need a guard.
    let (_, may_be_null) = resolved_symbol(models, &field.schema)?;
    if may_be_null && path.len() > 1 {
        body.push(format!("if ({binding} == null) return null"));
    }
    walk(models, &field.schema, &binding, &path[1..], counter, body)
}

fn resolve(models: &ModelPlan, body: &SchemaId, pointer: &str) -> Option<Walk> {
    let path = segments(pointer);
    let mut statements = Vec::new();
    let mut counter = 0usize;
    let leaf = walk(models, body, "page", &path, &mut counter, &mut statements)?;
    let place = if counter == 0 {
        "page".to_owned()
    } else {
        format!("h{}", counter - 1)
    };
    Some(Walk {
        body: statements,
        place,
        leaf,
    })
}

/// The native control kind one request parameter carries.
fn control_kind(models: &ModelPlan, id: &SchemaId) -> Option<ControlKind> {
    let (symbol, nullable) = resolved_symbol(models, id)?;
    if nullable {
        return None;
    }
    match &symbol.shape {
        Shape::Number => Some(ControlKind::Integer),
        Shape::String => Some(ControlKind::Text),
        _ => None,
    }
}

fn control(
    models: &ModelPlan,
    operation: &PlannedOperation,
    role: PaginationRequestRole,
    page: &wire::OperationPagination,
) -> Option<Control> {
    let name = page.request.get(&role)?;
    let parameter = operation
        .parameters
        .iter()
        .find(|parameter| {
            parameter.wire_name == *name
                && parameter.wire.location() == wire::ParameterLocation::Query
        })?;
    let kind = control_kind(models, &parameter.schema)?;
    Some(Control {
        name: parameter.name.clone(),
        kind,
        optional: !parameter.required,
    })
}

/// The request role that receives the computed continuation value, preferring
/// the pattern's own control and falling back to any declared pagination role.
fn apply_role(page: &wire::OperationPagination) -> Option<PaginationRequestRole> {
    let candidates: &[PaginationRequestRole] = match page.pattern {
        PaginationPattern::Cursor => &[
            PaginationRequestRole::Cursor,
            PaginationRequestRole::Offset,
            PaginationRequestRole::Page,
        ],
        PaginationPattern::PageNumber => &[
            PaginationRequestRole::Page,
            PaginationRequestRole::Offset,
            PaginationRequestRole::Cursor,
        ],
        PaginationPattern::LimitOffset | PaginationPattern::NextLink => &[
            PaginationRequestRole::Offset,
            PaginationRequestRole::Page,
            PaginationRequestRole::Cursor,
        ],
    };
    candidates
        .iter()
        .copied()
        .find(|role| page.request.contains_key(role))
}

/// The response pointer carrying the next continuation value.
fn continuation_roles(page: &wire::OperationPagination) -> &'static [PaginationResponseRole] {
    match page.pattern {
        PaginationPattern::Cursor => &[
            PaginationResponseRole::NextCursor,
            PaginationResponseRole::NextOffset,
        ],
        _ => &[
            PaginationResponseRole::NextOffset,
            PaginationResponseRole::NextCursor,
        ],
    }
}

/// The decoded JSON body the shared planner reads: the first exact/range 2xx
/// success response with a typed JSON payload.
fn success_payload(operation: &PlannedOperation) -> Option<SchemaId> {
    let response = operation.responses.iter().find(|response| {
        response.success
            && matches!(
                response.status,
                wire::ResponseStatus::Exact(200..=299) | wire::ResponseStatus::Range(2)
            )
    })?;
    if response.stream {
        return None;
    }
    let schema = response.schema.as_ref()?;
    match operation.result_data.as_ref()? {
        NativeType::Model(id) if id == schema => Some(schema.clone()),
        _ => None,
    }
}

/// Lower one shared pagination selection into this backend's emission.
fn compile(
    models: &ModelPlan,
    operation: &PlannedOperation,
    page: &wire::OperationPagination,
    methods: &mut BTreeSet<String>,
) -> Option<PaginationOperation> {
    if page.pattern == PaginationPattern::NextLink {
        return None;
    }
    // Nullable page bodies cannot be walked with plain member access.
    let body = success_payload(operation)?;
    if models.get(&body).nullable || operation.result_data_type.is_none() {
        return None;
    }
    let resolve = |pointer: &str| resolve(models, &body, pointer);

    let control_for = |role| control(models, operation, role, page);
    let apply = apply_role(page).and_then(control_for);
    let limit = control_for(PaginationRequestRole::Limit).zip(page.initial_limit);
    let continuation = |kind: ControlKind| -> Option<Walk> {
        continuation_roles(page).iter().find_map(|role| {
            let walk = resolve(page.response.get(role)?)?;
            match (&walk.leaf, kind) {
                (Leaf::Number, ControlKind::Integer) | (Leaf::Text, ControlKind::Text) => {
                    Some(walk)
                }
                _ => None,
            }
        })
    };

    let (advance, control, continuation) = match page.pattern {
        PaginationPattern::Cursor => {
            let control = apply?;
            let continuation = continuation(control.kind)?;
            (Advance::CursorPointer, control, Some(continuation))
        }
        PaginationPattern::LimitOffset | PaginationPattern::PageNumber => {
            let offset = control_for(PaginationRequestRole::Offset);
            let page_control = control_for(PaginationRequestRole::Page);
            match page.advance {
                PaginationAdvance::NextOffset => {
                    let pointer = apply
                        .clone()
                        .and_then(|control| continuation(control.kind));
                    if let (Some(control), Some(pointer)) = (apply, pointer) {
                        (Advance::OffsetPointer, control, Some(pointer))
                    } else if page.pattern == PaginationPattern::PageNumber {
                        (Advance::PageNext, page_control?, None)
                    } else {
                        (Advance::OffsetItemCount, offset?, None)
                    }
                }
                PaginationAdvance::ItemsReturned => {
                    if page.pattern == PaginationPattern::PageNumber {
                        (Advance::PageNext, page_control?, None)
                    } else {
                        (Advance::OffsetItemCount, offset?, None)
                    }
                }
            }
        }
        PaginationPattern::NextLink => return None,
    };

    let has_more = page
        .response
        .get(&PaginationResponseRole::HasMore)
        .and_then(|pointer| resolve(pointer))
        .filter(|walk| matches!(walk.leaf, Leaf::Boolean));

    // Count-driven walks advance by page contents and the item flow flattens
    // page contents, so both require a resolved items pointer.
    let items_walk = page
        .response
        .get(&PaginationResponseRole::Items)
        .and_then(|pointer| resolve(pointer))
        .filter(|walk| matches!(walk.leaf, Leaf::Items { .. }));
    if matches!(advance, Advance::OffsetItemCount | Advance::PageNext) && items_walk.is_none() {
        return None;
    }
    let item_type = items_walk.as_ref().map(|walk| match &walk.leaf {
        Leaf::Items { item } => item.clone(),
        _ => unreachable!("filtered above"),
    });

    let method = &operation.method_name;
    let pages = super::models::allocate(&format!("{method}Pages"), methods);
    let next_page = super::models::allocate(&format!("{method}NextPage"), methods);
    let advance_helper = super::models::allocate(&format!("{method}Advance"), methods);
    let items = items_walk
        .is_some()
        .then(|| super::models::allocate(&format!("{method}Items"), methods));
    let items_helper = items_walk
        .is_some()
        .then(|| super::models::allocate(&format!("{method}PageItems"), methods));
    let continuation_helper = continuation
        .as_ref()
        .map(|_| super::models::allocate(&format!("{method}PageContinuation"), methods));
    let has_more_helper = has_more
        .as_ref()
        .map(|_| super::models::allocate(&format!("{method}PageHasMore"), methods));

    // Pre-render the private typed accessors over the compiled pointers.
    let data_type = operation.result_data_type.clone().unwrap_or_default();
    let origin = operation.source.clone();
    let mut accessors = String::new();
    if let (Some(helper), Some(walk)) = (items_helper.as_deref(), items_walk.as_ref()) {
        accessors.push_str(&accessor_source(
            &data_type,
            &origin,
            helper,
            &format!(
                "List<{}>?",
                item_type.as_deref().unwrap_or("JsonValue")
            ),
            &walk.body,
            &walk.place,
            &format!(
                "Items collection of one decoded {method} page at the compiled pointer; an absent path reads as no items."
            ),
        ));
    }
    if let (Some(helper), Some(walk)) = (continuation_helper.as_deref(), continuation.as_ref()) {
        let returns = match control.kind {
            ControlKind::Integer => "JsonNumber?",
            ControlKind::Text => "String?",
        };
        accessors.push_str(&accessor_source(
            &data_type,
            &origin,
            helper,
            returns,
            &walk.body,
            &walk.place,
            &format!(
                "Continuation value of one decoded {method} page at the compiled pointer; an absent path reads as no continuation."
            ),
        ));
    }
    if let (Some(helper), Some(walk)) = (has_more_helper.as_deref(), has_more.as_ref()) {
        accessors.push_str(&accessor_source(
            &data_type,
            &origin,
            helper,
            "Boolean?",
            &walk.body,
            &walk.place,
            &format!(
                "Has-more evidence of one decoded {method} page at the compiled pointer; an absent path reads as no evidence."
            ),
        ));
    }

    Some(PaginationOperation {
        index: 0,
        operation: operation.operation_id.clone(),
        source: operation.source.clone(),
        method: method.clone(),
        input_type: operation.input_type.clone(),
        result_type: operation.result_type.clone(),
        data_type: operation.result_data_type.clone(),
        pages,
        items,
        next_page,
        advance_helper,
        items_helper,
        continuation_helper,
        has_more_helper,
        advance,
        control,
        limit,
        initial_offset: page.initial_offset,
        item_type,
        input_default: operation.input_has_default(),
        accessors,
        member: String::new(),
    })
}

/// Lower the shared pagination selection for one plan. Operations whose
/// selection cannot be expressed with typed field accesses are left without
/// members instead of emitting guesses.
pub(super) fn lower(
    models: &ModelPlan,
    outcome: &wire::PaginationOutcome,
    operations: &[PlannedOperation],
    methods: &mut BTreeSet<String>,
) -> PaginationPlan {
    let mut compiled = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        let Some(page) = outcome
            .paginated
            .iter()
            .find(|page| page.operation == operation.operation_id)
        else {
            continue;
        };
        if let Some(mut operation) = compile(models, operation, page, methods) {
            operation.index = index;
            operation.member = render_member(&operation);
            compiled.push(operation);
        }
    }
    PaginationPlan {
        outcome: outcome.clone(),
        operations: compiled,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

use super::emit::{kdoc, quote, source, source_value};

/// The typed traversal exception emitted once before the `Client` class.
pub(super) fn exception() -> String {
    "/** Raised instead of looping forever when a source API repeats an identical\n * pagination continuation value. */\npublic class PaginationStalled(\n    /** The repeated continuation value, exactly as the source returned it. */\n    public val continuation: String,\n    /** Original operation ID. */\n    public val operationId: String?,\n    /** Original operation source identity. */\n    public val source: SourceLocation?,\n) : RuntimeException(\n    \"the source API returned an identical continuation value; the paginated walk would never terminate\",\n)\n\n".to_owned()
}

fn stop_rule(operation: &PaginationOperation) -> String {
    match operation.advance {
        Advance::OffsetItemCount | Advance::PageNext => {
            if operation.has_more_helper.is_some() {
                "on a page with no items or when the has-more indicator is false".to_owned()
            } else {
                "on a page with no items".to_owned()
            }
        }
        Advance::CursorPointer => {
            "when the next cursor token is absent, null or an empty string".to_owned()
        }
        Advance::OffsetPointer => "when the next pointer is absent or null".to_owned(),
    }
}

/// The expression reading the control value that produced the current page.
fn read_control(operation: &PaginationOperation, expression: &str) -> String {
    let name = &operation.control.name;
    if operation.control.optional {
        format!(
            "(when (val member = {expression}.{name}) {{\n            is Presence.Present -> member.value\n            Presence.Absent -> null\n        }})"
        )
    } else {
        format!("{expression}.{name}")
    }
}

/// The named-argument expression writing the control back into the input.
fn write_control(operation: &PaginationOperation, value: &str) -> String {
    let name = &operation.control.name;
    if operation.control.optional {
        format!("{name} = Presence.Present({value})")
    } else {
        format!("{name} = {value}")
    }
}

/// The KDoc for the page walk of one operation.
fn pages_kdoc(operation: &PaginationOperation) -> String {
    let control = operation.control.name.as_str();
    let fallback = operation
        .limit
        .as_ref()
        .filter(|(limit_control, _)| limit_control.optional)
        .map(|(_, size)| {
            format!(
                " When the caller omits the limit, the first request supplies the documented SDK page size ({size}); later pages keep the limit the walk last used."
            )
        })
        .unwrap_or_default();
    format!(
        "Lazily yields every success page of {}. The first page is the direct {}(...) result, fetched when the flow is first collected and included exactly once; every collection re-fetches every page. Between requests only pagination controls change: every other input member is preserved exactly, and a caller-supplied {} is used for the first request and replaced by the computed continuation afterwards.{fallback} Stops {}. A repeated identical continuation value raises PaginationStalled instead of looping. Collection is lazy: no request happens before the flow is collected, cancelling or ending collection early prevents further requests, and pages are never prefetched.\nSource: {}",
        kdoc(&operation.operation),
        operation.method,
        control,
        stop_rule(operation),
        source(&operation.source),
    )
}

/// The KDoc for the item walk of one operation.
fn items_kdoc(operation: &PaginationOperation) -> String {
    format!(
        "Flattens the page items of {}(...) across every page, in order. Accepts the same input as the direct call; see {} for lazy fetching, continuation, stopping and caller-override semantics.\nSource: {}",
        operation.method,
        operation.pages,
        source(&operation.source),
    )
}

/// The KDoc for the next-input builder of one operation.
fn next_page_kdoc(operation: &PaginationOperation) -> String {
    format!(
        "Fetches the page described by the input and returns the rebuilt input for the following page, or null when the walk stops (including on failure; use {} when the cause matters). Source: {}",
        operation.pages,
        source(&operation.source),
    )
}

/// The private advance helper: the continuation computation shared by the
/// flows and the next-input builder.
fn advance_source(operation: &PaginationOperation) -> String {
    let input = &operation.input_type;
    let result = &operation.result_type;
    let helper = &operation.advance_helper;
    let operation_id = quote(&operation.operation);
    let source = source_value(&operation.source);
    let mut out = String::new();
    out.push_str(
        "    /** Computes the input for the page after [page], or null when the walk\n     * stops. A repeated identical continuation value raises\n     * PaginationStalled instead of looping. */\n",
    );
    out.push_str(&format!(
        "    private fun {helper}(current: {input}, page: {result}): {input}? {{\n"
    ));
    out.push_str("        val data = page.data\n");
    match operation.advance {
        Advance::OffsetItemCount | Advance::PageNext => {
            out.push_str(&format!(
                "        val items = {}(data) ?: emptyList()\n",
                operation.items_helper.as_deref().unwrap_or_default()
            ));
            if let Some(has_more) = &operation.has_more_helper {
                out.push_str(&format!(
                    "        if ({has_more}(data) == false) return null\n"
                ));
            }
            out.push_str("        val count = items.size\n");
            out.push_str("        if (count == 0) return null\n");
            let (fallback, message) = match operation.advance {
                Advance::PageNext => (
                    operation
                        .initial_offset
                        .map_or_else(|| "1L".to_owned(), |value| format!("{value}L")),
                    "the page number is not an integer",
                ),
                _ => (
                    operation
                        .initial_offset
                        .map_or_else(|| "0L".to_owned(), |value| format!("{value}L")),
                    "the pagination offset is not an integer",
                ),
            };
            out.push_str("        val base = ");
            out.push_str(&read_control(operation, "current"));
            if operation.control.optional {
                out.push_str(&format!(" ?: JsonNumber.of({fallback})"));
            }
            out.push('\n');
            out.push_str(&format!(
                "        if (!base.isInteger()) throw SdkException(FailureKind.REQUEST_VALIDATION, {message:?}, operationId = {operation_id}, source = {source})\n"
            ));
            let step = match operation.advance {
                Advance::PageNext => "java.math.BigInteger.ONE",
                _ => "java.math.BigInteger.valueOf(count.toLong())",
            };
            out.push_str(&format!(
                "        val advanced = base.toBigIntegerExact().add({step})\n"
            ));
            out.push_str(&format!(
                "        return current.copy({})\n",
                write_control(operation, "JsonNumber.of(advanced)")
            ));
        }
        Advance::CursorPointer | Advance::OffsetPointer => {
            out.push_str(&format!(
                "        val token = {}(data) ?: return null\n",
                operation.continuation_helper.as_deref().unwrap_or_default()
            ));
            if operation.advance == Advance::CursorPointer {
                out.push_str("        if (token.isEmpty()) return null\n");
            }
            out.push_str("        val previous = ");
            out.push_str(&read_control(operation, "current"));
            out.push('\n');
            let repeated = match operation.control.optional {
                true => "previous != null && token == previous".to_owned(),
                false => "token == previous".to_owned(),
            };
            let repeated_value = match operation.control.kind {
                ControlKind::Text => "token".to_owned(),
                ControlKind::Integer => "token.toString()".to_owned(),
            };
            out.push_str(&format!(
                "        if ({repeated}) throw PaginationStalled({repeated_value}, {operation_id}, {source})\n"
            ));
            out.push_str(&format!(
                "        return current.copy({})\n",
                write_control(operation, "token")
            ));
        }
    }
    out.push_str("    }\n");
    out
}

/// One accessor member for one compiled pointer.
fn accessor_source(
    data_type: &str,
    origin: &SourceId,
    helper: &str,
    returns: &str,
    body: &[String],
    place: &str,
    comment: &str,
) -> String {
    let mut out = format!(
        "    /** {comment} Source: {} */\n    private fun {helper}(page: {data_type}): {returns} {{\n",
        source(origin)
    );
    for line in body {
        out.push_str("        ");
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!("        return {place}\n    }}\n"));
    out
}

/// The complete generated members for one paginated operation.
fn render_member(operation: &PaginationOperation) -> String {
    let input = &operation.input_type;
    let default = if operation.input_default {
        format!(" = {input}()")
    } else {
        String::new()
    };
    let mut out = operation.accessors.clone();
    out.push_str(&advance_source(operation));
    if let (Some(items), Some(helper)) = (&operation.items, &operation.items_helper) {
        let item = operation.item_type.as_deref().unwrap_or("JsonValue");
        out.push_str(&format!(
            "    /** {} */\n    public fun {items}(input: {input}{default}, requestOptions: RequestOptions = RequestOptions()): Flow<{item}> = flow {{\n        {}(input, requestOptions).collect {{ page ->\n            for (item in {}(page.data) ?: emptyList()) emit(item)\n        }}\n    }}\n",
            kdoc(&items_kdoc(operation)),
            operation.pages,
            helper,
        ));
    }
    out.push_str(&format!(
        "    /** {} */\n    public fun {}(input: {input}{default}, requestOptions: RequestOptions = RequestOptions()): Flow<{}> = flow {{\n",
        kdoc(&pages_kdoc(operation)),
        operation.pages,
        operation.result_type
    ));
    out.push_str("        var current = input\n");
    if operation.control.optional
        && let Some(value) = operation.initial_offset
    {
        out.push_str(&format!(
            "        if (current.{} is Presence.Absent) current = current.copy({})\n",
            operation.control.name,
            write_control(operation, &format!("JsonNumber.of({value}L)")),
        ));
    }
    // The documented SDK fallback page size fills the limit control on the
    // first request only, when the caller left it absent; later pages keep
    // whatever limit the walk last used.
    if let Some((limit_control, size)) = &operation.limit
        && limit_control.optional {
            let name = &limit_control.name;
            out.push_str(&format!(
                "        if (current.{name} is Presence.Absent) current = current.copy({name} = Presence.Present(JsonNumber.of({size}L)))\n"
            ));
        }
    out.push_str("        while (true) {\n");
    out.push_str(&format!(
        "            val page = {}(current, requestOptions)\n",
        operation.method
    ));
    out.push_str("            emit(page)\n");
    out.push_str(&format!(
        "            current = {}(current, page) ?: return@flow\n",
        operation.advance_helper
    ));
    out.push_str("        }\n    }\n");
    out.push_str(&format!(
        "    /** {} */\n    public suspend fun {}(input: {input}{default}, requestOptions: RequestOptions = RequestOptions()): {input}? {{\n        val page = {}(input, requestOptions)\n        return {}(input, page)\n    }}\n",
        kdoc(&next_page_kdoc(operation)),
        operation.next_page,
        operation.method,
        operation.advance_helper,
    ));
    out
}

