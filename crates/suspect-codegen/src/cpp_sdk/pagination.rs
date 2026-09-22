//! Emitted-only pagination walkers for the native C++ client.
//!
//! The shared, generation-time `http_protocol::plan_pagination` outcome lowers
//! into one generated `include/<package>/pagination.hpp` holding, per
//! paginated operation: typed accessor functions over the decoded page models,
//! a RAII pull pager with `next`/`value`/`error`, an item-flattening pager and
//! a next-input builder, plus matching methods on the generated `Client`
//! (declared in the emitted `client.hpp` only when walkers exist; the pager
//! classes are forward declared there and defined here).
//!
//! Response pointers resolve at generation time into typed accessor chains
//! over the planned models: there is no runtime schema search and no JSON
//! re-parsing. Operations whose compiled selection cannot be expressed with
//! typed field accesses are left without walkers, exactly like the Rust
//! backend, so generated code never guesses. Without configured defaults, or
//! without any emittable paginated operation, nothing is emitted and every
//! other artifact stays byte-identical.

use std::collections::BTreeSet;

use super::emit::{source_expr, string};
use super::models::{allocate, pascal, ModelPlan, Shape};
use super::protocol::{PlannedOperation, PlannedResponseCase, ValueKind};
use super::SdkPlan;
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
    /// Exact `JsonInteger` control.
    Integer,
    /// Plain `std::string` continuation control.
    Text,
}

/// One request control the pager rewrites between pages.
#[derive(Debug, Clone)]
pub struct Control {
    /// Input struct member carrying the control.
    pub field: String,
    pub kind: ControlKind,
    /// Optional input members are `Presence<T>`; required ones are direct.
    pub optional: bool,
}

/// One generated accessor function over the decoded page model.
#[derive(Debug, Clone)]
pub struct Accessor {
    /// RFC 6901 pointer the accessor reads, for documentation.
    pub pointer: String,
    /// Complete inline function text.
    pub code: String,
}

/// One paginated operation's compiled emission.
#[derive(Debug, Clone)]
pub struct PaginationOperation {
    /// Source identity of the paginated operation.
    pub operation: String,
    pub source: SourceId,
    /// The generated client method this walk extends.
    pub method: String,
    pub input_type: String,
    pub success_type: String,
    pub error_type: String,
    /// Concrete success response case the walk reads.
    pub response: String,
    /// Allocated pager type and client method names.
    pub pager: String,
    pub pager_error: String,
    pub items_pager: Option<String>,
    pub pages: String,
    pub items: Option<String>,
    pub next_page: String,
    pub advance: Advance,
    pub control: Control,
    /// The limit control and the documented SDK fallback page size it carries
    /// on the first request, when the operation has one.
    pub limit: Option<(Control, u32)>,
    pub initial_offset: Option<u32>,
    /// The element type of the items collection.
    pub element: Option<String>,
    pub items_accessor: Option<Accessor>,
    pub continuation_accessor: Option<Accessor>,
    pub has_more_accessor: Option<Accessor>,
}

/// The compiled pagination emission carried by one plan.
#[derive(Debug, Clone)]
pub struct PaginationPlan {
    /// The shared selection this emission follows.
    pub outcome: wire::PaginationOutcome,
    pub operations: Vec<PaginationOperation>,
}

impl PaginationPlan {
    /// Whether this plan emits pagination support at all.
    #[must_use]
    pub fn emits(&self) -> bool {
        !self.operations.is_empty()
    }
}

/// The leaf shape one resolved pointer produces.
#[derive(Debug, Clone)]
enum Leaf {
    Items { element: String },
    Integer,
    Text,
    Boolean,
}

/// One resolved pointer: generated guard statements and the final expression.
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

/// Resolve one RFC 6901 pointer into a typed accessor chain over the planned
/// page model, starting at the decoded body. `None` means the pointer reads
/// through something the native model cannot express; the caller then leaves
/// the operation without walkers instead of emitting guesses.
fn walk(
    models: &ModelPlan,
    id: &SchemaId,
    place: &str,
    path: &[String],
    counter: &mut usize,
    body: &mut Vec<String>,
) -> Option<Leaf> {
    let mut current = id.clone();
    let mut expression = place.to_owned();
    let shape = loop {
        let symbol = models.symbol(&current)?;
        // A nullable model is Nullable<T>: std::variant<Null, T>.
        if symbol.nullable {
            *counter += 1;
            let next = format!("at{counter}");
            body.push(format!("if ({expression}.index() == 0) return std::nullopt;"));
            body.push(format!("const auto& {next} = std::get<1>({expression});"));
            expression = next;
        }
        match &symbol.shape {
            Shape::Ref { target, boxed } if !symbol.nullable => {
                if *boxed {
                    *counter += 1;
                    let next = format!("at{counter}");
                    body.push(format!("if (!{expression}.has_value()) return std::nullopt;"));
                    body.push(format!("const auto& {next} = {expression}.value();"));
                    expression = next;
                }
                current = target.clone();
            }
            Shape::Ref { target, .. } => {
                current = target.clone();
            }
            shape => break shape.clone(),
        }
    };
    let Some(segment) = path.first() else {
        return match shape {
            Shape::Array(item) => Some(Leaf::Items {
                element: match item {
                    Some(id) => models.get(&id).cpp_type.clone(),
                    None => "JsonValue".into(),
                },
            }),
            Shape::Integer => Some(Leaf::Integer),
            Shape::String => Some(Leaf::Text),
            Shape::Boolean => Some(Leaf::Boolean),
            _ => None,
        };
    };
    let Shape::Object { fields, .. } = &shape else {
        return None;
    };
    let field = fields.iter().find(|field| field.wire == *segment)?;
    *counter += 1;
    let next = format!("at{counter}");
    if field.required {
        body.push(format!("const auto& {next} = {expression}.{};", field.name));
    } else {
        body.push(format!(
            "if (!{expression}.{}) return std::nullopt;",
            field.name
        ));
        body.push(format!(
            "const auto& {next} = {expression}.{}.value();",
            field.name
        ));
    }
    walk(models, &field.schema, &next, &path[1..], counter, body)
}

fn resolve(models: &ModelPlan, body: &SchemaId, pointer: &str) -> Option<Walk> {
    let path = segments(pointer);
    let mut guards = Vec::new();
    let mut counter = 0usize;
    let leaf = walk(models, body, "page.data", &path, &mut counter, &mut guards)?;
    Some(Walk {
        body: guards,
        place: if counter == 0 {
            "page.data".to_owned()
        } else {
            format!("at{counter}")
        },
        leaf,
    })
}

fn render_accessor(
    response: &str,
    name: &str,
    comment: &str,
    returns: &str,
    walk: &Walk,
    result: impl Fn(&str) -> String,
) -> String {
    let mut code = format!("/// {comment}\ninline {returns} {name}(const {response}& page) {{\n");
    for line in &walk.body {
        code.push_str("    ");
        code.push_str(line);
        code.push('\n');
    }
    code.push_str("    ");
    code.push_str(&result(&walk.place));
    code.push_str("\n}\n");
    code
}

/// The native control kind one request parameter carries.
fn control_kind(models: &ModelPlan, id: &SchemaId) -> Option<ControlKind> {
    let mut current = models.get(id);
    for _ in 0..64 {
        if current.nullable {
            return None;
        }
        match &current.shape {
            Shape::Ref { target, .. } => current = models.get(target),
            Shape::Integer => return Some(ControlKind::Integer),
            Shape::String => return Some(ControlKind::Text),
            _ => return None,
        }
    }
    None
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
    let kind = match &parameter.value.kind {
        ValueKind::Scalar(wire::ScalarType::Integer) => ControlKind::Integer,
        ValueKind::Scalar(wire::ScalarType::String) => ControlKind::Text,
        ValueKind::Model(id) => control_kind(models, id)?,
        _ => return None,
    };
    Some(Control {
        field: parameter.field_name.clone(),
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

fn continuation_pointer(page: &wire::OperationPagination) -> String {
    continuation_roles(page)
        .iter()
        .find_map(|role| page.response.get(role).cloned())
        .unwrap_or_default()
}

/// The decoded JSON body the shared planner reads: the first exact/range 2xx
/// response case with a typed JSON representation.
fn success_case(operation: &PlannedOperation) -> Option<(&PlannedResponseCase, SchemaId)> {
    let response = operation.responses.iter().find(|response| {
        matches!(
            response.status,
            wire::ResponseStatus::Exact(200..=299) | wire::ResponseStatus::Range(2)
        )
    })?;
    let case = response.cases.iter().find(|case| {
        !case.forbidden
            && case.media.as_ref().is_some_and(|media| {
                matches!(
                    media.representation(),
                    wire::Representation::Json { codec: Some(_) }
                )
            })
    })?;
    let media = case.media.as_ref()?;
    let wire::Representation::Json { codec: Some(codec) } = media.representation() else {
        return None;
    };
    let body = codec.schema().id().clone();
    match &case.value.kind {
        ValueKind::Model(id) if *id == body => Some((case, body)),
        _ => None,
    }
}

/// Lower one shared pagination selection into this backend's emission.
fn compile(
    models: &ModelPlan,
    operation: &PlannedOperation,
    page: &wire::OperationPagination,
    names: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> Option<PaginationOperation> {
    if page.pattern == PaginationPattern::NextLink {
        return None;
    }
    let (case, body) = success_case(operation)?;
    let response = case.variant_type.clone();
    let resolve = |pointer: &str| resolve(models, &body, pointer);

    let control_for = |role| control(models, operation, role, page);
    let apply = apply_role(page).and_then(control_for);
    let limit = control_for(PaginationRequestRole::Limit)
        .zip(page.initial_limit);
    let continuation = |kind: ControlKind| -> Option<Walk> {
        continuation_roles(page).iter().find_map(|role| {
            let walk = resolve(page.response.get(role)?)?;
            match (&walk.leaf, kind) {
                (Leaf::Integer, ControlKind::Integer) | (Leaf::Text, ControlKind::Text) => {
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

    // Count-driven walks advance by page contents and the item pager flattens
    // page contents, so both require a resolved items pointer.
    let items_walk = page
        .response
        .get(&PaginationResponseRole::Items)
        .and_then(|pointer| resolve(pointer))
        .filter(|walk| matches!(walk.leaf, Leaf::Items { .. }));
    if matches!(advance, Advance::OffsetItemCount | Advance::PageNext) && items_walk.is_none() {
        return None;
    }
    let element = items_walk.as_ref().map(|walk| match &walk.leaf {
        Leaf::Items { element } => element.clone(),
        _ => unreachable!("filtered above"),
    });

    let method = &operation.method_name;
    let items_accessor = items_walk.map(|walk| Accessor {
        pointer: page
            .response
            .get(&PaginationResponseRole::Items)
            .cloned()
            .unwrap_or_default(),
        code: render_accessor(
            &response,
            &format!("{method}_page_items"),
            &format!(
                "Items collection of one decoded {method} page at the compiled\n// pointer; an absent path reads as no items."
            ),
            &format!(
                "Presence<const std::vector<{}>*>",
                element.as_deref().unwrap_or("JsonValue")
            ),
            &walk,
            |place| format!("return &{place};"),
        ),
    });
    let continuation_accessor = continuation.as_ref().map(|walk| Accessor {
        pointer: continuation_pointer(page),
        code: match control.kind {
            ControlKind::Integer => render_accessor(
                &response,
                &format!("{method}_page_offset"),
                &format!(
                    "Continuation offset of one decoded {method} page at the compiled\n// pointer; an absent or null path reads as no continuation."
                ),
                "Presence<JsonInteger>",
                walk,
                |place| format!("return {place};"),
            ),
            ControlKind::Text => render_accessor(
                &response,
                &format!("{method}_page_cursor"),
                &format!(
                    "Continuation cursor of one decoded {method} page at the compiled\n// pointer; an absent or null path reads as no continuation."
                ),
                "Presence<std::string>",
                walk,
                |place| format!("return {place};"),
            ),
        },
    });
    let has_more_accessor = has_more.as_ref().map(|walk| Accessor {
        pointer: page
            .response
            .get(&PaginationResponseRole::HasMore)
            .cloned()
            .unwrap_or_default(),
        code: render_accessor(
            &response,
            &format!("{method}_page_has_more"),
            &format!(
                "Has-more evidence of one decoded {method} page at the compiled\n// pointer; an absent path reads as no evidence."
            ),
            "Presence<bool>",
            walk,
            |place| format!("return {place};"),
        ),
    });

    let stem = pascal(method);
    let pager = allocate(&format!("{stem}Pager"), names);
    let pager_error = allocate(&format!("{stem}PagerError"), names);
    let items_pager = items_accessor
        .is_some()
        .then(|| allocate(&format!("{stem}ItemsPager"), names));
    let pages = allocate(&format!("{method}_pages"), methods);
    let next_page = allocate(&format!("{method}_next_page"), methods);
    let items = items_accessor
        .is_some()
        .then(|| allocate(&format!("{method}_items"), methods));

    Some(PaginationOperation {
        operation: operation.operation_id.clone(),
        source: operation.source.clone(),
        method: method.clone(),
        input_type: operation.input_type.clone(),
        success_type: operation.success_type.clone(),
        error_type: operation.error_type.clone(),
        response,
        pager,
        pager_error,
        items_pager,
        pages,
        items,
        next_page,
        advance,
        control,
        limit,
        initial_offset: page.initial_offset,
        element,
        items_accessor,
        continuation_accessor,
        has_more_accessor,
    })
}

/// Lower the shared pagination selection for one plan. Operations whose
/// selection cannot be expressed with typed field accesses are left without
/// walkers instead of emitting guesses.
pub(super) fn lower(
    models: &ModelPlan,
    outcome: &wire::PaginationOutcome,
    operations: &[PlannedOperation],
    names: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> PaginationPlan {
    let mut compiled = Vec::new();
    for page in &outcome.paginated {
        let Some(operation) = operations
            .iter()
            .find(|operation| operation.operation_id == page.operation)
        else {
            continue;
        };
        if let Some(operation) = compile(models, operation, page, names, methods) {
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

fn stop_rule(operation: &PaginationOperation) -> &'static str {
    match operation.advance {
        Advance::OffsetItemCount | Advance::PageNext => {
            if operation.has_more_accessor.is_some() {
                "The walk stops on a page with no items, when the source marks\n/// has-more false, or on failure."
            } else {
                "The walk stops on a page with no items, or on failure."
            }
        }
        Advance::CursorPointer => {
            "The walk stops when the next cursor is absent, null or empty, or\n/// on failure; a repeated identical cursor is a typed pagination error."
        }
        Advance::OffsetPointer => {
            "The walk stops when the next offset is absent or null, or on\n/// failure; a repeated identical offset is a typed pagination error."
        }
    }
}

/// The statement seeding the configured initial offset into an absent optional
/// control, so the first request carries it exactly like the direct call.
fn initial_fill(operation: &PaginationOperation, out: &mut String) {
    if let (true, Some(value)) = (operation.control.optional, operation.initial_offset) {
        let field = &operation.control.field;
        out.push_str("            if (!input_.");
        out.push_str(field);
        out.push_str(&format!(") input_.{field} = JsonInteger({value});\n"));
    }
    // The documented SDK fallback page size applies to the first request only,
    // when the caller left the limit control absent; later pages keep
    // whatever limit the walk last used.
    if let Some((limit_control, size)) = &operation.limit
        && limit_control.optional && matches!(limit_control.kind, ControlKind::Integer) {
            let field = &limit_control.field;
            out.push_str("            if (!input_.");
            out.push_str(field);
            out.push_str(&format!(") input_.{field} = JsonInteger({size});\n"));
        }
}

fn pagination_failure_call(operation: &PaginationOperation, kind: &str, message: &str) -> String {
    format!(
        "error_.cause = detail::{}_pagination_failure(SdkError::Kind::{}, \"{}\")",
        operation.method, kind, message
    )
}

/// The `next_input` body: the continuation computation between pages.
fn next_input(operation: &PaginationOperation, identity: &str) -> String {
    let field = operation.control.field.as_str();
    let method = operation.method.as_str();
    let page_expression = format!("std::get<{}>(*value_)", operation.response);
    let mut out = String::new();
    match operation.advance {
        Advance::OffsetItemCount | Advance::PageNext => {
            out.push_str(&format!(
                "            const auto items = detail::{method}_page_items({page_expression});\n"
            ));
            out.push_str(
                "            const auto count = static_cast<std::int64_t>(items ? items.value()->size() : std::size_t{0});\n",
            );
            out.push_str("            if (count == 0) return std::nullopt;\n");
            if operation.has_more_accessor.is_some() {
                out.push_str(&format!(
                    "            const auto more = detail::{method}_page_has_more({page_expression});\n"
                ));
                out.push_str("            if (more && !*more) return std::nullopt;\n");
            }
            let (fallback, message, overflow, step) = match operation.advance {
                Advance::PageNext => (
                    operation
                        .initial_offset
                        .map_or_else(|| "1".to_owned(), |value| value.to_string()),
                    "the page number is not representable",
                    "pagination page overflow",
                    "1",
                ),
                _ => (
                    operation
                        .initial_offset
                        .map_or_else(|| "0".to_owned(), |value| value.to_string()),
                    "the pagination offset is not representable",
                    "pagination offset overflow",
                    "count",
                ),
            };
            out.push_str(&format!("            std::int64_t base = {fallback};\n"));
            if operation.control.optional {
                out.push_str(&format!("            if (input_.{field}) {{\n"));
                out.push_str(&format!(
                    "                auto parsed = input_.{field}.value().to_int64();\n"
                ));
                out.push_str("                if (!parsed) {\n");
                out.push_str(&format!(
                    "                    {};\n",
                    pagination_failure_call(operation, "RequestValidation", message)
                ));
                out.push_str("                    return std::nullopt;\n");
                out.push_str("                }\n");
                out.push_str("                base = parsed.value();\n");
                out.push_str("            }\n");
            } else {
                out.push_str(&format!(
                    "            auto parsed = input_.{field}.to_int64();\n"
                ));
                out.push_str("            if (!parsed) {\n");
                out.push_str(&format!(
                    "                {};\n",
                    pagination_failure_call(operation, "RequestValidation", message)
                ));
                out.push_str("                return std::nullopt;\n");
                out.push_str("            }\n");
                out.push_str("            base = parsed.value();\n");
            }
            out.push_str(&format!(
                "            const auto advanced = detail::pagination_advance(base, {step});\n"
            ));
            out.push_str("            if (!advanced) {\n");
            out.push_str(&format!(
                "                {};\n",
                pagination_failure_call(operation, "RequestValidation", overflow)
            ));
            out.push_str("                return std::nullopt;\n");
            out.push_str("            }\n");
            out.push_str(&format!(
                "            input_.{field} = JsonInteger(advanced.value());\n"
            ));
            out.push_str("            return input_;\n");
        }
        Advance::CursorPointer | Advance::OffsetPointer => {
            let reader = match operation.control.kind {
                ControlKind::Text => format!("detail::{method}_page_cursor({page_expression})"),
                ControlKind::Integer => format!("detail::{method}_page_offset({page_expression})"),
            };
            out.push_str(&format!("            const auto token = {reader};\n"));
            if operation.advance == Advance::CursorPointer {
                out.push_str(
                    "            if (!token || token.value().empty()) return std::nullopt;\n",
                );
            } else {
                out.push_str("            if (!token) return std::nullopt;\n");
            }
            out.push_str("            if (previous_ && ");
            out.push_str(&match operation.control.kind {
                ControlKind::Text => "*previous_ == *token".to_owned(),
                ControlKind::Integer => "previous_.value() == token.value()".to_owned(),
            });
            out.push_str(") {\n");
            let repeated = match operation.control.kind {
                ControlKind::Text => "*token",
                ControlKind::Integer => "token.value().token()",
            };
            out.push_str(&format!(
                "                error_.loop = PaginationLoop{{{},{}}};\n",
                string(identity),
                repeated
            ));
            out.push_str(&format!(
                "                {};\n",
                pagination_failure_call(
                    operation,
                    "UnexpectedResponse",
                    "the source API returned an identical continuation value; the paginated walk would never terminate"
                )
            ));
            out.push_str("                return std::nullopt;\n");
            out.push_str("            }\n");
            out.push_str("            previous_ = token.value();\n");
            out.push_str(&format!("            input_.{field} = token.value();\n"));
            out.push_str("            return input_;\n");
        }
    }
    out
}

/// The per-operation class of the `pagination.hpp` detail namespace.
fn detail_section(plan: &SdkPlan, operation: &PaginationOperation) -> String {
    let mut out = String::new();
    for accessor in [
        operation.items_accessor.as_ref(),
        operation.continuation_accessor.as_ref(),
        operation.has_more_accessor.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        out.push_str(&accessor.code);
        out.push('\n');
    }
    let source = source_expr(plan, &operation.source);
    let identity = string(&operation.operation);
    out.push_str("/// Builds the operation's typed failure for pagination-specific refusals.\n");
    out.push_str(&format!(
        "inline {} {}_pagination_failure(SdkError::Kind kind, std::string message) {{\n",
        operation.error_type, operation.method
    ));
    out.push_str("    SdkError error;\n    error.kind = kind;\n");
    out.push_str(&format!("    error.operation_source = {source};\n"));
    out.push_str(&format!("    error.operation_id = {identity};\n"));
    out.push_str(&format!("    error.source = {source};\n"));
    out.push_str(
        "    error.message = std::move(message);\n    return ",
    );
    out.push_str(&format!(
        "{}(std::in_place_type<SdkError>, std::move(error));\n}}\n",
        operation.error_type
    ));
    out
}

fn previous_member(operation: &PaginationOperation) -> Option<&'static str> {
    match operation.advance {
        Advance::CursorPointer => Some("    Presence<std::string> previous_;\n"),
        Advance::OffsetPointer => Some("    Presence<JsonInteger> previous_;\n"),
        Advance::OffsetItemCount | Advance::PageNext => None,
    }
}

fn pager_struct(operation: &PaginationOperation) -> String {
    let method = operation.method.as_str();
    let pager = &operation.pager;
    let input = &operation.input_type;
    let success = &operation.success_type;
    let error = &operation.pager_error;
    let response = &operation.response;
    let mut out = String::new();
    out.push_str(&format!(
        "/// Terminal failure of one {pager} walk. `cause` carries the operation's\n/// own typed error for transport, decoding and declared API failures; `loop`\n/// is engaged exactly when the walk refused a repeated identical continuation\n/// value instead of looping forever.\nstruct {error} {{\n    /** Typed continuation-loop report. */\n    Presence<PaginationLoop> loop;\n    /** The operation's own error, unchanged from the operation call. */\n    Presence<{}> cause;\n}};\n\n",
        operation.error_type
    ));
    let fallback_doc = operation
        .limit
        .as_ref()
        .map(|(_, size)| format!("/// When the caller leaves the limit absent, the first request supplies\n/// the documented SDK page size ({size}); later pages keep the limit the\n/// walk last used.\n"))
        .unwrap_or_default();
    out.push_str(&format!(
        "/// Pull pager over {method}. Each next() issues at most one request through\n/// the client and its transport; the pager owns its request state, so\n/// destruction between calls stops the walk without firing another request.\n/// The first next() fetches page 1 exactly once with the input as given,\n/// filling the configured initial pagination value when the caller left the\n/// control absent; every later page rebuilds the input with only the\n/// pagination controls changed and preserves every other member exactly.\n/// {}\n{fallback_doc}",
        stop_rule(operation)
    ));
    out.push_str(&format!("class {pager} {{\npublic:\n"));
    out.push_str(&format!(
        "    {pager}(const Client& client, {input} input, CallOptions options)\n        : client_(&client), options_(std::move(options)), input_(std::move(input)) {{}}\n"
    ));
    out.push_str(
        "    /** Fetches the next page into out; false only after the walk has\n     * ended. The final page still ends the walk through a false result.\n     * After a false result, error() carries the cause; ordinary exhaustion\n     * leaves it disengaged. Successful pages are not default-constructible,\n     * so the caller passes an optional page slot. */\n",
    );
    out.push_str(&format!(
        "    bool next(Presence<{success}>& out) {{\n        if (!fetch()) return false;\n        out = *value_;\n        return true;\n    }}\n"
    ));
    out.push_str(&format!(
        "    /** The page fetched by the last true next(); valid only after true. */\n    const {success}& value() const {{ return value_.value(); }}\n"
    ));
    out.push_str(
        "    /** The terminal failure after a false next(); disengaged on ordinary\n     * exhaustion or before the walk started. */\n",
    );
    out.push_str(&format!(
        "    const {error}& error() const {{ return error_; }}\n"
    ));
    out.push_str(
        "    /** The rebuilt input for the page after the last delivered one,\n     * computed after that delivery; disengaged when the walk stops. */\n",
    );
    out.push_str(&format!(
        "    const Presence<{input}>& pending() const {{ return pending_; }}\n\n"
    ));
    out.push_str(
        "    /** The pull step shared with the item pager and next-input builder:\n     * at most one request, fetching into the pager's own storage and\n     * computing the continuation. */\n    bool fetch() {\n",
    );
    out.push_str("        if (done_ || error_.loop || error_.cause) return false;\n");
    out.push_str("        if (fetched_) {\n");
    out.push_str("            if (!pending_) {\n                done_ = true;\n                return false;\n            }\n");
    out.push_str("            input_ = std::move(*pending_);\n            pending_.reset();\n");
    out.push_str("        } else {\n            fetched_ = true;\n");
    initial_fill(operation, &mut out);
    if matches!(
        operation.advance,
        Advance::CursorPointer | Advance::OffsetPointer
    ) {
        out.push_str(&format!(
            "            previous_ = input_.{};\n",
            operation.control.field
        ));
    }
    out.push_str("        }\n");
    out.push_str(&format!(
        "        auto result = client_->{method}(input_, options_);\n"
    ));
    out.push_str("        if (!result) {\n");
    out.push_str("            error_.cause = std::move(result).error();\n");
    out.push_str("            return false;\n        }\n");
    out.push_str(&format!(
        "        auto* page = std::get_if<{response}>(&result.value());\n"
    ));
    out.push_str("        if (!page) {\n");
    out.push_str(&format!(
        "            {};\n",
        pagination_failure_call(
            operation,
            "UnexpectedResponse",
            "the walk met a success representation the pagination plan cannot read"
        )
    ));
    out.push_str("            return false;\n        }\n");
    out.push_str("        value_ = std::move(result).value();\n");
    out.push_str("        pending_ = next_input();\n");
    out.push_str("        return true;\n    }\n\nprivate:\n");
    out.push_str(
        "    /** Rebuilds the pagination controls for the page after the fetched\n     * one; disengaged when the walk stops. A repeated identical\n     * continuation engages the typed loop refusal instead of looping. */\n",
    );
    out.push_str(&format!("    Presence<{input}> next_input() {{\n"));
    out.push_str(&next_input(operation, &operation.operation));
    out.push_str("    }\n\n");
    out.push_str("    const Client* client_;\n    CallOptions options_;\n");
    out.push_str(&format!("    {input} input_;\n"));
    out.push_str(&format!("    Presence<{success}> value_;\n"));
    out.push_str(&format!("    Presence<{input}> pending_;\n"));
    if let Some(member) = previous_member(operation) {
        out.push_str(member);
    }
    out.push_str(&format!("    {error} error_;\n"));
    out.push_str("    bool fetched_ = false;\n    bool done_ = false;\n};\n");
    out
}

fn items_pager_struct(operation: &PaginationOperation, items_pager: &str) -> String {
    let pager = &operation.pager;
    let input = &operation.input_type;
    let error = &operation.pager_error;
    let response = &operation.response;
    let element = operation.element.as_deref().unwrap_or("JsonValue");
    let collection = format!("Presence<const std::vector<{element}>*>");
    let mut out = String::new();
    out.push_str(&format!(
        "/// Pull pager over every item of every {} page, flattened in order. Items\n/// of an already-delivered page are returned without a request, so early\n/// exit never fires another one; the page pager owns cancellation and error\n/// policy.\n",
        operation.method
    ));
    out.push_str(&format!("class {items_pager} {{\npublic:\n"));
    out.push_str(&format!(
        "    {items_pager}(const Client& client, {input} input, CallOptions options)\n        : pager_(client, std::move(input), std::move(options)) {{}}\n"
    ));
    out.push_str(
        "    /** Pulls the next item across all pages, flattened in order; false\n     * only after the final page's final item. Items of an already-delivered\n     * page are returned without a request, so early exit never fires another\n     * one. After a false result, error() carries the cause. */\n",
    );
    out.push_str("    bool next() {\n");
    out.push_str("        for (;;) {\n");
    out.push_str("            if (items_ && index_ < items_.value()->size()) {\n");
    out.push_str("                ++index_;\n                return true;\n            }\n");
    out.push_str("            if (!pager_.fetch()) return false;\n");
    out.push_str(&format!(
        "            const auto* concrete = std::get_if<{response}>(&pager_.value());\n"
    ));
    out.push_str("            items_ = concrete\n");
    out.push_str(&format!(
        "                ? detail::{}_page_items(*concrete)\n",
        operation.method
    ));
    out.push_str(&format!(
        "                : {collection}(std::nullopt);\n"
    ));
    out.push_str("            index_ = 0;\n        }\n    }\n");
    out.push_str(
        "    /** The item pulled by the last true next(); valid only after true. */\n",
    );
    out.push_str(&format!(
        "    const {element}& value() const {{ return (*items_.value())[index_ - 1]; }}\n"
    ));
    out.push_str(
        "    /** The terminal failure after a false next(); the page pager's. */\n",
    );
    out.push_str(&format!(
        "    const {error}& error() const {{ return pager_.error(); }}\n\nprivate:\n"
    ));
    out.push_str(&format!("    {pager} pager_;\n"));
    out.push_str(&format!("    {collection} items_;\n"));
    out.push_str("    std::size_t index_ = 0;\n};\n");
    out
}

fn client_definitions(operation: &PaginationOperation) -> String {
    let input = &operation.input_type;
    let mut out = String::new();
    out.push_str(&format!(
        "inline {} Client::{}(const {input}& input, CallOptions options) const {{\n",
        operation.pager, operation.pages
    ));
    out.push_str(&format!(
        "    return {}(*this, input, std::move(options));\n}}\n",
        operation.pager
    ));
    if let (Some(items), Some(items_pager)) = (&operation.items, &operation.items_pager) {
        out.push_str(&format!(
            "inline {items_pager} Client::{items}(const {input}& input, CallOptions options) const {{\n"
        ));
        out.push_str(&format!(
            "    return {items_pager}(*this, input, std::move(options));\n}}\n"
        ));
    }
    out.push_str(&format!(
        "inline std::optional<{input}> Client::{}(const {input}& input, CallOptions options) const {{\n",
        operation.next_page
    ));
    out.push_str(&format!(
        "    {} pager(*this, input, std::move(options));\n",
        operation.pager
    ));
    out.push_str("    if (!pager.fetch()) return std::nullopt;\n");
    out.push_str("    const auto& next = pager.pending();\n");
    out.push_str(
        "    if (!next || pager.error().loop || pager.error().cause) return std::nullopt;\n",
    );
    out.push_str("    return *next;\n}\n");
    out
}

/// The generated `include/<package>/pagination.hpp`.
pub(super) fn header(plan: &SdkPlan, pagination: &PaginationPlan) -> String {
    let name = &plan.config.name;
    let mut out = String::new();
    out.push_str("#pragma once\n");
    out.push_str(
        "/** @file pagination.hpp Generated pagination walkers for this package's\n * source-selected paginated operations.\n *\n * Every pager is a concrete RAII pull type: each next() issues at most one\n * request through the client's transport and owns its request state, so\n * destruction between calls stops the walk without firing another request,\n * and cancellation/deadline policy is the CallOptions passed at construction.\n * The first page of every walk is exactly the direct call's result, included\n * exactly once, honoring the caller's pagination values; every later page\n * rebuilds the input with only the pagination controls changed. Pointers\n * resolve through generated typed accessor chains only.\n */\n",
    );
    out.push_str(&format!("#include \"{name}/client.hpp\"\n"));
    out.push_str("#include <limits>\n#include <optional>\n#include <string>\n#include <variant>\n\n");
    out.push_str(&format!("namespace {} {{\n\n", plan.config.namespace));
    out.push_str(
        "/** Typed pagination traversal report: a source repeated an identical\n * continuation value, so continuing the walk would never terminate. */\nstruct PaginationLoop {\n    /** Source identity of the paginated operation. */\n    std::string operation;\n    /** The repeated continuation value, exactly as the source returned it. */\n    std::string value;\n};\n\n",
    );
    out.push_str("namespace detail {\n\n");
    out.push_str(
        "/// Exact bounded integer advance shared by this package's pagination walkers.\ninline Presence<std::int64_t> pagination_advance(std::int64_t base, std::int64_t count) {\n    if (count >= 0 ? base > std::numeric_limits<std::int64_t>::max() - count\n                   : base < std::numeric_limits<std::int64_t>::min() - count) {\n        return std::nullopt;\n    }\n    return Presence<std::int64_t>(base + count);\n}\n\n",
    );
    for operation in &pagination.operations {
        out.push_str(&detail_section(plan, operation));
        out.push('\n');
    }
    out.push_str("} // namespace detail\n\n");
    for operation in &pagination.operations {
        out.push_str(&pager_struct(operation));
        out.push('\n');
        if let Some(items_pager) = operation.items_pager.as_deref() {
            out.push_str(&items_pager_struct(operation, items_pager));
            out.push('\n');
        }
    }
    for operation in &pagination.operations {
        out.push_str(&client_definitions(operation));
    }
    out.push_str(&format!("}} // namespace {}\n", plan.config.namespace));
    out
}

/// Forward declarations injected into the emitted `client.hpp` before the
/// `Client` class, so the pager-returning methods can be declared there.
pub(super) fn client_forward_declarations(plan: &SdkPlan, pagination: &PaginationPlan) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "/// Generated pagination pull pagers; complete definitions live in\n/// <{}/pagination.hpp>.\n",
        plan.config.name
    ));
    for operation in &pagination.operations {
        out.push_str(&format!("class {};\n", operation.pager));
        if let Some(items_pager) = &operation.items_pager {
            out.push_str(&format!("class {items_pager};\n"));
        }
    }
    out
}

/// Per-operation pager-returning method declarations inside the `Client` class.
pub(super) fn client_declarations(pagination: &PaginationPlan) -> String {
    let mut out = String::new();
    for operation in &pagination.operations {
        let method = operation.method.as_str();
        out.push_str(&format!(
            "    /// Pull pager over {method}; the first next() fetches page 1 exactly\n    /// once with the input as given, honoring the caller's pagination values.\n"
        ));
        out.push_str(&format!(
            "    [[nodiscard]] {} {}(const {}& input, CallOptions options = {{}}) const;\n",
            operation.pager, operation.pages, operation.input_type
        ));
        if let (Some(items), Some(items_pager)) = (&operation.items, &operation.items_pager) {
            out.push_str(&format!(
                "    /// Item pager over {method}, flattening every page's items in order.\n"
            ));
            out.push_str(&format!(
                "    [[nodiscard]] {items_pager} {items}(const {}& input, CallOptions options = {{}}) const;\n",
                operation.input_type
            ));
        }
        out.push_str(
            "    /// Fetches the page described by the input and returns the rebuilt\n    /// input for the following page; disengaged at the end of the walk or\n    /// on failure. Use the pager methods when the failure cause matters.\n",
        );
        out.push_str(&format!(
            "    [[nodiscard]] std::optional<{}> {}(const {}& input, CallOptions options = {{}}) const;\n",
            operation.input_type, operation.next_page, operation.input_type
        ));
    }
    out
}
