//! Generated-only pagination emission for the Swift package.
//!
//! The shared, generation-time `http_protocol::plan_pagination` outcome
//! compiles into per-operation pull-based `AsyncSequence` types plus `Client`
//! factory methods — all emitted into the generated `Pagination.swift` file.
//! Static runtime files and the shared planner stay untouched, and plans
//! without configured SDK defaults (or without emittable paginated operations)
//! emit no new bytes at all.
//!
//! Traversal semantics shared with every backend: pages are fetched lazily one
//! at a time and the first page is exactly the direct call's result included
//! once; later requests rebuild only the pagination controls while preserving
//! every other input member; caller-supplied pagination values win for page 1;
//! the walk stops when the continuation pointer reads absent, null or empty,
//! on a zero-item page for contents-advanced walks, and on a mapped has-more
//! indicator reading false; a repeated identical continuation value throws a
//! typed pagination error instead of looping. RFC 6901 pointers resolve at
//! generation time into typed property chains over the decoded response
//! models — plain property access, never JSONPath or schema search.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::{
    SdkPlan,
    models::{Declaration, ModelPlan, ModelType, Type},
};
use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as wire;
use crate::sdk_defaults::{
    PaginationAdvance, PaginationPattern, PaginationRequestRole, PaginationResponseRole,
};
use suspect_ir::contract::{Contract, SchemaId};

/// How one walk computes and applies its continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The offset control advances by the previous page's item count.
    ItemsReturned,
    /// The one-based page control advances by one.
    PageNext,
    /// The page's mapped continuation pointer feeds the control.
    Pointer,
}

/// The request-side pagination control the walk rewrites between pages.
#[derive(Debug, Clone)]
struct Control {
    /// Swift property name on the operation input struct.
    field: String,
    /// String-typed cursors stay text; integer controls use `JsonInteger`.
    text: bool,
    /// Whether the input member wraps an `OptionalField`.
    optional: bool,
    /// Whether the wrapped value is a `Nullable`.
    nullable: bool,
    /// Checked-carrier struct name wrapping the control value, if any.
    carrier: Option<String>,
}

impl Control {
    /// The value-carrying expression over one input binding, before any
    /// optionality wrapper is opened.
    fn core_read(&self, base: &str) -> String {
        let mut expression = format!("{base}.{}", self.field);
        if self.optional {
            // OptionalField and Presence both unwrap through valueIfPresent.
            expression.push_str(".valueIfPresent");
        }
        if self.nullable {
            expression.push_str(".valueIfPresent");
        }
        if self.carrier.is_some() {
            expression.push_str(if self.optional || self.nullable {
                "?.value"
            } else {
                ".value"
            });
        }
        expression
    }

    /// An optional-valued read expression over one input binding.
    fn read(&self, base: &str) -> String {
        let expression = self.core_read(base);
        if self.optional || self.nullable {
            expression
        } else {
            format!("Optional({expression})")
        }
    }

    /// A statement storing one continuation value back into the input.
    fn write(&self, input: &str, value: &str) -> String {
        let inner = match (&self.carrier, self.text) {
            (Some(carrier), true) => format!("{carrier}(value: {value})"),
            (Some(carrier), false) => format!("{carrier}(value: JsonInteger({value}))"),
            (None, true) => value.to_owned(),
            (None, false) => format!("JsonInteger({value})"),
        };
        let mut wrapped = inner;
        if self.nullable {
            wrapped = format!(".value({wrapped})");
        }
        if self.optional {
            wrapped = format!(".value({wrapped})");
        }
        format!("        {input}.{} = {wrapped}\n", self.field)
    }
}

/// One resolved pointer reader: the emitted function body plus its value type.
#[derive(Debug, Clone)]
struct Walk {
    /// Indented body lines, ending in a `return` of the resolved value.
    body: String,
    /// The rendered Swift type of the returned value, without optionality.
    ty: String,
    /// For the items leaf: the element type inside the returned array.
    element: Option<String>,
}

/// The leaf flavor a pointer reader produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaf {
    /// The array at the items pointer.
    Items,
    /// A string (or numeric token) continuation value.
    Token,
    /// An exact integer continuation or total.
    Number,
    /// A boolean has-more flag.
    Flag,
}

/// The wrapper shape a declared field carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wrapper {
    None,
    Optional,
    Nullable,
    Presence,
}

/// One compiled paginated operation, ready to render.
#[derive(Debug, Clone)]
pub struct PaginationOperation {
    /// Index into `SdkPlan::operations`.
    pub operation: usize,
    /// The shared compiled pagination selection this emission follows.
    pub selection: wire::OperationPagination,
    /// Client method returning the page sequence.
    pub pages_method: String,
    /// Client method returning the flattened item sequence.
    pub items_method: String,
    /// Client method returning the next page's input.
    pub next_page_method: String,
    /// Page sequence type name.
    pub page_sequence: String,
    /// Item sequence type name.
    pub item_sequence: String,
    /// The decoded payload type the walkers read (`page.data`).
    payload: String,
    control: Control,
    /// The limit control carrying the documented SDK fallback page size, when
    /// the operation has one that can express an absent state.
    limit: Option<Control>,
    mode: Mode,
    items: Option<Walk>,
    item_element: Option<String>,
    continuation: Option<Walk>,
    has_more: Option<Walk>,
}

/// The compiled pagination emission carried by one Swift plan.
#[derive(Debug, Clone)]
pub struct PaginationPlan {
    /// The shared detection and override selection this emission follows.
    pub outcome: wire::PaginationOutcome,
    /// Allocated typed pagination error type.
    pub error_type: String,
    control_failure: String,
    advance_failure: String,
    pub operations: Vec<PaginationOperation>,
}

/// The shared planner's operation identity for one planned operation:
/// the operation id, or `METHOD /path` when unnamed.
fn identity(operation: &super::PlannedOperation) -> String {
    operation
        .wire
        .operation_id()
        .map(|located| located.value().clone())
        .unwrap_or_else(|| {
            format!(
                "{} {}",
                operation.wire.method().as_str(),
                operation.wire.path()
            )
        })
}

fn source(operation: &wire::OperationPagination) -> String {
    super::protocol_metadata::source(operation.source.use_site().source())
}

fn q(value: &str) -> String {
    super::protocol_metadata::q(value)
}

/// Compile the shared pagination selection into this backend's emission plan.
/// Selections this plan cannot express with typed property chains are skipped:
/// the operation stays an ordinary single-page call. A configured policy with
/// no emittable operations leaves the plan without pagination support.
pub(super) fn plan(
    contract: &Contract,
    wire_plan: &wire::ProtocolPlan,
    defaults: Option<&crate::sdk_defaults::SdkDefaults>,
    operations: &[super::PlannedOperation],
    models: &ModelPlan,
    type_names: &mut BTreeSet<String>,
    methods: &mut BTreeSet<String>,
) -> Result<Option<PaginationPlan>, Vec<HttpDiagnostic>> {
    let Some(defaults) = defaults else {
        return Ok(None);
    };
    let outcome = wire::plan_pagination(contract, wire_plan, Some(defaults))?;
    let mut compiled = Vec::new();
    for page in &outcome.paginated {
        let Some((index, operation)) = operations
            .iter()
            .enumerate()
            .find(|(_, operation)| identity(operation) == page.operation)
        else {
            continue;
        };
        if let Some(one) = compile(operation, page, models) {
            compiled.push((index, one));
        }
    }
    if compiled.is_empty() {
        return Ok(None);
    }
    let error_type = super::allocate("PaginationError", type_names);
    let control_failure = super::allocate("paginationControlFailure", type_names);
    let advance_failure = super::allocate("paginationAdvanceFailure", type_names);
    let mut planned = Vec::new();
    for (index, compiled) in compiled {
        let operation = &operations[index];
        let pages_method = super::allocate(&format!("{}Pages", operation.method_name), methods);
        let items_method = super::allocate(&format!("{}Items", operation.method_name), methods);
        let next_page_method =
            super::allocate(&format!("{}NextPage", operation.method_name), methods);
        let stem = super::exported(&operation.operation_id);
        let page_sequence = super::allocate(&format!("{stem}PageSequence"), type_names);
        let item_sequence = super::allocate(&format!("{stem}ItemSequence"), type_names);
        planned.push(PaginationOperation {
            operation: index,
            selection: compiled.selection,
            payload: compiled.payload,
            pages_method,
            items_method,
            next_page_method,
            page_sequence,
            item_sequence,
            control: compiled.control,
            limit: compiled.limit,
            mode: compiled.mode,
            items: compiled.items,
            item_element: compiled.item_element,
            continuation: compiled.continuation,
            has_more: compiled.has_more,
        });
    }
    Ok(Some(PaginationPlan {
        outcome,
        error_type,
        control_failure,
        advance_failure,
        operations: planned,
    }))
}

/// Strip transparency wrappers from a planned field type into the wrapper
/// flavor plus the core type, with `Indirect` cycle carriers unwrapped.
fn unwrap_field(model_type: &ModelType, required: bool) -> (Wrapper, Type) {
    let wrapper = match (required, model_type.nullable) {
        (true, false) => Wrapper::None,
        (true, true) => Wrapper::Nullable,
        (false, false) => Wrapper::Optional,
        (false, true) => Wrapper::Presence,
    };
    let mut core = model_type.core.clone();
    while let Type::Indirect(inner) = core {
        core = (*inner).clone();
    }
    (wrapper, core)
}

/// The request-side pagination control for one role's wire parameter.
fn control(models: &ModelPlan, operation: &super::PlannedOperation, name: &str) -> Option<Control> {
    use crate::http_protocol::ParameterLocation;
    let parameter = operation
        .parameters
        .iter()
        .find(|p| p.wire.location() == ParameterLocation::Query && p.wire.name() == name)?;
    let model_type = models.types.get(parameter.wire.codec().schema().id())?;
    let optional = !parameter.wire.required();
    let nullable = model_type.nullable;
    let mut core = model_type.core.clone();
    while let Type::Indirect(inner) = core {
        core = (*inner).clone();
    }
    // A $ref carrier with sibling assertions wraps the value in a checked
    // struct; plain scalars are the value directly.
    let (text, carrier) = match &core {
        Type::Primitive("String") => (true, None),
        Type::Primitive("JsonInteger") => (false, None),
        Type::Named(id) => match models.declarations.get(id) {
            Some(Declaration::Checked { value }) => match value {
                Type::Primitive("String") => (true, Some(models.names[id].clone())),
                Type::Primitive("JsonInteger") => (false, Some(models.names[id].clone())),
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };
    Some(Control {
        field: parameter.field_name.clone(),
        text,
        optional,
        nullable,
        carrier,
    })
}

/// Resolve one RFC 6901 pointer into the decoded page payload, starting at the
/// success response's JSON body. `None` means the pointer cannot be expressed
/// with typed property chains and the operation stays without walkers.
fn walk_pointer(models: &ModelPlan, root: &SchemaId, pointer: &str, leaf: Leaf) -> Option<Walk> {
    let segments: Vec<String> = if pointer.is_empty() {
        Vec::new()
    } else {
        pointer[1..]
            .split('/')
            .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
            .collect()
    };
    let model_type = models.types.get(root)?;
    let mut counter = 0usize;
    // The accessor receives the decoded payload directly.
    let mut place = "page".to_owned();
    let mut prefix = String::new();
    // A nullable body payload unwraps once before any property access.
    if model_type.nullable {
        counter += 1;
        let binding = format!("value_{counter}");
        prefix.push_str(&format!(
            "    guard case .value(let {binding}) = {place} else {{ return nil }}\n"
        ));
        place = binding;
    }
    if segments.is_empty() {
        let mut found = leaf_convert(
            models,
            &place,
            Wrapper::None,
            model_type.core.clone(),
            leaf,
            &mut counter,
        )?;
        found.body.insert_str(0, &prefix);
        return Some(found);
    }
    let key = match &model_type.core {
        Type::Named(next) => next.clone(),
        _ => return None,
    };
    let mut found = walk(models, &key, &place, &segments, leaf, &mut counter)?;
    found.body.insert_str(0, &prefix);
    Some(found)
}

/// Follow declared object properties through the planned page model.
fn walk(
    models: &ModelPlan,
    key: &SchemaId,
    place: &str,
    segments: &[String],
    leaf: Leaf,
    counter: &mut usize,
) -> Option<Walk> {
    let Declaration::Object { fields, .. } = models.declarations.get(key)? else {
        return None;
    };
    let field = fields.iter().find(|field| field.wire == segments[0])?;
    let access = format!("{place}.{}", field.name);
    let (wrapper, core) = unwrap_field(&field.model_type, field.required);
    if segments.len() == 1 {
        return leaf_convert(models, &access, wrapper, core, leaf, counter);
    }
    let Type::Named(next) = core else {
        return None;
    };
    let (binding, body) = unwrap_binding(&access, wrapper, counter);
    let child = walk(models, &next, &binding, &segments[1..], leaf, counter)?;
    Some(Walk {
        body: format!("{body}{}", child.body),
        ty: child.ty,
        element: child.element,
    })
}

/// Unwrap one wrapper level into a fresh binding, or read the value directly.
fn unwrap_binding(access: &str, wrapper: Wrapper, counter: &mut usize) -> (String, String) {
    match wrapper {
        Wrapper::None => (access.to_owned(), String::new()),
        Wrapper::Optional | Wrapper::Nullable | Wrapper::Presence => {
            *counter += 1;
            let binding = format!("value_{counter}");
            let body =
                format!("    guard case .value(let {binding}) = {access} else {{ return nil }}\n");
            (binding.clone(), body)
        }
    }
}

/// Build the accessor leaf expression for one resolved field access.
fn leaf_convert(
    models: &ModelPlan,
    access: &str,
    wrapper: Wrapper,
    core: Type,
    leaf: Leaf,
    counter: &mut usize,
) -> Option<Walk> {
    let (binding, body) = unwrap_binding(access, wrapper, counter);
    let (statement, ty, element) = match leaf {
        Leaf::Items => {
            let Type::Array(element) = &core else {
                return None;
            };
            let element = element.render(models);
            (
                format!("    return {binding}\n"),
                format!("[{element}]"),
                Some(element),
            )
        }
        Leaf::Token => match &core {
            Type::Primitive("String") => {
                (format!("    return {binding}\n"), "String".to_owned(), None)
            }
            Type::Primitive("JsonInteger") => (
                format!("    return {binding}.raw\n"),
                "String".to_owned(),
                None,
            ),
            Type::Named(id)
                if matches!(models.declarations.get(id), Some(Declaration::Literals(_))) =>
            {
                (
                    format!("    return {binding}.rawValue\n"),
                    "String".to_owned(),
                    None,
                )
            }
            _ => return None,
        },
        Leaf::Number => match &core {
            // An unrepresentable exact integer reads as an absent continuation.
            Type::Primitive("JsonInteger") => (
                format!("    return try? {binding}.int64Value()\n"),
                "Int64".to_owned(),
                None,
            ),
            _ => return None,
        },
        Leaf::Flag => match &core {
            Type::Primitive("Bool") => (format!("    return {binding}\n"), "Bool".to_owned(), None),
            _ => return None,
        },
    };
    Some(Walk {
        body: format!("{body}{statement}"),
        ty,
        element,
    })
}

/// Everything `compile` produces before name allocation.
struct Compiled {
    selection: wire::OperationPagination,
    payload: String,
    control: Control,
    limit: Option<Control>,
    mode: Mode,
    items: Option<Walk>,
    item_element: Option<String>,
    continuation: Option<Walk>,
    has_more: Option<Walk>,
}

/// Compile one paginated operation's walk, or `None` when this plan cannot
/// express the selection with typed property chains.
fn compile(
    operation: &super::PlannedOperation,
    page: &wire::OperationPagination,
    models: &ModelPlan,
) -> Option<Compiled> {
    use crate::http_protocol::Representation;
    if page.pattern == PaginationPattern::NextLink {
        return None;
    }
    // Walks read the decoded page through the single successful alternative's
    // typed JSON body.
    let success: Vec<_> = operation
        .responses
        .iter()
        .filter(|response| response.may_succeed())
        .collect();
    let [response] = success.as_slice() else {
        return None;
    };
    if response.is_enum || response.media.len() != 1 {
        return None;
    }
    let Representation::Json { codec: Some(codec) } = response.media[0].wire.representation()
    else {
        return None;
    };
    let root = codec.schema().id().clone();
    let payload = response.type_name.clone();

    let (mode, candidates) = match page.pattern {
        PaginationPattern::Cursor => (Mode::Pointer, vec![PaginationRequestRole::Cursor]),
        PaginationPattern::LimitOffset => {
            if page.advance == PaginationAdvance::NextOffset
                && page
                    .response
                    .contains_key(&PaginationResponseRole::NextOffset)
            {
                (
                    Mode::Pointer,
                    vec![
                        PaginationRequestRole::Offset,
                        PaginationRequestRole::Page,
                        PaginationRequestRole::Cursor,
                    ],
                )
            } else {
                (Mode::ItemsReturned, vec![PaginationRequestRole::Offset])
            }
        }
        PaginationPattern::PageNumber => {
            if page.advance == PaginationAdvance::NextOffset
                && page
                    .response
                    .contains_key(&PaginationResponseRole::NextOffset)
            {
                (
                    Mode::Pointer,
                    vec![
                        PaginationRequestRole::Page,
                        PaginationRequestRole::Offset,
                        PaginationRequestRole::Cursor,
                    ],
                )
            } else {
                (Mode::PageNext, vec![PaginationRequestRole::Page])
            }
        }
        PaginationPattern::NextLink => unreachable!("rejected above"),
    };
    let role = candidates
        .iter()
        .copied()
        .find(|role| page.request.contains_key(role))?;
    let found = control(models, operation, &page.request[&role])?;
    let leaf = if found.text {
        Leaf::Token
    } else {
        Leaf::Number
    };
    let items_pointer = page.response.get(&PaginationResponseRole::Items);
    let items = match mode {
        Mode::ItemsReturned | Mode::PageNext => {
            Some(walk_pointer(models, &root, items_pointer?, Leaf::Items)?)
        }
        Mode::Pointer => {
            items_pointer.and_then(|pointer| walk_pointer(models, &root, pointer, Leaf::Items))
        }
    };
    let item_element = items.as_ref().and_then(|found| found.element.clone());
    let continuation = match mode {
        Mode::Pointer => {
            let pointer = match page.pattern {
                PaginationPattern::Cursor => page
                    .response
                    .get(&PaginationResponseRole::NextCursor)
                    .or_else(|| page.response.get(&PaginationResponseRole::NextOffset)),
                _ => page.response.get(&PaginationResponseRole::NextOffset),
            }?;
            Some(walk_pointer(models, &root, pointer, leaf)?)
        }
        Mode::ItemsReturned | Mode::PageNext => None,
    };
    let has_more = match page.response.get(&PaginationResponseRole::HasMore) {
        Some(pointer) => Some(walk_pointer(models, &root, pointer, Leaf::Flag)?),
        None => None,
    };
    // The documented SDK fallback page size rewrites the limit control on the
    // first request only, when the caller left it absent. A required plain
    // control has no absent state, so it can never take a fallback.
    let limit = page
        .request
        .get(&PaginationRequestRole::Limit)
        .and_then(|name| control(models, operation, name))
        .filter(|limit| limit.optional || limit.nullable);
    Some(Compiled {
        selection: page.clone(),
        payload,
        control: found,
        limit,
        mode,
        items,
        item_element,
        continuation,
        has_more,
    })
}

fn pattern_label(page: &wire::OperationPagination) -> &'static str {
    match (page.pattern, page.advance) {
        (PaginationPattern::LimitOffset, PaginationAdvance::ItemsReturned) => {
            "limit/offset, advanced by items returned"
        }
        (PaginationPattern::LimitOffset, PaginationAdvance::NextOffset) => {
            "limit/offset, advanced by the declared next offset"
        }
        (PaginationPattern::Cursor, _) => "cursor continuation",
        (PaginationPattern::PageNumber, _) => "one-based page number",
        _ => "next-link continuation",
    }
}

/// Source-derived operation label for documentation comments.
fn operation_label(op: &super::PlannedOperation) -> String {
    format!(
        "{} {}",
        super::emit::prose(op.method.as_str()),
        super::emit::prose(op.path.as_str())
    )
}

fn doc(out: &mut String, text: &str, indent: &str) {
    for line in text.lines() {
        let _ = writeln!(out, "{indent}/// {}", super::emit::prose(line));
    }
}

/// Generator-controlled documentation is emitted verbatim; only embedded
/// source-derived fragments pass through `doc`.
fn doc_raw(out: &mut String, text: &str, indent: &str) {
    for line in text.lines() {
        let _ = writeln!(out, "{indent}/// {line}");
    }
}

/// Render the generated `Pagination.swift` module: the shared walk rules plus
/// one walk per emittable paginated operation. `None` keeps the package free
/// of any pagination byte.
pub(super) fn emit(plan: &SdkPlan) -> Option<String> {
    let pagination = plan.pagination.as_ref()?;
    if pagination.operations.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str("import Foundation\n\n");
    out.push_str(
        "/// Generated pagination walks for this package's source-selected\n/// operations. The walks reuse the emitted operation methods for transport,\n/// encoding and decoding, and resolve the compiled RFC 6901 pointers with\n/// typed property chains over the decoded page models: no runtime schema\n/// search, no path evaluation and no JSON re-parsing. Each walk keeps one\n/// request in flight: a page is fetched exactly once, every later page\n/// rebuilds the caller's input with only the pagination control changed, and\n/// stopping — breaking out, cancellation, or the walk's own end — never fires\n/// a not-yet-started request. A source that repeats an identical continuation\n/// value throws `PaginationError` instead of looping.\n\n",
    );
    let _ = write!(
        out,
        "/// Thrown instead of looping forever when a source API repeats an\n/// identical continuation value, which would never terminate.\npublic struct {}: Error, Sendable, CustomStringConvertible {{\n    /// The paginated operation's source identity.\n    public let operation: String\n    /// The repeated continuation value, exactly as the source returned it.\n    public let value: String\n    public var description: String {{\n        \"pagination continuation repeated for \\(operation): \\(value)\"\n    }}\n}}\n\n",
        pagination.error_type
    );
    let _ = writeln!(
        out,
        "/// A pagination control value the walk cannot interpret exactly is a\n/// typed validation failure, never a silent guess.\nfunc {}(_ source: SourceLocation, _ error: JsonError) -> SDKError {{\n    SDKError(.requestValidation, source: source, json: error)\n}}\n\n/// A pagination control the walk cannot advance within exact integer bounds\n/// is a typed validation failure.\nfunc {}(_ source: SourceLocation) -> SDKError {{\n    SDKError(.requestValidation, source: source)\n}}\n",
        pagination.control_failure, pagination.advance_failure,
    );
    let mut extension_methods = String::new();
    for operation in &pagination.operations {
        let op = &plan.operations[operation.operation];
        operation_code(pagination, operation, op, &mut out, &mut extension_methods);
    }
    out.push_str(&extension_methods);
    Some(out)
}

/// The conversion statement reading one control as an exact `Int64`, with the
/// documented refusal on unrepresentable values and the optional fill of the
/// configured initial offset.
fn read_int(
    pagination: &PaginationPlan,
    operation: &PaginationOperation,
    control: &Control,
    base: &str,
    target: &str,
    fallback: u32,
    fill: bool,
) -> String {
    let failure = format!(
        "throw {}({}, error)",
        pagination.control_failure,
        source(&operation.selection)
    );
    let mut out = format!("        var {target}: Int64 = {fallback}\n");
    if !control.optional && !control.nullable {
        let _ = writeln!(
            out,
            "        do {{ {target} = try {}.int64Value() }} catch let error as JsonError {{ {failure} }}",
            control.core_read(base)
        );
        return out;
    }
    let _ = writeln!(
        out,
        "        if let present = {} {{\n            do {{ {target} = try present.int64Value() }} catch let error as JsonError {{ {failure} }}",
        control.read(base)
    );
    if fill {
        writeln!(out, "        }} else {{").unwrap();
        let fill_statement = control.write(base, &fallback.to_string());
        out.push_str(&fill_statement.replacen("        ", "            ", 1));
    }
    out.push_str("        }\n");
    out
}

/// The conversion statement seeding the previous-continuation state.
fn seed_statement(control: &Control, base: &str) -> String {
    let read = control.read(base);
    if control.text {
        format!("        if let seed = {read} {{\n            previous = seed\n        }}\n")
    } else {
        format!(
            "        if let seed = {read} {{\n            previous = try? seed.int64Value()\n        }}\n"
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn operation_code(
    pagination: &PaginationPlan,
    operation: &PaginationOperation,
    op: &super::PlannedOperation,
    out: &mut String,
    extension_methods: &mut String,
) {
    let label = format!(
        "{} {} — {}",
        op.method,
        op.path,
        pattern_label(&operation.selection)
    );
    let _ = writeln!(out, "\n// ---- {} ----\n", super::emit::prose(&label));
    let result = &op.success_type;
    let input = &op.input_type;
    // Walks read the decoded page payload through the single successful
    // alternative; `page.data` produces exactly that payload type.
    let payload = operation.payload.clone();
    let control = &operation.control;
    let pointer = |role: PaginationResponseRole| {
        operation
            .selection
            .response
            .get(&role)
            .map(String::as_str)
            .unwrap_or_default()
    };
    if let Some(items) = &operation.items {
        let ty = format!("{}?", items.ty);
        let _ = write!(
            out,
            "/// Items collection of one decoded `{result}` page at `{}`.\nfunc {}_pageItems(_ page: {payload}) -> {ty} {{\n{}}}\n\n",
            pointer(PaginationResponseRole::Items),
            op.method_name,
            items.body,
        );
    }
    if let Some(continuation) = &operation.continuation {
        let role = match operation.selection.pattern {
            PaginationPattern::Cursor => PaginationResponseRole::NextCursor,
            _ => PaginationResponseRole::NextOffset,
        };
        let _ = write!(
            out,
            "/// Continuation value of one decoded `{result}` page at `{}`.\nfunc {}_pageContinuation(_ page: {payload}) -> {}? {{\n{}}}\n\n",
            pointer(role),
            op.method_name,
            continuation.ty,
            continuation.body,
        );
    }
    if let Some(has_more) = &operation.has_more {
        let _ = write!(
            out,
            "/// Declared has-more flag of one decoded `{result}` page at `{}`.\nfunc {}_pageHasMore(_ page: {payload}) -> Bool? {{\n{}}}\n\n",
            pointer(PaginationResponseRole::HasMore),
            op.method_name,
            has_more.body,
        );
    }

    let identity = q(&operation.selection.operation);
    let advance_failure = |_message: &str| {
        format!(
            "throw {}({})",
            pagination.advance_failure,
            source(&operation.selection)
        )
    };
    let has_more_stop = |outcome: &str| {
        operation.has_more.as_ref().map(|_| {
            format!(
                "        if {}_pageHasMore(page.data) == false {{ {outcome} }}\n",
                op.method_name
            )
        })
    };
    let mut fields = format!(
        "    /// Input for the next page request; `nil` once the walk has ended.\n    var pending: {input}?\n"
    );
    let mut seed = String::new();
    // The documented SDK fallback page size fills the limit control on the
    // first request only, when the caller left it absent; later pages keep
    // whatever limit the walk last used.
    let limit_fill = match (&operation.limit, operation.selection.initial_limit) {
        (Some(limit), Some(size)) => format!(
            "        if {} == nil {{\n{}        }}\n",
            limit.read("rebuilt"),
            limit
                .write("rebuilt", &size.to_string())
                .replacen("        ", "            ", 1),
        ),
        _ => String::new(),
    };
    let next_body = match operation.mode {
        Mode::ItemsReturned => {
            let mut body = String::from(
                "        guard var rebuilt = pending else { return nil }\n        pending = nil\n",
            );
            body.push_str(&limit_fill);
            body.push_str(&read_int(
                pagination,
                operation,
                control,
                "rebuilt",
                "offset",
                operation.selection.initial_offset.unwrap_or(0),
                operation.selection.initial_offset.is_some() && control.optional,
            ));
            body.push_str(&format!(
                "        let page = try await client.{}(rebuilt)\n",
                op.method_name
            ));
            body.push_str(&format!(
                "        let items = {}_pageItems(page.data)\n        let count = Int64(items?.count ?? 0)\n        if count == 0 {{ return page }}\n",
                op.method_name
            ));
            if let Some(stop) = has_more_stop("return page") {
                body.push_str(&stop);
            }
            body.push_str("        let advanced = offset.addingReportingOverflow(count)\n        if advanced.overflow {\n            ");
            body.push_str(&advance_failure("pagination offset overflow"));
            body.push_str("\n        }\n");
            body.push_str(&control.write("rebuilt", "advanced.partialValue"));
            body.push_str("        pending = rebuilt\n        return page\n");
            body
        }
        Mode::PageNext => {
            let mut body = String::from(
                "        guard var rebuilt = pending else { return nil }\n        pending = nil\n",
            );
            body.push_str(&limit_fill);
            body.push_str(&read_int(
                pagination, operation, control, "rebuilt", "number", 1, false,
            ));
            body.push_str(&format!(
                "        let page = try await client.{}(rebuilt)\n",
                op.method_name
            ));
            body.push_str(&format!(
                "        let items = {}_pageItems(page.data)\n        if (items?.count ?? 0) == 0 {{ return page }}\n",
                op.method_name
            ));
            if let Some(stop) = has_more_stop("return page") {
                body.push_str(&stop);
            }
            body.push_str("        let advanced = number.addingReportingOverflow(1)\n        if advanced.overflow {\n            ");
            body.push_str(&advance_failure("pagination page overflow"));
            body.push_str("\n        }\n");
            body.push_str(&control.write("rebuilt", "advanced.partialValue"));
            body.push_str("        pending = rebuilt\n        return page\n");
            body
        }
        Mode::Pointer => {
            let previous_type = operation
                .continuation
                .as_ref()
                .map(|found| found.ty.clone())
                .unwrap_or_else(|| "String".to_owned());
            fields.push_str(&format!(
                "    /// The continuation that produced the pending request; an identical\n    /// continuation throws `PaginationError` on the following pull instead of\n    /// looping.\n    var previous: {previous_type}?\n    /// The repeated continuation observed on the last delivered page; the\n    /// typed error surfaces on the following pull, so the offending page is\n    /// still included exactly once.\n    var repeated: String?\n"
            ));
            seed = seed_statement(control, "input");
            let mut body = String::from("        if let repeated {\n            throw ");
            let _ = write!(
                body,
                "{error_type}(operation: {identity}, value: repeated)\n        }}\n",
                error_type = pagination.error_type,
            );
            body.push_str(
                "        guard var rebuilt = pending else { return nil }\n        pending = nil\n",
            );
            body.push_str(&limit_fill);
            body.push_str(&format!(
                "        let page = try await client.{}(rebuilt)\n",
                op.method_name
            ));
            if let Some(stop) = has_more_stop("return page") {
                body.push_str(&stop);
            }
            body.push_str(&format!(
                "        guard let continuation = {}_pageContinuation(page.data) else {{ return page }}\n",
                op.method_name
            ));
            if control.text {
                body.push_str("        if continuation.isEmpty { return page }\n");
            }
            body.push_str("        if continuation == previous {\n            repeated = \"\\(continuation)\"\n            return page\n        }\n");
            body.push_str("        previous = continuation\n");
            body.push_str(&control.write("rebuilt", "continuation"));
            body.push_str("        pending = rebuilt\n        return page\n");
            body
        }
    };
    out.push_str(&walk_documentation(op, operation));
    let _ = write!(
        out,
        "public struct {sequence}: AsyncSequence, AsyncIteratorProtocol, Sendable {{\n    public typealias Element = {result}\n    let client: Client\n{fields}\n    public init(client: Client, input: {input}) {{\n        self.client = client\n        pending = input\n{seed}    }}\n\n    public func makeAsyncIterator() -> Self {{ self }}\n\n    /// The next page, or `nil` only after the walk has ended. The request\n    /// starts when `next` is first awaited; dropping the sequence before that\n    /// never issues one.\n    public mutating func next() async throws -> {result}? {{\n{next_body}    }}\n}}\n\n",
        sequence = operation.page_sequence,
    );
    if let (Some(items), Some(element)) = (&operation.items, &operation.item_element) {
        let _ = write!(
            out,
            "/// Flattens the items of one `{sequence}` walk in order. Queued items are\n/// returned without a request; requests fire only when the previous page's\n/// items are exhausted, so early loop exit never issues another request and\n/// cancellation propagates through the page walk.\npublic struct {item_sequence}: AsyncSequence, AsyncIteratorProtocol, Sendable {{\n    public typealias Element = {element}\n    var pages: {sequence}\n    var queue: {items_ty}\n    var index = 0\n\n    public init(pages: {sequence}) {{\n        self.pages = pages\n        queue = []\n    }}\n\n    public func makeAsyncIterator() -> Self {{ self }}\n\n    /// The next item, or `nil` only after the final page's final item.\n    public mutating func next() async throws -> {element}? {{\n        while true {{\n            if index < queue.count {{\n                let item = queue[index]\n                index += 1\n                return item\n            }}\n            guard let page = try await pages.next() else {{ return nil }}\n            queue = {method}_pageItems(page.data) ?? []\n            index = 0\n        }}\n    }}\n}}\n\n",
            sequence = operation.page_sequence,
            item_sequence = operation.item_sequence,
            items_ty = items.ty,
            element = element,
            method = op.method_name,
        );
    }
    let default_input = if op.default_input() { " = .init()" } else { "" };
    let _ = writeln!(
        extension_methods,
        "extension Client {{\n    /// Lazily walks every page of `{}` ({}). See\n    /// ``{}`` for the continuation and stopping rules; the caller's\n    /// pagination values win for the first request.\n    public func {}(_ input: {input}{default_input}) -> {} {{\n        {}(client: self, input: input)\n    }}\n",
        operation_label(op),
        pattern_label(&operation.selection),
        operation.page_sequence,
        operation.pages_method,
        operation.page_sequence,
        operation.page_sequence,
    );
    if operation.items.is_some() {
        let _ = writeln!(
            extension_methods,
            "    /// Flattens the items of every `{}` page, in order. See\n    /// ``{}``.\n    public func {}(_ input: {input}{default_input}) -> {} {{\n        {}(pages: {}(input))\n    }}\n",
            op.method_name,
            operation.page_sequence,
            operation.items_method,
            operation.item_sequence,
            operation.item_sequence,
            operation.pages_method,
        );
    }
    let _ = writeln!(
        extension_methods,
        "    /// Issues the request for `input` once and returns the rebuilt input\n    /// that fetches the following page, or `nil` when the walk stops.\n    public func {}(_ input: {input}{default_input}) async throws -> {input}? {{\n        var sequence = {}(input)\n        guard try await sequence.next() != nil else {{ return nil }}\n        return sequence.pending\n    }}\n}}\n",
        operation.next_page_method, operation.pages_method,
    );
}

fn walk_documentation(op: &super::PlannedOperation, operation: &PaginationOperation) -> String {
    let mut docs = String::new();
    doc_raw(
        &mut docs,
        &format!(
            "Lazily walks every page of `{}` — {}.",
            operation_label(op),
            pattern_label(&operation.selection)
        ),
        "",
    );
    docs.push_str("///\n/// Each `next` issues at most one request and awaits the transport directly:\n/// no tasks are spawned, the request starts only while `next` is awaited, and\n/// dropping the sequence (or stopping iteration) cancels a pending request\n/// without firing another one. The first call requests the input as given;\n/// every later call rebuilds the input with only the pagination control\n/// changed.\n");
    if let Some(size) = operation.selection.initial_limit {
        docs.push_str(&format!("/// When the caller omits the limit, the first request supplies the\n/// documented SDK page size ({size}); later pages keep the limit the walk\n/// last used.\n"));
    }
    match operation.mode {
        Mode::ItemsReturned => {
            let initial = operation.selection.initial_offset.unwrap_or(0);
            let has_more = if operation.has_more.is_some() {
                ", on a `has-more=false` page"
            } else {
                ""
            };
            docs.push_str(&format!(
                "/// The walk fills the configured initial offset ({initial}) when the\n/// caller left the offset absent, advances by the previous page's item\n/// count, and stops on a zero-item page{has_more}.\n"
            ));
        }
        Mode::PageNext => {
            docs.push_str("/// An absent page is assumed to be page 1 and every later page increments\n/// the page control; the walk stops on a zero-item page and, when the\n/// source maps one, on a `has-more=false` page.\n");
        }
        Mode::Pointer => {
            docs.push_str(&format!(
                "/// The walk stops when the continuation pointer reads absent{}{}, and\n/// throws `PaginationError` on the pull following a page that repeats an\n/// identical continuation (that page is still delivered exactly once).\n",
                if operation.control.text { ", empty" } else { "" },
                if operation.has_more.is_some() {
                    ", or delivers a `has-more=false` page and stops"
                } else {
                    ""
                }
            ));
        }
    }
    docs.push_str("///\n");
    doc(
        &mut docs,
        &format!("Source: {}#{}.", op.source.document(), op.source.pointer()),
        "",
    );
    docs
}
