//! Emitted-only pagination walkers for the C# HTTP adapter.
//!
//! The shared, generation-time `http_protocol::plan_pagination` outcome lowers
//! into per-operation page/item/next-page methods on the generated `Client`
//! plus one generated `Pagination.g.cs` file holding the typed traversal
//! error, the frozen per-operation descriptors and the pointer accessors.
//! Static runtime files gain nothing: with no configured policy, or with no
//! emittable paginated operation, the package stays byte-identical.
//!
//! Walkers reuse the direct operation method for transport, encoding, decoding
//! and attribution, rebuild the input record with a `with` expression that
//! changes only the pagination control, and resolve the compiled RFC 6901
//! pointers with generated property chains — never schema search or JSONPath.

use super::{
    SdkPlan, allocate,
    emit::{quote, source, xml},
    models::{CsDecl, CsType},
};
use crate::http_protocol as p;
use crate::sdk_defaults::{
    PaginationAdvance, PaginationPattern, PaginationRequestRole, PaginationResponseRole,
};
use std::collections::BTreeSet;
use suspect_ir::contract::SchemaId;

/// How the walk computes and applies its continuation, mirroring the shared
/// TypeScript walker's branch order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// The offset control advances by the previous page's item count.
    ItemsReturned,
    /// The resolved next-offset pointer feeds the offset, page or cursor control.
    OffsetPointer,
    /// The resolved next-cursor pointer feeds the cursor, offset or page control.
    CursorPointer,
    /// The one-based page control advances by one.
    PageNext,
}

/// The native carrier of a request-side pagination control value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The exact `JsonInteger` model.
    Integer,
    /// A plain `string`.
    Text,
}

/// One request control the walker rewrites between pages.
#[derive(Debug, Clone)]
struct Control {
    /// Input record property carrying the wire parameter.
    property: String,
    kind: Kind,
    required: bool,
}

impl Control {
    /// The generated expression reading this control as an exact integer.
    fn read_integer(&self, fallback: &str) -> String {
        if self.required {
            format!("input.{}.ToBigInteger()", self.property)
        } else {
            format!(
                "input.{}.HasValue ? input.{}.Value.ToBigInteger() : {}",
                self.property, self.property, fallback
            )
        }
    }

    /// The generated expression reading this control as text.
    fn read_text(&self) -> String {
        if self.required {
            format!("input.{}", self.property)
        } else {
            format!(
                "input.{}.HasValue ? input.{}.Value : null",
                self.property, self.property
            )
        }
    }

    /// The generated expression storing one continuation value back.
    fn write(&self, value: &str) -> String {
        let wrapped = match self.kind {
            Kind::Integer => format!("Optional<JsonInteger>.Present({value})"),
            Kind::Text => format!("Optional<string>.Present({value})"),
        };
        if self.required {
            value.to_owned()
        } else {
            wrapped
        }
    }
}

/// One operation's fully rendered pagination emission; the metadata fields
/// document the compiled walk for tests and introspection.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(super) struct Walker {
    /// Source operation identity.
    pub operation: String,
    /// Source location for documentation.
    pub location: String,
    /// Source description, when declared.
    pub description: String,
    /// The direct client method this walk extends.
    pub method: String,
    pub input_type: String,
    pub result_type: String,
    pub item_type: Option<String>,
    /// The documented input member the walk rewrites.
    pub control_name: String,
    pub mode: Mode,
    pub initial_offset: Option<u32>,
    /// Rendered Client methods.
    pub pages_method: String,
    pub items_method: Option<String>,
    pub next_page_method: String,
    /// Rendered pagination-class members.
    pub accessors: String,
    pub continuation_method: String,
    /// The frozen descriptor entry for this operation.
    pub descriptor: String,
}

/// The pointer precedence and control-role order of the shared walker.
fn pointer_selection(
    pattern: PaginationPattern,
    advance: PaginationAdvance,
    page: &p::OperationPagination,
) -> Option<(Mode, PaginationRequestRole)> {
    use PaginationRequestRole as Role;
    use PaginationResponseRole as Pointer;
    let next_offset = page.response.contains_key(&Pointer::NextOffset);
    let next_cursor = page.response.contains_key(&Pointer::NextCursor);
    if advance == PaginationAdvance::NextOffset && (next_offset || next_cursor) {
        // The shared walker prefers a mapped next-offset pointer and otherwise
        // a mapped next-cursor pointer, then applies the value to the pattern's
        // own control first, exactly as the TypeScript walker does.
        if next_offset {
            let role = [Role::Offset, Role::Page, Role::Cursor]
                .into_iter()
                .find(|role| page.request.contains_key(role))?;
            return Some((Mode::OffsetPointer, role));
        }
        let role = [Role::Cursor, Role::Offset, Role::Page]
            .into_iter()
            .find(|role| page.request.contains_key(role))?;
        return Some((Mode::CursorPointer, role));
    }
    if pattern == PaginationPattern::PageNumber && page.request.contains_key(&Role::Page) {
        return Some((Mode::PageNext, Role::Page));
    }
    None
}

/// The request control one mode advances: the wire-named query parameter's
/// native input property. Values must be string- or exact-integer-typed.
fn control(op: &super::PlannedOperation, name: &str) -> Option<(Control, String)> {
    let parameter = op.parameters.iter().find(|parameter| {
        parameter.wire_name == name && parameter.location == p::ParameterLocation::Query
    })?;
    let kind = match parameter.native_type.as_str() {
        "JsonInteger" => Kind::Integer,
        "string" => Kind::Text,
        _ => return None,
    };
    Some((
        Control {
            property: parameter.property_name.clone(),
            kind,
            required: parameter.required,
        },
        parameter.property_name.clone(),
    ))
}

/// Follow erased aliases to the concrete declaration and its nullability.
fn follow(plan: &SdkPlan, id: &SchemaId) -> Option<(CsDecl, bool)> {
    let mut current = id.clone();
    for _ in 0..64 {
        let key = super::models::key(&current);
        let declaration = plan.models().declarations.get(&key)?;
        let nullable = plan.models().is_nullable(&current)?;
        match declaration {
            CsDecl::Alias(CsType::Named(next)) => current = next.0.clone(),
            other => return Some((other.clone(), nullable)),
        }
    }
    None
}

/// The declaration one record field's type reads, with its nullability.
fn field_target(plan: &SdkPlan, ty: &CsType) -> Option<(CsDecl, bool)> {
    match ty {
        CsType::Nullable(inner) => Some((CsDecl::Alias(inner.as_ref().clone()), true)),
        CsType::Named(key) => follow(plan, &key.0),
        other => Some((CsDecl::Alias(other.clone()), false)),
    }
}

/// One resolved pointer: guard statements plus the final plain-property
/// expression and the declaration that expression reads.
struct Resolved {
    statements: String,
    expression: String,
    declaration: CsDecl,
}

/// Resolve one RFC 6901 pointer over the decoded page body with generated
/// property chains and absence guards. `None` means this plan cannot express
/// the pointer with plain property access, so the operation stays without
/// walkers.
fn resolve_pointer(plan: &SdkPlan, root: &SchemaId, pointer: &str) -> Option<Resolved> {
    let mut statements = String::new();
    let mut counter = 0usize;
    let (mut declaration, nullable) = follow(plan, root)?;
    let mut expression = String::from("page.Data");
    if nullable {
        counter += 1;
        let local = format!("value{counter}");
        statements.push_str(&format!(
            "        var {local} = page.Data;\n        if ({local} is null)\n        {{\n            return null;\n        }}\n"
        ));
        expression = local;
    }
    if pointer.is_empty() {
        return Some(Resolved {
            statements,
            expression,
            declaration,
        });
    }
    for raw in pointer[1..].split('/') {
        let segment = raw.replace("~1", "/").replace("~0", "~");
        let CsDecl::Record { fields, .. } = &declaration else {
            return None;
        };
        let field = fields.iter().find(|field| field.wire == segment)?;
        let access = format!("{expression}.{}", field.name);
        let (target, target_nullable) = field_target(plan, &field.ty)?;
        if !field.required {
            counter += 1;
            let local = format!("value{counter}");
            statements.push_str(&format!(
                "        var {local} = {access};\n        if (!{local}.HasValue)\n        {{\n            return null;\n        }}\n"
            ));
            expression = format!("{local}.Value");
        } else {
            expression = access;
        }
        if target_nullable {
            counter += 1;
            let local = format!("value{counter}");
            statements.push_str(&format!(
                "        var {local} = {expression};\n        if ({local} is null)\n        {{\n            return null;\n        }}\n"
            ));
            expression = local;
        }
        declaration = target;
    }
    Some(Resolved {
        statements,
        expression,
        declaration,
    })
}

/// The value one resolved expression reads, following erased aliases.
fn leaf(plan: &SdkPlan, declaration: &CsDecl) -> Option<CsType> {
    match declaration {
        CsDecl::Alias(CsType::Named(key)) => {
            let (inner, _) = follow(plan, &key.0)?;
            leaf(plan, &inner)
        }
        CsDecl::Alias(inner) => Some(inner.clone()),
        _ => None,
    }
}

/// Render one pointer reader as an unexported static method with a nullable
/// return, so absence reads as null at every stop rule.
fn render_accessor(
    result_type: &str,
    method: &str,
    returns: &str,
    comment: &str,
    resolved: &Resolved,
) -> String {
    format!(
        "    /// <summary>{comment}</summary>\n    internal static {returns} {method}({result_type} page)\n    {{\n{}        return {};\n    }}\n",
        resolved.statements, resolved.expression
    )
}

/// The single typed JSON success representation the walker reads: the direct
/// result's sole success media.
fn success_body(op: &super::PlannedOperation) -> Option<&super::protocol::PlannedMedia> {
    if !op.direct_result() {
        return None;
    }
    let response = op
        .responses
        .iter()
        .find(|response| response.may_succeed())?;
    if response.media.len() != 1 {
        return None;
    }
    match response.media[0].wire.representation() {
        p::Representation::Json { codec: Some(_) } => Some(&response.media[0]),
        _ => None,
    }
}

fn pattern_name(pattern: PaginationPattern) -> &'static str {
    match pattern {
        PaginationPattern::LimitOffset => "limit-offset",
        PaginationPattern::Cursor => "cursor",
        PaginationPattern::PageNumber => "page-number",
        PaginationPattern::NextLink => "next-link",
    }
}

fn advance_name(advance: PaginationAdvance) -> &'static str {
    match advance {
        PaginationAdvance::ItemsReturned => "items-returned",
        PaginationAdvance::NextOffset => "next-offset",
    }
}

fn response_key(role: PaginationResponseRole) -> &'static str {
    match role {
        PaginationResponseRole::Items => "items",
        PaginationResponseRole::NextCursor => "nextCursor",
        PaginationResponseRole::NextOffset => "nextOffset",
        PaginationResponseRole::HasMore => "hasMore",
        PaginationResponseRole::Total => "total",
        PaginationResponseRole::TotalPages => "totalPages",
    }
}

fn request_key(role: PaginationRequestRole) -> &'static str {
    match role {
        PaginationRequestRole::Limit => "limit",
        PaginationRequestRole::Offset => "offset",
        PaginationRequestRole::Cursor => "cursor",
        PaginationRequestRole::Page => "page",
    }
}

/// Compile the emittable subset of the configured outcome, allocating every
/// generated name from the client's method set. A configured policy with no
/// emittable operation emits nothing.
pub(super) fn compiled(plan: &SdkPlan) -> Vec<Walker> {
    let Some(outcome) = plan.pagination() else {
        return Vec::new();
    };
    if outcome.paginated.is_empty() {
        return Vec::new();
    }
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
    let mut walkers = Vec::new();
    for op in plan.operations() {
        let Some(page) = outcome
            .paginated
            .iter()
            .find(|page| page.operation == op.operation_id)
        else {
            continue;
        };
        if let Some(walker) = compile(plan, op, page, &mut methods) {
            walkers.push(walker);
        }
    }
    walkers
}

/// Whether any configured operation lowers into a generated walker.
pub(super) fn emits(plan: &SdkPlan) -> bool {
    !compiled(plan).is_empty()
}

/// The documented stop rule of one compiled walk.
fn stop_rule(mode: Mode, has_more: bool) -> &'static str {
    match mode {
        Mode::ItemsReturned => {
            if has_more {
                "after a page with no items or when the has-more indicator is false"
            } else {
                "after a page with no items"
            }
        }
        Mode::CursorPointer => "when the next cursor token is absent, null or an empty string",
        Mode::OffsetPointer => "when the next offset pointer is absent or null",
        Mode::PageNext => {
            if has_more {
                "after a page with no items, when the has-more indicator is false, or after the last declared page"
            } else {
                "after a page with no items"
            }
        }
    }
}

/// The first-page fill of the configured initial offset; a caller-supplied
/// control value still wins.
fn initial_fill(control: &Control, initial: Option<u32>) -> String {
    let Some(initial) = initial else {
        return String::new();
    };
    if control.required || !matches!(control.kind, Kind::Integer) {
        return String::new();
    }
    format!(
        "        if (!current.{property}.HasValue)\n        {{\n            current = current with {{ {property} = Optional<JsonInteger>.Present(new JsonInteger({initial})) }};\n        }}\n",
        property = control.property,
        initial = quote(&initial.to_string())
    )
}

/// The first-page fill of the documented SDK fallback page size; a
/// caller-supplied limit still wins, and later pages keep whatever limit the
/// walk last used.
fn limit_fill(control: &Option<Control>, size: Option<u32>) -> String {
    let (Some(control), Some(size)) = (control, size) else {
        return String::new();
    };
    if control.required || !matches!(control.kind, Kind::Integer) {
        return String::new();
    }
    format!(
        "        if (!current.{property}.HasValue)\n        {{\n            current = current with {{ {property} = Optional<JsonInteger>.Present(new JsonInteger({size})) }};\n        }}\n",
        property = control.property,
        size = quote(&size.to_string())
    )
}

#[allow(clippy::too_many_lines)]
fn compile(
    plan: &SdkPlan,
    op: &super::PlannedOperation,
    page: &p::OperationPagination,
    methods: &mut BTreeSet<String>,
) -> Option<Walker> {
    let media = success_body(op)?;
    let root = media.schema()?;
    let stem = op
        .method_name
        .strip_suffix("Async")
        .unwrap_or(&op.method_name);
    let stem = if stem.is_empty() {
        op.method_name.as_str()
    } else {
        stem
    };
    // Resolve the limit control before the `control` binding below shadows
    // the resolver function.
    let limit_control = page
        .request
        .get(&PaginationRequestRole::Limit)
        .and_then(|name| control(op, name).map(|(control, _)| control));
    let (mode, control, control_name) = match (page.pattern, page.advance) {
        (PaginationPattern::LimitOffset, PaginationAdvance::ItemsReturned) => {
            let name = page.request.get(&PaginationRequestRole::Offset)?;
            let (control, property) = control(op, name)?;
            (Mode::ItemsReturned, control, property)
        }
        _ => {
            let (mode, role) = pointer_selection(page.pattern, page.advance, page)?;
            let name = page.request.get(&role)?;
            let (control, property) = control(op, name)?;
            (mode, control, property)
        }
    };
    let mut used: BTreeSet<String> = methods.clone();
    let mut items_accessor = None;
    let mut item_type = None;
    let mut items_rendered = None;
    if let Some(pointer) = page.response.get(&PaginationResponseRole::Items) {
        let resolved = resolve_pointer(plan, root, pointer)?;
        let CsType::List(element) = leaf(plan, &resolved.declaration)? else {
            return None;
        };
        let method = allocate(&format!("{stem}PageItems"), &mut used);
        let element_type = plan
            .models()
            .qualified_type(&element, &plan.config().namespace);
        let rendered = render_accessor(
            op.result_type.as_str(),
            &method,
            &format!("global::System.Collections.Generic.List<{element_type}>?"),
            &format!(
                "Items collection of one decoded page at {}; an absent path reads as no items.",
                xml(pointer)
            ),
            &resolved,
        );
        items_rendered = Some(rendered);
        item_type = Some(element_type);
        items_accessor = Some(method);
    }
    let mut cursor_accessor = None;
    let mut offset_accessor = None;
    if mode == Mode::OffsetPointer || mode == Mode::CursorPointer {
        let pointer = if mode == Mode::OffsetPointer {
            page.response
                .get(&PaginationResponseRole::NextOffset)
                .or_else(|| page.response.get(&PaginationResponseRole::NextCursor))?
        } else {
            page.response
                .get(&PaginationResponseRole::NextCursor)
                .or_else(|| page.response.get(&PaginationResponseRole::NextOffset))?
        };
        let resolved = resolve_pointer(plan, root, pointer)?;
        let value = leaf(plan, &resolved.declaration)?;
        let kind = match &value {
            CsType::Native("string") => Kind::Text,
            CsType::Integer => Kind::Integer,
            _ => return None,
        };
        if kind != control.kind {
            return None;
        }
        let (suffix, returns, comment) = match mode {
            Mode::OffsetPointer => (
                "NextOffset",
                "JsonInteger?",
                "Continuation offset of one decoded page at the configured pointer; absent paths read as null.",
            ),
            _ => (
                "Cursor",
                "string?",
                "Continuation cursor of one decoded page at the configured pointer; absent paths read as null.",
            ),
        };
        let method = allocate(&format!("{stem}Page{suffix}"), &mut used);
        let rendered = render_accessor(
            op.result_type.as_str(),
            &method,
            returns,
            comment,
            &resolved,
        );
        if mode == Mode::OffsetPointer {
            offset_accessor = Some(rendered);
        } else {
            cursor_accessor = Some(rendered);
        }
    }
    let mut has_more_accessor = None;
    if let Some(pointer) = page.response.get(&PaginationResponseRole::HasMore) {
        let resolved = resolve_pointer(plan, root, pointer)?;
        if !matches!(leaf(plan, &resolved.declaration)?, CsType::Native("bool")) {
            return None;
        }
        let method = allocate(&format!("{stem}PageHasMore"), &mut used);
        has_more_accessor = Some(render_accessor(
            op.result_type.as_str(),
            &method,
            "bool?",
            "Declared has-more evidence of one decoded page at the configured pointer.",
            &resolved,
        ));
    }
    let mut total_pages_accessor = None;
    if mode == Mode::PageNext
        && let Some(pointer) = page.response.get(&PaginationResponseRole::TotalPages)
        && let Some(resolved) = resolve_pointer(plan, root, pointer).filter(|resolved| {
            matches!(
                leaf(plan, &resolved.declaration),
                Some(CsType::Integer | CsType::Number)
            )
        })
    {
        let method = allocate(&format!("{stem}PageTotalPages"), &mut used);
        total_pages_accessor = Some(render_accessor(
            op.result_type.as_str(),
            &method,
            "JsonInteger?",
            "Declared total page count of one decoded page at the configured pointer.",
            &resolved,
        ));
    }
    // Counting modes cannot advance without the items collection; pointer
    // modes advance without it but then emit no items method.
    if items_accessor.is_none() && matches!(mode, Mode::ItemsReturned | Mode::PageNext) {
        return None;
    }
    let pages = allocate(&format!("{stem}PagesAsync"), methods);
    let next_page = allocate(&format!("{stem}NextPageAsync"), methods);
    let items = items_accessor
        .clone()
        .map(|_| allocate(&format!("{stem}Items"), methods));
    let continuation = allocate(&format!("{stem}Continuation"), methods);
    let identity = quote(&page.operation);
    let location = source(&op.source);
    let initial = if mode == Mode::ItemsReturned {
        initial_fill(&control, page.initial_offset)
    } else {
        String::new()
    };
    // The documented SDK fallback page size fills the limit control on page 1
    // only, and only when the caller left it absent.
    let limit = limit_fill(&limit_control, page.initial_limit);
    let initial = format!("{initial}{limit}");
    let fallback_doc = page.initial_limit.map_or_else(String::new, |size| {
        format!(
            " When the caller omits the limit, the first request supplies the documented SDK page size ({size}); later pages keep the limit the walk last used."
        )
    });
    let mut accessors = String::new();
    accessors.push_str(items_rendered.as_deref().unwrap_or_default());
    accessors.push_str(cursor_accessor.as_deref().unwrap_or_default());
    accessors.push_str(offset_accessor.as_deref().unwrap_or_default());
    accessors.push_str(has_more_accessor.as_deref().unwrap_or_default());
    accessors.push_str(total_pages_accessor.as_deref().unwrap_or_default());
    let continuation_method = continuation_method(
        mode,
        &control,
        &identity,
        op.input_type.as_str(),
        op.result_type.as_str(),
        &continuation,
        items_accessor.as_deref(),
        cursor_accessor
            .as_deref()
            .map(|_| format!("{stem}PageCursor")),
        offset_accessor
            .as_deref()
            .map(|_| format!("{stem}PageNextOffset")),
        has_more_accessor
            .as_deref()
            .map(|_| format!("{stem}PageHasMore")),
        total_pages_accessor
            .as_deref()
            .map(|_| format!("{stem}PageTotalPages")),
        page.initial_offset,
    );
    let descriptor = descriptor_entry(
        &page.operation,
        pattern_name(page.pattern),
        advance_name(page.advance),
        page.initial_offset,
        &page.request,
        &op.parameters,
        &page.response,
    );
    let pages_method = pages_method(
        op,
        &control_name,
        &pages,
        &continuation,
        stop_rule(mode, has_more_accessor.is_some()),
        &location,
        &initial,
        &fallback_doc,
    );
    let items_method = items.map(|items| {
        items_method(
            op,
            &pages,
            &items,
            &continuation,
            items_accessor.as_deref().unwrap_or_default(),
            item_type.as_deref().unwrap_or("object"),
            &location,
            &initial,
        )
    });
    let next_page_method = next_page_method(
        op,
        &pages,
        &next_page,
        &continuation,
        &location,
        limit_fill(&limit_control, page.initial_limit),
    );
    Some(Walker {
        operation: page.operation.clone(),
        location: location.to_string(),
        description: op.description.clone(),
        method: op.method_name.clone(),
        input_type: op.input_type.clone(),
        result_type: op.result_type.clone(),
        item_type,
        control_name,
        mode,
        initial_offset: page.initial_offset,
        pages_method,
        items_method,
        next_page_method,
        accessors,
        continuation_method,
        descriptor,
    })
}

/// The documented lazy-walk summary of one compiled walk.
fn pages_summary(
    op: &super::PlannedOperation,
    control_name: &str,
    stop: &'static str,
    location: &str,
    fallback: &str,
) -> String {
    let description = if op.description.is_empty() {
        String::new()
    } else {
        format!("{} ", xml(&op.description))
    };
    format!(
        "{description}Lazily yields every success page of {}. The first page is the direct {}(...) result, fetched on the first MoveNextAsync and included exactly once. Between requests only the {} pagination control changes: every other input member is preserved exactly, and a caller-supplied {} is used for the first request and replaced by the computed continuation afterwards.{fallback} Stops {}. A repeated identical continuation value throws PaginationException instead of looping. Iteration is lazy: no request happens before the first MoveNextAsync, breaking out or cancelling the enumeration prevents further requests, and pages are never prefetched. Source: {}.",
        xml(&op.operation_id),
        op.method_name,
        control_name,
        control_name,
        stop,
        xml(location)
    )
}

#[allow(clippy::too_many_arguments)]
fn pages_method(
    op: &super::PlannedOperation,
    control_name: &str,
    pages: &str,
    continuation: &str,
    stop: &'static str,
    location: &str,
    initial: &str,
    fallback: &str,
) -> String {
    let summary = pages_summary(op, control_name, stop, location, fallback);
    format!(
        "    /// <summary>{summary}</summary>\n    public async global::System.Collections.Generic.IAsyncEnumerable<{}> {}({} input, [global::System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)\n    {{\n        var current = input;\n{initial}        while (true)\n        {{\n            var page = await {}(current, cancellationToken: cancellationToken).ConfigureAwait(false);\n            yield return page;\n            var next = Pagination.{}(current, page);\n            if (next is null)\n            {{\n                break;\n            }}\n            current = next;\n        }}\n    }}\n",
        op.result_type,
        pages,
        op.input_type,
        op.method_name,
        continuation,
        initial = initial
    )
}

#[allow(clippy::too_many_arguments)]
fn items_method(
    op: &super::PlannedOperation,
    pages: &str,
    items: &str,
    continuation: &str,
    items_accessor: &str,
    item_type: &str,
    location: &str,
    initial: &str,
) -> String {
    format!(
        "    /// <summary>Flattens the page items of {} across every page, in order. Accepts the same input as the direct call; see {} for lazy fetching, continuation, stopping and caller-override semantics. Source: {}.</summary>\n    public async global::System.Collections.Generic.IAsyncEnumerable<{}> {}({} input, [global::System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)\n    {{\n        var current = input;\n{initial}        while (true)\n        {{\n            var page = await {}(current, cancellationToken: cancellationToken).ConfigureAwait(false);\n            var pageItems = Pagination.{}(page);\n            if (pageItems is not null)\n            {{\n                foreach (var item in pageItems)\n                {{\n                    yield return item;\n                }}\n            }}\n            var next = Pagination.{}(current, page);\n            if (next is null)\n            {{\n                break;\n            }}\n            current = next;\n        }}\n    }}\n",
        xml(&op.operation_id),
        pages,
        xml(location),
        item_type,
        items,
        op.input_type,
        op.method_name,
        items_accessor,
        continuation,
        initial = initial
    )
}

fn next_page_method(
    op: &super::PlannedOperation,
    pages: &str,
    next_page: &str,
    continuation: &str,
    location: &str,
    limit: String,
) -> String {
    // Without a configured fallback page size the helper fetches the input as
    // given, exactly as before; with one, page 1 supplies it when the caller
    // omitted the limit control.
    let body = if limit.is_empty() {
        format!(
            "        var page = await {}(input, cancellationToken: cancellationToken).ConfigureAwait(false);\n        return Pagination.{}(input, page);\n",
            op.method_name, continuation
        )
    } else {
        format!(
            "        var current = input;\n{limit}        var page = await {}(current, cancellationToken: cancellationToken).ConfigureAwait(false);\n        return Pagination.{}(current, page);\n",
            op.method_name,
            continuation,
            limit = limit,
        )
    };
    format!(
        "    /// <summary>Fetches the page described by input and returns the rebuilt input for the following page, or null when the walk stops, so callers can drive pages manually. See {} for the stop rules and caller-override semantics. Source: {}.</summary>\n    public async Task<{}?> {}({} input, CancellationToken cancellationToken = default)\n    {{\n{body}    }}\n",
        pages,
        xml(location),
        op.input_type,
        next_page,
        op.input_type,
        body = body,
    )
}

#[allow(clippy::too_many_arguments)]
fn continuation_method(
    mode: Mode,
    control: &Control,
    identity: &str,
    input_type: &str,
    result_type: &str,
    continuation: &str,
    items_accessor: Option<&str>,
    cursor_accessor: Option<String>,
    offset_accessor: Option<String>,
    has_more_accessor: Option<String>,
    total_pages_accessor: Option<String>,
    initial_offset: Option<u32>,
) -> String {
    let has_more_stop = has_more_accessor
        .as_ref()
        .map(|accessor| {
            format!(
                "        if ({accessor}(page) is false)\n        {{\n            return null;\n        }}\n"
            )
        })
        .unwrap_or_default();
    let repeats_stop = |expression: String| {
        format!(
            "        if ({expression})\n        {{\n            throw new PaginationException({identity}, \"the source API returned an identical continuation value; the paginated walk would never terminate\");\n        }}\n",
            identity = identity
        )
    };
    let body = match mode {
        Mode::ItemsReturned => {
            let base = initial_offset.unwrap_or(0).to_string();
            format!(
                "{has_more_stop}        var items = {items}(page);\n        var count = items is null ? 0 : items.Count;\n        if (count == 0)\n        {{\n            return null;\n        }}\n        global::System.Numerics.BigInteger baseOffset;\n        try\n        {{\n            baseOffset = {read};\n        }}\n        catch (CodecException error)\n        {{\n            throw new PaginationException({identity}, \"the current pagination offset is not an exact integer\", error);\n        }}\n        JsonInteger advanced;\n        try\n        {{\n            advanced = JsonInteger.FromInteger(baseOffset + count);\n        }}\n        catch (CodecException error)\n        {{\n            throw new PaginationException({identity}, \"the advanced pagination offset is not representable\", error);\n        }}\n        return input with {{ {control} = {write} }};\n",
                has_more_stop = has_more_stop,
                items = items_accessor.unwrap_or_default(),
                read = control.read_integer(&base),
                identity = identity,
                control = control.property,
                write = control.write("advanced"),
            )
        }
        Mode::CursorPointer => {
            let accessor = cursor_accessor.unwrap_or_default();
            format!(
                "        var token = {accessor}(page);\n        if (token is null || token.Length == 0)\n        {{\n            return null;\n        }}\n{repeats}        return input with {{ {control} = {write} }};\n",
                accessor = accessor,
                repeats = repeats_stop(format!(
                    "global::System.StringComparer.Ordinal.Equals({}, token)",
                    control.read_text()
                )),
                control = control.property,
                write = control.write("token"),
            )
        }
        Mode::OffsetPointer => {
            let accessor = offset_accessor.unwrap_or_default();
            let repeats = match (control.kind, control.required) {
                (Kind::Text, _) => repeats_stop(format!(
                    "global::System.StringComparer.Ordinal.Equals({}, value)",
                    control.read_text()
                )),
                (Kind::Integer, false) => repeats_stop(format!(
                    "input.{}.HasValue && input.{}.Value.Equals(value)",
                    control.property, control.property
                )),
                (Kind::Integer, true) => {
                    repeats_stop(format!("input.{}.Equals(value)", control.property))
                }
            };
            format!(
                "        var value = {accessor}(page);\n        if (value is null)\n        {{\n            return null;\n        }}\n{repeats}        return input with {{ {control} = {write} }};\n",
                accessor = accessor,
                repeats = repeats,
                control = control.property,
                write = control.write("value"),
            )
        }
        Mode::PageNext => {
            let total = total_pages_accessor
                .as_ref()
                .map(|accessor| {
                    format!(
                        "        var total = {accessor}(page);\n        if (total is not null)\n        {{\n            try\n            {{\n                if (current >= total.Value.ToBigInteger())\n                {{\n                    return null;\n                }}\n            }}\n            catch (CodecException error)\n            {{\n                throw new PaginationException({identity}, \"the declared total page count is not an exact integer\", error);\n            }}\n        }}\n",
                        accessor = accessor,
                        identity = identity
                    )
                })
                .unwrap_or_default();
            format!(
                "{has_more_stop}        var items = {items}(page);\n        if (items is null || items.Count == 0)\n        {{\n            return null;\n        }}\n        global::System.Numerics.BigInteger current;\n        try\n        {{\n            current = {read};\n        }}\n        catch (CodecException error)\n        {{\n            throw new PaginationException({identity}, \"the current page number is not an exact integer\", error);\n        }}\n{total}        JsonInteger advanced;\n        try\n        {{\n            advanced = JsonInteger.FromInteger(current + 1);\n        }}\n        catch (CodecException error)\n        {{\n            throw new PaginationException({identity}, \"the advanced page number is not representable\", error);\n        }}\n        return input with {{ {control} = {write} }};\n",
                has_more_stop = has_more_stop,
                items = items_accessor.unwrap_or_default(),
                read = control.read_integer("1"),
                identity = identity,
                total = total,
                control = control.property,
                write = control.write("advanced"),
            )
        }
    };
    format!(
        "    /// <summary>Computes the input for the page after one fetched page, or null when the walk stops. Only the pagination control changes; every other input member is preserved exactly.</summary>\n    internal static {input_type}? {continuation}({input_type} input, {result_type} page)\n    {{\n{body}    }}\n",
        input_type = input_type,
        result_type = result_type,
        continuation = continuation,
        body = body
    )
}

fn descriptor_entry(
    operation: &str,
    pattern: &'static str,
    advance: &'static str,
    initial_offset: Option<u32>,
    request: &std::collections::BTreeMap<PaginationRequestRole, String>,
    parameters: &[super::PlannedParameter],
    response: &std::collections::BTreeMap<PaginationResponseRole, String>,
) -> String {
    let render = |map: &std::collections::BTreeMap<&'static str, String>| {
        if map.is_empty() {
            return "new Dictionary<string, string>(StringComparer.Ordinal) { }".to_owned();
        }
        let entries = map
            .iter()
            .map(|(key, value)| format!("[{}] = {},", quote(key), quote(value)))
            .collect::<Vec<_>>()
            .join(" ");
        format!("new Dictionary<string, string>(StringComparer.Ordinal) {{ {entries} }}")
    };
    let request_map = render(
        &request
            .iter()
            .map(|(role, name)| {
                (
                    request_key(*role),
                    parameters
                        .iter()
                        .find(|parameter| {
                            parameter.wire_name == name.as_str()
                                && parameter.location == p::ParameterLocation::Query
                        })
                        .map(|parameter| parameter.property_name.clone())
                        .unwrap_or_else(|| name.clone()),
                )
            })
            .collect(),
    );
    let response_map = render(
        &response
            .iter()
            .map(|(role, pointer)| (response_key(*role), pointer.clone()))
            .collect(),
    );
    format!(
        "        [{}] = new PaginationDescriptor({}, {}, {}, {}, {}, {}),\n",
        quote(operation),
        quote(operation),
        quote(pattern),
        request_map,
        response_map,
        initial_offset.map_or_else(|| "null".to_owned(), |offset| offset.to_string()),
        quote(advance)
    )
}

/// The generated `src/Pagination.g.cs`: the typed traversal error, the frozen
/// per-operation descriptors and the pointer readers and continuation rules.
/// Called only when at least one operation lowers into a walker.
pub(super) fn module(plan: &SdkPlan) -> String {
    let walkers = compiled(plan);
    let mut out = super::emit::header(plan);
    out.push_str("/// <summary>Thrown instead of looping when a source API repeats an identical pagination continuation value, or when the walk cannot compute an exact continuation.</summary>\npublic sealed class PaginationException : Exception\n{\n    /// <summary>The paginated operation's source identity.</summary>\n    public string Operation { get; }\n    /// <summary>Construct the typed pagination traversal failure.</summary>\n    public PaginationException(string operation, string reason, Exception? cause = null) : base(reason, cause) { Operation = operation; }\n}\n\n");
    out.push_str("/// <summary>One operation's compiled pagination behavior: pattern, the input members carrying each control, RFC 6901 pointers into the decoded page body, the configured initial offset and the advance rule.</summary>\npublic sealed record PaginationDescriptor(string Operation, string Pattern, IReadOnlyDictionary<string, string> Request, IReadOnlyDictionary<string, string> Response, int? InitialOffset, string Advance);\n\n");
    out.push_str("/// <summary>Frozen per-operation pagination descriptors compiled from the generation-time pagination policy. Pointers resolve with plain property access only; the walker never searches schemas or evaluates paths.</summary>\npublic static class PaginationDescriptors\n{\n    /// <summary>Compiled descriptors keyed by source operation identity.</summary>\n    public static readonly IReadOnlyDictionary<string, PaginationDescriptor> Operations = new global::System.Collections.ObjectModel.ReadOnlyDictionary<string, PaginationDescriptor>(new Dictionary<string, PaginationDescriptor>(StringComparer.Ordinal)\n    {\n");
    for walker in &walkers {
        out.push_str(&walker.descriptor);
    }
    out.push_str("    });\n}\n\n");
    out.push_str("/// <summary>Generated pagination pointer readers and continuation rules for this package's paginated operations. A page that satisfies the pattern's stop rule ends the walk; the first page of every walk is exactly the direct call's result.</summary>\ninternal static class Pagination\n{\n");
    for walker in &walkers {
        out.push_str(&walker.accessors);
        out.push_str(&walker.continuation_method);
    }
    out.push_str("}\n");
    out
}

/// The per-operation walker methods appended to the generated `Client`.
pub(super) fn client_methods(plan: &SdkPlan) -> String {
    let mut out = String::new();
    for walker in &compiled(plan) {
        out.push_str(&walker.pages_method);
        if let Some(items) = &walker.items_method {
            out.push_str(items);
        }
        out.push_str(&walker.next_page_method);
    }
    out
}
