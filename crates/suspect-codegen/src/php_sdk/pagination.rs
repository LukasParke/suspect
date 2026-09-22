//! Emitted-only pagination traversal for the generated PHP client.
//!
//! The shared, generation-time `http_protocol::plan_pagination` outcome compiles
//! into per-operation page/item generator methods plus a next-page arguments
//! builder on the generated `Client` class — all inside the generated
//! `Client.php`. Static runtime files and the shared planner stay untouched, and
//! plans without configured client defaults (or without an emittable paginated
//! operation) emit no new bytes at all.
//!
//! Continuation rules follow the compiled entry exactly: `ItemsReturned`
//! advances an integer offset by the previous page's item count; `NextOffset`
//! applies the resolved next pointer to the pattern's continuation parameter.
//! Only pagination controls change between requests — every other constructor
//! argument is preserved exactly, and caller-supplied controls win for the
//! first request. Generators are lazy, never prefetch, and a repeated identical
//! continuation value throws `SdkError('pagination-stalled')` instead of
//! looping. Response pointers resolve at generation time into generated
//! property chains over the decoded result models; there is no runtime schema
//! search and no JSONPath evaluation.

use std::collections::BTreeSet;

use super::emit::{doc_tags, php};
use super::models::{self, Shape};
use super::{Operation, Payload, Response, SdkPlan};
use crate::http_protocol::{OperationPagination, ParameterLocation, ResponseStatus};
use crate::sdk_defaults::{
    PaginationAdvance, PaginationPattern, PaginationRequestRole, PaginationResponseRole,
};
use suspect_ir::contract::SchemaId;

/// How one walk computes and applies its continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Advance {
    /// The offset control advances by the previous page's item count. The
    /// documented stop rules are a zero-item page or a false has-more flag.
    OffsetItemCount,
    /// The one-based page control advances by one (the documented fallback when
    /// no server continuation pointer is mapped).
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

/// One compiled pagination walk, ready to render onto the generated client.
pub(super) struct PaginatedOperation<'a> {
    operation: &'a Operation,
    page: &'a OperationPagination,
    /// The direct client method this walk extends.
    method: String,
    /// The input class of the direct call.
    input: String,
    /// Allocated public method names.
    pages: String,
    items: String,
    next_page: String,
    /// Allocated private helper names.
    items_reader: String,
    continuation_reader: String,
    has_more_reader: Option<String>,
    input_helper: String,
    arguments_helper: String,
    advance: Advance,
    /// The input member carrying the rewritten control.
    control_member: String,
    /// `JsonNumber` or `string` — the native type of the control value.
    control_type: String,
    control_required: bool,
    /// The configured default when the caller supplies no control value.
    initial: u32,
    /// True when page 1 rebuilds an absent control with the configured initial.
    inject_initial: bool,
    /// The optional input member carrying the documented SDK fallback page
    /// size, with its allocated rebuild helper and the configured size.
    limit: Option<(String, String, u32)>,
    /// Union of the operation's success response classes.
    result: String,
    /// The success response class the compiled pointers resolve against.
    response: String,
    /// True when the result union has more than one class.
    multiple_results: bool,
    /// PHPDoc item type of the flattened items generator.
    item_type: String,
    /// Constructor-argument default fragment reused from the direct method.
    argument_default: String,
    items_chain: Option<Chain>,
    continuation_chain: Option<(Chain, PaginationResponseRole)>,
    has_more_chain: Option<Chain>,
}

/// One resolved pointer reader: PHP statements over `$value` plus the schema of
/// the value they leave behind.
struct Chain {
    steps: Vec<String>,
    id: SchemaId,
}

fn deref<'a>(plan: &'a models::ModelPlan, id: &'a SchemaId) -> &'a SchemaId {
    match &plan.nodes[id].shape {
        Shape::Ref(target) => deref(plan, target),
        _ => id,
    }
}

/// The native PHP type of one request control value, or `None` when the planned
/// parameter cannot serve as a rewriteable pagination control.
fn control_type(plan: &SdkPlan, id: &SchemaId) -> Option<String> {
    let node = &plan.models().nodes[deref(plan.models(), id)];
    if node.nullable {
        return None;
    }
    match &node.shape {
        Shape::String => Some("string".into()),
        Shape::Number => Some("JsonNumber".into()),
        _ => None,
    }
}

/// The decoded-body schema of the operation's first JSON success response —
/// the same response the shared planner compiled pointers against.
fn success_payload(op: &Operation) -> Option<(&Response, &SchemaId)> {
    let key = op
        .responses
        .iter()
        .find(|response| {
            matches!(
                response.wire.status(),
                ResponseStatus::Exact(200..=299) | ResponseStatus::Range(2)
            )
        })?
        .status_key
        .clone();
    let response = op.responses.iter().find(|response| {
        response.status_key == key && matches!(response.payload, Payload::Schema(_))
    })?;
    match &response.payload {
        Payload::Schema(id) => Some((response, id)),
        _ => None,
    }
}

/// Resolve one RFC 6901 pointer into generated property chains over the
/// decoded body model. Every intermediate segment must name a declared field
/// of a native object model (or a member of a JSON-value carrier); anything
/// else cannot be walked and the operation stays without walkers.
fn resolve(plan: &SdkPlan, root: &SchemaId, pointer: &str) -> Option<Chain> {
    let mut steps = vec!["$value = $page->body;".to_owned()];
    let mut current = deref(plan.models(), root).clone();
    if pointer.is_empty() {
        return Some(Chain { steps, id: current });
    }
    for raw in pointer[1..].split('/') {
        let segment = raw.replace("~1", "/").replace("~0", "~");
        match &plan.models().nodes[&current].shape {
            Shape::Object { fields, .. } => {
                let field = fields.iter().find(|field| field.wire == segment)?;
                let field_node = &plan.models().nodes[&field.source];
                steps.push(format!("$value = $value->{};", field.name));
                let mut guards = Vec::new();
                if !field.required {
                    guards.push("$value === Absent::Value".to_owned());
                }
                if field_node.nullable {
                    guards.push("$value === null".to_owned());
                }
                if !guards.is_empty() {
                    steps.push(format!("if ({}) {{ return null; }}", guards.join(" || ")));
                }
                current = deref(plan.models(), &field.source).clone();
            }
            Shape::Json => {
                steps.push(format!(
                    "$value = $value->kind === JsonKind::Object ? ($value->asObject()[{}] ?? null) : null;",
                    php(&segment)
                ));
                steps.push("if ($value === null) { return null; }".to_owned());
            }
            _ => return None,
        }
    }
    Some(Chain { steps, id: current })
}

/// The final expression, PHP return type and optional item type reading one
/// role's value from a resolved chain.
fn leaf(
    plan: &SdkPlan,
    chain: &Chain,
    role: PaginationResponseRole,
) -> Option<(String, &'static str, Option<String>)> {
    let node = &plan.models().nodes[&chain.id];
    if node.nullable && !matches!(node.shape, Shape::Types(_)) {
        // The planner validated the pointer against the declared schema; a
        // value-domain-null leaf cannot distinguish absence from null here.
        return None;
    }
    let value = "$value".to_owned();
    Some(match (&node.shape, role) {
        (Shape::Array { item }, PaginationResponseRole::Items) => (
            value,
            "?array",
            Some(
                item.as_ref()
                    .map(|item| plan.models().type_name(item, true))
                    .unwrap_or_else(|| "mixed".into()),
            ),
        ),
        (Shape::String, PaginationResponseRole::NextCursor) => (value, "?string", None),
        (Shape::Enum { .. }, PaginationResponseRole::NextCursor) => {
            ("$value->value".into(), "?string", None)
        }
        (Shape::Number, PaginationResponseRole::NextCursor) => {
            ("$value->toDecimalString()".into(), "?string", None)
        }
        (Shape::Number, PaginationResponseRole::NextOffset) => (value, "?JsonNumber", None),
        (Shape::Boolean, PaginationResponseRole::HasMore) => (value, "?bool", None),
        (Shape::Json, PaginationResponseRole::Items) => (
            "$value->kind === JsonKind::Array ? $value->asArray() : null".into(),
            "?array",
            Some("mixed".into()),
        ),
        (Shape::Json, PaginationResponseRole::NextCursor) => (
            "$value->kind === JsonKind::String ? $value->asString() : null".into(),
            "?string",
            None,
        ),
        (Shape::Json, PaginationResponseRole::NextOffset) => (
            "$value->kind === JsonKind::Number ? $value->asNumber() : null".into(),
            "?JsonNumber",
            None,
        ),
        (Shape::Json, PaginationResponseRole::HasMore) => (
            "$value->kind === JsonKind::Boolean ? $value->asBool() : null".into(),
            "?bool",
            None,
        ),
        _ => return None,
    })
}

fn pointer_documentation(pointer: &str) -> String {
    if pointer.is_empty() {
        "whole-body".to_owned()
    } else {
        format!("`{pointer}`")
    }
}

fn reader_name(compiled: &PaginatedOperation<'_>, role: PaginationResponseRole) -> Option<String> {
    match role {
        PaginationResponseRole::Items => Some(compiled.items_reader.clone()),
        PaginationResponseRole::NextCursor | PaginationResponseRole::NextOffset => {
            Some(compiled.continuation_reader.clone())
        }
        PaginationResponseRole::HasMore => compiled.has_more_reader.clone(),
        _ => None,
    }
}

fn reader_comment(compiled: &PaginatedOperation<'_>, role: PaginationResponseRole) -> String {
    let pointer = compiled
        .page
        .response
        .get(&role)
        .map(String::as_str)
        .unwrap_or_default();
    match role {
        PaginationResponseRole::Items => format!(
            "Page items of one {} result at the configured {} pointer; an absent path reads as no items.",
            compiled.operation.id,
            pointer_documentation(pointer)
        ),
        PaginationResponseRole::NextCursor | PaginationResponseRole::NextOffset => format!(
            "Continuation value of one {} result at the configured {} pointer; an absent path stops the walk.",
            compiled.operation.id,
            pointer_documentation(pointer)
        ),
        PaginationResponseRole::HasMore => format!(
            "Whether the source declared no further pages of {} at the configured {} pointer.",
            compiled.operation.id,
            pointer_documentation(pointer)
        ),
        _ => String::new(),
    }
}

/// Render one private static pointer reader on the client class.
fn reader(
    plan: &SdkPlan,
    compiled: &PaginatedOperation<'_>,
    chain: &Chain,
    role: PaginationResponseRole,
) -> Option<String> {
    let (expression, returns, _) = leaf(plan, chain, role)?;
    let name = reader_name(compiled, role)?;
    let mut out = String::new();
    doc_tags(&mut out, &reader_comment(compiled, role), &[]);
    out.push_str(&format!(
        "    private static function {name}({} $page): {returns}\n    {{\n",
        compiled.result
    ));
    if compiled.multiple_results {
        out.push_str(&format!(
            "        if (!$page instanceof {}) {{ return null; }}\n",
            compiled.response
        ));
    }
    for step in &chain.steps {
        out.push_str(&format!("        {step}\n"));
    }
    out.push_str(&format!("        return {expression};\n    }}\n"));
    Some(out)
}

/// The operation's input members: allocated parameter members plus the body.
fn arguments(operation: &Operation) -> Vec<(String, bool)> {
    let mut members: Vec<(String, bool)> = operation
        .parameters
        .iter()
        .map(|parameter| (parameter.name.clone(), parameter.wire.required()))
        .collect();
    if !operation.body.is_empty() {
        members.push(("body".into(), operation.body_required));
    }
    members
}

fn payload_type(plan: &SdkPlan, payload: &Payload, doc: bool) -> String {
    match payload {
        Payload::Schema(id) => plan.models().type_name(id, doc),
        Payload::Json => "JsonValue".into(),
        Payload::Text => "string".into(),
        Payload::Bytes => "Bytes".into(),
        Payload::NoBody => "NoBody".into(),
        Payload::Object(name) => name.clone(),
        Payload::Stream(_) => "ItemStream".into(),
    }
}

fn input_type(plan: &SdkPlan, operation: &Operation, doc: bool) -> String {
    models::union(
        operation
            .body
            .iter()
            .map(|media| {
                media
                    .wrapper
                    .clone()
                    .unwrap_or_else(|| payload_type(plan, &media.payload, doc))
            })
            .collect(),
    )
}

/// The input member names, PHPDoc types and requiredness.
fn member_types(plan: &SdkPlan, operation: &Operation) -> Vec<(String, String, bool)> {
    let mut members: Vec<(String, String, bool)> = operation
        .parameters
        .iter()
        .map(|parameter| {
            (
                parameter.name.clone(),
                plan.models().type_name(&parameter.schema, true),
                parameter.wire.required(),
            )
        })
        .collect();
    if !operation.body.is_empty() {
        members.push((
            "body".into(),
            input_type(plan, operation, true),
            operation.body_required,
        ));
    }
    members
}

/// One input with only the pagination control replaced.
fn input_reader(plan: &SdkPlan, compiled: &PaginatedOperation<'_>) -> String {
    let arguments = member_types(plan, compiled.operation)
        .iter()
        .map(|(name, _, _)| {
            if *name == compiled.control_member {
                format!("{name}: ${}", compiled.control_member)
            } else {
                format!("{name}: $input->{name}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = String::new();
    doc_tags(
        &mut out,
        &format!(
            "One {} input with only the {} control replaced; every other constructor argument is preserved exactly.",
            compiled.operation.id, compiled.control_member
        ),
        &[],
    );
    out.push_str(&format!(
        "    private static function {}({} $input, {} ${}): {}\n    {{\n        return new {}({arguments});\n    }}\n",
        compiled.input_helper,
        compiled.input,
        compiled.control_type,
        compiled.control_member,
        compiled.input,
        compiled.input,
    ));
    out
}

/// One input with only the fallback page-size member replaced.
fn limit_reader(plan: &SdkPlan, compiled: &PaginatedOperation<'_>) -> String {
    let Some((member, helper, _size)) = &compiled.limit else {
        return String::new();
    };
    let arguments = member_types(plan, compiled.operation)
        .iter()
        .map(|(name, _, _)| {
            if name == member {
                format!("{name}: ${member}")
            } else {
                format!("{name}: $input->{name}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = String::new();
    doc_tags(
        &mut out,
        &format!(
            "One {} input with only the {member} limit replaced; every other constructor argument is preserved exactly.",
            compiled.operation.id
        ),
        &[],
    );
    out.push_str(&format!(
        "    private static function {helper}({} $input, JsonNumber ${member}): {}\n    {{\n        return new {}({arguments});\n    }}\n",
        compiled.input, compiled.input, compiled.input,
    ));
    out
}

/// The constructor arguments for the page after one fetch.
fn arguments_reader(plan: &SdkPlan, compiled: &PaginatedOperation<'_>) -> String {
    let mut shape = Vec::new();
    let mut values = Vec::new();
    for (name, ty, required) in member_types(plan, compiled.operation) {
        if name == compiled.control_member {
            shape.push(format!("{name}: {}", compiled.control_type));
            values.push(format!("'{name}' => ${name}"));
        } else {
            let member = if required {
                ty
            } else {
                models::union(vec![ty, "Absent".into()])
            };
            shape.push(format!("{name}: {member}"));
            values.push(format!("'{name}' => $input->{name}"));
        }
    }
    let mut out = String::new();
    doc_tags(
        &mut out,
        &format!(
            "Constructor arguments for the page after one {} fetch; pass them with `new {}(...$next)`.",
            compiled.operation.id, compiled.input
        ),
        &[format!("@return array{{{}}}", shape.join(", "))],
    );
    out.push_str(&format!(
        "    private static function {}({} $input, {} ${}): array\n    {{\n        return [{}];\n    }}\n",
        compiled.arguments_helper,
        compiled.input,
        compiled.control_type,
        compiled.control_member,
        values.join(", "),
    ));
    out
}

/// Compile every emittable paginated operation of the plan, in plan order.
/// The planner already validated every request name and response pointer, so
/// this only lowers the compiled entry onto native member names and drops
/// operations whose native representations cannot express the walk.
pub(super) fn operations(plan: &SdkPlan) -> Vec<PaginatedOperation<'_>> {
    let Some(outcome) = plan.pagination() else {
        return Vec::new();
    };
    if outcome.paginated.is_empty() {
        return Vec::new();
    }
    let mut used: BTreeSet<String> = ["__construct", "exchange"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if plan.credential_env().is_some() {
        used.insert("fromenv".into());
    }
    for operation in plan.operations() {
        used.insert(operation.method.clone());
    }
    let mut compiled = Vec::new();
    for operation in plan.operations() {
        let Some(page) = outcome
            .paginated
            .iter()
            .find(|page| page.operation == operation.id)
        else {
            continue;
        };
        if let Some(built) = build(plan, operation, page, &mut used) {
            compiled.push(built);
        }
    }
    compiled
}

#[allow(clippy::too_many_lines)]
fn build<'a>(
    plan: &'a SdkPlan,
    operation: &'a Operation,
    page: &'a OperationPagination,
    used: &mut BTreeSet<String>,
) -> Option<PaginatedOperation<'a>> {
    use PaginationRequestRole as Role;
    if page.pattern == PaginationPattern::NextLink {
        // A continuation URL cannot replace the request target through the
        // typed input; next-link following needs declared URL policy.
        return None;
    }
    let (response, root) = success_payload(operation)?;
    let successes: Vec<String> = operation
        .responses
        .iter()
        .filter(|candidate| candidate.success)
        .map(|candidate| candidate.name.clone())
        .collect();
    if successes.is_empty() {
        return None;
    }
    let control_role = match page.pattern {
        PaginationPattern::Cursor => Role::Cursor,
        PaginationPattern::PageNumber => match page.advance {
            PaginationAdvance::NextOffset => Role::Offset,
            PaginationAdvance::ItemsReturned => Role::Page,
        },
        PaginationPattern::LimitOffset | PaginationPattern::NextLink => Role::Offset,
    };
    let control_wire = page.request.get(&control_role)?;
    let parameter = operation
        .parameters
        .iter()
        .find(|parameter| {
            parameter.wire.location() == ParameterLocation::Query
                && parameter.wire.name() == control_wire.as_str()
        })?;
    let control_member = parameter.name.clone();
    let control_type_value = control_type(plan, &parameter.schema)?;
    let control_required = parameter.wire.required();
    // The documented SDK fallback page size fills an optional limit member on
    // page 1 only, when the caller left it absent; a required limit member has
    // no absent state to fall back from.
    let limit_member = page.request.get(&Role::Limit).and_then(|wire| {
        operation
            .parameters
            .iter()
            .find(|parameter| {
                parameter.wire.location() == ParameterLocation::Query
                    && parameter.wire.name() == wire.as_str()
            })
            .filter(|parameter| !parameter.wire.required())
            .map(|parameter| parameter.name.clone())
    });
    let limit_size = page.initial_limit;

    let items_chain = page
        .response
        .get(&PaginationResponseRole::Items)
        .and_then(|pointer| resolve(plan, root, pointer));
    let cursor_chain = page
        .response
        .get(&PaginationResponseRole::NextCursor)
        .and_then(|pointer| resolve(plan, root, pointer));
    let offset_chain = page
        .response
        .get(&PaginationResponseRole::NextOffset)
        .and_then(|pointer| resolve(plan, root, pointer));
    let has_more_chain = page
        .response
        .get(&PaginationResponseRole::HasMore)
        .and_then(|pointer| resolve(plan, root, pointer));

    let advance = match page.pattern {
        PaginationPattern::Cursor => {
            cursor_chain.as_ref()?;
            Advance::CursorPointer
        }
        PaginationPattern::LimitOffset | PaginationPattern::PageNumber => {
            let pointer_advance = page.advance == PaginationAdvance::NextOffset
                && offset_chain.is_some()
                && page.request.contains_key(&Role::Offset);
            if pointer_advance {
                Advance::OffsetPointer
            } else if page.pattern == PaginationPattern::PageNumber {
                if page.request.contains_key(&Role::Page) && items_chain.is_some() {
                    Advance::PageNext
                } else {
                    return None;
                }
            } else if page.request.contains_key(&Role::Offset) && items_chain.is_some() {
                Advance::OffsetItemCount
            } else {
                return None;
            }
        }
        PaginationPattern::NextLink => unreachable!("rejected above"),
    };
    let continuation_chain = match advance {
        Advance::CursorPointer => cursor_chain.map(|chain| (chain, PaginationResponseRole::NextCursor)),
        Advance::OffsetPointer => offset_chain.map(|chain| (chain, PaginationResponseRole::NextOffset)),
        Advance::OffsetItemCount | Advance::PageNext => None,
    };

    let mut allocate = |base: &str| models::allocate(base, used);
    let pages = allocate(&format!("{}Pages", operation.method));
    let items = allocate(&format!("{}Items", operation.method));
    let next_page = allocate(&format!("{}NextPage", operation.method));
    let items_reader = allocate(&format!("{}PageItems", operation.method));
    let continuation_reader = allocate(&format!(
        "{}Page{}",
        operation.method,
        match advance {
            Advance::CursorPointer => "Cursor",
            _ => "Offset",
        }
    ));
    let has_more_reader = has_more_chain
        .as_ref()
        .map(|_| allocate(&format!("{}PageHasMore", operation.method)));
    let input_helper = allocate(&format!("{}PageInput", operation.method));
    let arguments_helper = allocate(&format!("{}PageArguments", operation.method));
    let limit = limit_member
        .zip(limit_size)
        .map(|(member, size)| {
            let helper = allocate(&format!("{}PageLimit", operation.method));
            (member, helper, size)
        });

    let item_type = items_chain
        .as_ref()
        .and_then(|chain| leaf(plan, chain, PaginationResponseRole::Items))
        .and_then(|(_, _, item)| item)
        .unwrap_or_else(|| "mixed".into());
    let argument_default = if arguments(operation)
        .iter()
        .any(|(_, required)| *required)
    {
        String::new()
    } else {
        format!(" = new {}()", operation.input)
    };
    Some(PaginatedOperation {
        operation,
        page,
        method: operation.method.clone(),
        input: operation.input.clone(),
        pages,
        items,
        next_page,
        items_reader,
        continuation_reader,
        has_more_reader,
        input_helper,
        arguments_helper,
        advance,
        control_member,
        control_type: control_type_value,
        control_required,
        initial: page.initial_offset.unwrap_or(0),
        inject_initial: page.initial_offset.is_some()
            && matches!(control_role, Role::Offset | Role::Page),
        limit,
        result: models::union(successes.clone()),
        response: response.name.clone(),
        multiple_results: successes.len() > 1,
        item_type,
        argument_default,
        items_chain,
        continuation_chain,
        has_more_chain,
    })
}

/// The stop-rule prose shared by the page and item walk documentation.
fn stop_rule(compiled: &PaginatedOperation<'_>) -> String {
    match compiled.advance {
        Advance::CursorPointer => {
            "when the next cursor token is absent, null or an empty string".into()
        }
        Advance::OffsetPointer => "when the next pointer is absent or null".into(),
        Advance::OffsetItemCount | Advance::PageNext => {
            if compiled.has_more_chain.is_some() {
                "after a page with no items or when the has-more indicator is false".into()
            } else {
                "after a page with no items".into()
            }
        }
    }
}

fn pages_doc(compiled: &PaginatedOperation<'_>) -> String {
    let fallback = compiled
        .limit
        .as_ref()
        .map(|(_, _, size)| {
            format!(
                " When the caller omits the limit, the first request supplies the documented SDK page size ({size}); later pages keep the limit the walk last used."
            )
        })
        .unwrap_or_default();
    format!(
        "Lazily yields every success page of {}. The first page is the direct {}(...) result, fetched on the first iteration and included exactly once. Between requests only pagination controls change: every other constructor argument is preserved exactly, and a caller-supplied {} is used for the first request and replaced by the computed continuation afterwards.{fallback} Stops {}. A repeated identical continuation value throws SdkError('pagination-stalled') instead of looping. Iteration is lazy: no request happens before the first iteration, breaking out of the foreach prevents further requests, and pages are never prefetched.\nSource: {}#{}",
        compiled.page.operation,
        compiled.method,
        compiled.control_member,
        stop_rule(compiled),
        compiled.operation.source.document(),
        compiled.operation.source.pointer()
    )
}

fn items_doc(compiled: &PaginatedOperation<'_>) -> String {
    format!(
        "Flattens the page items of {}(...) across every page, in order. Accepts the same arguments as the direct call; see {} for lazy fetching, continuation, stopping and caller-override semantics.\nSource: {}#{}",
        compiled.method,
        compiled.pages,
        compiled.operation.source.document(),
        compiled.operation.source.pointer()
    )
}

fn next_page_doc(compiled: &PaginatedOperation<'_>) -> String {
    format!(
        "Fetches the page described by $input and returns the constructor arguments for the following page's {}, or null when the walk stops; pass them with `new {}(...$next)`. Every other constructor argument is preserved exactly. A repeated identical continuation value throws SdkError('pagination-stalled') instead of looping.\nSource: {}#{}",
        compiled.input,
        compiled.input,
        compiled.operation.source.document(),
        compiled.operation.source.pointer()
    )
}

#[allow(clippy::too_many_lines)]
fn pages_method(compiled: &PaginatedOperation<'_>) -> String {
    let control = &compiled.control_member;
    let mut body = String::new();
    if let Some((member, helper, size)) = &compiled.limit {
        // The documented SDK fallback page size applies to the first request
        // only, when the caller left the limit absent; later pages keep
        // whatever limit the walk last used.
        body.push_str(&format!(
            "        if ($input->{member} === Absent::Value) {{\n            $input = self::{helper}($input, JsonNumber::fromInt({size}));\n        }}\n"
        ));
    }
    match compiled.advance {
        Advance::OffsetItemCount | Advance::PageNext => {
            let fallback = match compiled.advance {
                Advance::OffsetItemCount => compiled.initial.to_string(),
                _ => "1".to_owned(),
            };
            if compiled.control_required {
                body.push_str(&format!(
                    "        ${control} = $input->{control}->toInt();\n"
                ));
            } else {
                body.push_str(&format!(
                    "        $supplied = $input->{control} === Absent::Value ? null : $input->{control};\n        ${control} = $supplied === null ? {fallback} : $supplied->toInt();\n"
                ));
            }
            if compiled.inject_initial {
                body.push_str(&format!(
                    "        $input = self::{}($input, JsonNumber::fromInt(${control}));\n",
                    compiled.input_helper
                ));
            }
            body.push_str("        while (true) {\n");
            body.push_str(&format!(
                "            $page = $this->{}($input, $options);\n            yield $page;\n",
                compiled.method
            ));
            body.push_str(&format!(
                "            $items = self::{}($page);\n            $count = $items === null ? 0 : count($items);\n            if ($count === 0) {{ return; }}\n",
                compiled.items_reader
            ));
            if let Some(has_more) = &compiled.has_more_reader {
                body.push_str(&format!(
                    "            if (self::{has_more}($page) === false) {{ return; }}\n"
                ));
            }
            body.push_str(&format!(
                "            ${control} += $count;\n            $input = self::{}($input, JsonNumber::fromInt(${control}));\n        }}\n",
                compiled.input_helper
            ));
        }
        Advance::CursorPointer | Advance::OffsetPointer => {
            if compiled.control_required {
                body.push_str(&format!(
                    "        $previous = $input->{control};\n"
                ));
            } else {
                body.push_str(&format!(
                    "        $previous = $input->{control} === Absent::Value ? null : $input->{control};\n"
                ));
            }
            if compiled.inject_initial {
                body.push_str("        if ($previous === null) {\n");
                body.push_str(&format!(
                    "            $previous = {};\n",
                    match compiled.control_type.as_str() {
                        "JsonNumber" => format!("JsonNumber::fromInt({})", compiled.initial),
                        _ => php(&compiled.initial.to_string()),
                    }
                ));
                body.push_str(&format!(
                    "            $input = self::{}($input, $previous);\n        }}\n",
                    compiled.input_helper
                ));
            }
            body.push_str("        while (true) {\n");
            body.push_str(&format!(
                "            $page = $this->{}($input, $options);\n            yield $page;\n            $value = self::{}($page);\n",
                compiled.method, compiled.continuation_reader
            ));
            if compiled.advance == Advance::CursorPointer {
                body.push_str("            if ($value === null || $value === '') { return; }\n");
            } else {
                body.push_str("            if ($value === null) { return; }\n");
            }
            // The typed repeat guard, expressed over the statically known
            // control type without always-false comparisons.
            match compiled.control_type.as_str() {
                "JsonNumber" => {
                    if compiled.control_required {
                        body.push_str("            if ($value->compare($previous) === 0) { throw new SdkError('pagination-stalled', 'the source API returned an identical continuation value; the paginated walk would never terminate'); }\n");
                    } else {
                        body.push_str("            if ($previous !== null && $value->compare($previous) === 0) { throw new SdkError('pagination-stalled', 'the source API returned an identical continuation value; the paginated walk would never terminate'); }\n");
                    }
                }
                _ => {
                    body.push_str("            if ($value === $previous) { throw new SdkError('pagination-stalled', 'the source API returned an identical continuation value; the paginated walk would never terminate'); }\n");
                }
            }
            body.push_str("            $previous = $value;\n");
            body.push_str(&format!(
                "            $input = self::{}($input, $value);\n        }}\n",
                compiled.input_helper
            ));
        }
    }
    let mut out = String::new();
    doc_tags(
        &mut out,
        &pages_doc(compiled),
        &[
            format!("@param {} $input Source-bound native arguments.", compiled.input),
            "@param RequestOptions|null $options Per-call limits and explicit security selection."
                .to_owned(),
            format!("@return \\Generator<{}>", compiled.result),
            "@throws SdkError Preparation, transport, response validation, resource or traversal failure."
                .to_owned(),
        ],
    );
    out.push_str(&format!(
        "    public function {}({} $input{}, ?RequestOptions $options = null): \\Generator\n    {{\n",
        compiled.pages, compiled.input, compiled.argument_default
    ));
    out.push_str(&body);
    out.push_str("    }\n");
    out
}

fn items_method(compiled: &PaginatedOperation<'_>) -> String {
    let mut out = String::new();
    doc_tags(
        &mut out,
        &items_doc(compiled),
        &[
            format!("@param {} $input Source-bound native arguments.", compiled.input),
            "@param RequestOptions|null $options Per-call limits and explicit security selection."
                .to_owned(),
            format!("@return \\Generator<{}>", compiled.item_type),
            "@throws SdkError Preparation, transport, response validation, resource or traversal failure."
                .to_owned(),
        ],
    );
    out.push_str(&format!(
        "    public function {}({} $input{}, ?RequestOptions $options = null): \\Generator\n    {{\n        foreach ($this->{}($input, $options) as $page) {{\n            $items = self::{}($page);\n            if ($items !== null) {{\n                yield from $items;\n            }}\n        }}\n    }}\n",
        compiled.items,
        compiled.input,
        compiled.argument_default,
        compiled.pages,
        compiled.items_reader
    ));
    out
}

#[allow(clippy::too_many_lines)]
fn next_page_method(compiled: &PaginatedOperation<'_>) -> String {
    let control = &compiled.control_member;
    let mut body = String::new();
    if let Some((member, helper, size)) = &compiled.limit {
        body.push_str(&format!(
            "        if ($input->{member} === Absent::Value) {{\n            $input = self::{helper}($input, JsonNumber::fromInt({size}));\n        }}\n"
        ));
    }
    body.push_str(&format!(
        "        $page = $this->{}($input, $options);\n",
        compiled.method
    ));
    match compiled.advance {
        Advance::OffsetItemCount | Advance::PageNext => {
            let fallback = match compiled.advance {
                Advance::OffsetItemCount => compiled.initial.to_string(),
                _ => "1".to_owned(),
            };
            body.push_str(&format!(
                "        $items = self::{}($page);\n        $count = $items === null ? 0 : count($items);\n        if ($count === 0) {{ return null; }}\n",
                compiled.items_reader
            ));
            if let Some(has_more) = &compiled.has_more_reader {
                body.push_str(&format!(
                    "        if (self::{has_more}($page) === false) {{ return null; }}\n"
                ));
            }
            if compiled.control_required {
                body.push_str(&format!("        ${control} = $input->{control}->toInt();\n"));
            } else {
                body.push_str(&format!(
                    "        $supplied = $input->{control} === Absent::Value ? null : $input->{control};\n        ${control} = $supplied === null ? {fallback} : $supplied->toInt();\n"
                ));
            }
            body.push_str(&format!(
                "        return self::{}($input, JsonNumber::fromInt(${control} + $count));\n",
                compiled.arguments_helper
            ));
        }
        Advance::CursorPointer | Advance::OffsetPointer => {
            body.push_str(&format!(
                "        $value = self::{}($page);\n",
                compiled.continuation_reader
            ));
            if compiled.advance == Advance::CursorPointer {
                body.push_str("        if ($value === null || $value === '') { return null; }\n");
            } else {
                body.push_str("        if ($value === null) { return null; }\n");
            }
            if compiled.control_required {
                body.push_str(&format!("        $previous = $input->{control};\n"));
            } else {
                body.push_str(&format!(
                    "        $previous = $input->{control} === Absent::Value ? null : $input->{control};\n"
                ));
            }
            match compiled.control_type.as_str() {
                "JsonNumber" => {
                    if compiled.control_required {
                        body.push_str("        if ($value->compare($previous) === 0) { throw new SdkError('pagination-stalled', 'the source API returned an identical continuation value; the paginated walk would never terminate'); }\n");
                    } else {
                        body.push_str("        if ($previous !== null && $value->compare($previous) === 0) { throw new SdkError('pagination-stalled', 'the source API returned an identical continuation value; the paginated walk would never terminate'); }\n");
                    }
                }
                _ => {
                    body.push_str("        if ($value === $previous) { throw new SdkError('pagination-stalled', 'the source API returned an identical continuation value; the paginated walk would never terminate'); }\n");
                }
            }
            body.push_str(&format!(
                "        return self::{}($input, $value);\n",
                compiled.arguments_helper
            ));
        }
    }
    let mut out = String::new();
    doc_tags(
        &mut out,
        &next_page_doc(compiled),
        &[
            format!("@param {} $input Source-bound native arguments.", compiled.input),
            "@param RequestOptions|null $options Per-call limits and explicit security selection."
                .to_owned(),
            "@return array<string,mixed>|null".to_owned(),
            "@throws SdkError Preparation, transport, response validation, resource or traversal failure."
                .to_owned(),
        ],
    );
    out.push_str(&format!(
        "    public function {}({} $input{}, ?RequestOptions $options = null): ?array\n    {{\n",
        compiled.next_page, compiled.input, compiled.argument_default
    ));
    out.push_str(&body);
    out.push_str("    }\n");
    out
}

/// The pagination methods appended inside the generated client class, in plan
/// order. Empty when nothing paginates.
pub(super) fn client_methods(plan: &SdkPlan) -> String {
    let compiled = operations(plan);
    let mut out = String::new();
    for walk in &compiled {
        out.push_str(&helpers(plan, walk));
    }
    for walk in &compiled {
        out.push_str(&pages_method(walk));
        if walk.items_chain.is_some() {
            out.push_str(&items_method(walk));
        }
        out.push_str(&next_page_method(walk));
    }
    out
}

/// The private static pointer readers and input rebuilders for one walk.
fn helpers(plan: &SdkPlan, compiled: &PaginatedOperation<'_>) -> String {
    let mut out = String::new();
    if let Some(chain) = &compiled.items_chain {
        out.push_str(
            &reader(plan, compiled, chain, PaginationResponseRole::Items)
                .expect("compiled items reader"),
        );
    }
    if let Some((chain, role)) = &compiled.continuation_chain {
        out.push_str(
            &reader(plan, compiled, chain, *role).expect("compiled continuation reader"),
        );
    }
    if let Some(chain) = &compiled.has_more_chain {
        out.push_str(
            &reader(plan, compiled, chain, PaginationResponseRole::HasMore)
                .expect("compiled has-more reader"),
        );
    }
    out.push_str(&input_reader(plan, compiled));
    out.push_str(&limit_reader(plan, compiled));
    out.push_str(&arguments_reader(plan, compiled));
    out
}
